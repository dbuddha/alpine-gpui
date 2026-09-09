use super::*;

fn workspace_acceptance_assert_refresh_delivery(messages: &[serde_json::Value], request_id: u64) {
    assert_eq!(
        messages
            .iter()
            .filter(|message| message["id"] == 91
                && message
                    .get("result")
                    .is_some_and(serde_json::Value::is_null))
            .count(),
        1
    );
    assert_eq!(
        messages
            .iter()
            .filter(|message| message["method"] == "$/cancelRequest"
                && message["params"]["id"] == request_id)
            .count(),
        1
    );
    assert!(!messages.iter().any(|message| matches!(
        message["method"].as_str(),
        Some("initialize" | "textDocument/didOpen")
    )));
}

#[test]
fn workspace_acceptance_framed_refresh_preserves_delivery_ownership() -> Result<(), Box<dyn Error>>
{
    for saved_owner in [0, 1, 2] {
        let (mut model, first, second) = workspace_acceptance_ready_workspace()?;
        let mut observer = model
            .session
            .as_mut()
            .ok_or("workspace")?
            .client
            .take_input_observer_for_test()?;
        let _ = model.pump_diagnostics();
        let request = workspace_acceptance_messages(&mut observer)?;
        assert_eq!(request.len(), 1);
        assert_eq!(request[0]["method"], "textDocument/diagnostic");
        let request_id = request[0]["id"].as_u64().ok_or("diagnostic id")?;
        workspace_acceptance_fill_input(&mut model)?;
        if saved_owner == 1 {
            let _ = model.record_saved_document(first.identity);
        } else if saved_owner == 2 {
            let _ = model.record_saved_document(second.identity);
            let _ = model.sync_workspace([first.clone()], Some(1), |_| Arc::new(|| {}));
        }
        let before = model.session.as_ref().ok_or("workspace")?;
        let ready = before.workspace_ready();
        let close_count = before.overlay_closes.len();
        let _ = workspace_acceptance_receive(
            &mut model,
            &serde_json::json!({
                "jsonrpc":"2.0","id":91,"method":"workspace/diagnostic/refresh"
            }),
        )?;
        let session = model.session.as_ref().ok_or("workspace after refresh")?;
        assert_eq!(session.state, SessionState::Open);
        assert_eq!(session.workspace_ready(), ready);
        assert!(session.document_opened);
        assert!(session.diagnostic_pull.enabled);
        assert!(session.diagnostic_pull.pending.is_none());
        assert_eq!(session.overlay_closes.len(), close_count);
        assert_eq!(session.client.snapshot().protocol_writes.queued, 1);
        assert_eq!(model.snapshot().restarts, 0);
        assert!(
            !model
                .status_message()
                .as_deref()
                .is_some_and(|s| s.contains("unavailable"))
        );
        workspace_acceptance_drain_pressure(&mut observer)?;
        let wake = LanguageWake {
            generation: model.session.as_ref().ok_or("workspace")?.generation,
        };
        let _ = model.poll(wake);
        let messages = workspace_acceptance_messages(&mut observer)?;
        workspace_acceptance_assert_refresh_delivery(&messages, request_id);
        assert_eq!(model.snapshot().protocol_writes.retained_bytes, 0);
        if saved_owner != 0 {
            let expected = if saved_owner == 1 { &first } else { &second };
            let uri = LspDocument::from_file_path(&expected.path, "rust", 1)?;
            assert!(
                messages
                    .iter()
                    .any(|message| message["method"] == "textDocument/didSave"
                        && message["params"]["textDocument"]["uri"] == uri.uri())
            );
            // No writer acknowledgement was forged: close must remain owned.
            assert_eq!(
                model
                    .session
                    .as_ref()
                    .ok_or("workspace")?
                    .overlay_closes
                    .len(),
                close_count
            );
        }
        let _ = workspace_acceptance_receive(
            &mut model,
            &serde_json::json!({
                "jsonrpc":"2.0","id":request_id,"result":{"kind":"full","items":[{
                    "range":{"start":{"line":0,"character":0},"end":{"line":0,"character":3}},
                    "severity":1,"message":"obsolete pre-refresh diagnostic"
                }]}
            }),
        )?;
        assert_eq!(model.snapshot().diagnostic_items, 0);
        assert_eq!(model.snapshot().restarts, 0);
        let _ = model.stop();
    }
    Ok(())
}

