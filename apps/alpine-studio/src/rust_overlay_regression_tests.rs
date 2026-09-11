//! Production-client regression for #576, not physical or full LSP qualification.

use std::{
    error::Error,
    fs, io,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use alpine_text::Buffer;
use serde_json::{Value, json};

use super::{LanguageIdentity, LanguageWake, RustDiagnostics, RustDocumentInput};
use crate::lsp_process::ProcessWake;

#[path = "rust_saved_close_tests.rs"]
mod saved_close;

const SAVED_A: &str = "pub fn value() -> u32 { 1 }\n";
const UNSAVED_A: &str = "pub fn value() -> &'static str { \"unsaved\" }\n";
const DOCUMENT_B: &str = "fn main() { let _ = crate::a::value(); }\n";

#[path = "rust_saved_wire_tests.rs"]
mod saved_notifications;

#[path = "rust_diagnostic_wake_tests.rs"]
mod diagnostic_wakes;

#[path = "rust_unsupported_diagnostic_tests.rs"]
mod unsupported_diagnostics;

static NEXT_FIXTURE: AtomicUsize = AtomicUsize::new(0);

fn reserve_fixture_root(
    parent: &Path,
    prefix: &str,
    sequence: &AtomicUsize,
) -> io::Result<PathBuf> {
    for _ in 0..16 {
        let suffix = sequence
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_add(1)
            })
            .map_err(|_| io::Error::other("fixture identity exhausted"))?;
        let root = parent.join(format!("{prefix}-{suffix}"));
        match fs::create_dir(&root) {
            Ok(()) => return Ok(root),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "fixture namespace is occupied",
    ))
}

const MOCK_SERVER: &str = r"
import json
import os
import sys

def read_message():
    headers = {}
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            return None
        if line in (b'\r\n', b'\n'):
            break
        name, _, value = line.partition(b':')
        headers[name.lower()] = value.strip()
    size = int(headers[b'content-length'])
    if not 0 < size <= 16777216:
        raise ValueError('fixture message limit')
    body = sys.stdin.buffer.read(size)
    if len(body) != size:
        raise ValueError('truncated fixture message')
    return json.loads(body)

def reply(identifier, result):
    body = json.dumps({'jsonrpc': '2.0', 'id': identifier, 'result': result}).encode('utf-8')
    sys.stdout.buffer.write(('Content-Length: %d\r\n\r\n' % len(body)).encode('ascii') + body)
    sys.stdout.buffer.flush()

while True:
    message = read_message()
    if message is None:
        break
    with open(LOG_PATH, 'a', encoding='utf-8') as log:
        log.write(json.dumps({'pid': os.getpid(), 'message': message}) + '\n')
        log.flush()
    method = message.get('method')
    if method == 'initialize':
        reply(message['id'], {'capabilities': {'positionEncoding': 'utf-16', 'textDocumentSync': {'openClose': True, 'change': 2}, 'diagnosticProvider': {'interFileDependencies': True, 'workspaceDiagnostics': False}}})
    elif method == 'textDocument/diagnostic':
        reply(message['id'], {'kind': 'full', 'items': []})
    elif method == 'shutdown':
        reply(message['id'], None)
    elif method == 'exit':
        break
";

struct Fixture {
    root: PathBuf,
    retained: bool,
}

