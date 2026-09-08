use std::{error::Error, path::PathBuf, sync::Arc};

use serde_json::{Value, json, value::RawValue};

use super::super::{
    DiagnosticBatch, LanguageIdentity, LanguageProtocolError, LspDocument, PollCandidates,
    RustDiagnostics, RustDiagnosticsError, RustDocumentInput, SessionState,
    tests as diagnostic_tests,
};
use super::{DiagnosticKey, MAX_ATTEMPTS, PendingDiagnostic, PullState};
use crate::{
    lsp_framing::{LspFrameLimits, LspFramer},
    lsp_json::{LspPeer, PeerEvent},
    lsp_process::InputSequence,
};

fn raw(value: &Value) -> Result<Box<RawValue>, serde_json::Error> {
    RawValue::from_string(value.to_string())
}

fn byte_sized_wire(mut value: Value, bytes: usize) -> Result<Box<RawValue>, Box<dyn Error>> {
    let padding = bytes
        .checked_sub(raw(&value)?.get().len())
        .ok_or("fixture envelope exceeds requested byte size")?;
    // Multi-byte text distinguishes raw UTF-8 bytes from character counts.
    value["padding"] = Value::String("\u{00e9}".repeat(padding / 2) + &"x".repeat(padding % 2));
    let wire = raw(&value)?;
    assert_eq!(wire.get().len(), bytes);
    assert!(wire.get().chars().count() < bytes);
    Ok(wire)
}

fn installed() -> Result<(RustDiagnostics, RustDocumentInput, PathBuf), Box<dyn Error>> {
    let (root, path, snapshot, identity) = diagnostic_tests::fixture();
    let input = RustDocumentInput::new(&path, &root, identity, snapshot);
    let mut model = RustDiagnostics::default();
    model.install_for_test(
        input.clone(),
        &diagnostic_tests::diagnostics(&path, 1),
        diagnostic_tests::mock_executable(),
    )?;
    Ok((model, input, root))
}

#[test]
fn diagnostic_key_separates_dependency_and_process_identity_from_selection() {
    let identity = LanguageIdentity {
        workspace_id: 1,
        workspace_revision: 2,
        document_id: 3,
        document_revision: 4,
        buffer_revision: 5,
        selection_revision: 6,
    };
    let key = DiagnosticKey::new(identity, 7, 8, 9);
    for field in 0..7 {
        let mut changed = key;
        match field {
            0 => changed.workspace += 1,
            1 => changed.workspace_revision += 1,
            2 => changed.document += 1,
            3 => changed.buffer_revision += 1,
            4 => changed.overlay_epoch += 1,
            5 => changed.process_epoch += 1,
            _ => changed.lsp_version += 1,
        }
        assert_ne!(changed, key);
    }
    let mut moved = identity;
    moved.selection_revision += 1;
    moved.document_revision += 1;
    assert_eq!(DiagnosticKey::new(moved, 7, 8, 9), key);
}