#[test]
fn workspace_acceptance_refresh_reply_pressure_preserves_batch_invalidation()
-> Result<(), Box<dyn Error>> {
    for response_first in [false, true] {
        let (mut model, _, _) = workspace_acceptance_ready_workspace()?;
        let mut observer = model
            .session
            .as_mut()
            .ok_or("workspace")?
            .client
            .take_input_observer_for_test()?;
        let _ = model.pump_diagnostics();
        let request = workspace_acceptance_messages(&mut observer)?;
        let id = request[0]["id"].as_u64().ok_or("diagnostic id")?;
        workspace_acceptance_fill_input(&mut model)?;
        let response = workspace_acceptance_frame(&serde_json::json!({
            "jsonrpc":"2.0","id":id,"result":{"kind":"full","items":[{
                "range":{"start":{"line":0,"character":0},"end":{"line":0,"character":3}},
                "severity":1,"message":"same-batch obsolete result"
            }]}
        }));
        let mut refreshes = workspace_acceptance_frame(&serde_json::json!({
            "jsonrpc":"2.0","id":91,"method":"workspace/diagnostic/refresh"
        }));
        refreshes.extend(workspace_acceptance_frame(&serde_json::json!({
            "jsonrpc":"2.0","id":92,"method":"workspace/diagnostic/refresh"
        })));
        let bytes = if response_first {
            [response, refreshes].concat()
        } else {
            [refreshes, response].concat()
        };
        let session = model.session.as_mut().ok_or("workspace")?;
        let wake = LanguageWake {
            generation: session.generation,
        };
        session.client.inject_stdout_for_test(&bytes)?;
        let _ = model.poll(wake);
        assert_eq!(model.snapshot().restarts, 0);
        assert_eq!(model.snapshot().diagnostic_items, 0);
        assert_eq!(model.snapshot().protocol_writes.queued, 2);
        assert!(model.session.as_ref().ok_or("workspace")?.workspace_ready());
        workspace_acceptance_drain_pressure(&mut observer)?;
        let _ = model.poll(wake);
        let messages = workspace_acceptance_messages(&mut observer)?;
        let ids: Vec<_> = messages
            .iter()
            .filter(|message| message.get("result").is_some())
            .map(|message| message["id"].as_u64().ok_or("response id"))
            .collect::<Result<_, _>>()?;
        assert_eq!(ids, [91, 92]);
        assert!(
            !messages
                .iter()
                .any(|message| message["method"] == "$/cancelRequest")
        );
        assert_eq!(model.snapshot().diagnostic_items, 0);
        assert_eq!(model.snapshot().restarts, 0);
        let _ = model.stop();
    }
    Ok(())
}

#[test]
fn workspace_acceptance_healthy_restart_control_preserves_native_caches()
-> Result<(), Box<dyn Error>> {
    workspace_acceptance_restart_cache_case(false)
}

#[test]
fn workspace_acceptance_rejected_restart_revokes_native_caches() -> Result<(), Box<dyn Error>> {
    workspace_acceptance_restart_cache_case(true)
}