impl Fixture {
    fn new() -> Result<Self, Box<dyn Error>> {
        let requested = std::env::var_os("ALPINE_OVERLAY_REGRESSION_EVIDENCE_DIR");
        let retained = requested.is_some();
        let root = if let Some(requested) = requested {
            let root = PathBuf::from(requested);
            // Explicit evidence directories must never be reused.
            fs::create_dir(&root)?;
            root
        } else {
            let prefix = format!(
                "alpine-overlay-576-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_or(0, |elapsed| elapsed.as_nanos())
            );
            reserve_fixture_root(&std::env::temp_dir(), &prefix, &NEXT_FIXTURE)?
        };
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
        Ok(Self {
            root: root.canonicalize()?,
            retained,
        })
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if !self.retained {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}

#[test]
fn parallel_fixture_roots_remain_unique_with_an_identical_clock_prefix()
-> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new()?;
    let sequence = AtomicUsize::new(0);
    let roots = std::thread::scope(|scope| {
        let workers = (0..8)
            .map(|_| scope.spawn(|| reserve_fixture_root(&fixture.root, "same-clock", &sequence)))
            .collect::<Vec<_>>();
        workers
            .into_iter()
            .map(|worker| {
                worker
                    .join()
                    .map_err(|_| io::Error::other("fixture worker panicked"))?
            })
            .collect::<io::Result<Vec<_>>>()
    })?;
    assert_eq!(roots.len(), 8);
    assert_eq!(
        roots
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        8
    );
    assert!(roots.iter().all(|root| root.is_dir()));
    Ok(())
}

#[test]
fn occupied_fixture_roots_are_preserved_and_allocation_exhaustion_is_bounded()
-> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new()?;
    let occupied = fixture.root.join("collision-0");
    fs::create_dir(&occupied)?;
    let marker = occupied.join("keep.txt");
    fs::write(&marker, "existing owner")?;
    let sequence = AtomicUsize::new(0);
    assert_eq!(
        reserve_fixture_root(&fixture.root, "collision", &sequence)?,
        fixture.root.join("collision-1")
    );
    assert_eq!(fs::read_to_string(&marker)?, "existing owner");
    for index in 0..16 {
        fs::create_dir(fixture.root.join(format!("full-{index}")))?;
    }
    let full = AtomicUsize::new(0);
    assert_eq!(
        reserve_fixture_root(&fixture.root, "full", &full)
            .err()
            .ok_or("collision accepted")?
            .kind(),
        io::ErrorKind::AlreadyExists
    );
    assert_eq!(full.load(Ordering::Relaxed), 16);
    let exhausted = AtomicUsize::new(usize::MAX);
    assert!(reserve_fixture_root(&fixture.root, "exhausted", &exhausted).is_err());
    assert_eq!(exhausted.load(Ordering::Relaxed), usize::MAX);
    Ok(())
}

#[test]
fn fixture_allocation_returns_non_collision_errors_without_retrying() -> Result<(), Box<dyn Error>>
{
    let fixture = Fixture::new()?;
    let parent = fixture.root.join("not-a-directory");
    fs::write(&parent, "existing file owner")?;
    let expected = fs::create_dir(parent.join("direct-control"))
        .err()
        .ok_or("non-directory parent accepted")?
        .kind();
    assert_ne!(expected, io::ErrorKind::AlreadyExists);
    let sequence = AtomicUsize::new(0);
    let actual = reserve_fixture_root(&parent, "allocation", &sequence)
        .err()
        .ok_or("fixture accepted a non-directory parent")?;
    assert_eq!(actual.kind(), expected);
    assert_eq!(sequence.load(Ordering::Relaxed), 1);
    assert_eq!(fs::read_to_string(&parent)?, "existing file owner");
    Ok(())
}

#[test]
fn fixture_drop_releases_only_unretained_owned_roots() -> Result<(), Box<dyn Error>> {
    let owner = Fixture::new()?;
    let temporary = owner.root.join("temporary");
    let retained = owner.root.join("retained");
    let sibling = owner.root.join("sibling.txt");
    fs::create_dir(&temporary)?;
    fs::create_dir(&retained)?;
    fs::write(temporary.join("owned.txt"), "temporary contents")?;
    fs::write(retained.join("evidence.txt"), "retained evidence")?;
    fs::write(&sibling, "unrelated sibling")?;

    drop(Fixture {
        root: temporary.clone(),
        retained: false,
    });
    assert!(
        !temporary.try_exists()?,
        "temporary fixture was not removed"
    );
    assert_eq!(fs::read_to_string(&sibling)?, "unrelated sibling");

    drop(Fixture {
        root: retained.clone(),
        retained: true,
    });
    assert_eq!(
        fs::read_to_string(retained.join("evidence.txt"))?,
        "retained evidence"
    );
    assert_eq!(fs::read_to_string(&sibling)?, "unrelated sibling");
    Ok(())
}

