//! The open-document roster: what a server holds open across tab switches,
//! closes, restarts and full rosters, read back from the mock's stdin.

use std::{
    error::Error,
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use serde_json::Value;

use super::LanguageServices;
use crate::{
    language_registry::LanguageRegistry,
    lsp_language::{LspPosition, file_uri},
    lsp_process::ProcessWake,
    rust_diagnostics::{
        LanguageIdentity, LanguageWake, LanguageWakeLatch, NavigationRequestKind, RustDiagnostics,
        RustDocumentInput,
        tests::{fixture, mock_executable},
    },
};

// Bounds a hang only; no assertion depends on elapsed time.
const WAIT: Duration = Duration::from_secs(20);
const SAVED_A: &str = "fn saved_a() {}\n";
const UNSAVED_A: &str = "fn unsaved_a() {}\nfn added_without_saving() {}\n";

type TestResult = Result<(), Box<dyn Error>>;

static NEXT_DOCUMENT_REVISION: AtomicU64 = AtomicU64::new(1);

/// A scratch workspace whose Rust server is the mock, recording its stdin.
pub(crate) struct Recording {
    pub(crate) root: PathBuf,
    record: PathBuf,
}

impl Recording {
    pub(crate) fn new() -> Self {
        let (root, _, _, _) = fixture();
        let record = root.join("stdin.record");
        Self { root, record }
    }

    /// Services that start the mock for `.rs` files and nothing else. The
    /// server runs in the workspace root, so the record path is relative.
    pub(crate) fn services(&self) -> Result<LanguageServices, Box<dyn Error>> {
        let record = self
            .record
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .ok_or("record file name")?;
        let registry = LanguageRegistry::parse(&format!(
            "[[language]]\nid = \"rust\"\nextensions = [\"rs\"]\nserver_id = \"rust\"\n\
             binaries = [\"rust-analyzer\"]\narguments = [\"--record-stdin\", \"{record}\"]\n"
        ))?;
        Ok(LanguageServices {
            registry,
            fallback: RustDiagnostics::with_server(mock_executable()),
            ..LanguageServices::default()
        })
    }

    /// Every frame the current server process has read, in order.
    pub(crate) fn wire(&self) -> Wire {
        let bytes = fs::read(&self.record).unwrap_or_default();
        let text = String::from_utf8_lossy(&bytes);
        let mut frames = Vec::new();
        let mut rest = text.as_ref();
        while let Some((_, frame)) = rest.split_once("Content-Length: ") {
            let Some((length, body)) = frame.split_once("\r\n\r\n") else {
                break;
            };
            let Ok(length) = length.parse::<usize>() else {
                break;
            };
            let Some(content) = body.get(..length) else {
                break;
            };
            frames.push(serde_json::from_str(content).unwrap_or(Value::Null));
            rest = &body[length..];
        }
        Wire { frames }
    }
}

impl Drop for Recording {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

pub(crate) struct Wire {
    frames: Vec<Value>,
}

impl Wire {
    /// Frames of one method for one document, each with its wire position.
    pub(crate) fn of(&self, method: &str, path: &Path) -> Vec<(usize, &Value)> {
        let uri = file_uri(path).unwrap_or_default();
        self.frames
            .iter()
            .enumerate()
            .filter(|(_, frame)| {
                frame["method"] == method && frame["params"]["textDocument"]["uri"] == uri.as_str()
            })
            .collect()
    }

    pub(crate) fn count(&self, method: &str) -> usize {
        self.frames
            .iter()
            .filter(|frame| frame["method"] == method)
            .count()
    }

    /// The text the server last received for a document.
    pub(crate) fn last_text(&self, path: &Path) -> Option<String> {
        let mut text = None;
        for (_, frame) in self.of_any(&["textDocument/didOpen", "textDocument/didChange"], path) {
            let params = &frame["params"];
            let value = if frame["method"] == "textDocument/didOpen" {
                &params["textDocument"]["text"]
            } else {
                &params["contentChanges"][0]["text"]
            };
            text = value.as_str().map(str::to_owned);
        }
        text
    }

    fn of_any(&self, methods: &[&str], path: &Path) -> Vec<(usize, &Value)> {
        let mut frames: Vec<_> = methods
            .iter()
            .flat_map(|method| self.of(method, path))
            .collect();
        frames.sort_by_key(|(index, _)| *index);
        frames
    }
}

pub(crate) fn version(frame: &Value) -> i64 {
    frame["params"]["textDocument"]["version"]
        .as_i64()
        .unwrap_or(-1)
}

/// One tab: a file on disk and the buffer the editor holds for it.
pub(crate) struct Tab {
    pub(crate) path: PathBuf,
    buffer: alpine_text::Buffer,
    document_id: u64,
}

impl Tab {
    pub(crate) fn new(
        root: &Path,
        name: &str,
        text: &str,
        document_id: u64,
    ) -> Result<Self, Box<dyn Error>> {
        let path = root.join(name);
        fs::write(&path, text)?;
        Ok(Self {
            path,
            buffer: alpine_text::Buffer::new(text),
            document_id,
        })
    }

    /// Replaces the buffer's text without saving it.
    pub(crate) fn edit(&mut self, text: &str) -> TestResult {
        let mut transaction = alpine_text::Transaction::new(self.buffer.revision());
        transaction.replace(0..self.buffer.snapshot().len_bytes(), text)?;
        self.buffer.apply(transaction)?;
        Ok(())
    }

    pub(crate) fn input(&self, root: &Path) -> RustDocumentInput {
        let identity = LanguageIdentity {
            workspace_id: 1,
            workspace_revision: 1,
            document_id: self.document_id,
            document_revision: NEXT_DOCUMENT_REVISION.fetch_add(1, Ordering::Relaxed),
            buffer_revision: self.buffer.revision().get(),
            selection_revision: 1,
        };
        RustDocumentInput::new(&self.path, root, identity, self.buffer.snapshot())
    }
}

pub(crate) fn waker(latch: &LanguageWakeLatch) -> impl FnOnce(LanguageWake) -> ProcessWake {
    let latch = latch.clone();
    move |wake| Arc::new(move || latch.publish(wake))
}

/// Polls on every wake until `done` holds.
pub(crate) fn drive_until(
    services: &mut LanguageServices,
    latch: &LanguageWakeLatch,
    what: &str,
    mut done: impl FnMut(&LanguageServices) -> bool,
) -> TestResult {
    let deadline = Instant::now() + WAIT;
    while !done(services) {
        if let Some(wake) = latch.take() {
            if let Some(continuation) = services.poll(wake).continuation {
                latch.publish(continuation);
            }
        } else {
            std::thread::sleep(Duration::from_millis(1));
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "{what} timed out: {:?}, status {:?}",
                services.snapshot(),
                services.status_message()
            )
            .into());
        }
    }
    Ok(())
}