fn workspace_acceptance_restart_cache_case(fail: bool) -> Result<(), Box<dyn Error>> {
    let (mut model, first, second) = workspace_acceptance_ready_workspace()?;
    workspace_acceptance_admit_native(&mut model)?;
    let _ = model.sync_workspace([first.clone(), second.clone()], Some(2), |_| {
        Arc::new(|| {})
    });
    workspace_acceptance_admit_native(&mut model)?;
    let session = model
        .session
        .as_ref()
        .ok_or("workspace with native caches")?;
    assert!(session.diagnostics.is_some());
    assert!(session.parked.iter().any(|document| {
        document.identity.document_id == first.identity.document_id
            && document.diagnostics.is_some()
    }));
    let mut observer = model
        .session
        .as_mut()
        .ok_or("workspace")?
        .client
        .take_input_observer_for_test()?;
    workspace_acceptance_fill_input(&mut model)?;
    if fail {
        assert_eq!(
            model
                .session
                .as_ref()
                .ok_or("workspace before rejected restart")?
                .client
                .fill_control_for_test()?,
            8 - crate::lsp_process::INPUT_CAPACITY
        );
        let failure = RustDiagnosticsError::Client(crate::lsp_client::LspClientError::Protocol(
            crate::lsp_json::ProtocolError::InvalidEnvelope,
        ));
        let _ = model.restart_or_fail(failure);
        let session = model
            .session
            .as_ref()
            .ok_or("workspace after protocol failure")?;
        assert_eq!(session.state, SessionState::Starting);
        assert!(session.diagnostics.is_none());
        assert!(
            session
                .parked
                .iter()
                .all(|document| document.diagnostics.is_none())
        );
        assert_eq!(model.snapshot().diagnostic_items, 0);
    }
    let _ = model.sync_workspace([first, second], Some(1), |_| Arc::new(|| {}));
    assert_eq!(model.snapshot().restarts, 0);
    if fail {
        assert_eq!(model.snapshot().diagnostic_items, 0);
        assert!(!model.session.as_ref().ok_or("workspace")?.workspace_ready());
        assert!(
            !model
                .status_message()
                .as_deref()
                .is_some_and(|status| status.contains("current native diagnostic"))
        );
    } else {
        assert_eq!(model.snapshot().diagnostic_items, 1);
        // Tab restoration publishes the cached primary message; the "Rust:"
        // prefix belongs to the separate response-admission status path.
        assert_eq!(
            model.status_message().as_deref(),
            Some("current native diagnostic")
        );
        assert!(model.session.as_ref().ok_or("workspace")?.workspace_ready());
    }
    workspace_acceptance_drain_pressure(&mut observer)?;
    let _ = model.stop();
    Ok(())
}

#[test]
fn workspace_acceptance_rejected_restart_preserves_overlay_ownership() -> Result<(), Box<dyn Error>>
{
    workspace_acceptance_restart_ownership(false)
}

#[test]
fn workspace_acceptance_exhausted_restart_preserves_overlay_ownership() -> Result<(), Box<dyn Error>>
{
    workspace_acceptance_restart_ownership(true)
}

fn workspace_acceptance_restart_ownership(exhausted: bool) -> Result<(), Box<dyn Error>> {
    let (mut model, first, second) = workspace_acceptance_ready_workspace()?;
    let mut observer = model
        .session
        .as_mut()
        .ok_or("workspace")?
        .client
        .take_input_observer_for_test()?;
    workspace_acceptance_fill_input(&mut model)?;
    let _ = model.record_saved_document(first.identity);
    let _ = model.record_saved_document(second.identity);
    let _ = model.sync_workspace([first.clone()], Some(1), |_| Arc::new(|| {}));
    let session = model.session.as_mut().ok_or("workspace")?;
    assert_eq!(session.state, SessionState::Open);
    assert!(session.document_opened);
    assert_eq!(session.overlay_closes.len(), 1);
    let generation = session.process_generation;
    let active_save_revision = session
        .pending_save
        .as_ref()
        .ok_or("active save intent")?
        .buffer_revision;
    let closing_uri = session.overlay_closes[0].document.uri().to_owned();
    let closing_save_revision = session.overlay_closes[0]
        .pending_save
        .as_ref()
        .ok_or("closing save intent")?
        .buffer_revision;
    if exhausted {
        // Isolate the budget guard: replacement enqueue would otherwise succeed.
        workspace_acceptance_drain_pressure(&mut observer)?;
        session.restart_count = crate::rust_diagnostics::MAX_RESTARTS_PER_DOCUMENT;
    } else {
        // Full input ownership is not full management admission.
        assert_eq!(
            session.client.fill_control_for_test()?,
            8 - crate::lsp_process::INPUT_CAPACITY
        );
    }
    let restart_count = session.restart_count;
    let failure = RustDiagnosticsError::Client(crate::lsp_client::LspClientError::Protocol(
        crate::lsp_json::ProtocolError::InvalidEnvelope,
    ));

    let _ = model.restart_or_fail(failure);
    let session = model
        .session
        .as_ref()
        .ok_or("workspace after rejected restart")?;
    // A genuine protocol failure must suppress publication even though the
    // rejected replacement cannot take ownership of the existing transport.
    assert_eq!(session.state, SessionState::Starting);
    assert!(!session.workspace_ready());
    assert_eq!(session.process_generation, generation);
    assert_eq!(session.restart_count, restart_count);
    assert!(session.client.snapshot().started);
    assert_eq!(
        session.client.snapshot().peer.lifecycle(),
        crate::lsp_json::PeerLifecycle::Running
    );
    assert!(session.document_opened);
    assert!(session.diagnostic_pull.enabled);
    assert_eq!(
        session
            .pending_save
            .as_ref()
            .map(|save| save.buffer_revision),
        Some(active_save_revision)
    );
    assert_eq!(session.overlay_closes.len(), 1);
    assert_eq!(session.overlay_closes[0].document.uri(), closing_uri);
    assert_eq!(
        session.overlay_closes[0]
            .pending_save
            .as_ref()
            .map(|save| save.buffer_revision),
        Some(closing_save_revision)
    );
    assert_eq!(model.snapshot().restarts, 0);

    if !exhausted {
        workspace_acceptance_drain_pressure(&mut observer)?;
        // This is an explicit recovery retry, not evidence of automatic retry
        // scheduling. Ownership changes only after the real bounded enqueue.
        let _ = model.restart_or_fail(failure);
        let session = model
            .session
            .as_ref()
            .ok_or("workspace after admitted restart")?;
        assert_eq!(session.process_generation, generation + 1);
        assert_eq!(session.restart_count, 1);
        assert!(!session.client.snapshot().started);
        assert!(!session.document_opened);
        assert!(!session.diagnostic_pull.enabled);
        assert!(session.overlay_closes.is_empty());
        assert_eq!(
            session
                .pending_save
                .as_ref()
                .map(|save| save.buffer_revision),
            Some(active_save_revision)
        );
        assert_eq!(model.snapshot().restarts, 1);
    }
    let _ = model.stop();
    Ok(())
}

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