#[test]
fn wire_record_summary_keeps_only_twelve_newest_metadata_records() {
    assert!(wire_record_summary(&[]).is_empty());
    let records = (0..15)
        .map(|index| {
            json!({
                "pid": 2000 + index,
                "message": {
                    "method": "textDocument/didChange",
                    "id": 100 + index,
                    "params": {
                        "textDocument": {
                            "uri": format!("file:///workspace/document-{index}.rs"),
                            "version": index,
                            "text": "document contents must not enter the summary"
                        },
                        "contentChanges": [{"text": "private edit contents"}]
                    }
                },
                "unrelated": "not summary metadata"
            })
        })
        .collect::<Vec<_>>();
    let expected = (3..15)
        .rev()
        .map(|index| {
            json!({
                "pid": 2000 + index,
                "method": "textDocument/didChange",
                "id": 100 + index,
                "uri": format!("file:///workspace/document-{index}.rs"),
                "version": index
            })
        })
        .collect::<Vec<_>>();
    assert_eq!(wire_record_summary(&records), expected);
}

fn wake_factory(
    sender: SyncSender<LanguageWake>,
    starts: Arc<AtomicUsize>,
) -> impl FnOnce(LanguageWake) -> ProcessWake {
    move |wake| -> ProcessWake {
        starts.fetch_add(1, Ordering::SeqCst);
        Arc::new(move || {
            let _ = sender.try_send(wake);
        })
    }
}

fn input(root: &Path, file: &str, document_id: u64, text: &str) -> RustDocumentInput {
    let snapshot = Buffer::new(text).snapshot();
    RustDocumentInput::new(
        &root.join(file),
        root,
        LanguageIdentity {
            workspace_id: 1,
            workspace_revision: 1,
            document_id,
            document_revision: 1,
            buffer_revision: snapshot.revision().get(),
            selection_revision: 1,
        },
        snapshot,
    )
}

fn is_document_message(record: &Value, method: &str, suffix: &str) -> bool {
    record["message"]["method"] == method
        && record["message"]["params"]["textDocument"]["uri"]
            .as_str()
            .is_some_and(|uri| uri.ends_with(suffix))
}

// Continuations belong to the driver, not one successful wire predicate. A
// phase may finish while the production client still owns a scheduled poll.
// Generic tokens let the ownership control run independently of the server.
struct WireDriver<Wake = LanguageWake> {
    receiver: Receiver<Wake>,
    continuation: Option<Wake>,
}

impl<Wake> WireDriver<Wake> {
    fn new(receiver: Receiver<Wake>) -> Self {
        Self {
            receiver,
            continuation: None,
        }
    }

    fn next_wake(&mut self, timeout: Duration) -> Result<Wake, RecvTimeoutError> {
        if let Some(wake) = self.continuation.take() {
            Ok(wake)
        } else {
            self.receiver.recv_timeout(timeout)
        }
    }
}

#[test]
fn wire_driver_preserves_continuation_order_and_channel_errors() -> Result<(), Box<dyn Error>> {
    let (sender, receiver) = mpsc::sync_channel(2);
    let mut driver = WireDriver::new(receiver);
    sender.send(2_u8)?;
    driver.continuation = Some(1);
    assert_eq!(driver.next_wake(Duration::ZERO)?, 1);
    assert!(driver.continuation.is_none());
    assert_eq!(driver.next_wake(Duration::ZERO)?, 2);
    assert_eq!(
        driver.next_wake(Duration::ZERO),
        Err(RecvTimeoutError::Timeout)
    );
    drop(sender);
    // Already-owned work remains runnable even after delivery disconnects.
    driver.continuation = Some(3);
    assert_eq!(driver.next_wake(Duration::ZERO)?, 3);
    assert_eq!(
        driver.next_wake(Duration::ZERO),
        Err(RecvTimeoutError::Disconnected)
    );
    Ok(())
}

#[track_caller]
fn wait_for_open(
    diagnostics: &mut RustDiagnostics,
    driver: &mut WireDriver,
    log: &Path,
    suffix: &str,
) -> Result<Vec<Value>, Box<dyn Error>> {
    wait_for_wire(diagnostics, driver, log, |records| {
        records
            .iter()
            .any(|record| is_document_message(record, "textDocument/didOpen", suffix))
    })
}