#[test]
fn attempts_and_request_ownership_are_bounded_and_exhaustion_does_not_wrap()
-> Result<(), Box<dyn Error>> {
    let (mut model, input, root) = installed()?;
    let key = model.session.as_ref().ok_or("session")?.diagnostic_key();
    let mut state = PullState::default();
    for attempt in 0..10 {
        assert_eq!(state.admit_attempt(key), attempt < MAX_ATTEMPTS);
    }
    assert_eq!(state.attempts, MAX_ATTEMPTS);
    let stamp = input.identity.request_stamp().ok_or("stamp")?;
    state.pending = Some(PendingDiagnostic {
        request_id: 42,
        stamp,
        key,
    });
    assert!(state.take_matching(41).is_none());
    assert_eq!(state.pending.ok_or("request lost")?.request_id, 42);
    assert_eq!(state.invalidate()?, Some(42));
    assert!(state.pending.is_none());
    assert_eq!(state.attempts, 0);
    assert_eq!(state.epoch, 1);
    assert!(state.admit_attempt(key));
    state.settled = Some(key);
    assert!(!state.admit_attempt(key));
    state.epoch = u64::MAX;
    assert_eq!(
        state.invalidate(),
        Err(RustDiagnosticsError::GenerationExhausted)
    );
    assert_eq!(state.epoch, u64::MAX);
    let _ = model.shutdown();
    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn initialization_negotiates_inter_file_pull_without_assuming_an_empty_capability()
-> Result<(), Box<dyn Error>> {
    for (provider, expected) in [
        (
            json!({"interFileDependencies":true,"workspaceDiagnostics":false}),
            true,
        ),
        (
            json!({"interFileDependencies":true,"workspaceDiagnostics":true}),
            true,
        ),
        (
            json!({"interFileDependencies":false,"workspaceDiagnostics":false}),
            false,
        ),
        (
            json!({"interFileDependencies":"true","workspaceDiagnostics":false}),
            false,
        ),
        (json!({"interFileDependencies":true}), false),
        (json!(true), false),
        (Value::Null, false),
    ] {
        let mut peer = LspPeer::new();
        assert!(!peer.diagnostic_pull_supported());
        let initialize = peer.begin_initialize()?;
        let body = serde_json::to_vec(&json!({
            "jsonrpc":"2.0", "id": initialize.request_id(),
            "result":{"capabilities":{"diagnosticProvider":provider}}
        }))?;
        assert!(matches!(
            peer.receive(&body, None)?,
            PeerEvent::Initialized(_)
        ));
        assert_eq!(peer.diagnostic_pull_supported(), expected);
    }
    Ok(())
}

#[test]
fn full_reports_reuse_diagnostic_bounds_not_the_servers_constant_result_id()
-> Result<(), Box<dyn Error>> {
    let document =
        LspDocument::from_file_path(&std::env::temp_dir().join("alpine-pull.rs"), "rust", 7)?;
    let empty = raw(&json!({"kind":"full","resultId":"rust-analyzer","items":[]}))?;
    let accepted = DiagnosticBatch::admit_pull(&empty, &document)?;
    assert!(accepted.is_empty());
    assert_eq!(accepted.document_version(), Some(7));
    let item = json!({
        "range":{"start":{"line":0,"character":0},"end":{"line":0,"character":1}},
        "message":"dependency changed", "severity":1
    });
    let changed = raw(&json!({"kind":"full","resultId":"rust-analyzer","items":[item.clone()]}))?;
    assert_eq!(
        DiagnosticBatch::admit_pull(&changed, &document)?
            .diagnostics()
            .len(),
        1
    );
    for rejected in [
        json!({"kind":"unchanged","resultId":"rust-analyzer"}),
        json!({"items":[]}),
        json!({"kind":"full","items":null}),
        json!({"kind":"full","items":[],"relatedDocuments":{}}),
        Value::Null,
    ] {
        assert!(DiagnosticBatch::admit_pull(&raw(&rejected)?, &document).is_err());
    }
    let too_many = raw(&json!({"kind":"full","items":vec![item.clone();257]}))?;
    assert_eq!(
        DiagnosticBatch::admit_pull(&too_many, &document),
        Err(LanguageProtocolError::TooManyDiagnostics)
    );
    let mut large_item = item;
    large_item["message"] = Value::String("x".repeat(4097));
    let too_long = raw(&json!({"kind":"full","items":[large_item]}))?;
    assert_eq!(
        DiagnosticBatch::admit_pull(&too_long, &document),
        Err(LanguageProtocolError::DiagnosticMessageTooLong)
    );
    let too_large = raw(&json!({"kind":"full","items":[],"padding":"x".repeat(1_048_576)}))?;
    assert_eq!(
        DiagnosticBatch::admit_pull(&too_large, &document),
        Err(LanguageProtocolError::DiagnosticWireTooLarge)
    );
    Ok(())
}

#[test]
fn diagnostic_wire_boundary_accepts_the_exact_limit_without_retaining_padding()
-> Result<(), Box<dyn Error>> {
    let document =
        LspDocument::from_file_path(&std::env::temp_dir().join("pull-byte-limit.rs"), "rust", 7)?;
    let empty = DiagnosticBatch::admit_pull(&raw(&json!({"kind":"full","items":[]}))?, &document)?;
    let limit = 1_048_576;
    for bytes in [limit - 1, limit, limit + 1] {
        let wire = byte_sized_wire(json!({"kind":"full","items":[],"padding":""}), bytes)?;
        let result = DiagnosticBatch::admit_pull(&wire, &document);
        if bytes <= limit {
            let batch = result?;
            assert!(batch.is_empty());
            assert_eq!(batch.uri(), document.uri());
            assert_eq!(batch.document_version(), Some(7));
            assert_eq!(batch.retained_bytes(), empty.retained_bytes());
        } else {
            assert_eq!(result, Err(LanguageProtocolError::DiagnosticWireTooLarge));
        }
    }
    Ok(())
}

#[test]
fn diagnostic_wire_boundary_preserves_both_cancellation_directives_at_the_limit()
-> Result<(), Box<dyn Error>> {
    let limit = 1_048_576;
    for retrigger_request in [false, true] {
        for bytes in [limit - 1, limit, limit + 1] {
            let data = byte_sized_wire(
                json!({"retriggerRequest":retrigger_request,"padding":""}),
                bytes,
            )?;
            let source = format!(
                r#"{{"code":-32802,"message":"cancel","data":{}}}"#,
                data.get(),
            );
            let error = serde_json::from_str(&source)?;
            assert_eq!(
                LanguageProtocolError::from_diagnostic_error(error),
                if bytes <= limit {
                    LanguageProtocolError::DiagnosticServerCancelled { retrigger_request }
                } else {
                    LanguageProtocolError::DiagnosticWireTooLarge
                }
            );
        }
    }
    Ok(())
}

#[test]
fn stale_diagnostic_wire_response_refunds_only_its_matching_request() -> Result<(), Box<dyn Error>>
{
    let (mut model, root, pending) = pending_pull()?;
    let _ = drain_inputs(&mut model)?;
    let session = model.session.as_mut().ok_or("session")?;
    let attempts = session.diagnostic_pull.attempts;
    let other = session
        .client
        .begin_request("textDocument/hover", None, pending.stamp)?;
    assert_ne!(other.request_id, pending.request_id);
    session.client.cancel(other.request_id)?;
    let submitted = drain_inputs(&mut model)?;
    assert_eq!(submitted.len(), 2);
    assert_eq!(submitted[0]["method"], "textDocument/hover");
    assert_eq!(submitted[1]["method"], "$/cancelRequest");
    let stale_before = model.stale_diagnostics;
    assert!(!receive(
        &mut model,
        &framed(&json!({
            "jsonrpc":"2.0", "id":other.request_id, "result":null
        }))
    )?);
    let session = model.session.as_mut().ok_or("session")?;
    assert_eq!(
        session
            .diagnostic_pull
            .pending
            .ok_or("diagnostic request lost")?
            .request_id,
        pending.request_id
    );
    assert_eq!(session.diagnostic_pull.attempts, attempts);
    assert_eq!(model.stale_diagnostics, stale_before);
    // The peer now rejects the actual diagnostic response as cancelled. The
    // model must release that exact pending owner and refund only its attempt.
    session.client.cancel(pending.request_id)?;
    let submitted = drain_inputs(&mut model)?;
    assert_eq!(submitted.len(), 1);
    assert_eq!(submitted[0]["method"], "$/cancelRequest");
    assert!(!receive(&mut model, &response(pending.request_id))?);
    let session = model.session.as_ref().ok_or("session")?;
    assert!(session.diagnostic_pull.pending.is_none());
    assert!(session.diagnostics.is_none());
    assert_eq!(session.diagnostic_pull.attempts, attempts - 1);
    assert_eq!(model.stale_diagnostics, stale_before + 1);
    assert!(!model.pump_diagnostics());
    let session = model.session.as_ref().ok_or("session")?;
    let fresh = session
        .diagnostic_pull
        .pending
        .ok_or("retry not admitted")?;
    assert!(fresh.request_id > pending.request_id);
    assert_eq!(fresh.key, pending.key);
    assert_eq!(session.diagnostic_pull.attempts, attempts);
    let _ = model.shutdown();
    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn pending_responses_require_exact_request_overlay_process_and_document_authority()
-> Result<(), Box<dyn Error>> {
    let (mut model, input, root) = installed()?;
    let stamp = input.identity.request_stamp().ok_or("stamp")?;
    for mismatch in 0..5 {
        let session = model.session.as_mut().ok_or("session")?;
        session.diagnostics = None;
        let key = session.diagnostic_key();
        session.diagnostic_pull.pending = Some(PendingDiagnostic {
            request_id: 42,
            stamp,
            key,
        });
        match mismatch {
            0 => session.diagnostic_pull.epoch += 1,
            1 => session.process_epoch += 1,
            2 => session.identity.document_id += 1,
            3 => session.identity.buffer_revision += 1,
            _ => session.active_view = false,
        }
        let batch = DiagnosticBatch::admit(
            &diagnostic_tests::diagnostics(&input.path, 1),
            &session.document,
        )?;
        assert!(!model.admit_pull_diagnostics(42, stamp, Ok(batch)));
        assert!(
            model
                .session
                .as_ref()
                .ok_or("session")?
                .diagnostics
                .is_none()
        );
        model.session.as_mut().ok_or("session")?.active_view = true;
    }
    let session = model.session.as_mut().ok_or("session")?;
    let current_stamp = session.identity.request_stamp().ok_or("stamp")?;
    let key = session.diagnostic_key();
    session.diagnostic_pull.pending = Some(PendingDiagnostic {
        request_id: 43,
        stamp: current_stamp,
        key,
    });
    assert!(!model.admit_pull_diagnostics(42, stamp, Err(LanguageProtocolError::StaleDiagnostics)));
    let session = model.session.as_ref().ok_or("session")?;
    assert_eq!(
        session
            .diagnostic_pull
            .pending
            .ok_or("current request lost")?
            .request_id,
        43
    );
    let batch = DiagnosticBatch::admit(
        &diagnostic_tests::diagnostics(&input.path, 1),
        &session.document,
    )?;
    assert!(model.admit_pull_diagnostics(43, current_stamp, Ok(batch)));
    assert!(
        model
            .session
            .as_ref()
            .ok_or("session")?
            .diagnostics
            .is_some()
    );
    let _ = model.shutdown();
    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn an_inactive_dependency_edit_revokes_unchanged_active_diagnostics() -> Result<(), Box<dyn Error>>
{
    let (mut model, mut active, root) = installed()?;
    let mut buffer = alpine_text::Buffer::new("pub fn value() -> u32 { 1 }\n");
    let mut dependent = active.clone();
    dependent.path = root.join("a.rs");
    dependent.identity.document_id = 2;
    dependent.snapshot = buffer.snapshot();
    dependent.identity.buffer_revision = buffer.revision().get();
    model.session.as_mut().ok_or("session")?.overlay_write = Some(InputSequence::for_test(7));
    let _ = model.sync_workspace([active.clone(), dependent.clone()], Some(1), |_| {
        Arc::new(|| {})
    });
    let session = model.session.as_mut().ok_or("session")?;
    assert!(session.diagnostics.is_none());
    let batch = DiagnosticBatch::admit(
        &diagnostic_tests::diagnostics(&active.path, 1),
        &session.document,
    )?;
    assert!(model.admit(Ok(batch)));
    let mut edit = alpine_text::Transaction::new(buffer.revision());
    edit.replace(
        0..buffer.snapshot().len_bytes(),
        "pub fn value() -> &'static str { \"unsaved\" }\n",
    )?;
    let _ = buffer.apply(edit)?;
    dependent.snapshot = buffer.snapshot();
    dependent.identity.buffer_revision = buffer.revision().get();
    let epoch = model
        .session
        .as_ref()
        .ok_or("session")?
        .diagnostic_pull
        .epoch;
    let effect = model.sync_workspace([active.clone(), dependent.clone()], Some(1), |_| {
        Arc::new(|| {})
    });
    assert!(effect.visual_changed);
    let session = model.session.as_ref().ok_or("session")?;
    assert!(session.diagnostics.is_none());
    assert_eq!(session.lsp_version, 1);
    assert_eq!(session.identity.document_id, 1);
    assert_eq!(session.diagnostic_pull.epoch, epoch + 1);

    // The workspace is the invalidation owner for an admitted roster. An
    // active edit must not invalidate again inside single-document dispatch.
    let mut active_buffer = alpine_text::Buffer::new(&active.snapshot.text());
    let mut edit = alpine_text::Transaction::new(active_buffer.revision());
    edit.replace(
        0..active.snapshot.len_bytes(),
        "fn main() { let _ = value(); }\n",
    )?;
    let _ = active_buffer.apply(edit)?;
    active.snapshot = active_buffer.snapshot();
    active.identity.buffer_revision = active_buffer.revision().get();
    let epoch = session.diagnostic_pull.epoch;
    let roster = [active.clone(), dependent];
    let _ = model.sync_workspace(roster.clone(), Some(active.identity.document_id), |_| {
        Arc::new(|| {})
    });
    let session = model
        .session
        .as_ref()
        .ok_or("session lost during active edit")?;
    assert_eq!(
        session.diagnostic_pull.epoch,
        epoch + 1,
        "one admitted workspace edit must invalidate diagnostics exactly once"
    );
    assert_eq!(session.snapshot.text(), active.snapshot.text());
    assert_eq!(session.lsp_version, 2);
    assert_eq!(session.overlay_write, Some(InputSequence::for_test(7)));
    assert!(session.pending_change);
    assert!(session.diagnostics.is_none());

    let epoch = session.diagnostic_pull.epoch;
    let effect = model.sync_workspace(roster, Some(active.identity.document_id), |_| {
        Arc::new(|| {})
    });
    assert!(!effect.visual_changed);
    assert_eq!(
        model
            .session
            .as_ref()
            .ok_or("session")?
            .diagnostic_pull
            .epoch,
        epoch,
        "an unchanged roster must preserve diagnostic authority"
    );
    let _ = model.shutdown();
    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn ten_thousand_idle_polls_do_not_issue_diagnostic_requests_for_a_settled_view()
-> Result<(), Box<dyn Error>> {
    let (mut model, _, root) = installed()?;
    model
        .session
        .as_mut()
        .ok_or("session")?
        .diagnostic_pull
        .enabled = true;
    let before = model.snapshot().process_submitted_inputs;
    for _ in 0..10_000 {
        assert!(!model.pump_diagnostics());
    }
    assert_eq!(model.snapshot().process_submitted_inputs, before);
    assert!(
        model
            .session
            .as_ref()
            .ok_or("session")?
            .diagnostic_pull
            .pending
            .is_none()
    );
    let _ = model.shutdown();
    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn duplicate_diagnostic_fields_never_select_a_winning_interpretation() -> Result<(), Box<dyn Error>>
{
    let document = LspDocument::from_file_path(&std::env::temp_dir().join("unique.rs"), "rust", 1)?;
    for source in [
        r#"{"kind":"unchanged","kind":"full","items":[]}"#,
        r#"{"kind":"full","items":null,"items":[]}"#,
        r#"{"kind":"full","items":[],"\u0069tems":[]}"#,
        r#"{"kind":"full","items":[{"range":{"start":{"line":99,"line":0,"character":0},"end":{"line":0,"character":1}},"message":"x"}]}"#,
        r#"{"kind":"full","items":[{"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":1}},"message":"old","message":"new"}]}"#,
        r#"{"kind":"full","items":[],"relatedDocuments":null}"#,
    ] {
        let raw = RawValue::from_string(source.into())?;
        assert_eq!(
            DiagnosticBatch::admit_pull(&raw, &document),
            Err(LanguageProtocolError::MalformedDiagnostics)
        );
    }
    let uri = serde_json::to_string(document.uri())?;
    let push = RawValue::from_string(format!(
        r#"{{"uri":{uri},"version":0,"version":1,"diagnostics":[]}}"#
    ))?;
    assert_eq!(
        DiagnosticBatch::admit(&push, &document),
        Err(LanguageProtocolError::MalformedDiagnostics)
    );
    Ok(())
}

#[test]
fn named_provider_identity_is_negotiated_and_transmitted_by_the_client()
-> Result<(), Box<dyn Error>> {
    for identifier in [None, Some(""), Some("native\"diagnostics\\v1")] {
        let (mut model, _, root) = installed()?;
        let session = model.session.as_mut().ok_or("session")?;
        let initialize = session.client.begin_initialize()?;
        let mut provider = json!({"interFileDependencies":true,"workspaceDiagnostics":false});
        if let Some(identifier) = identifier {
            provider["identifier"] = Value::String(identifier.into());
        }
        session.client.inject_stdout_for_test(&framed(&json!({
            "jsonrpc":"2.0", "id":initialize.request_id,
            "result":{"capabilities":{"diagnosticProvider":provider}}
        })))?;
        let (_, candidates) = model.collect_poll_candidates()?;
        assert_eq!(candidates.initialized, Some(true));
        // The installed fixture already owns an open document. Exercise the
        // production negotiation and request paths, not a second synthetic open.
        let session = model.session.as_mut().ok_or("session")?;
        assert_eq!(session.client.diagnostic_provider_identifier(), identifier);
        session.diagnostic_pull.enabled = candidates.initialized.ok_or("initialization")?;
        session.diagnostics = None;
        assert!(!model.pump_diagnostics());
        let messages = drain_inputs(&mut model)?;
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0]["method"], "initialize");
        assert_eq!(messages[1]["method"], "initialized");
        assert_eq!(messages[2]["method"], "textDocument/diagnostic");
        assert_eq!(
            messages[2]["params"]
                .get("identifier")
                .and_then(Value::as_str),
            identifier
        );
        if identifier.is_none() {
            assert!(messages[2]["params"].get("identifier").is_none());
        }
        assert!(messages[2]["params"].get("previousResultId").is_none());
        let _ = model.shutdown();
        std::fs::remove_dir_all(root)?;
    }
    Ok(())
}

#[test]
fn provider_identity_is_bounded_accounted_and_ambiguous_capabilities_are_rejected()
-> Result<(), Box<dyn Error>> {
    let mut baseline = None;
    for (identifier, supported, bytes) in [
        (None, true, 0),
        (Some(json!("x".repeat(256))), true, 256),
        (Some(json!("x".repeat(257))), false, 0),
        (Some(Value::Null), false, 0),
        (Some(json!(42)), false, 0),
    ] {
        let mut peer = LspPeer::new();
        let initialize = peer.begin_initialize()?;
        let mut provider = json!({"interFileDependencies":true,"workspaceDiagnostics":false});
        if let Some(identifier) = identifier {
            provider["identifier"] = identifier;
        }
        let body = serde_json::to_vec(
            &json!({"jsonrpc":"2.0","id":initialize.request_id(),"result":{"capabilities":{"diagnosticProvider":provider}}}),
        )?;
        assert!(matches!(
            peer.receive(&body, None)?,
            PeerEvent::Initialized(_)
        ));
        assert_eq!(peer.diagnostic_pull_supported(), supported);
        let snapshot = peer.snapshot();
        let base = *baseline.get_or_insert(snapshot.retained_bytes());
        assert_eq!(snapshot.retained_bytes(), base + bytes);
        assert!(snapshot.peak_retained_bytes() >= snapshot.retained_bytes());
        assert_eq!(
            peer.diagnostic_provider_identifier().map_or(0, str::len),
            bytes
        );
    }
    for provider in [
        r#"{"interFileDependencies":false,"interFileDependencies":true,"workspaceDiagnostics":false}"#,
        r#"{"interFileDependencies":true,"workspaceDiagnostics":false,"identifier":"a","identifier":"b"}"#,
        r#"{"interFileDependencies":true,"workspaceDiagnostics":false,"identifier":"a","\u0069dentifier":"b"}"#,
    ] {
        let mut peer = LspPeer::new();
        let id = peer.begin_initialize()?.request_id().ok_or("initialize")?;
        let body = format!(
            r#"{{"jsonrpc":"2.0","id":{id},"result":{{"capabilities":{{"diagnosticProvider":{provider}}}}}}}"#
        );
        assert!(matches!(
            peer.receive(body.as_bytes(), None)?,
            PeerEvent::Initialized(_)
        ));
        assert!(!peer.diagnostic_pull_supported());
        assert!(peer.diagnostic_provider_identifier().is_none());
    }
    Ok(())
}

#[test]
fn server_cancellation_retry_directive_and_default_preserve_the_request_budget()
-> Result<(), Box<dyn Error>> {
    for data in [None, Some(json!({"retriggerRequest":true}))] {
        let (mut model, root, _) = pending_pull()?;
        let _ = drain_inputs(&mut model)?;
        let stale = model.stale_diagnostics;
        for attempt in 1..=MAX_ATTEMPTS {
            let pending = model
                .session
                .as_ref()
                .and_then(|s| s.diagnostic_pull.pending)
                .ok_or("retry")?;
            let mut error = json!({"code":-32802,"message":"server busy"});
            if let Some(data) = &data {
                error["data"] = data.clone();
            }
            receive(
                &mut model,
                &framed(&json!({"jsonrpc":"2.0","id":pending.request_id,"error":error})),
            )?;
            let session = model.session.as_ref().ok_or("session")?;
            assert_eq!(session.diagnostic_pull.attempts, attempt);
            assert_eq!(
                model.status_message().as_deref(),
                Some(if attempt == MAX_ATTEMPTS {
                    "Rust diagnostic retry budget exhausted; waiting for the next invalidation."
                } else {
                    "Rust diagnostics canceled by the server; bounded retry pending."
                })
            );
            assert!(session.diagnostic_pull.pending.is_none());
            assert!(session.diagnostics.is_none());
            assert_eq!(model.stale_diagnostics, stale);
            assert!(!model.pump_diagnostics());
            let messages = drain_inputs(&mut model)?;
            assert_eq!(messages.len(), usize::from(attempt < MAX_ATTEMPTS));
            if let Some(request) = messages.first() {
                assert_eq!(request["method"], "textDocument/diagnostic");
            }
        }
        let before = model.snapshot().process_submitted_inputs;
        for _ in 0..10_000 {
            assert!(!model.pump_diagnostics());
        }
        assert_eq!(model.snapshot().process_submitted_inputs, before);
        assert_eq!(model.restarts, 0);
        let _ = model.shutdown();
        std::fs::remove_dir_all(root)?;
    }
    Ok(())
}

#[test]
fn server_deferred_diagnostics_wait_for_invalidation_not_idle_polling() -> Result<(), Box<dyn Error>>
{
    let (mut model, root, pending) = pending_pull()?;
    let _ = drain_inputs(&mut model)?;
    let stale = model.stale_diagnostics;
    receive(
        &mut model,
        &framed(&json!({
            "jsonrpc":"2.0", "id":pending.request_id,
            "error":{"code":-32802,"message":"defer","data":{"retriggerRequest":false}}
        })),
    )?;
    let before = model.snapshot().process_submitted_inputs;
    for _ in 0..10_000 {
        assert!(!model.pump_diagnostics());
    }
    assert_eq!(model.snapshot().process_submitted_inputs, before);
    let session = model.session.as_ref().ok_or("session")?;
    assert_eq!(session.diagnostic_pull.attempts, 1);
    assert!(session.diagnostic_pull.pending.is_none());
    assert!(session.diagnostics.is_none());
    assert_eq!(model.stale_diagnostics, stale);
    let _ = model.refresh_diagnostics();
    assert!(!model.pump_diagnostics());
    let fresh = model
        .session
        .as_ref()
        .and_then(|s| s.diagnostic_pull.pending)
        .ok_or("refresh pull")?;
    assert!(fresh.request_id > pending.request_id);
    let messages = drain_inputs(&mut model)?;
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["method"], "textDocument/diagnostic");
    receive(&mut model, &response(fresh.request_id))?;
    assert!(
        model
            .session
            .as_ref()
            .ok_or("session")?
            .diagnostics
            .is_some()
    );
    assert_eq!(model.restarts, 0);
    let _ = model.shutdown();
    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn malformed_cancellation_data_never_becomes_a_server_retry_directive() -> Result<(), Box<dyn Error>>
{
    let document = LspDocument::from_file_path(&std::env::temp_dir().join("cancel.rs"), "rust", 1)?;
    for data in [
        "null",
        "{}",
        r#"{"retriggerRequest":"false"}"#,
        r#"{"retriggerRequest":true,"retriggerRequest":false}"#,
        r#"{"retriggerRequest":false,"\u0072etriggerRequest":true}"#,
    ] {
        let source = format!(r#"{{"code":-32802,"message":"cancel","data":{data}}}"#);
        let error = serde_json::from_str(&source)?;
        assert_eq!(
            super::batch_from_response(crate::lsp_json::ResponseValue::Error(error), &document),
            Err(LanguageProtocolError::MalformedDiagnostics)
        );
    }
    let other = serde_json::from_str(
        r#"{"code":-32603,"message":"internal","data":{"retriggerRequest":false}}"#,
    )?;
    assert_eq!(
        super::batch_from_response(crate::lsp_json::ResponseValue::Error(other), &document),
        Err(LanguageProtocolError::MalformedDiagnostics)
    );
    let large = json!({"code":-32802,"message":"cancel","data":{"retriggerRequest":true,"padding":"x".repeat(1_048_576)}}).to_string();
    let error = serde_json::from_str(&large)?;
    assert_eq!(
        super::batch_from_response(crate::lsp_json::ResponseValue::Error(error), &document),
        Err(LanguageProtocolError::DiagnosticWireTooLarge)
    );
    Ok(())
}

#[test]
fn terminal_peer_exit_drains_provider_and_cancelled_request_storage() -> Result<(), Box<dyn Error>>
{
    for identifier in [String::new(), "rust-analyzer".into(), "x".repeat(256)] {
        for cancel_request in [false, true] {
            let mut peer = LspPeer::new();
            let initialize = peer.begin_initialize()?;
            let body = serde_json::to_vec(&json!({
                "jsonrpc":"2.0", "id":initialize.request_id(),
                "result":{"capabilities":{"diagnosticProvider":{
                    "identifier":identifier,"interFileDependencies":true,"workspaceDiagnostics":false
                }}}
            }))?;
            assert!(matches!(
                peer.receive(&body, None)?,
                PeerEvent::Initialized(_)
            ));
            assert!(peer.diagnostic_pull_supported());
            assert_eq!(
                peer.diagnostic_provider_identifier(),
                Some(identifier.as_str())
            );
            if cancel_request {
                let stamp = crate::lsp_json::RequestStamp::new(1, 1, 1, 1, 0, 1).ok_or("stamp")?;
                let request = peer.begin_request("textDocument/diagnostic", None, stamp)?;
                let _ = peer.cancel(request.request_id().ok_or("request")?)?;
                assert_eq!(peer.snapshot().cancelled_requests(), 1);
            }
            let shutdown = peer.begin_shutdown()?;
            let body = serde_json::to_vec(
                &json!({"jsonrpc":"2.0","id":shutdown.request_id(),"result":null}),
            )?;
            assert!(matches!(
                peer.receive(&body, None)?,
                PeerEvent::ShutdownAcknowledged
            ));
            let before = peer.snapshot();
            assert!(before.retained_bytes() > identifier.len());
            let exit = peer.exit()?;
            let body: Value = serde_json::from_slice(exit.body())?;
            assert_eq!(body["method"], "exit");
            assert!(!peer.diagnostic_pull_supported());
            assert!(peer.diagnostic_provider_identifier().is_none());
            assert_eq!(peer.snapshot().retained_bytes(), 0);
            assert_eq!(
                peer.snapshot().peak_retained_bytes(),
                before.peak_retained_bytes()
            );
        }
    }
    Ok(())
}

fn pending_pull() -> Result<(RustDiagnostics, PathBuf, PendingDiagnostic), Box<dyn Error>> {
    let (mut model, _, root) = installed()?;
    let session = model.session.as_mut().ok_or("session")?;
    session.client.initialize_inert_for_test();
    session.diagnostics = None;
    session.diagnostic_pull.enabled = true;
    assert!(!model.pump_diagnostics());
    let pending = model
        .session
        .as_ref()
        .and_then(|session| session.diagnostic_pull.pending)
        .ok_or("initial pull")?;
    Ok((model, root, pending))
}

fn framed(value: &Value) -> Vec<u8> {
    let body = value.to_string();
    format!("Content-Length: {}\r\n\r\n{body}", body.len()).into_bytes()
}

fn response(id: u32) -> Vec<u8> {
    framed(&json!({"jsonrpc":"2.0", "id":id, "result":{"kind":"full","items":[]}}))
}

fn receive(model: &mut RustDiagnostics, bytes: &[u8]) -> Result<bool, Box<dyn Error>> {
    model
        .session
        .as_mut()
        .ok_or("session")?
        .client
        .inject_stdout_for_test(bytes)?;
    let (_, mut candidates) = model.collect_poll_candidates()?;
    Ok(model.apply_diagnostic_candidates(&mut candidates))
}

fn drain_inputs(model: &mut RustDiagnostics) -> Result<Vec<Value>, Box<dyn Error>> {
    let client = &mut model.session.as_mut().ok_or("session")?.client;
    let mut messages = Vec::new();
    while let Some(bytes) = client.take_input_for_test()? {
        let mut framer = LspFramer::new(LspFrameLimits::default());
        let batch = framer.ingest(&bytes)?;
        assert_eq!(batch.consumed(), bytes.len());
        assert_eq!(batch.frames().len(), 1);
        messages.push(serde_json::from_slice(batch.frames()[0].body())?);
        framer.finish()?;
    }
    Ok(messages)
}

#[test]
fn coalesced_response_and_refresh_retire_completed_ownership_in_both_wire_orders()
-> Result<(), Box<dyn Error>> {
    for refresh_first in [false, true] {
        let (mut model, root, pending) = pending_pull()?;
        let epoch = model
            .session
            .as_ref()
            .ok_or("session")?
            .diagnostic_pull
            .epoch;
        let completed = response(pending.request_id);
        let refresh = framed(&json!({
            "jsonrpc":"2.0", "id":900, "method":"workspace/diagnostic/refresh"
        }));
        let bytes = if refresh_first {
            [refresh, completed].concat()
        } else {
            [completed, refresh].concat()
        };
        receive(&mut model, &bytes)?;
        let session = model.session.as_ref().ok_or("session lost")?;
        assert_eq!(model.restarts, 0);
        assert_eq!(session.process_epoch, 1);
        assert_eq!(session.diagnostic_pull.epoch, epoch + 1);
        assert_eq!(session.client.snapshot().peer.pending_requests(), 0);
        assert!(session.diagnostic_pull.pending.is_none());
        assert!(session.diagnostics.is_none());
        assert_eq!(model.stale_diagnostics, 1);

        assert!(!model.pump_diagnostics());
        let session = model.session.as_ref().ok_or("session")?;
        let fresh = session.diagnostic_pull.pending.ok_or("fresh pull")?;
        assert!(fresh.request_id > pending.request_id);
        assert_eq!(session.client.snapshot().peer.pending_requests(), 1);
        assert_eq!(session.diagnostic_pull.attempts, 1);
        receive(&mut model, &response(fresh.request_id))?;
        let before = model.snapshot().process_submitted_inputs;
        assert!(!model.pump_diagnostics());
        assert_eq!(model.snapshot().process_submitted_inputs, before);
        assert!(
            model
                .session
                .as_ref()
                .ok_or("session")?
                .diagnostics
                .is_some()
        );
        let _ = model.shutdown();
        std::fs::remove_dir_all(root)?;
    }
    Ok(())
}

#[test]
fn repeated_view_cancellation_preserves_a_stable_targets_retry_budget() -> Result<(), Box<dyn Error>>
{
    let (mut model, root, _) = pending_pull()?;
    let initial = drain_inputs(&mut model)?;
    assert_eq!(initial.len(), 2);
    assert_eq!(initial[0]["method"], "initialize");
    assert_eq!(initial[1]["method"], "textDocument/diagnostic");
    let mut interrupted = Vec::new();
    for _ in 0..MAX_ATTEMPTS + 2 {
        let session = model.session.as_mut().ok_or("session")?;
        interrupted.push(
            session
                .diagnostic_pull
                .pending
                .ok_or("pending pull")?
                .request_id,
        );
        assert_eq!(session.diagnostic_pull.attempts, 1);
        session.active_view = false;
        assert!(!model.pump_diagnostics());
        let cancelled = drain_inputs(&mut model)?;
        assert_eq!(cancelled.len(), 1);
        assert_eq!(cancelled[0]["method"], "$/cancelRequest");
        assert_eq!(
            cancelled[0]["params"]["id"].as_u64(),
            interrupted.last().copied().map(u64::from)
        );
        let session = model.session.as_mut().ok_or("session")?;
        assert!(session.diagnostic_pull.pending.is_none());
        assert_eq!(session.diagnostic_pull.attempts, 0);
        session.active_view = true;
        assert!(!model.pump_diagnostics());
        let requested = drain_inputs(&mut model)?;
        assert_eq!(requested.len(), 1);
        assert_eq!(requested[0]["method"], "textDocument/diagnostic");
    }
    let fresh = model
        .session
        .as_ref()
        .and_then(|session| session.diagnostic_pull.pending)
        .ok_or("stable view permanently starved")?;
    for id in interrupted {
        receive(&mut model, &response(id))?;
        let session = model.session.as_ref().ok_or("session")?;
        assert_eq!(
            session
                .diagnostic_pull
                .pending
                .ok_or("fresh request lost")?
                .request_id,
            fresh.request_id
        );
        assert_eq!(session.diagnostic_pull.attempts, 1);
        assert!(session.diagnostics.is_none());
    }
    receive(&mut model, &response(fresh.request_id))?;
    assert!(
        model
            .session
            .as_ref()
            .ok_or("session")?
            .diagnostics
            .is_some()
    );
    assert_eq!(model.restarts, 0);
    let _ = model.shutdown();
    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn malformed_replies_still_exhaust_the_bounded_failure_budget() -> Result<(), Box<dyn Error>> {
    let (mut model, root, _) = pending_pull()?;
    for attempt in 1..=MAX_ATTEMPTS {
        let session = model.session.as_ref().ok_or("session")?;
        let pending = session.diagnostic_pull.pending.ok_or("bounded retry")?;
        receive(
            &mut model,
            &framed(&json!({
                "jsonrpc":"2.0", "id":pending.request_id,
                "result":{"kind":"unchanged","resultId":"not-requested"}
            })),
        )?;
        assert_eq!(
            model
                .session
                .as_ref()
                .ok_or("session")?
                .diagnostic_pull
                .attempts,
            attempt
        );
        assert!(!model.pump_diagnostics());
    }
    let before = model.snapshot().process_submitted_inputs;
    for _ in 0..10 {
        assert!(!model.pump_diagnostics());
    }
    let session = model.session.as_ref().ok_or("session")?;
    assert!(session.diagnostic_pull.pending.is_none());
    assert!(session.diagnostics.is_none());
    assert_eq!(session.diagnostic_pull.attempts, MAX_ATTEMPTS);
    assert_eq!(model.snapshot().process_submitted_inputs, before);
    let _ = model.shutdown();
    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn initialization_reports_each_independent_status_change() -> Result<(), Box<dyn Error>> {
    for supported in [false, true] {
        for had_status in [false, true] {
            let (mut model, _, root) = installed()?;
            let session = model.session.as_mut().ok_or("session")?;
            session.client.initialize_inert_for_test();
            session.state = SessionState::Initializing;
            session.diagnostics = None;
            let _ = drain_inputs(&mut model)?;
            model.status = had_status.then(|| Arc::from("Rust analysis is initializing."));
            let mut candidates = PollCandidates {
                initialized: Some(supported),
                ..PollCandidates::default()
            };
            assert_eq!(
                model.apply_diagnostic_candidates(&mut candidates),
                had_status || !supported
            );
            let session = model.session.as_ref().ok_or("session")?;
            assert_eq!(session.state, SessionState::Open);
            assert_eq!(session.diagnostic_pull.enabled, supported);
            assert_eq!(
                model.status.as_deref(),
                (!supported)
                    .then_some("Rust diagnostics require inter-file pull diagnostic support.")
            );
            let messages = drain_inputs(&mut model)?;
            assert_eq!(messages.len(), 1);
            assert_eq!(messages[0]["method"], "textDocument/didOpen");
            assert!(!model.apply_diagnostic_candidates(&mut PollCandidates::default()));
            let _ = model.shutdown();
            std::fs::remove_dir_all(root)?;
        }
    }
    Ok(())
}

#[test]
fn wire_report_and_refresh_propagate_visual_changes_then_become_quiet() -> Result<(), Box<dyn Error>>
{
    let (mut model, root, pending) = pending_pull()?;
    let report = framed(&json!({
        "jsonrpc":"2.0", "id":pending.request_id,
        "result":{"kind":"full","items":[{
            "range":{"start":{"line":0,"character":0},"end":{"line":0,"character":1}},
            "message":"current correlated diagnostic", "severity":1
        }]}
    }));
    assert!(receive(&mut model, &report)?);
    assert_eq!(model.snapshot().diagnostic_items, 1);
    assert_eq!(model.snapshot().diagnostic_version, Some(1));
    assert!(model.status.is_some());
    for (id, changed) in [(900, true), (901, false)] {
        let epoch = model
            .session
            .as_ref()
            .ok_or("session")?
            .diagnostic_pull
            .epoch;
        let refresh = framed(&json!({
            "jsonrpc":"2.0", "id":id, "method":"workspace/diagnostic/refresh"
        }));
        assert_eq!(receive(&mut model, &refresh)?, changed);
        let session = model.session.as_ref().ok_or("session")?;
        assert_eq!(session.diagnostic_pull.epoch, epoch + 1);
        assert!(session.diagnostics.is_none());
        assert!(model.status.is_none());
    }
    assert_eq!(model.restarts, 0);
    let _ = model.shutdown();
    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn refresh_invalidates_diagnostics_without_dismissing_visible_completion()
-> Result<(), Box<dyn Error>> {
    for mask in 0_u8..8 {
        let (mut model, input, root) = installed()?;
        if mask & 2 == 0 {
            model.session.as_mut().ok_or("session")?.diagnostics = None;
        }
        if mask & 4 != 0 {
            model.install_completion_for_test(
                42,
                input.identity,
                &raw(&json!([{"label":"visible completion"}]))?,
            )?;
        }
        model.status = (mask & 1 != 0).then(|| Arc::from("old diagnostic status"));
        assert_eq!(model.status.is_some(), mask & 1 != 0);
        assert_eq!(model.snapshot().diagnostic_items > 0, mask & 2 != 0);
        assert_eq!(model.snapshot().completion_items > 0, mask & 4 != 0);
        assert_eq!(model.refresh_diagnostics(), mask & 3 != 0);
        let session = model.session.as_ref().ok_or("session")?;
        assert!(session.diagnostics.is_none());
        assert_eq!(session.completion.is_some(), mask & 4 != 0);
        assert!(model.status.is_none());
        assert!(!model.refresh_diagnostics());
        assert_eq!(model.restarts, 0);
        let _ = model.shutdown();
        std::fs::remove_dir_all(root)?;
    }
    Ok(())
}

#[test]
fn unsolicited_native_push_cannot_bypass_correlated_pull_admission() -> Result<(), Box<dyn Error>> {
    let (mut model, root, pending) = pending_pull()?;
    let stale = model.stale_diagnostics;
    let uri = model.session.as_ref().ok_or("session")?.document.uri();
    let push = framed(&json!({
        "jsonrpc":"2.0", "method":"textDocument/publishDiagnostics",
        "params":{"uri":uri,"version":1,"diagnostics":[{
            "range":{"start":{"line":0,"character":0},"end":{"line":0,"character":1}},
            "message":"unsolicited native report", "severity":1, "source":"rust-analyzer"
        }]}
    }));
    assert!(!receive(&mut model, &push)?);
    let session = model.session.as_ref().ok_or("session")?;
    assert!(session.diagnostics.is_none());
    assert_eq!(
        session
            .diagnostic_pull
            .pending
            .ok_or("pull lost")?
            .request_id,
        pending.request_id
    );
    assert_eq!(model.stale_diagnostics, stale + 1);
    assert!(receive(&mut model, &response(pending.request_id))?);
    assert_eq!(model.snapshot().diagnostic_version, Some(1));
    assert_eq!(model.snapshot().diagnostic_items, 0);
    assert_eq!(model.restarts, 0);
    let _ = model.shutdown();
    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn server_diagnostic_refresh_preserves_an_active_workspace_symbol_query()
-> Result<(), Box<dyn Error>> {
    use crate::rust_symbols::SymbolRequestKind;

    let (mut model, root, diagnostic) = pending_pull()?;
    let identity = model.session.as_ref().ok_or("session")?.identity;
    let _ = drain_inputs(&mut model)?;
    assert!(
        model
            .open_symbols(SymbolRequestKind::Workspace)
            .visual_changed
    );
    let _ = drain_inputs(&mut model)?;
    assert!(
        model
            .commit_symbol_text(identity, "deliberately_invalid")
            .visual_changed
    );
    let _ = drain_inputs(&mut model)?;
    let session = model.session.as_ref().ok_or("session")?;
    let pending_symbol = session.pending_symbols.ok_or("pending symbol query")?;
    let diagnostic_epoch = session.diagnostic_pull.epoch;
    assert!(model.symbols_are_open(identity));
    assert!(model.snapshot().symbol_pending());
    let cancellations = model.snapshot().symbol_cancellations;

    let refresh = framed(&json!({
        "jsonrpc":"2.0", "id":900, "method":"workspace/diagnostic/refresh"
    }));
    let visual_changed = receive(&mut model, &refresh)?;
    let messages = drain_inputs(&mut model)?;
    let session = model.session.as_ref().ok_or("session")?;
    assert_eq!(session.diagnostic_pull.epoch, diagnostic_epoch + 1);
    assert!(session.diagnostic_pull.pending.is_none());
    assert!(session.diagnostics.is_none());
    assert!(messages.iter().any(|message| {
        message["method"] == "$/cancelRequest"
            && message["params"]["id"].as_u64() == Some(u64::from(diagnostic.request_id))
    }));
    assert!(
        model.symbols_are_open(identity),
        "diagnostic-only refresh dismissed the symbol query: visual={visual_changed}, \
         pending={:?}, cancellations={cancellations}->{}, wire={messages:?}",
        session.pending_symbols,
        model.snapshot().symbol_cancellations,
    );
    let retained = session
        .pending_symbols
        .ok_or("symbol request was canceled")?;
    assert_eq!(retained.request_id, pending_symbol.request_id);
    assert_eq!(retained.stamp, pending_symbol.stamp);
    assert_eq!(retained.query_revision, pending_symbol.query_revision);
    assert_eq!(model.snapshot().symbol_cancellations, cancellations);
    assert!(!messages.iter().any(|message| {
        message["method"] == "$/cancelRequest"
            && message["params"]["id"].as_u64() == Some(u64::from(pending_symbol.request_id))
    }));
    assert!(!visual_changed);
    assert_eq!(model.restarts, 0);
    let _ = model.shutdown();
    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn diagnostic_refresh_preserves_other_pending_language_requests() -> Result<(), Box<dyn Error>> {
    use super::super::{LspPosition, NavigationRequestKind};
    use crate::rust_symbols::SymbolRequestKind;

    for kind in 0..4 {
        let (mut model, root, _) = pending_pull()?;
        let _ = drain_inputs(&mut model)?;
        let position = LspPosition::new(0, 0)?;
        let _ = match kind {
            0 => model.request_completion(position),
            1 => model.request_navigation(NavigationRequestKind::Hover, position),
            2 => model.open_symbols(SymbolRequestKind::Workspace),
            _ => model.request_formatting(4, true),
        };
        let _ = drain_inputs(&mut model)?;
        let before = model.snapshot();
        assert_eq!(before.completion_pending(), kind == 0);
        assert_eq!(before.navigation_pending(), kind == 1);
        assert_eq!(before.symbol_pending(), kind == 2);
        assert_eq!(before.workspace_edit_pending, kind == 3);
        let status_before = model.status.clone();
        assert_eq!(
            status_before.is_some(),
            kind == 3,
            "request kind={kind}, status={status_before:?}"
        );
        let refresh = framed(&json!({
            "jsonrpc":"2.0", "id":900, "method":"workspace/diagnostic/refresh"
        }));
        let visual_changed = receive(&mut model, &refresh)?;
        let after = model.snapshot();
        assert_eq!(after.completion_pending(), before.completion_pending());
        assert_eq!(after.navigation_pending(), before.navigation_pending());
        assert_eq!(after.symbol_pending(), before.symbol_pending());
        assert_eq!(after.workspace_edit_pending, before.workspace_edit_pending);
        assert_eq!(
            after.completion_cancellations,
            before.completion_cancellations
        );
        assert_eq!(
            after.navigation_cancellations,
            before.navigation_cancellations
        );
        assert_eq!(after.symbol_cancellations, before.symbol_cancellations);
        assert_eq!(
            after.workspace_edit_cancellations,
            before.workspace_edit_cancellations
        );
        let session = model.session.as_ref().ok_or("session")?;
        assert_eq!(session.client.snapshot().peer.pending_requests(), 1);
        assert!(session.diagnostic_pull.pending.is_none());
        assert!(model.status.is_none());
        assert_eq!(
            visual_changed,
            status_before.is_some(),
            "request kind={kind}, previous status={status_before:?}, before={before:?}, after={after:?}"
        );
        // Refresh retains existing shared-status clearing, which can require
        // one redraw. That is not cancellation of the outstanding request.
        let repeated = framed(&json!({
            "jsonrpc":"2.0", "id":901, "method":"workspace/diagnostic/refresh"
        }));
        assert!(!receive(&mut model, &repeated)?);
        assert_eq!(
            model
                .session
                .as_ref()
                .ok_or("session")?
                .client
                .snapshot()
                .peer
                .pending_requests(),
            1
        );
        assert_eq!(model.restarts, 0);
        let _ = model.shutdown();
        std::fs::remove_dir_all(root)?;
    }
    Ok(())
}

#[test]
fn diagnostic_refresh_preserves_admitted_hover_and_symbol_results() -> Result<(), Box<dyn Error>> {
    use super::super::NavigationRequestKind;
    use crate::rust_symbols::SymbolRequestKind;

    for symbols in [false, true] {
        let (mut model, input, root) = installed()?;
        if symbols {
            let uri = model.session.as_ref().ok_or("session")?.document.uri();
            let result = raw(&json!([{
                "name":"visible_symbol", "kind":12,
                "location":{"uri":uri,"range":{
                    "start":{"line":0,"character":0},
                    "end":{"line":0,"character":1}
                }}
            }]))?;
            model.install_symbols_for_test(
                input.identity,
                SymbolRequestKind::Workspace,
                &result,
            )?;
            assert!(model.selected_symbol_location(input.identity).is_some());
        } else {
            model.install_navigation_for_test(
                input.identity,
                NavigationRequestKind::Hover,
                &raw(&json!({"contents":{"kind":"plaintext","value":"visible hover"}}))?,
            )?;
            assert!(model.hover_content(input.identity).is_some());
        }
        let before = model.snapshot();
        assert!(model.refresh_diagnostics());
        let after = model.snapshot();
        assert_eq!(after.hover_bytes, before.hover_bytes);
        assert_eq!(after.symbol_items, before.symbol_items);
        assert_eq!(after.symbol_matches, before.symbol_matches);
        assert_eq!(after.symbol_bytes, before.symbol_bytes);
        assert_eq!(
            model.selected_symbol_location(input.identity).is_some(),
            symbols
        );
        assert_eq!(model.hover_content(input.identity).is_some(), !symbols);
        assert!(!model.refresh_diagnostics());
        let _ = model.shutdown();
        std::fs::remove_dir_all(root)?;
    }
    Ok(())
}

#[test]
fn symbol_response_and_diagnostic_refresh_preserve_results_in_both_wire_orders()
-> Result<(), Box<dyn Error>> {
    use super::super::LanguageWake;
    use crate::rust_symbols::SymbolRequestKind;

    for refresh_first in [false, true] {
        let (mut model, root, _) = pending_pull()?;
        let _ = drain_inputs(&mut model)?;
        let identity = model.session.as_ref().ok_or("session")?.identity;
        assert!(
            model
                .open_symbols(SymbolRequestKind::Workspace)
                .visual_changed
        );
        let _ = drain_inputs(&mut model)?;
        assert!(
            model
                .commit_symbol_text(identity, "visible_symbol")
                .visual_changed
        );
        let _ = drain_inputs(&mut model)?;
        let session = model.session.as_ref().ok_or("session")?;
        let request_id = session.pending_symbols.ok_or("symbol query")?.request_id;
        let wake = LanguageWake {
            generation: session.generation,
        };
        let completed = framed(&json!({
            "jsonrpc":"2.0", "id":request_id, "result":[{
                "name":"visible_symbol", "kind":12,
                "location":{"uri":session.document.uri(),"range":{
                    "start":{"line":0,"character":0},
                    "end":{"line":0,"character":1}
                }}
            }]
        }));
        let refresh = framed(&json!({
            "jsonrpc":"2.0", "id":900, "method":"workspace/diagnostic/refresh"
        }));
        let bytes = if refresh_first {
            [refresh, completed].concat()
        } else {
            [completed, refresh].concat()
        };
        model
            .session
            .as_mut()
            .ok_or("session")?
            .client
            .inject_stdout_for_test(&bytes)?;
        assert!(model.poll(wake).visual_changed);
        assert!(model.symbols_are_open(identity));
        assert!(model.selected_symbol_location(identity).is_some());
        let snapshot = model.snapshot();
        assert_eq!(snapshot.symbol_items, 1);
        assert_eq!(snapshot.symbol_matches, 1);
        assert!(!snapshot.symbol_pending());
        assert_eq!(snapshot.stale_symbols, 0);
        let session = model.session.as_ref().ok_or("session")?;
        assert!(session.diagnostics.is_none());
        assert!(session.diagnostic_pull.pending.is_some());
        assert_eq!(model.restarts, 0);
        let _ = model.shutdown();
        std::fs::remove_dir_all(root)?;
    }
    Ok(())
}