#[test]
fn workspace_acceptance_parked_position_rejection_preserves_pending_text()
-> Result<(), Box<dyn Error>> {
    let (mut model, _, mut second) = workspace_acceptance_ready_workspace()?;
    let mut buffer = alpine_text::Buffer::new(&second.snapshot.text());
    let long_line = "x".repeat(1_000_001);
    let mut edit = alpine_text::Transaction::new(buffer.revision());
    edit.replace(0..second.snapshot.len_bytes(), &long_line)?;
    let _ = buffer.apply(edit)?;
    second.snapshot = buffer.snapshot();
    second.identity.buffer_revision = buffer.revision().get();
    let session = model.session.as_mut().ok_or("workspace")?;
    assert!(session.update_parked(second.clone())?);
    assert!(session.flush_overlay()?);
    let sequence = session.overlay_write.ok_or("long-line write")?;
    let bytes = session
        .client
        .take_input_for_test()?
        .ok_or("long-line wire missing")?;
    let message = workspace_acceptance_decode(&bytes)?;
    assert_eq!(message["method"], "textDocument/didChange");
    assert_eq!(message["params"]["contentChanges"][0]["text"], long_line);
    // Consume the real queued bytes before this state-control acknowledgement.
    // No actual process writer or native qualification is represented here.
    assert!(session.acknowledge_overlay(sequence));
    assert_eq!(session.client.snapshot().process.written_inputs, 0);
    let synced_revision = session.parked[0].synced_snapshot.revision();

    let end = second.snapshot.len_bytes();
    let mut edit = alpine_text::Transaction::new(buffer.revision());
    edit.replace(end..end, "y")?;
    let _ = buffer.apply(edit)?;
    second.snapshot = buffer.snapshot();
    second.identity.buffer_revision = buffer.revision().get();
    assert!(session.update_parked(second.clone())?);
    let submitted = session.client.snapshot().process.submitted_inputs;
    assert!(matches!(
        session.flush_overlay(),
        Err(RustDiagnosticsError::Language(
            LanguageProtocolError::InvalidPosition
        ))
    ));
    assert!(session.overlay_write.is_none());
    assert!(session.parked[0].pending_change);
    assert_eq!(
        session.parked[0].synced_snapshot.revision(),
        synced_revision
    );
    assert_eq!(session.parked[0].synced_snapshot.text(), long_line);
    assert_eq!(session.parked[0].snapshot.text(), second.snapshot.text());
    assert_eq!(
        session.client.snapshot().process.submitted_inputs,
        submitted
    );
    assert!(session.client.take_input_for_test()?.is_none());
    let _ = model.stop();
    Ok(())
}

