//! Deterministic native-versus-saved status correspondence through editing.

use super::*;
use crate::rust_diagnostics::{LanguageEffect, LanguageWake};
use crate::{lsp_language::DiagnosticBatch, rust_diagnostics::saved_compiler::SavedCompilerReport};

fn saved_params(uri: &str, clear: bool) -> serde_json::Value {
    let diagnostics = if clear {
        serde_json::json!([])
    } else {
        serde_json::json!([{
            "source": "rustc", "code": "E0502", "severity": 1,
            "range": {"start": {"line": 0, "character": 0},
                      "end": {"line": 0, "character": 1}},
            "message": "saved compiler diagnostic"
        }])
    };
    serde_json::json!({
        "uri": uri, "version": 1, "diagnostics": diagnostics
    })
}

fn saved_report(uri: &str, clear: bool) -> Result<SavedCompilerReport, Box<dyn Error>> {
    let wire = serde_json::value::to_raw_value(&saved_params(uri, clear))?;
    SavedCompilerReport::parse(&wire)?.ok_or_else(|| "missing saved report".into())
}

fn poll_saved_frames(
    model: &mut RustDiagnostics,
    reports: &[serde_json::Value],
) -> Result<LanguageEffect, Box<dyn Error>> {
    let mut wire = Vec::new();
    for params in reports {
        let body = serde_json::json!({
            "jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": params
        })
        .to_string();
        wire.extend_from_slice(format!("Content-Length: {}\r\n\r\n{body}", body.len()).as_bytes());
    }
    let session = model.session.as_mut().ok_or("workspace")?;
    let wake = LanguageWake {
        generation: session.generation,
    };
    session.client.inject_stdout_for_test(&wire)?;
    Ok(model.poll(wake))
}

fn publish_saved(
    model: &mut RustDiagnostics,
    report: SavedCompilerReport,
) -> Result<bool, Box<dyn Error>> {
    let session = model.session.as_mut().ok_or("workspace")?;
    Ok(route_saved_compiler(
        &session.document,
        session.active_view,
        &mut session.saved_compiler,
        &mut session.parked,
        report,
    )?)
}