#[track_caller]
fn wait_for_wire(
    diagnostics: &mut RustDiagnostics,
    driver: &mut WireDriver,
    log: &Path,
    admitted: impl Fn(&[Value]) -> bool,
) -> Result<Vec<Value>, Box<dyn Error>> {
    let caller = std::panic::Location::caller();
    let started = Instant::now();
    let deadline = started + Duration::from_secs(10);
    let mut processed_wakes = 0_u64;
    loop {
        let text = fs::read_to_string(log)?;
        let records = text
            .split_inclusive('\n')
            .filter(|line| line.ends_with('\n'))
            .map(serde_json::from_str)
            .collect::<Result<Vec<Value>, _>>()?;
        if admitted(&records) {
            eprintln!(
                "overlay wire admitted at {caller}: elapsed_ms={}, wakes={processed_wakes}, \
             records={}, continuation_pending={}, snapshot={:?}",
                started.elapsed().as_millis(),
                records.len(),
                driver.continuation.is_some(),
                diagnostics.snapshot(),
            );
            return Ok(records);
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!(
                    "missing expected wire transition at {caller}; elapsed_ms={}; \
                 wakes={processed_wakes}; continuation_pending={}; status={:?}; snapshot={:?}; \
                     records={}; newest_first_wire_tail={:?}",
                    started.elapsed().as_millis(),
                    driver.continuation.is_some(),
                    diagnostics.status,
                    diagnostics.snapshot(),
                    records.len(),
                    wire_record_summary(&records),
                ),
            )
            .into());
        }
        let wake = match driver.next_wake(remaining.min(Duration::from_millis(10))) {
            Ok(wake) => wake,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(error) => return Err(error.into()),
        };
        processed_wakes = processed_wakes.saturating_add(1);
        driver.continuation = diagnostics.poll(wake).continuation;
    }
}

fn wire_record_summary(records: &[Value]) -> Vec<Value> {
    records
        .iter()
        .rev()
        .take(12)
        .map(|record| {
            let message = &record["message"];
            let document = &message["params"]["textDocument"];
            json!({
                "pid":record["pid"], "method":message["method"], "id":message["id"],
                "uri":document["uri"], "version":document["version"]
            })
        })
        .collect()
}

