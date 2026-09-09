use super::*;

fn workspace_acceptance_frame(value: &serde_json::Value) -> Vec<u8> {
    let body = value.to_string();
    format!("Content-Length: {}\r\n\r\n{body}", body.len()).into_bytes()
}

fn workspace_acceptance_decode(bytes: &[u8]) -> Result<serde_json::Value, Box<dyn Error>> {
    let mut framer =
        crate::lsp_framing::LspFramer::new(crate::lsp_framing::LspFrameLimits::default());
    let batch = framer.ingest(bytes)?;
    assert_eq!(batch.consumed(), bytes.len());
    assert_eq!(batch.frames().len(), 1);
    let value = serde_json::from_slice(batch.frames()[0].body())?;
    framer.finish()?;
    Ok(value)
}

fn workspace_acceptance_messages(
    observer: &mut crate::lsp_process::ProcessInputObserver,
) -> Result<Vec<serde_json::Value>, Box<dyn Error>> {
    let mut messages = Vec::new();
    for _ in 0..8 {
        let Some(bytes) = observer.take_input()? else {
            assert_eq!(observer.retained_bytes(), 0);
            return Ok(messages);
        };
        messages.push(workspace_acceptance_decode(&bytes)?);
    }
    Err("workspace acceptance fixture exceeded eight outbound messages".into())
}

fn workspace_acceptance_ready_workspace()
-> Result<(RustDiagnostics, RustDocumentInput, RustDocumentInput), Box<dyn Error>> {
    let root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("workspace-acceptance-virtual-documents");
    let snapshot = alpine_text::Buffer::new("pub fn value() -> u32 { 1 }\n").snapshot();
    let identity = LanguageIdentity {
        workspace_id: 1,
        workspace_revision: 1,
        document_id: 1,
        document_revision: 1,
        buffer_revision: snapshot.revision().get(),
        selection_revision: 1,
    };
    let first = RustDocumentInput::new(&root.join("a.rs"), &root, identity, snapshot);
    let document = LspDocument::from_file_path(&first.path, "rust", 1)?;
    let empty = crate::lsp_language::DiagnosticBatch::from_saved_items(document.uri(), &[])?;
    let mut session = super::super::super::test_session(
        first.clone(),
        document,
        empty,
        Path::new("/inert-workspace-acceptance-no-child"),
    );
    session.diagnostics = None;
    let mut second = first.clone();
    second.path = root.join("b.rs");
    second.identity.document_id = 2;
    second.snapshot =
        alpine_text::Buffer::new("fn main() { let _ = crate::a::value(); }\n").snapshot();
    assert!(session.update_parked(second.clone())?);
    session.parked[0].opened = true;

    let params = crate::lsp_language::initialize_pull_params(&root)?;
    let request = session.client.begin_initialize_with(&params)?;
    let init = session
        .client
        .take_input_for_test()?
        .ok_or("initialize missing")?;
    assert_eq!(workspace_acceptance_decode(&init)?["method"], "initialize");
    let response = workspace_acceptance_frame(&serde_json::json!({
        "jsonrpc": "2.0", "id": request.request_id,
        "result": {"capabilities": {
            "positionEncoding": "utf-16",
            "textDocumentSync": {"openClose": true, "change": 2},
            "diagnosticProvider": {"interFileDependencies": true, "workspaceDiagnostics": false}
        }}
    }));
    session.client.inject_stdout_for_test(&response)?;
    let _ = session.client.poll(None, |_| {})?;
    let initialized = session
        .client
        .take_input_for_test()?
        .ok_or("initialized missing")?;
    assert_eq!(
        workspace_acceptance_decode(&initialized)?["method"],
        "initialized"
    );
    assert!(session.client.take_input_for_test()?.is_none());
    session.diagnostic_pull.enabled = session.client.diagnostic_pull_supported();
    assert!(session.diagnostic_pull.enabled);
    assert!(session.workspace_ready());
    let target = session.target.clone();
    Ok((
        RustDiagnostics {
            target: Some(target),
            session: Some(session),
            ..RustDiagnostics::default()
        },
        first,
        second,
    ))
}

fn workspace_acceptance_edit(input: &mut RustDocumentInput) -> Result<(), Box<dyn Error>> {
    let mut buffer = alpine_text::Buffer::new(&input.snapshot.text());
    let mut edit = alpine_text::Transaction::new(buffer.revision());
    edit.replace(0..0, "// workspace acceptance changed this document\n")?;
    let _ = buffer.apply(edit)?;
    input.snapshot = buffer.snapshot();
    input.identity.buffer_revision = buffer.revision().get();
    input.identity.document_revision += 1;
    Ok(())
}