#[test]
fn native_invalidation_preserves_saved_status_without_accepting_stale_native_reports()
-> Result<(), Box<dyn Error>> {
    let (mut model, mut input, root) = installed_workspace()?;
    model.initialize_installed_transport_for_test()?;
    let session = model.session.as_ref().ok_or("workspace")?;
    let uri = session.document.uri().to_owned();
    let native = session
        .diagnostics
        .as_ref()
        .ok_or("native report")?
        .batch
        .clone();
    let _ = model.admit(Ok(native));
    assert!(model.status.is_some());
    assert_eq!(model.snapshot().diagnostic_version, Some(1));
    assert!(model.snapshot().diagnostic_items > 0);
    let saved = saved_report(&uri, false)?;
    let saved_status = saved.status().ok_or("saved status")?;
    let saved_bytes = saved.retained_bytes();
    assert!(publish_saved(&mut model, saved)?);
    let mut buffer = alpine_text::Buffer::new(&input.snapshot.text());
    let mut transaction = alpine_text::Transaction::new(buffer.revision());
    transaction.replace(0..0, "// unsaved edit\n")?;
    let _ = buffer.apply(transaction)?;
    input.snapshot = buffer.snapshot();
    input.identity.buffer_revision = buffer.revision().get();
    input.identity.document_revision += 1;
    let changed = model.sync_workspace([input.clone()], Some(input.identity.document_id), |_| {
        Arc::new(|| {})
    });
    assert!(changed.visual_changed);
    assert_eq!(model.snapshot().lsp_version, 2);
    assert_eq!(model.snapshot().diagnostic_version, None);
    assert_eq!(model.snapshot().diagnostic_items, 0);
    assert!(model.status.is_none());
    assert_eq!(model.snapshot().saved_compiler_items, 1);
    assert_eq!(model.snapshot().saved_compiler_bytes, saved_bytes);
    assert_eq!(
        model.status_message().as_deref(),
        Some(saved_status.as_ref())
    );
    assert!(saved_status.contains("Saved compiler (not current buffer)"));
    // This is a deterministic counterexample to treating combined status as
    // native-only status. The saved report is not invalidated by this edit.
    let session = model.session.as_ref().ok_or("workspace")?;
    assert!(
        DiagnosticBatch::admit(
            &diagnostic_tests::diagnostics(&input.path, 1),
            &session.document,
        )
        .is_err()
    );
    let current = DiagnosticBatch::admit(
        &diagnostic_tests::diagnostics(&input.path, 2),
        &session.document,
    )?;
    let _ = model.admit(Ok(current));
    let native_items = model.snapshot().diagnostic_items;
    assert!(native_items > 0);
    assert!(model.status.is_some());
    assert!(publish_saved(&mut model, saved_report(&uri, true)?)?);
    assert_eq!(model.snapshot().saved_compiler_items, 0);
    assert_eq!(model.snapshot().saved_compiler_bytes, 0);
    assert_eq!(model.snapshot().diagnostic_items, native_items);
    assert_eq!(model.snapshot().diagnostic_version, Some(2));
    assert_eq!(model.status_message(), model.status);
    assert!(!model.shutdown().active);
    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn saved_poll_batch_keeps_one_redraw_and_separate_native_authority() -> Result<(), Box<dyn Error>> {
    let (mut model, _, root) = installed_workspace()?;
    model.initialize_installed_transport_for_test()?;
    let uri = model
        .session
        .as_ref()
        .ok_or("workspace")?
        .document
        .uri()
        .to_owned();
    let native = model.snapshot();
    let report = saved_params(&uri, false);
    // Both frames enter one real framing/poll batch. A duplicate must not
    // overwrite the first frame's true invalidation with false.
    assert!(poll_saved_frames(&mut model, &[report.clone(), report.clone()])?.visual_changed);
    let accepted = model.snapshot();
    assert_eq!(accepted.saved_compiler_items, 1);
    assert!(accepted.saved_compiler_bytes > 0);
    assert_eq!(accepted.diagnostic_items, native.diagnostic_items);
    assert_eq!(accepted.diagnostic_version, native.diagnostic_version);
    assert!(
        model
            .status_message()
            .ok_or("saved status")?
            .contains("not current buffer")
    );
    assert!(!poll_saved_frames(&mut model, &[report])?.visual_changed);
    assert_eq!(
        model.snapshot().saved_compiler_bytes,
        accepted.saved_compiler_bytes
    );
    assert!(!model.shutdown().active);
    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn saved_poll_malformed_report_redraws_once_without_replacing_accepted_data()
-> Result<(), Box<dyn Error>> {
    let (mut model, _, root) = installed_workspace()?;
    model.initialize_installed_transport_for_test()?;
    let uri = model
        .session
        .as_ref()
        .ok_or("workspace")?
        .document
        .uri()
        .to_owned();
    assert!(poll_saved_frames(&mut model, &[saved_params(&uri, false)])?.visual_changed);
    let accepted = model.snapshot();
    let mut malformed = saved_params(&uri, false);
    malformed["diagnostics"][0]["source"] = serde_json::Value::Bool(true);
    assert!(poll_saved_frames(&mut model, &[malformed.clone()])?.visual_changed);
    assert!(
        model
            .status
            .as_deref()
            .is_some_and(|status| { status.starts_with("Saved compiler report rejected: ") })
    );
    let rejected = model.snapshot();
    assert_eq!(
        rejected.saved_compiler_rejections,
        accepted.saved_compiler_rejections + 1
    );
    assert_eq!(rejected.saved_compiler_items, accepted.saved_compiler_items);
    assert_eq!(rejected.saved_compiler_bytes, accepted.saved_compiler_bytes);
    assert_eq!(rejected.diagnostic_items, accepted.diagnostic_items);
    assert_eq!(rejected.diagnostic_version, accepted.diagnostic_version);
    assert!(
        model
            .status_message()
            .ok_or("saved status")?
            .contains("E0502")
    );
    assert!(!poll_saved_frames(&mut model, &[malformed])?.visual_changed);
    assert_eq!(
        model.snapshot().saved_compiler_rejections,
        accepted.saved_compiler_rejections + 2
    );
    assert!(!model.shutdown().active);
    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn saved_poll_foreign_reports_are_quiet_and_owned_clear_preserves_native_data()
-> Result<(), Box<dyn Error>> {
    let (mut model, _, root) = installed_workspace()?;
    model.initialize_installed_transport_for_test()?;
    let uri = model
        .session
        .as_ref()
        .ok_or("workspace")?
        .document
        .uri()
        .to_owned();
    assert!(poll_saved_frames(&mut model, &[saved_params(&uri, false)])?.visual_changed);
    let accepted = model.snapshot();
    let status = model.status_message();
    assert!(
        !poll_saved_frames(
            &mut model,
            &[
                saved_params("file:///tmp/unloaded-compiler-report.rs", false),
                saved_params("https://invalid.example/not-local.rs", false),
            ]
        )?
        .visual_changed
    );
    assert_eq!(model.status_message(), status);
    let rejected = model.snapshot();
    assert_eq!(
        rejected.saved_compiler_rejections,
        accepted.saved_compiler_rejections + 2
    );
    assert_eq!(rejected.saved_compiler_items, accepted.saved_compiler_items);
    assert_eq!(rejected.saved_compiler_bytes, accepted.saved_compiler_bytes);
    assert!(poll_saved_frames(&mut model, &[saved_params(&uri, true)])?.visual_changed);
    let cleared = model.snapshot();
    assert_eq!(cleared.saved_compiler_items, 0);
    assert_eq!(cleared.saved_compiler_bytes, 0);
    assert_eq!(cleared.diagnostic_items, accepted.diagnostic_items);
    assert_eq!(cleared.diagnostic_version, accepted.diagnostic_version);
    assert!(!model.shutdown().active);
    std::fs::remove_dir_all(root)?;
    Ok(())
}
