use std::{fs, sync::Arc};

use serde_json::value::RawValue;

use super::*;

fn fixture() -> (PathBuf, PathBuf, BufferSnapshot, LanguageIdentity) {
    let root = std::env::temp_dir().join(format!("alpine-rust-diagnostics-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap_or_else(|_| unreachable!());
    let root = fs::canonicalize(root).unwrap_or_else(|_| unreachable!());
    let path = root.join("main.rs");
    fs::write(&path, "fn main() {}\n").unwrap_or_else(|_| unreachable!());
    let snapshot = alpine_text::Buffer::new("fn main() {}\n").snapshot();
    let identity = LanguageIdentity {
        workspace_revision: 1,
        document_revision: 2,
        buffer_revision: snapshot.revision().get(),
        selection_revision: 3,
    };
    (root, path, snapshot, identity)
}

fn diagnostics(path: &Path, version: i32) -> Box<RawValue> {
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
        .install_for_test(input, &diagnostics(&path, 1))
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
    assert_eq!(latch.take(), None);
    latch.publish(first);
    latch.publish(second);
    latch.clear(first);
    assert_eq!(latch.take(), Some(second));
    assert_eq!(latch.take(), None);
    latch.publish(first);
    latch.clear(first);
    assert_eq!(latch.take(), None);
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn wait_for_product_diagnostics(
    model: &mut RustDiagnostics,
    latch: &LanguageWakeLatch,
    minimum_publications: u64,
    version: i32,
    allow_omitted_version: bool,
    expect_items: bool,
) -> Result<RustDiagnosticsSnapshot, Box<dyn std::error::Error>> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
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
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[ignore = "requires the checksum-verified Task #208 rust-analyzer binary"]
fn pinned_rust_analyzer_drives_product_open_edit_and_diagnostic_admission()
-> Result<(), Box<dyn std::error::Error>> {
    let workspace = fs::canonicalize(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rust-analyzer-workspace"),
    )?;
    let path = fs::canonicalize(workspace.join("src/lib.rs"))?;
    let mut buffer = alpine_text::Buffer::new(&fs::read_to_string(&path)?);
    let mut identity = LanguageIdentity {
        workspace_revision: 1,
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

    let replacement = "pub fn deliberately_invalid() -> u32 { 7 }\n";
    let mut transaction = alpine_text::Transaction::new(buffer.revision());
    transaction.replace(0..buffer.snapshot().len_bytes(), replacement)?;
    buffer.apply(transaction)?;
    identity.document_revision += 1;
    identity.buffer_revision = buffer.revision().get();
    let corrected = RustDocumentInput::new(&path, &workspace, identity, buffer.snapshot());
    let wake_latch = latch.clone();
    let effect = model.sync(Some(corrected), move |wake| {
        let wake_latch = wake_latch.clone();
        Arc::new(move || wake_latch.publish(wake))
    });
    assert!(effect.visual_changed);
    assert_eq!(model.snapshot().diagnostic_version, None);
    assert_eq!(model.snapshot().diagnostic_items, 0);
    assert_eq!(model.snapshot().lsp_version, 3);
    assert!(model.status_message().is_none());

    let drained = model.shutdown();
    assert!(!drained.active);
    assert_eq!(drained.diagnostic_items, 0);
    assert_eq!(drained.diagnostic_bytes, 0);
    Ok(())
}