#[test]
fn workspace_acceptance_closed_cancel_retires_authority() -> Result<(), Box<dyn Error>> {
    let (mut model, mut first, second) = workspace_acceptance_ready_workspace()?;
    let mut observer = model
        .session
        .as_mut()
        .ok_or("workspace")?
        .client
        .take_input_observer_for_test()?;
    let _ = model.pump_diagnostics();
    let messages = workspace_acceptance_messages(&mut observer)?;
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["method"], "textDocument/diagnostic");
    assert!(
        model
            .session
            .as_ref()
            .ok_or("workspace")?
            .diagnostic_pull
            .pending
            .is_some()
    );
    drop(observer);
    workspace_acceptance_edit(&mut first)?;
    let expected_text = first.snapshot.text();
    let starts = std::cell::Cell::new(0);
    let effect = model.sync_workspace([first.clone(), second], Some(1), |_| {
        starts.set(starts.get() + 1);
        Arc::new(|| {})
    });
    assert!(effect.visual_changed);
    assert!(effect.continuation.is_none());
    assert!(model.session.is_none());
    assert!(model.target.is_none());
    assert_eq!(starts.get(), 0);
    assert_eq!(model.snapshot().process_starts, 0);
    let expected = RustDiagnosticsError::Client(crate::lsp_client::LspClientError::Submit(
        crate::lsp_process::SubmitError::Closed,
    ))
    .to_string();
    assert_eq!(model.status_message().as_deref(), Some(expected.as_str()));
    assert_eq!(first.snapshot.text(), expected_text);
    Ok(())
}