/// Shows a tab and waits for the server's next publication for it. The
/// count is read after the sync, from the model that then shows the tab.
fn show_and_publish(
    services: &mut LanguageServices,
    latch: &LanguageWakeLatch,
    root: &Path,
    tab: &Tab,
) -> TestResult {
    let _ = services.sync(Some(tab.input(root)), waker(latch));
    let before = services.snapshot().diagnostic_publications;
    drive_until(services, latch, "publication", |services| {
        services.snapshot().diagnostic_publications > before
    })
}

/// Waits until the server has read everything sent so far: it answers in
/// order, and a stop may drop input still queued.
pub(crate) fn barrier(services: &mut LanguageServices, latch: &LanguageWakeLatch) -> TestResult {
    let _ = services.request_navigation(NavigationRequestKind::Hover, LspPosition::new(0, 0)?);
    drive_until(services, latch, "barrier", |services| {
        services.snapshot().hover_bytes > 0
    })
}

fn show(services: &mut LanguageServices, latch: &LanguageWakeLatch, root: &Path, tab: &Tab) {
    let _ = services.sync(Some(tab.input(root)), waker(latch));
}

fn open_tabs(services: &mut LanguageServices, paths: &[&Path]) {
    services.replace_open_paths(paths.iter().copied());
}

#[test]
#[cfg_attr(miri, ignore = "Miri cannot emulate child-process creation")]
fn unsaved_text_stays_open_across_rust_and_plain_text_switches() -> TestResult {
    let recording = Recording::new();
    let root = recording.root.clone();
    let mut services = recording.services()?;
    let latch = LanguageWakeLatch::default();
    let mut a = Tab::new(&root, "a.rs", SAVED_A, 1)?;
    let b = Tab::new(&root, "b.rs", "fn b( {\n", 2)?;
    let notes = root.join("notes.txt");
    fs::write(&notes, "plain text\n")?;

    open_tabs(&mut services, &[&a.path]);
    show_and_publish(&mut services, &latch, &root, &a)?;
    a.edit(UNSAVED_A)?;
    show_and_publish(&mut services, &latch, &root, &a)?;
    let edited = services.snapshot();
    open_tabs(&mut services, &[&a.path, &b.path]);
    show_and_publish(&mut services, &latch, &root, &b)?;
    open_tabs(&mut services, &[&a.path, &b.path, &notes]);
    let _ = services.sync(None, waker(&latch));
    show(&mut services, &latch, &root, &a);
    let returned = services.snapshot();
    barrier(&mut services, &latch)?;
    assert!(!services.shutdown().active);

    let wire = recording.wire();
    assert!(
        wire.of("textDocument/didClose", &a.path).is_empty(),
        "a tab switch closed the unsaved document"
    );
    assert_eq!(wire.last_text(&a.path).as_deref(), Some(UNSAVED_A));
    assert_eq!(
        wire.of("textDocument/didOpen", &a.path).len(),
        1,
        "switching back reopened a document the server still held"
    );
    assert_eq!(returned.process_starts, 1);
    // The edited version's diagnostics come back with the tab.
    assert_eq!(returned.diagnostic_version, edited.diagnostic_version);
    assert_eq!(returned.lsp_version, edited.lsp_version);
    assert!(returned.diagnostic_items > 0);
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "Miri cannot emulate child-process creation")]
fn a_seventeenth_document_closes_only_the_least_recently_shown() -> TestResult {
    let recording = Recording::new();
    let root = recording.root.clone();
    let mut services = recording.services()?;
    let latch = LanguageWakeLatch::default();
    let mut tabs = Vec::new();
    for index in 0..17_u64 {
        tabs.push(Tab::new(
            &root,
            &format!("d{index}.rs"),
            "fn broken( {\n",
            index + 1,
        )?);
    }
    let paths: Vec<&Path> = tabs.iter().map(|tab| tab.path.as_path()).collect();
    open_tabs(&mut services, &paths);
    for tab in &tabs[..16] {
        show_and_publish(&mut services, &latch, &root, tab)?;
    }
    // Showing d0 again leaves d1 the least recently shown.
    show(&mut services, &latch, &root, &tabs[0]);
    show_and_publish(&mut services, &latch, &root, &tabs[16])?;
    assert!(!services.shutdown().active);

    let wire = recording.wire();
    assert_eq!(wire.count("textDocument/didOpen"), 17);
    for tab in &tabs {
        assert_eq!(wire.of("textDocument/didOpen", &tab.path).len(), 1);
    }
    assert_eq!(wire.count("textDocument/didClose"), 1);
    let closed = wire.of("textDocument/didClose", &tabs[1].path);
    let opened = wire.of("textDocument/didOpen", &tabs[16].path);
    assert_eq!(closed.len(), 1, "the least recently shown was not closed");
    assert!(closed[0].0 < opened[0].0);
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "Miri cannot emulate child-process creation")]
fn closing_a_tab_closes_its_document_and_only_then() -> TestResult {
    let recording = Recording::new();
    let root = recording.root.clone();
    let mut services = recording.services()?;
    let latch = LanguageWakeLatch::default();
    let mut a = Tab::new(&root, "a.rs", "fn a( {\n", 1)?;
    let b = Tab::new(&root, "b.rs", "fn b( {\n", 2)?;
    let notes = root.join("notes.txt");
    fs::write(&notes, "plain text\n")?;

    open_tabs(&mut services, &[&a.path, &b.path]);
    show_and_publish(&mut services, &latch, &root, &a)?;
    show_and_publish(&mut services, &latch, &root, &b)?;
    show(&mut services, &latch, &root, &a);
    a.edit("fn a( {\n// before close\n")?;
    show_and_publish(&mut services, &latch, &root, &a)?;
    // b's tab closes while a is shown.
    open_tabs(&mut services, &[&a.path]);
    show(&mut services, &latch, &root, &a);
    // a's tab closes and plain text is shown.
    open_tabs(&mut services, &[&notes]);
    let _ = services.sync(None, waker(&latch));
    // b's file opens in a new tab.
    open_tabs(&mut services, &[&notes, &b.path]);
    show_and_publish(&mut services, &latch, &root, &b)?;
    assert!(!services.shutdown().active);

    let wire = recording.wire();
    let marker = wire
        .of("textDocument/didChange", &a.path)
        .last()
        .map(|(index, _)| *index)
        .ok_or("missing the edit before the close")?;
    let closed_b = wire.of("textDocument/didClose", &b.path);
    assert_eq!(closed_b.len(), 1);
    assert!(closed_b[0].0 > marker, "b closed before its tab did");
    let closed_a = wire.of("textDocument/didClose", &a.path);
    assert_eq!(closed_a.len(), 1);
    assert!(closed_a[0].0 > closed_b[0].0);
    let opened_b = wire.of("textDocument/didOpen", &b.path);
    assert_eq!(opened_b.len(), 2, "a closed document was still open");
    assert!(opened_b[1].0 > closed_b[0].0);
    assert!(version(opened_b[1].1) > version(opened_b[0].1));
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "Miri cannot emulate child-process creation")]
fn a_restart_reopens_every_open_document_with_its_unsaved_text() -> TestResult {
    let recording = Recording::new();
    let root = recording.root.clone();
    let mut services = recording.services()?;
    let latch = LanguageWakeLatch::default();
    let mut a = Tab::new(&root, "a.rs", SAVED_A, 1)?;
    let mut b = Tab::new(&root, "b.rs", "fn b( {\n", 2)?;
    let c = Tab::new(&root, "c.rs", "fn c( {\n", 3)?;

    open_tabs(&mut services, &[&a.path, &b.path, &c.path]);
    show_and_publish(&mut services, &latch, &root, &a)?;
    a.edit(UNSAVED_A)?;
    show_and_publish(&mut services, &latch, &root, &a)?;
    show_and_publish(&mut services, &latch, &root, &c)?;
    show_and_publish(&mut services, &latch, &root, &b)?;
    let before = recording.wire();
    let published = services.snapshot().diagnostic_publications;

    // The mock exits on this change; the restarted one rewrites the record.
    b.edit("fn b() { ALPINE_CRASH }\n")?;
    let crashed = b.input(&root);
    let _ = services.sync(Some(crashed.clone()), waker(&latch));
    drive_until(&mut services, &latch, "restart", |services| {
        services.snapshot().restarts == 1
    })?;
    // c's tab closes while the server restarts.
    open_tabs(&mut services, &[&a.path, &b.path]);
    let _ = services.sync(Some(crashed), waker(&latch));
    drive_until(&mut services, &latch, "reopened", |services| {
        services.snapshot().diagnostic_publications > published
    })?;
    barrier(&mut services, &latch)?;
    assert!(!services.shutdown().active);

    let after = recording.wire();
    assert_eq!(after.count("initialize"), 1);
    let reopened_a = after.of("textDocument/didOpen", &a.path);
    assert_eq!(
        reopened_a.len(),
        1,
        "the unsaved background document was lost"
    );
    assert_eq!(after.last_text(&a.path).as_deref(), Some(UNSAVED_A));
    assert_eq!(
        after.last_text(&b.path).as_deref(),
        Some("fn b() { ALPINE_CRASH }\n")
    );
    assert!(after.of("textDocument/didOpen", &c.path).is_empty());
    for tab in [&a, &b] {
        let newest = before
            .of_any(
                &["textDocument/didOpen", "textDocument/didChange"],
                &tab.path,
            )
            .iter()
            .map(|(_, frame)| version(frame))
            .max()
            .unwrap_or(0);
        let reopened = after.of("textDocument/didOpen", &tab.path);
        assert!(reopened.iter().all(|(_, frame)| version(frame) > newest));
    }
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "Miri cannot emulate child-process creation")]
fn a_stale_publication_for_an_open_document_is_dropped() -> TestResult {
    let recording = Recording::new();
    let root = recording.root.clone();
    let mut services = recording.services()?;
    let latch = LanguageWakeLatch::default();
    let mut a = Tab::new(&root, "a.rs", "fn a( {\n", 1)?;
    let b = Tab::new(&root, "b.rs", "fn b( {\n", 2)?;

    open_tabs(&mut services, &[&a.path, &b.path]);
    show_and_publish(&mut services, &latch, &root, &a)?;
    a.edit("fn a_edited( {\n")?;
    show_and_publish(&mut services, &latch, &root, &a)?;
    let edited = services.snapshot();
    show_and_publish(&mut services, &latch, &root, &b)?;
    let shown = services.snapshot();

    // The mock answers this hover after a version-1 publication for every
    // document it holds open.
    let _ = services.request_navigation(NavigationRequestKind::Hover, LspPosition::new(0, 77)?);
    drive_until(&mut services, &latch, "hover", |services| {
        services.snapshot().hover_bytes > 0
    })?;
    let replayed = services.snapshot();
    assert_eq!(
        replayed.stale_diagnostics,
        shown.stale_diagnostics + 2,
        "a stale publication was admitted"
    );
    assert_ne!(services.status_message().as_deref(), Some("mock stale"));
    assert_eq!(replayed.diagnostic_version, shown.diagnostic_version);

    show(&mut services, &latch, &root, &a);
    let returned = services.snapshot();
    assert_eq!(returned.diagnostic_version, edited.diagnostic_version);
    assert!(returned.diagnostic_version > Some(1));
    assert!(!services.shutdown().active);
    Ok(())
}