fn workspace_acceptance_anchor_case(exhausted: bool) -> Result<(), Box<dyn Error>> {
    let (mut model, mut first, mut second) = workspace_acceptance_ready_workspace()?;
    let session = model.session.as_mut().ok_or("workspace")?;
    if exhausted {
        session.lsp_version = i32::MAX;
        session.document.set_version(i32::MAX);
    }
    let mut observer = session.client.take_input_observer_for_test()?;
    workspace_acceptance_edit(&mut first)?;
    workspace_acceptance_edit(&mut second)?;
    let effect = model.sync_workspace([first.clone(), second.clone()], Some(1), |_| {
        Arc::new(|| {})
    });
    let retained = model.session.is_some();
    let state = model.session.as_ref().map(|session| {
        serde_json::json!({
            "active_buffer_revision": session.identity.buffer_revision,
            "active_lsp_version": session.lsp_version,
            "active_text": session.snapshot.text(),
            "parked": session.parked.iter().map(|document| serde_json::json!({
                "document_id": document.identity.document_id,
                "buffer_revision": document.identity.buffer_revision,
                "lsp_version": document.lsp_version,
                "text": document.snapshot.text()
            })).collect::<Vec<_>>()
        })
    });
    let status = model.status_message();
    assert_eq!(model.snapshot().process_starts, 0);
    assert_eq!(model.snapshot().process_written_inputs, 0);
    let _ = model.stop();
    let messages = workspace_acceptance_messages(&mut observer)?;
    eprintln!(
        "WORKSPACE_ACCEPTANCE_ANCHOR exhausted={exhausted} retained={retained} visual_changed={} status={status:?} state={state:?} outbound={messages:?}",
        effect.visual_changed
    );
    if exhausted {
        assert!(
            !retained,
            "active version exhaustion retained a partially admitted workspace"
        );
        assert!(
            messages.is_empty(),
            "a rejected active anchor published parked text"
        );
    } else {
        assert!(retained);
        let state = state.ok_or("healthy workspace missing")?;
        assert_eq!(
            state["active_buffer_revision"],
            first.identity.buffer_revision
        );
        assert_eq!(state["active_lsp_version"], 2);
        assert_eq!(
            state["parked"][0]["buffer_revision"],
            second.identity.buffer_revision
        );
        assert_eq!(state["parked"][0]["lsp_version"], 2);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["method"], "textDocument/didChange");
        assert_eq!(
            messages[0]["params"]["textDocument"]["uri"],
            LspDocument::from_file_path(&first.path, "rust", 2)?.uri()
        );
        assert_eq!(
            messages[0]["params"]["contentChanges"][0]["text"],
            first.snapshot.text()
        );
    }
    Ok(())
}

fn workspace_acceptance_admit_native(model: &mut RustDiagnostics) -> Result<(), Box<dyn Error>> {
    let _ = model.pump_diagnostics();
    let session = model.session.as_mut().ok_or("workspace")?;
    let request_id = session
        .diagnostic_pull
        .pending
        .as_ref()
        .ok_or("diagnostic request missing")?
        .request_id;
    let request = session
        .client
        .take_input_for_test()?
        .ok_or("diagnostic wire missing")?;
    assert_eq!(
        workspace_acceptance_decode(&request)?["method"],
        "textDocument/diagnostic"
    );
    let response = workspace_acceptance_frame(&serde_json::json!({
        "jsonrpc": "2.0", "id": request_id,
        "result": {"kind": "full", "items": [{
            "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 3}},
            "severity": 1, "message": "current native diagnostic"
        }]}
    }));
    let wake = LanguageWake {
        generation: session.generation,
    };
    session.client.inject_stdout_for_test(&response)?;
    assert!(model.poll(wake).visual_changed);
    assert_eq!(model.snapshot().diagnostic_items, 1);
    assert_eq!(model.snapshot().diagnostic_version, Some(1));
    assert_eq!(
        model.status_message().as_deref(),
        Some("Rust: current native diagnostic")
    );
    Ok(())
}