#[test]
fn workspace_acceptance_closing_budget_rejects_before_owner_replacement()
-> Result<(), Box<dyn Error>> {
    let (mut model, first, mut second) = workspace_acceptance_ready_workspace()?;
    let first_text = first.snapshot.text();
    let parked_text = second.snapshot.text();
    let mut observer = model
        .session
        .as_mut()
        .ok_or("workspace")?
        .client
        .take_input_observer_for_test()?;
    workspace_acceptance_fill_input(&mut model)?;
    for closed in 0..MAX_OVERLAY_DOCUMENTS {
        let _ = model.record_saved_document(second.identity);
        let session = model.session.as_ref().ok_or("workspace before close")?;
        assert_eq!(session.overlay_closes.len(), closed);
        assert_eq!(session.parked.len(), 1);
        assert!(session.parked[0].pending_save.is_some());
        let mut next = second.clone();
        next.identity.document_id += 1;
        next.path = next
            .workspace_root
            .join(format!("closing-budget-{}.rs", next.identity.document_id));
        let _ = model.sync_workspace([first.clone(), next.clone()], Some(1), |_| Arc::new(|| {}));
        let session = model
            .session
            .as_ref()
            .ok_or("bounded close lost workspace")?;
        assert_eq!(session.overlay_closes.len(), closed + 1);
        assert_eq!(session.parked.len(), 1);
        assert_eq!(session.parked[0].identity, next.identity);
        assert!(session.overlay_write.is_none());
        assert_eq!(model.snapshot().process_starts, 0);
        second = next;
    }
    let _ = model.record_saved_document(second.identity);
    assert_eq!(
        model
            .session
            .as_ref()
            .ok_or("full closing queue")?
            .overlay_closes
            .len(),
        MAX_OVERLAY_DOCUMENTS
    );
    let mut next = second.clone();
    next.identity.document_id += 1;
    next.path = next.workspace_root.join("closing-budget-rejected.rs");
    let starts = std::cell::Cell::new(0);
    let effect = model.sync_workspace([first.clone(), next.clone()], Some(1), |_| {
        starts.set(starts.get() + 1);
        Arc::new(|| {})
    });
    assert!(effect.visual_changed);
    assert!(effect.continuation.is_none());
    assert!(model.session.is_none());
    assert!(model.target.is_none());
    assert_eq!(starts.get(), 0);
    assert_eq!(model.snapshot().process_starts, 0);
    let expected = RustDiagnosticsError::OverlayBudget.to_string();
    assert_eq!(model.status_message().as_deref(), Some(expected.as_str()));
    assert_eq!(first.snapshot.text(), first_text);
    assert_eq!(second.snapshot.text(), parked_text);
    assert_eq!(next.snapshot.text(), parked_text);
    workspace_acceptance_drain_pressure(&mut observer)?;
    Ok(())
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

fn workspace_acceptance_fill_input(model: &mut RustDiagnostics) -> Result<(), Box<dyn Error>> {
    let client = &mut model.session.as_mut().ok_or("workspace")?.client;
    let params = serde_json::value::RawValue::from_string("{\"value\":\"off\"}".into())?;
    for _ in 0..crate::lsp_process::INPUT_CAPACITY {
        let _ = client.notify("$/setTrace", Some(&params))?;
    }
    assert!(matches!(
        client.notify("$/setTrace", Some(&params)),
        Err(crate::lsp_client::LspClientError::Submit(
            crate::lsp_process::SubmitError::Saturated
        ))
    ));
    Ok(())
}

fn workspace_acceptance_drain_pressure(
    observer: &mut crate::lsp_process::ProcessInputObserver,
) -> Result<(), Box<dyn Error>> {
    for _ in 0..crate::lsp_process::INPUT_CAPACITY {
        let bytes = observer.take_input()?.ok_or("bounded control missing")?;
        assert_eq!(workspace_acceptance_decode(&bytes)?["method"], "$/setTrace");
    }
    assert!(observer.take_input()?.is_none());
    assert_eq!(observer.retained_bytes(), 0);
    Ok(())
}

fn workspace_acceptance_receive(
    model: &mut RustDiagnostics,
    value: &serde_json::Value,
) -> Result<LanguageEffect, Box<dyn Error>> {
    let session = model.session.as_mut().ok_or("workspace")?;
    let wake = LanguageWake {
        generation: session.generation,
    };
    session
        .client
        .inject_stdout_for_test(&workspace_acceptance_frame(value))?;
    Ok(model.poll(wake))
}

fn workspace_acceptance_assert_open(model: &RustDiagnostics) -> Result<(), Box<dyn Error>> {
    let session = model.session.as_ref().ok_or("workspace lost")?;
    assert_eq!(session.state, SessionState::Open);
    assert!(session.document_opened);
    assert!(session.parked.iter().all(|document| document.opened));
    assert!(session.diagnostic_pull.enabled);
    assert_eq!(session.restart_count, 0);
    assert_eq!(model.snapshot().restarts, 0);
    assert_eq!(model.snapshot().process_starts, 0);
    assert_eq!(model.snapshot().process_written_inputs, 0);
    Ok(())
}

#[test]
fn workspace_acceptance_pressure_preserves_readiness_and_real_retry_budget()
-> Result<(), Box<dyn Error>> {
    let (mut model, _, _) = workspace_acceptance_ready_workspace()?;
    let mut observer = model
        .session
        .as_mut()
        .ok_or("workspace")?
        .client
        .take_input_observer_for_test()?;
    workspace_acceptance_fill_input(&mut model)?;
    for _ in 0..8 {
        assert!(!model.pump_diagnostics());
        workspace_acceptance_assert_open(&model)?;
        let session = model.session.as_ref().ok_or("workspace")?;
        assert!(session.workspace_ready());
        assert!(session.diagnostic_pull.pending.is_none());
        assert_eq!(session.client.snapshot().peer.pending_requests(), 0);
        assert!(model.status_message().is_none());
    }
    workspace_acceptance_drain_pressure(&mut observer)?;
    assert!(!model.pump_diagnostics());
    for attempt in 1..=3 {
        let messages = workspace_acceptance_messages(&mut observer)?;
        assert_eq!(messages.len(), 1, "pressure consumed an admitted retry");
        assert_eq!(messages[0]["method"], "textDocument/diagnostic");
        let id = messages[0]["id"].as_u64().ok_or("diagnostic id")?;
        workspace_acceptance_fill_input(&mut model)?;
        let _ = workspace_acceptance_receive(
            &mut model,
            &serde_json::json!({"jsonrpc":"2.0", "id":id, "error":{
                "code":-32802, "message":"server busy", "data":{"retriggerRequest":true}
            }}),
        )?;
        for _ in 0..8 {
            assert!(!model.pump_diagnostics());
            workspace_acceptance_assert_open(&model)?;
        }
        assert_eq!(
            model.status_message().as_deref(),
            Some(if attempt == 3 {
                "Rust diagnostic retry budget exhausted; waiting for the next invalidation."
            } else {
                "Rust diagnostics canceled by the server; bounded retry pending."
            })
        );
        workspace_acceptance_drain_pressure(&mut observer)?;
        assert!(!model.pump_diagnostics());
    }
    assert!(workspace_acceptance_messages(&mut observer)?.is_empty());
    let submitted = model.snapshot().process_submitted_inputs;
    for _ in 0..32 {
        assert!(!model.pump_diagnostics());
    }
    assert_eq!(model.snapshot().process_submitted_inputs, submitted);
    let _ = model.stop();
    Ok(())
}

#[test]
fn workspace_acceptance_pressure_cancellation_delivers_once_and_rejects_late_response()
-> Result<(), Box<dyn Error>> {
    for inactive in [false, true] {
        for late_before_retry in [false, true] {
            let (mut model, _, _) = workspace_acceptance_ready_workspace()?;
            let mut observer = model
                .session
                .as_mut()
                .ok_or("workspace")?
                .client
                .take_input_observer_for_test()?;
            assert!(!model.pump_diagnostics());
            let original = workspace_acceptance_messages(&mut observer)?;
            assert_eq!(original.len(), 1);
            let id = original[0]["id"].as_u64().ok_or("original id")?;
            workspace_acceptance_fill_input(&mut model)?;
            if inactive {
                model.session.as_mut().ok_or("workspace")?.active_view = false;
                assert!(!model.pump_diagnostics());
            } else {
                // Local invalidation only. Admission of a server-request
                // response under pressure has a separate transport boundary.
                let _ = model.refresh_diagnostics();
            }
            for _ in 0..8 {
                assert!(!model.pump_diagnostics());
                workspace_acceptance_assert_open(&model)?;
                assert!(
                    model
                        .session
                        .as_ref()
                        .ok_or("workspace")?
                        .diagnostic_pull
                        .pending
                        .is_none()
                );
            }
            let late = serde_json::json!({"jsonrpc":"2.0", "id":id,
                "result":{"kind":"full", "items":[]}});
            if late_before_retry {
                let _ = workspace_acceptance_receive(&mut model, &late)?;
            }
            workspace_acceptance_drain_pressure(&mut observer)?;
            assert!(!model.pump_diagnostics());
            let mut messages = workspace_acceptance_messages(&mut observer)?;
            assert_eq!(messages.len(), if inactive { 1 } else { 2 });
            assert_eq!(messages[0]["method"], "$/cancelRequest");
            assert_eq!(messages[0]["params"]["id"], id);
            if inactive {
                model.session.as_mut().ok_or("workspace")?.active_view = true;
                assert!(!model.pump_diagnostics());
                messages.extend(workspace_acceptance_messages(&mut observer)?);
            }
            assert_eq!(messages.len(), 2);
            assert_eq!(messages[1]["method"], "textDocument/diagnostic");
            let fresh = messages[1]["id"].as_u64().ok_or("fresh id")?;
            assert!(fresh > id);
            if !late_before_retry {
                let _ = workspace_acceptance_receive(&mut model, &late)?;
            }
            let session = model.session.as_ref().ok_or("workspace")?;
            assert_eq!(
                u64::from(
                    session
                        .diagnostic_pull
                        .pending
                        .ok_or("fresh owner lost")?
                        .request_id
                ),
                fresh
            );
            assert!(session.diagnostics.is_none());
            let _ = workspace_acceptance_receive(
                &mut model,
                &serde_json::json!({"jsonrpc":"2.0", "id":fresh,
                    "result":{"kind":"full", "items":[]}}),
            )?;
            assert!(
                model
                    .session
                    .as_ref()
                    .ok_or("workspace")?
                    .diagnostics
                    .is_some()
            );
            workspace_acceptance_assert_open(&model)?;
            assert!(model.status_message().is_none());
            assert!(workspace_acceptance_messages(&mut observer)?.is_empty());
            let _ = model.stop();
        }
    }
    Ok(())
}

#[test]
fn workspace_acceptance_pressure_preserves_unsaved_change_and_save_intent()
-> Result<(), Box<dyn Error>> {
    let (mut model, mut first, second) = workspace_acceptance_ready_workspace()?;
    let mut observer = model
        .session
        .as_mut()
        .ok_or("workspace")?
        .client
        .take_input_observer_for_test()?;
    assert!(!model.pump_diagnostics());
    let original = workspace_acceptance_messages(&mut observer)?;
    assert_eq!(original.len(), 1);
    workspace_acceptance_fill_input(&mut model)?;
    workspace_acceptance_edit(&mut first)?;
    let _ = model.sync_workspace([first.clone(), second], Some(1), |_| Arc::new(|| {}));
    let save = model.record_saved_document(first.identity);
    assert!(save.continuation.is_none());
    workspace_acceptance_assert_open(&model)?;
    let session = model.session.as_ref().ok_or("workspace")?;
    assert_eq!(session.lsp_version, 2);
    assert_eq!(session.snapshot.text(), first.snapshot.text());
    assert!(session.pending_change);
    assert!(session.overlay_write.is_none());
    assert!(
        session
            .pending_save
            .as_ref()
            .ok_or("save lost")?
            .submitted
            .is_none()
    );
    assert!(!session.workspace_ready());
    workspace_acceptance_drain_pressure(&mut observer)?;
    let wake = LanguageWake {
        generation: session.generation,
    };
    let effect = model.poll(wake);
    assert!(effect.continuation.is_none());
    let messages = workspace_acceptance_messages(&mut observer)?;
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["method"], "textDocument/didChange");
    assert_eq!(messages[0]["params"]["textDocument"]["version"], 2);
    assert_eq!(
        messages[0]["params"]["contentChanges"][0]["text"],
        first.snapshot.text()
    );
    assert_eq!(messages[1]["method"], "$/cancelRequest");
    assert_eq!(messages[1]["params"]["id"], original[0]["id"]);
    workspace_acceptance_assert_open(&model)?;
    let session = model.session.as_ref().ok_or("workspace")?;
    assert!(session.overlay_write.is_some());
    assert!(!session.pending_change);
    assert!(session.pending_save.is_some());
    assert!(session.diagnostic_pull.pending.is_none());
    assert!(
        !session.workspace_ready(),
        "observer must not fabricate a writer acknowledgement"
    );
    let change_sequence = session.overlay_write.ok_or("change sequence")?;
    // Drive the ownership transition with its actual admitted sequence. This
    // is a protocol control, not evidence that a physical child wrote bytes.
    assert!(
        model
            .session
            .as_mut()
            .ok_or("workspace")?
            .acknowledge_overlay(change_sequence)
    );
    let _ = model.poll(wake);
    let saved = workspace_acceptance_messages(&mut observer)?;
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0]["method"], "textDocument/didSave");
    let session = model.session.as_mut().ok_or("workspace")?;
    let save_sequence = session.overlay_write.ok_or("save sequence")?;
    assert_ne!(save_sequence, change_sequence);
    assert_eq!(
        session
            .pending_save
            .as_ref()
            .ok_or("save ownership")?
            .submitted,
        Some(save_sequence)
    );
    assert!(!session.acknowledge_overlay(change_sequence));
    assert!(!session.workspace_ready());
    assert!(session.acknowledge_overlay(save_sequence));
    assert!(session.pending_save.is_none());
    assert!(session.workspace_ready());
    let _ = model.poll(wake);
    let resumed = workspace_acceptance_messages(&mut observer)?;
    assert_eq!(resumed.len(), 1);
    assert_eq!(resumed[0]["method"], "textDocument/diagnostic");
    assert!(
        model
            .session
            .as_ref()
            .ok_or("workspace")?
            .diagnostic_pull
            .pending
            .is_some()
    );
    workspace_acceptance_assert_open(&model)?;
    let _ = model.stop();
    Ok(())
}

#[test]
fn workspace_acceptance_pressure_does_not_hide_a_closed_transport() -> Result<(), Box<dyn Error>> {
    let (mut model, _, _) = workspace_acceptance_ready_workspace()?;
    let observer = model
        .session
        .as_mut()
        .ok_or("workspace")?
        .client
        .take_input_observer_for_test()?;
    drop(observer);
    assert!(model.pump_diagnostics());
    assert!(!model.session.as_ref().ok_or("workspace")?.workspace_ready());
    assert!(
        model
            .status_message()
            .ok_or("closed transport hidden")?
            .contains("Closed")
    );
    let _ = model.stop();
    Ok(())
}