#[test]
fn unsaved_overlay_survives_rust_and_non_rust_tab_switches() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new()?;
    let log = fixture.root.join("wire.jsonl");
    let server = fixture.root.join("mock-rust-analyzer");
    fs::write(&log, "")?;
    fs::write(fixture.root.join("a.rs"), SAVED_A)?;
    fs::write(fixture.root.join("b.rs"), DOCUMENT_B)?;
    let log_path = serde_json::to_string(
        log.to_str()
            .ok_or_else(|| io::Error::other("fixture path is not UTF-8"))?,
    )?;
    fs::write(
        &server,
        format!("#!/usr/bin/env python3\nLOG_PATH = {log_path}\n{MOCK_SERVER}"),
    )?;
    fs::set_permissions(&server, fs::Permissions::from_mode(0o700))?;

    let (sender, receiver) = mpsc::sync_channel(32);
    let mut driver = WireDriver::new(receiver);
    let starts = Arc::new(AtomicUsize::new(0));
    let mut diagnostics = RustDiagnostics {
        server_path: Some(server),
        ..RustDiagnostics::default()
    };
    let _ = diagnostics.sync(
        Some(input(&fixture.root, "a.rs", 1, UNSAVED_A)),
        wake_factory(sender.clone(), Arc::clone(&starts)),
    );
    let _ = wait_for_open(&mut diagnostics, &mut driver, &log, "/a.rs")?;
    let _ = diagnostics.sync(
        Some(input(&fixture.root, "b.rs", 2, DOCUMENT_B)),
        wake_factory(sender.clone(), Arc::clone(&starts)),
    );
    let records = wait_for_open(&mut diagnostics, &mut driver, &log, "/b.rs")?;
    let open_a = records
        .iter()
        .find(|record| is_document_message(record, "textDocument/didOpen", "/a.rs"))
        .ok_or_else(|| io::Error::other("missing A positive control"))?;
    let open_b = records
        .iter()
        .find(|record| is_document_message(record, "textDocument/didOpen", "/b.rs"))
        .ok_or_else(|| io::Error::other("missing B positive control"))?;
    let same_child = open_a["pid"].as_u64().is_some() && open_a["pid"] == open_b["pid"];
    let opened_unsaved_a = open_a["message"]["params"]["textDocument"]["text"] == UNSAVED_A;
    let closed_a = records
        .iter()
        .any(|record| is_document_message(record, "textDocument/didClose", "/a.rs"));

    // This is the current Studio adapter's no-active-Rust input. The corrected
    // adapter must carry workspace/overlay ownership separately from this view.
    let _ = diagnostics.sync(None, wake_factory(sender, Arc::clone(&starts)));
    let alive_without_rust_view = diagnostics.session.is_some();
    let _ = diagnostics.stop();
    let disk_unchanged = fs::read_to_string(fixture.root.join("a.rs"))? == SAVED_A;
    let outcome = json!({
        "baseline": "93df44bce2a1d957616cdcb0935e98d04f7e2041",
        "same_child_across_rust_switch": same_child,
        "wake_factories": starts.load(Ordering::SeqCst),
        "opened_unsaved_a": opened_unsaved_a,
        "closed_a_on_switch": closed_a,
        "session_alive_without_rust_view": alive_without_rust_view,
        "saved_a_unchanged": disk_unchanged,
        "physical_qualification": false,
        "full_workspace_acceptance": false
    });
    fs::write(
        fixture.root.join("outcome.json"),
        serde_json::to_vec_pretty(&outcome)?,
    )?;
    eprintln!("{outcome}");
    assert!(same_child && opened_unsaved_a && disk_unchanged);
    assert_eq!(starts.load(Ordering::SeqCst), 1);
    assert!(
        !closed_a && alive_without_rust_view,
        "#576: active-view switches must not revoke unsaved workspace overlays"
    );
    Ok(())
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one ordered production-client workspace journey"
)]
fn workspace_roster_versions_close_reopen_and_restart_reach_the_wire() -> Result<(), Box<dyn Error>>
{
    let fixture = Fixture::new()?;
    let log = fixture.root.join("wire.jsonl");
    let server = fixture.root.join("mock-rust-analyzer");
    fs::write(&log, "")?;
    fs::write(fixture.root.join("a.rs"), SAVED_A)?;
    fs::write(fixture.root.join("b.rs"), DOCUMENT_B)?;
    let log_path = serde_json::to_string(log.to_str().ok_or("fixture UTF-8 path")?)?;
    fs::write(
        &server,
        format!("#!/usr/bin/env python3\nLOG_PATH = {log_path}\n{MOCK_SERVER}"),
    )?;
    fs::set_permissions(&server, fs::Permissions::from_mode(0o700))?;
    let (sender, receiver) = mpsc::sync_channel(32);
    let mut driver = WireDriver::new(receiver);
    let starts = Arc::new(AtomicUsize::new(0));
    let mut diagnostics = RustDiagnostics {
        server_path: Some(server),
        ..RustDiagnostics::default()
    };
    let mut a = input(&fixture.root, "a.rs", 1, UNSAVED_A);
    let b = input(&fixture.root, "b.rs", 2, DOCUMENT_B);
    let _ = diagnostics.sync_workspace(
        [a.clone(), b.clone()],
        Some(1),
        wake_factory(sender.clone(), starts.clone()),
    );
    let initial = wait_for_open(&mut diagnostics, &mut driver, &log, "/b.rs")?;
    assert_eq!(
        initial
            .iter()
            .filter(|record| is_document_message(record, "textDocument/didOpen", "/a.rs"))
            .count(),
        1
    );
    let first_pid = initial
        .iter()
        .find(|record| is_document_message(record, "textDocument/didOpen", "/a.rs"))
        .ok_or("A opened")?["pid"]
        .as_u64()
        .ok_or("child PID")?;

    let _ = diagnostics.sync_workspace(
        [a.clone(), b.clone()],
        Some(2),
        wake_factory(sender.clone(), starts.clone()),
    );
    let _ = diagnostics.sync_workspace(
        [a.clone(), b.clone()],
        None,
        wake_factory(sender.clone(), starts.clone()),
    );
    assert!(diagnostics.session.is_some());
    assert!(
        !diagnostics
            .sync_workspace(
                [a.clone(), b.clone()],
                None,
                wake_factory(sender.clone(), starts.clone())
            )
            .visual_changed
    );
    let _ = diagnostics.sync_workspace(
        [a.clone(), b.clone()],
        Some(1),
        wake_factory(sender.clone(), starts.clone()),
    );
    let _ = diagnostics.sync_workspace(
        [a.clone(), b.clone()],
        Some(2),
        wake_factory(sender.clone(), starts.clone()),
    );

    let mut buffer = Buffer::new(UNSAVED_A);
    let mut transaction = alpine_text::Transaction::new(buffer.revision());
    transaction.replace(0..UNSAVED_A.len(), SAVED_A)?;
    let _ = buffer.apply(transaction)?;
    a.snapshot = buffer.snapshot();
    a.identity.buffer_revision = buffer.revision().get();
    let _ = diagnostics.sync_workspace(
        [a.clone(), b.clone()],
        Some(2),
        wake_factory(sender.clone(), starts.clone()),
    );
    let changed = wait_for_wire(&mut diagnostics, &mut driver, &log, |records| {
        records.iter().any(|record| {
            is_document_message(record, "textDocument/didChange", "/a.rs")
                && record["message"]["params"]["textDocument"]["version"] == 2
        })
    })?;
    assert!(!changed.iter().any(|record| is_document_message(
        record,
        "textDocument/didClose",
        "/a.rs"
    )));
    assert_eq!(
        changed
            .iter()
            .filter(|record| is_document_message(record, "textDocument/didOpen", "/a.rs"))
            .count(),
        1
    );
    assert_eq!(diagnostics.snapshot().process_starts, 1);

    // Close and reopen the same URI without an intervening poll. The roster
    // must serialize its close before publishing the replacement document.
    let _ = diagnostics.sync_workspace(
        [b.clone()],
        Some(2),
        wake_factory(sender.clone(), starts.clone()),
    );
    let reopened = input(&fixture.root, "a.rs", 3, UNSAVED_A);
    let _ = diagnostics.sync_workspace(
        [reopened, b],
        Some(3),
        wake_factory(sender.clone(), starts.clone()),
    );
    let reopened_records = wait_for_wire(&mut diagnostics, &mut driver, &log, |records| {
        records
            .iter()
            .filter(|record| is_document_message(record, "textDocument/didOpen", "/a.rs"))
            .count()
            == 2
    })?;
    let close_index = reopened_records
        .iter()
        .position(|record| is_document_message(record, "textDocument/didClose", "/a.rs"))
        .ok_or("A closed")?;
    let reopen_index = reopened_records
        .iter()
        .rposition(|record| is_document_message(record, "textDocument/didOpen", "/a.rs"))
        .ok_or("A reopened")?;
    assert!(close_index < reopen_index);
    assert_eq!(
        reopened_records[reopen_index]["message"]["params"]["textDocument"]["version"],
        1
    );
    assert_eq!(diagnostics.snapshot().overlay_documents, 2);
    assert!(diagnostics.restart_or_fail(super::RustDiagnosticsError::MissingServer));
    let restarted = wait_for_wire(&mut diagnostics, &mut driver, &log, |records| {
        records.iter().any(|record| {
            is_document_message(record, "textDocument/didOpen", "/b.rs")
                && record["pid"].as_u64().is_some_and(|pid| pid != first_pid)
        })
    })?;
    assert!(restarted.iter().any(|record| is_document_message(
        record,
        "textDocument/didOpen",
        "/a.rs"
    )
        && record["pid"].as_u64().is_some_and(|pid| pid != first_pid)));
    assert_eq!(diagnostics.snapshot().process_starts, 2);
    assert_eq!(diagnostics.snapshot().restarts, 1);
    assert_eq!(starts.load(Ordering::SeqCst), 1);
    let _ = diagnostics.sync_workspace(std::iter::empty(), None, wake_factory(sender, starts));
    assert!(!diagnostics.snapshot().active);
    assert_eq!(fs::read_to_string(fixture.root.join("a.rs"))?, SAVED_A);
    let outcome = json!({"roster_lifetime":true,"inactive_change_version":2,"close_before_reopen":true,"restart_replayed_both_documents":true,"server_starts":2,"physical_qualification":false,"full_workspace_acceptance":false});
    fs::write(
        fixture.root.join("outcome.json"),
        serde_json::to_vec_pretty(&outcome)?,
    )?;
    eprintln!("{outcome}");
    Ok(())
}
