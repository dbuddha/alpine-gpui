use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{self, Command},
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};

use serde_json::value::RawValue;

use super::*;

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);
#[cfg(not(miri))]
static MOCK_EXECUTABLE: OnceLock<PathBuf> = OnceLock::new();
#[cfg(miri)]
pub(crate) fn mock_executable() -> &'static Path {
    Path::new("/alpine-miri-inert-rust-analyzer")
}

#[cfg(not(miri))]
pub(crate) fn mock_executable() -> &'static Path {
    MOCK_EXECUTABLE
        .get_or_init(|| {
            let current = env::current_exe().unwrap_or_else(|_| unreachable!());
            let directory = current.parent().unwrap_or_else(|| unreachable!());
            let path = directory.join(format!(
                "alpine-rust-diagnostics-mock-{}{}",
                process::id(),
                env::consts::EXE_SUFFIX
            ));
            let source =
                Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/lsp_mock_server.rs");
            let rustc = env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
            let output = Command::new(rustc)
                .args(["--edition=2024", "-o"])
                .arg(&path)
                .arg(source)
                .output()
                .unwrap_or_else(|_| unreachable!());
            assert!(
                output.status.success(),
                "failed to compile Rust diagnostics mock: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            path
        })
        .as_path()
}

pub(crate) fn fixture() -> (PathBuf, PathBuf, BufferSnapshot, LanguageIdentity) {
    let root = env::temp_dir().join(format!(
        "alpine-rust-diagnostics-{}-{}",
        process::id(),
        NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap_or_else(|_| unreachable!());
    let root = fs::canonicalize(root).unwrap_or_else(|_| unreachable!());
    let path = root.join("main.rs");
    fs::write(&path, "fn main() {}\n").unwrap_or_else(|_| unreachable!());
    let snapshot = alpine_text::Buffer::new("fn main() {}\n").snapshot();
    let identity = LanguageIdentity {
        workspace_id: 1,
        workspace_revision: 1,
        document_id: 1,
        document_revision: 2,
        buffer_revision: snapshot.revision().get(),
        selection_revision: 3,
    };
    (root, path, snapshot, identity)
}

pub(crate) fn diagnostics(path: &Path, version: i32) -> Box<RawValue> {
    let document =
        LspDocument::from_file_path(path, "rust", version).unwrap_or_else(|_| unreachable!());
    let open = document
        .did_open_params("")
        .unwrap_or_else(|_| unreachable!());
    let uri_value: serde_json::Value =
        serde_json::from_str(open.get()).unwrap_or_else(|_| unreachable!());
    let uri = uri_value["textDocument"]["uri"]
        .as_str()
        .unwrap_or_else(|| unreachable!());
    serde_json::from_str(&format!(
        r#"{{"uri":"{uri}","version":{version},"diagnostics":[{{"range":{{"start":{{"line":0,"character":3}},"end":{{"line":0,"character":7}}}},"severity":1,"message":"broken"}},{{"range":{{"start":{{"line":0,"character":0}},"end":{{"line":2,"character":1}}}},"severity":2,"message":"span"}}]}}"#
    ))
    .unwrap_or_else(|_| unreachable!())
}

#[test]
fn marker_admission_is_identity_epoch_visible_and_bounded() {
    let (root, path, snapshot, identity) = fixture();
    let input = RustDocumentInput::new(&path, &root, identity, snapshot);
    let mut model = RustDiagnostics::default();
    model
        .install_for_test(input, &diagnostics(&path, 1), mock_executable())
        .unwrap_or_else(|_| unreachable!());
    let mut markers = Vec::new();
    assert_eq!(
        model.for_each_marker(identity, 0, 1, |marker| {
            markers.push(marker);
            Ok::<(), ()>(())
        }),
        Ok(1)
    );
    assert_eq!(markers[0].start_utf16, 3);
    assert_eq!(markers[0].end_utf16, Some(7));
    assert_eq!(markers[0].severity, Some(1));
    for field in 0..4 {
        let mut changed = identity;
        match field {
            0 => changed.workspace_revision += 1,
            1 => changed.document_revision += 1,
            2 => changed.buffer_revision += 1,
            _ => changed.selection_revision += 1,
        }
        assert_eq!(
            model.for_each_marker(changed, 0, 8, |_| Ok::<(), ()>(())),
            Ok(0)
        );
    }
    let mut second_line = Vec::new();
    assert_eq!(
        model.for_each_marker(identity, 1, 8, |marker| {
            second_line.push(marker);
            Ok::<(), ()>(())
        }),
        Ok(1)
    );
    assert_eq!(second_line[0].start_utf16, 0);
    assert_eq!(second_line[0].end_utf16, None);
    assert_eq!(model.status_message().as_deref(), Some("broken"));
    let snapshot = model.snapshot();
    assert!(snapshot.active);
    assert_eq!(snapshot.diagnostic_publications, 0);
    assert_eq!(snapshot.diagnostic_items, 2);
    assert!(snapshot.diagnostic_bytes > 0);
    let drained = model.shutdown();
    assert!(!drained.active);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn status_replacement_and_stale_wakes_are_quiet() {
    let mut model = RustDiagnostics::default();
    assert!(!replace_status(&mut model.status, None));
    assert!(replace_status(
        &mut model.status,
        Some(Arc::from("diagnostic"))
    ));
    assert!(!replace_status(
        &mut model.status,
        Some(Arc::from("diagnostic"))
    ));
    assert_eq!(
        model.poll(LanguageWake { generation: 1 }),
        LanguageEffect::default()
    );
    assert_eq!(model.snapshot().stale_wakes, 1);
    assert!(model.stop());
    assert!(!model.stop());
}

#[test]
fn document_end_preserves_lsp_line_and_utf16_boundaries() -> Result<(), Box<dyn Error>> {
    let cases = [
        ("", LspPosition::new(0, 0)?),
        ("a", LspPosition::new(0, 1)?),
        ("a\n", LspPosition::new(1, 0)?),
        ("a\r\n", LspPosition::new(1, 0)?),
        ("a\rb", LspPosition::new(1, 1)?),
        ("a\n🙂", LspPosition::new(1, 2)?),
    ];
    for (text, expected) in cases {
        let buffer = alpine_text::Buffer::new(text);
        assert_eq!(document_end(&buffer.snapshot())?, expected);
    }
    Ok(())
}

#[test]
fn wake_latch_preserves_the_latest_generation_until_foreground_admission() {
    let latch = LanguageWakeLatch::default();
    let first = LanguageWake { generation: 1 };
    let second = LanguageWake { generation: 2 };
    assert_eq!(first.generation(), 1);
    assert_eq!(second.generation(), 2);
    assert_eq!(latch.pending_generation(), 0);
    assert_eq!(latch.take(), None);
    latch.publish(first);
    latch.publish(second);
    assert_eq!(latch.pending_generation(), 2);
    latch.clear(first);
    assert_eq!(latch.pending_generation(), 2);
    assert_eq!(latch.take(), Some(second));
    assert_eq!(latch.pending_generation(), 0);
    assert_eq!(latch.take(), None);
    latch.publish(first);
    assert_eq!(latch.pending_generation(), 1);
    latch.clear(first);
    assert_eq!(latch.pending_generation(), 0);
    assert_eq!(latch.take(), None);
}

fn wait_for_product_diagnostics(
    model: &mut RustDiagnostics,
    latch: &LanguageWakeLatch,
    minimum_publications: u64,
    version: i32,
    allow_omitted_version: bool,
    expect_items: bool,
) -> Result<RustDiagnosticsSnapshot, Box<dyn std::error::Error>> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        for _ in 0..32 {
            let Some(wake) = latch.take() else {
                break;
            };
            let effect = model.poll(wake);
            if let Some(continuation) = effect.continuation {
                latch.publish(continuation);
            }
        }
        let snapshot = model.snapshot();
        let version_matches = snapshot.diagnostic_version == Some(version)
            || (allow_omitted_version && snapshot.diagnostic_version.is_none());
        if snapshot.diagnostic_publications >= minimum_publications
            && version_matches
            && (snapshot.diagnostic_items > 0) == expect_items
        {
            return Ok(snapshot);
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    Err(format!(
        "rust-analyzer did not publish the expected document state for version {version}: \
         snapshot={:?}, status={:?}",
        model.snapshot(),
        model.status_message()
    )
    .into())
}

#[test]
#[cfg_attr(miri, ignore = "Miri cannot emulate child-process creation")]
fn portable_mock_drives_open_change_clear_and_shutdown() -> Result<(), Box<dyn Error>> {
    let (root, path, _, mut identity) = fixture();
    let mut buffer = alpine_text::Buffer::new("fn broken( {\n");
    identity.buffer_revision = buffer.revision().get();
    let latch = LanguageWakeLatch::default();
    let mut model = RustDiagnostics::with_server(mock_executable());
    let input = RustDocumentInput::new(&path, &root, identity, buffer.snapshot());
    let wake_latch = latch.clone();
    assert!(
        model
            .sync(Some(input), move |wake| {
                let wake_latch = wake_latch.clone();
                Arc::new(move || wake_latch.publish(wake))
            })
            .visual_changed
    );
    let opened = wait_for_product_diagnostics(&mut model, &latch, 1, 1, false, true)?;
    assert_eq!(opened.diagnostic_version, Some(1));
    assert_eq!(model.status_message().as_deref(), Some("Rust: mock broken"));

    let mut transaction = alpine_text::Transaction::new(buffer.revision());
    transaction.replace(0..buffer.snapshot().len_bytes(), "fn still_broken( {\n")?;
    buffer.apply(transaction)?;
    identity.document_revision += 1;
    identity.buffer_revision = buffer.revision().get();
    identity.selection_revision += 1;
    let changed = RustDocumentInput::new(&path, &root, identity, buffer.snapshot());
    let effect = model.sync(Some(changed), |_| Arc::new(|| {}));
    assert!(effect.visual_changed);
    let changed = wait_for_product_diagnostics(
        &mut model,
        &latch,
        opened.diagnostic_publications + 1,
        2,
        false,
        true,
    )?;
    assert_eq!(changed.diagnostic_version, Some(2));

    let mut transaction = alpine_text::Transaction::new(buffer.revision());
    transaction.replace(0..buffer.snapshot().len_bytes(), "let ok = 1;\n")?;
    buffer.apply(transaction)?;
    identity.document_revision += 1;
    identity.buffer_revision = buffer.revision().get();
    let valid = RustDocumentInput::new(&path, &root, identity, buffer.snapshot());
    assert!(model.sync(Some(valid), |_| Arc::new(|| {})).visual_changed);
    let cleared = wait_for_product_diagnostics(
        &mut model,
        &latch,
        changed.diagnostic_publications + 1,
        3,
        false,
        false,
    )?;
    assert_eq!(cleared.diagnostic_items, 0);
    assert!(model.status_message().is_none());
    assert!(model.sync(None, |_| Arc::new(|| {})).visual_changed);
    assert!(!model.snapshot().active);
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "Miri cannot emulate child-process creation")]
fn portable_mock_protocol_failure_restarts_the_active_document() -> Result<(), Box<dyn Error>> {
    let (root, path, _, mut identity) = fixture();
    let mut buffer = alpine_text::Buffer::new("fn broken( {\n");
    identity.buffer_revision = buffer.revision().get();
    let latch = LanguageWakeLatch::default();
    let mut model = RustDiagnostics::with_server(mock_executable());
    let input = RustDocumentInput::new(&path, &root, identity, buffer.snapshot());
    let wake_latch = latch.clone();
    assert!(
        model
            .sync(Some(input), move |wake| {
                let wake_latch = wake_latch.clone();
                Arc::new(move || wake_latch.publish(wake))
            })
            .visual_changed
    );
    let _ = wait_for_product_diagnostics(&mut model, &latch, 1, 1, false, true)?;

    let mut transaction = alpine_text::Transaction::new(buffer.revision());
    transaction.replace(0..buffer.snapshot().len_bytes(), "ALPINE_PROTOCOL_ERROR\n")?;
    buffer.apply(transaction)?;
    identity.document_revision += 1;
    identity.buffer_revision = buffer.revision().get();
    let changed = RustDocumentInput::new(&path, &root, identity, buffer.snapshot());
    assert!(
        model
            .sync(Some(changed), |_| Arc::new(|| {}))
            .visual_changed
    );

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while model.snapshot().restarts == 0 {
        if let Some(wake) = latch.take() {
            let _ = model.poll(wake);
        } else {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        if std::time::Instant::now() >= deadline {
            return Err("mock protocol failure did not restart rust-analyzer".into());
        }
    }
    assert_eq!(model.snapshot().restarts, 1);
    assert!(!model.shutdown().active);
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "Miri cannot emulate child-process creation")]
fn portable_mock_completion_is_bounded_revision_safe_and_undoable() -> Result<(), Box<dyn Error>> {
    let (root, path, _, mut identity) = fixture();
    let mut buffer = alpine_text::Buffer::new("fn broken( {\n");
    identity.buffer_revision = buffer.revision().get();
    let latch = LanguageWakeLatch::default();
    let mut model = RustDiagnostics::with_server(mock_executable());
    let input = RustDocumentInput::new(&path, &root, identity, buffer.snapshot());
    let wake_latch = latch.clone();
    assert!(
        model
            .sync(Some(input), move |wake| {
                let wake_latch = wake_latch.clone();
                Arc::new(move || wake_latch.publish(wake))
            })
            .visual_changed
    );
    let _ = wait_for_product_diagnostics(&mut model, &latch, 1, 1, false, true)?;
    let position = LspPosition::new(0, 2)?;
    let _ = model.request_completion(position);
    assert!(
        model.snapshot().completion_pending(),
        "completion request was rejected: status={:?}, snapshot={:?}",
        model.status_message(),
        model.snapshot()
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while model.snapshot().completion_items == 0 {
        if let Some(wake) = latch.take() {
            let effect = model.poll(wake);
            if let Some(continuation) = effect.continuation {
                latch.publish(continuation);
            }
        } else {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        if std::time::Instant::now() >= deadline {
            return Err(format!("completion timed out: {:?}", model.snapshot()).into());
        }
    }
    let snapshot = model.snapshot();
    assert_eq!(snapshot.completion_items, 2);
    assert!(snapshot.completion_bytes > 0);
    assert_eq!(snapshot.completion_requests, 1);
    assert_eq!(model.completion_visible_range(identity), Some(0..2));
    assert_eq!(
        model
            .completion_row(identity, 0)
            .map(|row| (row.label, row.selected)),
        Some(("println!", true))
    );
    let application = model
        .take_selected_completion(identity, &buffer.snapshot(), 2..2)?
        .ok_or("completion selection missing")?;
    assert_eq!(application.range, 0..2);
    assert_eq!(application.text.as_ref(), "println!");
    let before = buffer.snapshot().text();
    let mut transaction = alpine_text::Transaction::new(buffer.revision());
    transaction.replace(application.range, application.text.as_ref())?;
    transaction.set_selections(alpine_text::SelectionSet::caret(
        alpine_text::ByteOffset::new(application.text.len()),
    ));
    buffer.apply(transaction)?;
    assert!(buffer.snapshot().text().starts_with("println!"));
    assert!(buffer.undo()?);
    assert_eq!(buffer.snapshot().text(), before);
    assert_eq!(model.snapshot().completion_items, 0);
    assert!(!model.shutdown().active);
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn late_cancelled_response_cannot_clear_a_newer_completion() -> Result<(), Box<dyn Error>> {
    let (root, path, snapshot, identity) = fixture();
    let input = RustDocumentInput::new(&path, &root, identity, snapshot);
    let params = diagnostics(&path, 1);
    let completion =
        RawValue::from_string(r#"[{"label":"newer","insertText":"newer"}]"#.to_owned())?;
    let mut model = RustDiagnostics::default();
    model.install_for_test(input, &params, mock_executable())?;
    model.install_completion_for_test(2, identity, &completion)?;

    assert!(!model.reject_stale_completion(1));
    assert_eq!(model.snapshot().completion_items, 1);
    assert_eq!(model.snapshot().stale_completions, 1);
    assert!(model.reject_stale_completion(2));
    assert_eq!(model.snapshot().completion_items, 0);
    assert_eq!(model.snapshot().stale_completions, 2);

    assert!(!model.shutdown().active);
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "Miri cannot emulate child-process creation")]
fn mock_completion_supersession_rejects_the_late_response_without_restart()
-> Result<(), Box<dyn Error>> {
    let (root, path, snapshot, identity) = fixture();
    let latch = LanguageWakeLatch::default();
    let mut model = RustDiagnostics::with_server(mock_executable());
    let input = RustDocumentInput::new(&path, &root, identity, snapshot);
    let wake_latch = latch.clone();
    assert!(
        model
            .sync(Some(input), move |wake| {
                let wake_latch = wake_latch.clone();
                Arc::new(move || wake_latch.publish(wake))
            })
            .visual_changed
    );
    let _ = wait_for_product_diagnostics(&mut model, &latch, 1, 1, false, true)?;

    let _ = model.request_completion(LspPosition::new(0, 99)?);
    assert!(model.snapshot().completion_pending());
    let _ = model.request_completion(LspPosition::new(0, 2)?);
    assert_eq!(model.snapshot().completion_cancellations, 1);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while model.snapshot().completion_items == 0 {
        if let Some(wake) = latch.take() {
            let effect = model.poll(wake);
            if let Some(continuation) = effect.continuation {
                latch.publish(continuation);
            }
        } else {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        if std::time::Instant::now() >= deadline {
            return Err(
                format!("completion supersession timed out: {:?}", model.snapshot()).into(),
            );
        }
    }
    let actual = model.snapshot();
    assert_eq!(actual.completion_items, 2);
    assert!(!actual.completion_pending());
    assert_eq!(actual.stale_completions, 1);
    assert_eq!(actual.restarts, 0);
    assert!(!model.shutdown().active);
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "Miri cannot emulate child-process creation")]
fn mock_navigation_is_bounded_current_only_and_supersedes_interactive_work()
-> Result<(), Box<dyn Error>> {
    let (root, path, snapshot, identity) = fixture();
    let latch = LanguageWakeLatch::default();
    let mut model = RustDiagnostics::with_server(mock_executable());
    let input = RustDocumentInput::new(&path, &root, identity, snapshot);
    let wake_latch = latch.clone();
    assert!(
        model
            .sync(Some(input), move |wake| {
                let wake_latch = wake_latch.clone();
                Arc::new(move || wake_latch.publish(wake))
            })
            .visual_changed
    );
    let _ = wait_for_product_diagnostics(&mut model, &latch, 1, 1, false, true)?;

    model.status = Some(Arc::from("clear before navigation"));
    assert!(
        model
            .request_navigation(NavigationRequestKind::Hover, LspPosition::new(0, 2)?)
            .visual_changed
    );
    assert!(model.status_message().is_none());
    assert!(model.snapshot().navigation_pending());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while model.snapshot().hover_bytes == 0 {
        if let Some(wake) = latch.take() {
            let effect = model.poll(wake);
            if let Some(continuation) = effect.continuation {
                latch.publish(continuation);
            }
        } else {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        if std::time::Instant::now() >= deadline {
            return Err(format!("hover timed out: {:?}", model.snapshot()).into());
        }
    }
    assert_eq!(
        model.hover_content(identity).map(HoverContent::text),
        Some("`fn main()`\n\nMock hover")
    );
    assert!(model.snapshot().hover_bytes <= crate::rust_navigation::MAX_HOVER_RETAINED_BYTES);

    assert!(
        model
            .request_navigation(NavigationRequestKind::References, LspPosition::new(0, 2)?)
            .visual_changed
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while model.snapshot().location_items == 0 {
        if let Some(wake) = latch.take() {
            let effect = model.poll(wake);
            if let Some(continuation) = effect.continuation {
                latch.publish(continuation);
            }
        } else {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        if std::time::Instant::now() >= deadline {
            return Err(format!("references timed out: {:?}", model.snapshot()).into());
        }
    }
    let references = model.snapshot();
    assert_eq!(references.location_items, 2);
    assert!(references.location_bytes <= crate::rust_navigation::MAX_LOCATION_RETAINED_BYTES);
    assert_eq!(references.navigation_requests, 2);
    assert!(model.hover_content(identity).is_none());

    let _ = model.request_navigation(NavigationRequestKind::Definition, LspPosition::new(0, 99)?);
    assert!(model.snapshot().navigation_pending());
    let _ = model.request_navigation(NavigationRequestKind::Hover, LspPosition::new(0, 2)?);
    assert_eq!(model.snapshot().navigation_cancellations, 1);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while model.snapshot().hover_bytes == 0 || model.snapshot().stale_navigation == 0 {
        if let Some(wake) = latch.take() {
            let effect = model.poll(wake);
            if let Some(continuation) = effect.continuation {
                latch.publish(continuation);
            }
        } else {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        if std::time::Instant::now() >= deadline {
            return Err(
                format!("navigation supersession timed out: {:?}", model.snapshot()).into(),
            );
        }
    }
    let final_snapshot = model.snapshot();
    assert_eq!(final_snapshot.navigation_requests, 4);
    assert_eq!(final_snapshot.stale_navigation, 1);
    assert_eq!(final_snapshot.restarts, 0);
    assert!(!model.shutdown().active);
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "Miri cannot emulate child-process creation")]
fn mock_symbols_are_bounded_query_safe_and_release_on_shutdown() -> Result<(), Box<dyn Error>> {
    let (root, path, snapshot, identity) = fixture();
    let latch = LanguageWakeLatch::default();
    let mut model = RustDiagnostics::with_server(mock_executable());
    let input = RustDocumentInput::new(&path, &root, identity, snapshot);
    let wake_latch = latch.clone();
    assert!(
        model
            .sync(Some(input), move |wake| {
                let wake_latch = wake_latch.clone();
                Arc::new(move || wake_latch.publish(wake))
            })
            .visual_changed
    );
    let _ = wait_for_product_diagnostics(&mut model, &latch, 1, 1, false, true)?;

    assert!(
        model
            .open_symbols(SymbolRequestKind::Document)
            .visual_changed
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while model.snapshot().symbol_items == 0 {
        if let Some(wake) = latch.take() {
            let effect = model.poll(wake);
            if let Some(continuation) = effect.continuation {
                latch.publish(continuation);
            }
        } else {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        if std::time::Instant::now() >= deadline {
            return Err(format!("document symbols timed out: {:?}", model.snapshot()).into());
        }
    }
    assert_eq!(model.snapshot().symbol_items, 2);
    assert_eq!(model.snapshot().symbol_matches, 2);
    assert_eq!(
        model.symbol_row(identity, 0).map(|row| row.label),
        Some("main  fn()")
    );
    assert!(model.commit_symbol_text(identity, "inner").visual_changed);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while model.snapshot().symbol_matches != 1 {
        if let Some(wake) = latch.take() {
            let effect = model.poll(wake);
            if let Some(continuation) = effect.continuation {
                latch.publish(continuation);
            }
        } else {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        if std::time::Instant::now() >= deadline {
            return Err(format!("filtered symbols timed out: {:?}", model.snapshot()).into());
        }
    }
    assert_eq!(
        model.symbol_row(identity, 0).map(|row| row.label),
        Some("  inner")
    );

    assert!(
        model
            .open_symbols(SymbolRequestKind::Workspace)
            .visual_changed
    );
    assert!(model.commit_symbol_text(identity, "main").visual_changed);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while model.snapshot().symbol_matches != 1 {
        if let Some(wake) = latch.take() {
            let effect = model.poll(wake);
            if let Some(continuation) = effect.continuation {
                latch.publish(continuation);
            }
        } else {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        if std::time::Instant::now() >= deadline {
            return Err(format!("workspace symbols timed out: {:?}", model.snapshot()).into());
        }
    }
    let symbols = model.snapshot();
    assert!(symbols.symbol_items <= crate::rust_symbols::MAX_SYMBOL_ITEMS);
    assert!(symbols.symbol_bytes <= crate::rust_symbols::MAX_SYMBOL_RETAINED_BYTES);
    assert!(symbols.symbol_requests >= 4);
    assert!(symbols.symbol_cancellations >= 1);
    assert!(model.symbol_visible_range(identity).is_some());

    let drained = model.shutdown();
    assert!(!drained.active);
    assert_eq!(drained.symbol_items, 0);
    assert_eq!(drained.symbol_bytes, 0);
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "Miri cannot emulate child-process creation")]
fn mock_workspace_edits_are_revision_bound_and_prepared_off_the_poll_path()
-> Result<(), Box<dyn Error>> {
    let (root, path, snapshot, identity) = fixture();
    let latch = LanguageWakeLatch::default();
    let mut model = RustDiagnostics::with_server(mock_executable());
    let input = RustDocumentInput::new(&path, &root, identity, snapshot);
    let wake_latch = latch.clone();
    assert!(
        model
            .sync(Some(input), move |wake| {
                let wake_latch = wake_latch.clone();
                Arc::new(move || wake_latch.publish(wake))
            })
            .visual_changed
    );
    let _ = wait_for_product_diagnostics(&mut model, &latch, 1, 1, false, true)?;

    let _ = model.request_formatting(4, true);
    assert!(model.snapshot().workspace_edit_pending);
    let formatting = wait_for_workspace_edit(&mut model, &latch)?;
    assert_eq!(formatting.identity().kind(), WorkspaceEditKind::Formatting);
    let formatting = formatting.execute();
    assert!(formatting.wire_bytes > 0);
    let prepared = formatting.result?;
    assert_eq!(prepared.file_count(), 1);
    assert_eq!(prepared.edit_count(), 1);
    assert_eq!(prepared.files()[0].replacement(), "pub fn main() {}\n");

    let _ = model.request_rename(LspPosition::new(0, 4)?, "renamed");
    let rename = wait_for_workspace_edit(&mut model, &latch)?;
    assert_eq!(rename.identity().kind(), WorkspaceEditKind::Rename);
    let rename = rename.execute().result?;
    assert_eq!(rename.file_count(), 1);
    assert_eq!(rename.edit_count(), 1);
    assert_eq!(rename.files()[0].replacement(), "fn renamed() {}\n");

    let snapshot = model.snapshot();
    assert_eq!(snapshot.workspace_edit_requests, 2);
    assert!(!snapshot.workspace_edit_pending);
    assert!(!snapshot.workspace_edit_preparing);
    assert!(snapshot.peak_workspace_edit_wire_bytes > 0);
    assert!(!model.shutdown().active);
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "Miri cannot emulate child-process creation")]
fn superseded_workspace_edit_cannot_publish_over_the_newer_request() -> Result<(), Box<dyn Error>> {
    let (root, path, snapshot, identity) = fixture();
    let latch = LanguageWakeLatch::default();
    let mut model = RustDiagnostics::with_server(mock_executable());
    let input = RustDocumentInput::new(&path, &root, identity, snapshot);
    let wake_latch = latch.clone();
    assert!(
        model
            .sync(Some(input), move |wake| {
                let wake_latch = wake_latch.clone();
                Arc::new(move || wake_latch.publish(wake))
            })
            .visual_changed
    );
    let _ = wait_for_product_diagnostics(&mut model, &latch, 1, 1, false, true)?;
    let _ = model.request_rename(LspPosition::new(0, 4)?, "never_respond");
    assert!(model.snapshot().workspace_edit_pending);
    let _ = model.request_formatting(4, true);
    assert_eq!(model.snapshot().workspace_edit_cancellations, 1);
    let request = wait_for_workspace_edit(&mut model, &latch)?;
    assert_eq!(request.identity().kind(), WorkspaceEditKind::Formatting);
    assert!(request.execute().result.is_ok());
    assert!(model.snapshot().stale_workspace_edits >= 1);
    assert!(!model.shutdown().active);
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn workspace_edit_identity_and_reduction_axes_are_independently_discriminating() {
    let language = LanguageIdentity {
        workspace_id: 1,
        workspace_revision: 2,
        document_id: 3,
        document_revision: 4,
        buffer_revision: 5,
        selection_revision: 6,
    };
    let mut snapshot = RustDiagnosticsSnapshot {
        active: true,
        process_epoch: 7,
        lsp_version: 8,
        ..RustDiagnosticsSnapshot::default()
    };
    let identity = WorkspaceEditIdentity::for_test(
        language,
        snapshot.process_epoch,
        snapshot.lsp_version,
        9,
        WorkspaceEditKind::Rename,
    );
    assert!(identity.matches(language, &snapshot));
    assert!(identity.matches_document(language));
    assert_eq!(identity.kind().label(), "Rust rename");
    for axis in 0..6 {
        let mut changed = language;
        match axis {
            0 => changed.workspace_id += 1,
            1 => changed.workspace_revision += 1,
            2 => changed.document_id += 1,
            3 => changed.document_revision += 1,
            4 => changed.buffer_revision += 1,
            _ => changed.selection_revision += 1,
        }
        assert!(!identity.matches(changed, &snapshot), "match axis {axis}");
        if axis < 5 {
            assert!(!identity.matches_document(changed), "document axis {axis}");
        }
    }
    snapshot.process_epoch += 1;
    assert!(!identity.matches(language, &snapshot));
    snapshot.process_epoch -= 1;
    snapshot.lsp_version += 1;
    assert!(!identity.matches(language, &snapshot));
    snapshot.lsp_version -= 1;
    snapshot.active = false;
    assert!(!identity.matches(language, &snapshot));
    for true_index in 0..5 {
        let mut changes = [false; 5];
        changes[true_index] = true;
        assert!(crate::any_workspace_edit_change(changes));
    }
    assert!(!crate::any_workspace_edit_change([false; 5]));
    assert!(crate::rust_workspace_command_blocked(true, true));
    assert!(crate::rust_workspace_command_blocked(false, false));
    assert!(!crate::rust_workspace_command_blocked(false, true));
    assert!((crate::workspace_edit_line_baseline(10.0, 2, 7.0) - 64.0).abs() < f32::EPSILON);
    assert!((crate::workspace_edit_text_x(11.0) - 19.0).abs() < f32::EPSILON);
}

#[test]
fn workspace_edit_not_ready_and_admission_axes_are_observable() -> Result<(), Box<dyn Error>> {
    let mut unavailable = RustDiagnostics::default();
    assert!(unavailable.request_formatting(4, true).visual_changed);
    assert!(!unavailable.request_formatting(4, true).visual_changed);
    let (root, path, snapshot, language) = fixture();
    let mut model = RustDiagnostics::default();
    model.install_for_test(
        RustDocumentInput::new(&path, &root, language, snapshot),
        &diagnostics(&path, 1),
        mock_executable(),
    )?;
    let session = model.session.as_ref().ok_or("session")?;
    let current_identity = session.identity;
    let current_epoch = session.process_epoch;
    let current_version = session.lsp_version;
    let template = PendingWorkspaceEdit {
        request_id: 41,
        stamp: current_identity.request_stamp().ok_or("request stamp")?,
        kind: WorkspaceEditKind::Formatting,
        identity: current_identity,
        process_epoch: current_epoch,
        lsp_version: current_version,
    };
    model
        .session
        .as_mut()
        .ok_or("session")?
        .pending_workspace_edit = Some(template);
    assert!(model.cancel_workspace_edit());
    assert!(!model.snapshot().workspace_edit_pending);
    let wrong_stamp = LanguageIdentity {
        workspace_id: current_identity.workspace_id + 1,
        ..current_identity
    }
    .request_stamp()
    .ok_or("wrong stamp")?;
    for axis in 0..6 {
        let mut pending = template;
        let mut id = template.request_id;
        let mut stamp = template.stamp;
        let mut kind = template.kind;
        match axis {
            0 => id += 1,
            1 => stamp = wrong_stamp,
            2 => kind = WorkspaceEditKind::Rename,
            3 => pending.identity.workspace_id += 1,
            4 => pending.process_epoch += 1,
            _ => pending.lsp_version += 1,
        }
        let session = model.session.as_mut().ok_or("session")?;
        session.identity = current_identity;
        session.process_epoch = current_epoch;
        session.lsp_version = current_version;
        session.pending_workspace_edit = Some(pending);
        let stale_before = model.stale_workspace_edits;
        assert!(!model.admit_workspace_edit(id, stamp, kind, Err(WorkspaceEditError::Malformed),));
        assert_eq!(model.stale_workspace_edits, stale_before + 1, "axis {axis}");
    }
    let session = model.session.as_mut().ok_or("session")?;
    session.identity = current_identity;
    session.process_epoch = current_epoch;
    session.lsp_version = current_version;
    session.pending_workspace_edit = Some(template);
    assert!(model.admit_workspace_edit(
        template.request_id,
        template.stamp,
        template.kind,
        Err(WorkspaceEditError::Malformed),
    ));
    assert!(model.take_workspace_edit_preparation().is_none());
    assert!(!model.shutdown().active);
    fs::remove_dir_all(root)?;
    Ok(())
}

fn wait_for_workspace_edit(
    model: &mut RustDiagnostics,
    latch: &LanguageWakeLatch,
) -> Result<WorkspaceEditPreparationRequest, Box<dyn Error>> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if let Some(request) = model.take_workspace_edit_preparation() {
            return Ok(request);
        }
        if let Some(wake) = latch.take() {
            let effect = model.poll(wake);
            if let Some(continuation) = effect.continuation {
                latch.publish(continuation);
            }
        } else {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        if std::time::Instant::now() >= deadline {
            return Err(format!("workspace edit timed out: {:?}", model.snapshot()).into());
        }
    }
}

#[test]
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[ignore = "requires the checksum-verified Task #208 rust-analyzer binary"]
#[allow(
    clippy::too_many_lines,
    reason = "the pinned product journey keeps its ordered lifecycle and evidence assertions together"
)]
fn pinned_rust_analyzer_drives_product_open_edit_and_diagnostic_admission()
-> Result<(), Box<dyn std::error::Error>> {
    let workspace = fs::canonicalize(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rust-analyzer-workspace"),
    )?;
    let path = fs::canonicalize(workspace.join("src/lib.rs"))?;
    let mut buffer = alpine_text::Buffer::new(&fs::read_to_string(&path)?);
    let mut identity = LanguageIdentity {
        workspace_id: 1,
        workspace_revision: 1,
        document_id: 1,
        document_revision: 1,
        buffer_revision: buffer.revision().get(),
        selection_revision: 1,
    };
    let latch = LanguageWakeLatch::default();
    let mut model = RustDiagnostics::default();
    let input = RustDocumentInput::new(&path, &workspace, identity, buffer.snapshot());
    let wake_latch = latch.clone();
    let started = model.sync(Some(input), move |wake| {
        let wake_latch = wake_latch.clone();
        Arc::new(move || wake_latch.publish(wake))
    });
    assert!(started.visual_changed);
    let opened = wait_for_product_diagnostics(&mut model, &latch, 1, 1, false, true)?;
    assert!(opened.active);
    assert!(opened.diagnostic_items > 0);
    assert!(opened.diagnostic_bytes > 0);
    assert!(opened.peak_diagnostic_items >= opened.diagnostic_items);
    assert!(opened.peak_diagnostic_bytes >= opened.diagnostic_bytes);
    assert!(model.status_message().is_some());

    let initial = buffer.snapshot().text();
    let navigation_offset = initial
        .find("deliberately_invalid")
        .and_then(|offset| offset.checked_add(2))
        .ok_or("pinned navigation context is missing")?;
    let navigation_position = crate::rust_completion::position_for_byte(
        &buffer.snapshot(),
        alpine_text::ByteOffset::new(navigation_offset),
    )?;
    let _ = model.request_navigation(NavigationRequestKind::Hover, navigation_position);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while model.snapshot().hover_bytes == 0 {
        if let Some(wake) = latch.take() {
            let effect = model.poll(wake);
            if let Some(continuation) = effect.continuation {
                latch.publish(continuation);
            }
        } else {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        if std::time::Instant::now() >= deadline {
            return Err(format!("pinned hover timed out: {:?}", model.snapshot()).into());
        }
    }
    assert!(model.hover_content(identity).is_some());

    for kind in [
        NavigationRequestKind::Definition,
        NavigationRequestKind::References,
    ] {
        let _ = model.request_navigation(kind, navigation_position);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while model.snapshot().location_items == 0 {
            if let Some(wake) = latch.take() {
                let effect = model.poll(wake);
                if let Some(continuation) = effect.continuation {
                    latch.publish(continuation);
                }
            } else {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            if std::time::Instant::now() >= deadline {
                return Err(format!("pinned {kind:?} timed out: {:?}", model.snapshot()).into());
            }
        }
        let navigation = model.snapshot();
        assert!(navigation.location_items <= crate::rust_navigation::MAX_SOURCE_LOCATIONS);
        assert!(navigation.location_bytes <= crate::rust_navigation::MAX_LOCATION_RETAINED_BYTES);
        assert!(model.navigation_visible_range(identity).is_some());
    }
    assert_eq!(model.snapshot().navigation_requests, 3);

    let replacement = "pub fn deliberately_invalid() -> u32 {\n";
    let mut transaction = alpine_text::Transaction::new(buffer.revision());
    transaction.replace(0..buffer.snapshot().len_bytes(), replacement)?;
    buffer.apply(transaction)?;
    identity.document_revision += 1;
    identity.buffer_revision = buffer.revision().get();
    let changed = RustDocumentInput::new(&path, &workspace, identity, buffer.snapshot());
    let wake_latch = latch.clone();
    let effect = model.sync(Some(changed), move |wake| {
        let wake_latch = wake_latch.clone();
        Arc::new(move || wake_latch.publish(wake))
    });
    assert!(effect.visual_changed);
    assert_eq!(model.snapshot().diagnostic_version, None);
    let corrected = wait_for_product_diagnostics(
        &mut model,
        &latch,
        opened.diagnostic_publications + 1,
        2,
        true,
        true,
    )?;
    assert!(corrected.diagnostic_publications > opened.diagnostic_publications);
    assert!(corrected.diagnostic_items > 0);
    assert_eq!(corrected.lsp_version, 2);
    assert!(model.status_message().is_some());

    let replacement =
        "pub fn deliberately_invalid() -> String {\n    let value = String::new();\n    val\n}\n";
    let mut transaction = alpine_text::Transaction::new(buffer.revision());
    transaction.replace(0..buffer.snapshot().len_bytes(), replacement)?;
    buffer.apply(transaction)?;
    identity.document_revision += 1;
    identity.buffer_revision = buffer.revision().get();
    let completion_input = RustDocumentInput::new(&path, &workspace, identity, buffer.snapshot());
    let wake_latch = latch.clone();
    let effect = model.sync(Some(completion_input), move |wake| {
        let wake_latch = wake_latch.clone();
        Arc::new(move || wake_latch.publish(wake))
    });
    assert!(effect.visual_changed);
    assert_eq!(model.snapshot().diagnostic_version, None);
    assert_eq!(model.snapshot().diagnostic_items, 0);
    assert_eq!(model.snapshot().lsp_version, 3);
    assert!(model.status_message().is_none());
    let ready = wait_for_product_diagnostics(
        &mut model,
        &latch,
        corrected.diagnostic_publications + 1,
        3,
        false,
        true,
    )?;
    assert_eq!(ready.diagnostic_version, Some(3));
    assert!(ready.diagnostic_items > 0);

    let completion_offset = replacement
        .rfind("val\n")
        .and_then(|offset| offset.checked_add("val".len()))
        .ok_or("pinned completion context is missing")?;
    let completion_position = crate::rust_completion::position_for_byte(
        &buffer.snapshot(),
        alpine_text::ByteOffset::new(completion_offset),
    )?;
    let _ = model.request_completion(completion_position);
    assert!(model.snapshot().completion_pending());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while model.snapshot().completion_items == 0 {
        if let Some(wake) = latch.take() {
            let effect = model.poll(wake);
            if let Some(continuation) = effect.continuation {
                latch.publish(continuation);
            }
        } else {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let snapshot = model.snapshot();
        if snapshot.completion_items == 0
            && snapshot.completion_requests > 0
            && !snapshot.completion_pending()
        {
            return Err(format!(
                "pinned completion returned no items: snapshot={snapshot:?}, raw={:?}",
                super::LAST_COMPLETION_RESPONSE.with(std::cell::RefCell::take)
            )
            .into());
        }
        if std::time::Instant::now() >= deadline {
            return Err(format!("pinned completion timed out: {:?}", model.snapshot()).into());
        }
    }
    let completion = model.snapshot();
    assert!(completion.completion_items <= crate::rust_completion::MAX_COMPLETION_ITEMS);
    assert!(completion.completion_bytes > 0);
    assert!(completion.completion_bytes <= crate::rust_completion::MAX_COMPLETION_RETAINED_BYTES);
    assert_eq!(completion.completion_requests, 1);
    assert!(model.completion_visible_range(identity).is_some());

    assert!(
        model
            .open_symbols(SymbolRequestKind::Document)
            .visual_changed
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while model.snapshot().symbol_items == 0 {
        if let Some(wake) = latch.take() {
            let effect = model.poll(wake);
            if let Some(continuation) = effect.continuation {
                latch.publish(continuation);
            }
        } else {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let snapshot = model.snapshot();
        if snapshot.symbol_items == 0 && snapshot.symbol_requests > 0 && !snapshot.symbol_pending()
        {
            return Err(format!("pinned document symbols returned no items: {snapshot:?}").into());
        }
        if std::time::Instant::now() >= deadline {
            return Err(format!("pinned document symbols timed out: {snapshot:?}").into());
        }
    }
    let document_symbols = model.snapshot();
    assert!(document_symbols.symbol_items <= crate::rust_symbols::MAX_SYMBOL_ITEMS);
    assert!(document_symbols.symbol_bytes <= crate::rust_symbols::MAX_SYMBOL_RETAINED_BYTES);

    assert!(
        model
            .open_symbols(SymbolRequestKind::Workspace)
            .visual_changed
    );
    assert!(
        model
            .commit_symbol_text(identity, "deliberately_invalid")
            .visual_changed
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while model.snapshot().symbol_matches == 0 {
        if let Some(wake) = latch.take() {
            let effect = model.poll(wake);
            if let Some(continuation) = effect.continuation {
                latch.publish(continuation);
            }
        } else {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let snapshot = model.snapshot();
        if snapshot.symbol_matches == 0
            && snapshot.symbol_requests > document_symbols.symbol_requests
            && !snapshot.symbol_pending()
        {
            return Err(
                format!("pinned workspace symbols returned no matches: {snapshot:?}").into(),
            );
        }
        if std::time::Instant::now() >= deadline {
            return Err(format!("pinned workspace symbols timed out: {snapshot:?}").into());
        }
    }
    assert!(model.selected_symbol_location(identity).is_some());

    let drained = model.shutdown();
    assert!(!drained.active);
    assert_eq!(drained.diagnostic_items, 0);
    assert_eq!(drained.diagnostic_bytes, 0);
    Ok(())
}