fn workspace_acceptance_save_case(save: bool) -> Result<(), Box<dyn Error>> {
    let (mut model, first, second) = workspace_acceptance_ready_workspace()?;
    workspace_acceptance_admit_native(&mut model)?;
    let before = model.snapshot();
    let before_status = model.status_message();
    let mut observer = model
        .session
        .as_mut()
        .ok_or("workspace")?
        .client
        .take_input_observer_for_test()?;
    let save_effect = if save {
        model.record_saved_document(first.identity)
    } else {
        LanguageEffect::default()
    };
    let _ = model.sync_workspace([first, second], Some(1), |_| Arc::new(|| {}));
    let after = model.snapshot();
    let after_status = model.status_message();
    assert_eq!(after.process_starts, 0);
    assert_eq!(after.process_written_inputs, 0);
    let _ = model.stop();
    let messages = workspace_acceptance_messages(&mut observer)?;
    eprintln!(
        "WORKSPACE_ACCEPTANCE_SAVE save={save} visual_changed={} before_status={before_status:?} after_status={after_status:?} native_items={}->{} native_version={:?}->{:?} lsp_version={}->{} outbound={messages:?}",
        save_effect.visual_changed,
        before.diagnostic_items,
        after.diagnostic_items,
        before.diagnostic_version,
        after.diagnostic_version,
        before.lsp_version,
        after.lsp_version
    );
    assert_eq!(after.diagnostic_items, before.diagnostic_items);
    assert_eq!(after.diagnostic_version, before.diagnostic_version);
    assert_eq!(after.lsp_version, before.lsp_version);
    if save {
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["method"], "textDocument/didSave");
        assert!(
            messages[0]["params"]["textDocument"]
                .get("version")
                .is_none()
        );
        assert!(messages[0]["params"]["textDocument"].get("text").is_none());
    } else {
        assert!(messages.is_empty());
    }
    assert_eq!(
        after_status, before_status,
        "save-only submission cleared current native diagnostic status"
    );
    Ok(())
}

#[test]
fn workspace_acceptance_anchor_healthy_control() -> Result<(), Box<dyn Error>> {
    workspace_acceptance_anchor_case(false)
}

#[test]
fn workspace_acceptance_anchor_exhaustion_rejects_before_publication() -> Result<(), Box<dyn Error>>
{
    workspace_acceptance_anchor_case(true)
}

#[test]
fn workspace_acceptance_save_healthy_no_save_control() -> Result<(), Box<dyn Error>> {
    workspace_acceptance_save_case(false)
}

#[test]
fn workspace_acceptance_save_preserves_current_native_status() -> Result<(), Box<dyn Error>> {
    workspace_acceptance_save_case(true)
}

#[test]
fn workspace_acceptance_save_clears_non_diagnostic_status() -> Result<(), Box<dyn Error>> {
    let (mut model, first, _) = workspace_acceptance_ready_workspace()?;
    model.status = Some(Arc::from("Rust: recoverable transport status"));
    let mut observer = model
        .session
        .as_mut()
        .ok_or("workspace")?
        .client
        .take_input_observer_for_test()?;
    let effect = model.record_saved_document(first.identity);
    assert!(effect.visual_changed);
    assert!(model.status_message().is_none());
    assert_eq!(model.snapshot().diagnostic_items, 0);
    assert_eq!(model.snapshot().process_starts, 0);
    assert_eq!(model.snapshot().process_written_inputs, 0);
    let _ = model.stop();
    let messages = workspace_acceptance_messages(&mut observer)?;
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["method"], "textDocument/didSave");
    Ok(())
}

#[test]
fn workspace_acceptance_unchanged_exhausted_anchor_allows_parked_edit() -> Result<(), Box<dyn Error>>
{
    let (mut model, first, mut second) = workspace_acceptance_ready_workspace()?;
    let session = model.session.as_mut().ok_or("workspace")?;
    session.lsp_version = i32::MAX;
    session.document.set_version(i32::MAX);
    let mut observer = session.client.take_input_observer_for_test()?;
    workspace_acceptance_edit(&mut second)?;
    let _ = model.sync_workspace([first.clone(), second.clone()], Some(1), |_| {
        Arc::new(|| {})
    });
    let session = model
        .session
        .as_ref()
        .ok_or("unchanged anchor was rejected")?;
    assert_eq!(session.lsp_version, i32::MAX);
    assert_eq!(
        session.identity.buffer_revision,
        first.identity.buffer_revision
    );
    assert_eq!(session.parked[0].lsp_version, 2);
    assert_eq!(
        session.parked[0].identity.buffer_revision,
        second.identity.buffer_revision
    );
    assert_eq!(model.snapshot().process_starts, 0);
    assert_eq!(model.snapshot().process_written_inputs, 0);
    let _ = model.stop();
    let messages = workspace_acceptance_messages(&mut observer)?;
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["method"], "textDocument/didChange");
    assert_eq!(
        messages[0]["params"]["textDocument"]["uri"],
        LspDocument::from_file_path(&second.path, "rust", 2)?.uri()
    );
    assert_eq!(
        messages[0]["params"]["contentChanges"][0]["text"],
        second.snapshot.text()
    );
    Ok(())
}
