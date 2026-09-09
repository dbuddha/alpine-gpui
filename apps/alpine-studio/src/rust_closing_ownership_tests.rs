//! Closing-text budgets and writer acknowledgement state controls.

use super::*;

#[test]
fn closing_saved_notifications_account_for_each_owner_identity_and_parked_removal()
-> Result<(), Box<dyn Error>> {
    let (mut model, mut active, root) = installed_workspace()?;
    active.snapshot = alpine_text::Buffer::new("active").snapshot();
    let mut parked = active.clone();
    parked.identity.document_id = 2;
    parked.path = root.join("parked.rs");
    parked.snapshot = alpine_text::Buffer::new("parked document").snapshot();
    let active_bytes = active.snapshot.len_bytes() * 2;
    let parked_bytes = parked.snapshot.len_bytes() * 2;
    assert_ne!(active_bytes, parked_bytes);
    let session = model.session.as_mut().ok_or("workspace")?;
    session.snapshot = active.snapshot.clone();
    session.synced_snapshot = active.snapshot.clone();
    session.pending_change = true;
    session.pending_save = Some(PendingSave {
        buffer_revision: session.identity.buffer_revision,
        submitted: None,
    });
    let mut parked_owner = ParkedDocument::new(parked.clone())?;
    parked_owner.pending_save = Some(PendingSave {
        buffer_revision: parked.identity.buffer_revision,
        submitted: None,
    });
    session.parked.push(parked_owner);
    assert_eq!(
        session.prospective_closed_text_reservation(&[active.clone(), parked.clone()]),
        0,
    );
    assert_eq!(
        session.prospective_closed_text_reservation(&[active.clone()]),
        parked_bytes
    );
    assert_eq!(
        session.prospective_closed_text_reservation(&[parked.clone()]),
        active_bytes
    );
    assert_eq!(
        session.prospective_closed_text_reservation(&[]),
        active_bytes + parked_bytes
    );
    let mut wrong_id = active.clone();
    wrong_id.identity.document_id = 3;
    let mut wrong_path = active;
    wrong_path.path = root.join("different.rs");
    for replacement in [wrong_id, wrong_path] {
        assert_eq!(
            session.prospective_closed_text_reservation(&[replacement, parked.clone()]),
            active_bytes,
            "both the document ID and path must identify the retained active owner",
        );
    }
    assert!(!model.shutdown().active);
    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn closing_saved_notifications_keep_text_save_close_and_reopen_order() -> Result<(), Box<dyn Error>>
{
    for opened in [false, true] {
        let (mut model, input, root) = installed_workspace()?;
        model.initialize_installed_transport_for_test()?;
        let mut next = input.clone();
        next.path = root.join("b.rs");
        next.identity.document_id = 2;
        let session = model.session.as_mut().ok_or("workspace")?;
        consume_closing_input(&mut session.client, "initialize", None)?;
        let closing_uri = session.document.uri().to_owned();
        session.document_opened = opened;
        session.pending_change = opened;
        session.pending_save = Some(PendingSave {
            buffer_revision: input.identity.buffer_revision,
            submitted: None,
        });
        let expected = session.retained_overlay_text_bytes();
        session.park_and_activate(next, false)?;
        let reopened_uri = session.document.uri().to_owned();
        assert_eq!(session.overlay_closes.len(), 1);
        assert_eq!(session.overlay_closes[0].retained_text_bytes(), expected);
        assert!(session.flush_overlay()?);
        let text = session.overlay_write.ok_or("text write")?;
        assert!(session.overlay_closes[0].opened);
        assert_eq!(session.overlay_closes[0].retained_text_bytes(), 0);
        assert!(session.overlay_closes[0].pending_save.is_some());
        assert!(!session.flush_overlay()?);
        consume_closing_input(
            &mut session.client,
            if opened {
                "textDocument/didChange"
            } else {
                "textDocument/didOpen"
            },
            Some(&closing_uri),
        )?;
        assert!(session.acknowledge_overlay(text));
        assert!(session.flush_overlay()?);
        let save = session.overlay_write.ok_or("save write")?;
        assert_ne!(text, save);
        assert_eq!(
            session.overlay_closes[0]
                .pending_save
                .as_ref()
                .ok_or("save")?
                .submitted,
            Some(save),
        );
        assert!(!session.acknowledge_overlay(text));
        assert!(session.overlay_closes[0].pending_save.is_some());
        assert!(!session.flush_overlay()?);
        consume_closing_input(
            &mut session.client,
            "textDocument/didSave",
            Some(&closing_uri),
        )?;
        assert!(session.acknowledge_overlay(save));
        assert!(session.overlay_closes[0].pending_save.is_none());
        assert!(session.flush_overlay()?);
        let close = session.overlay_write.ok_or("close write")?;
        assert_ne!(save, close);
        assert!(session.overlay_closes.is_empty());
        assert!(!session.document_opened);
        assert!(!session.flush_overlay()?);
        assert!(!session.acknowledge_overlay(save));
        consume_closing_input(
            &mut session.client,
            "textDocument/didClose",
            Some(&closing_uri),
        )?;
        assert!(session.acknowledge_overlay(close));
        assert!(session.flush_overlay()?);
        assert!(session.document_opened);
        consume_closing_input(
            &mut session.client,
            "textDocument/didOpen",
            Some(&reopened_uri),
        )?;
        assert!(session.client.take_input_for_test()?.is_none());
        assert_eq!(session.client.snapshot().process.retained_bytes, 0);
        assert!(!model.shutdown().active);
        std::fs::remove_dir_all(root)?;
    }
    Ok(())
}

// This is a state-transition control. Consume actual queued bytes before its
// manual acknowledgement, but do not fabricate process writer-success counts.
fn consume_closing_input(
    client: &mut crate::lsp_client::LspClient,
    method: &str,
    uri: Option<&str>,
) -> Result<(), Box<dyn Error>> {
    let bytes = client
        .take_input_for_test()?
        .ok_or("missing closing input")?;
    let mut framer =
        crate::lsp_framing::LspFramer::new(crate::lsp_framing::LspFrameLimits::default());
    let batch = framer.ingest(&bytes)?;
    assert_eq!(batch.consumed(), bytes.len());
    assert_eq!(batch.frames().len(), 1);
    let message: serde_json::Value = serde_json::from_slice(batch.frames()[0].body())?;
    framer.finish()?;
    assert_eq!(message["method"], method);
    if let Some(uri) = uri {
        assert_eq!(message["params"]["textDocument"]["uri"], uri);
    }
    Ok(())
}

#[test]
fn closing_saved_notifications_share_the_existing_text_budget_and_drain_on_reset()
-> Result<(), Box<dyn Error>> {
    let (mut model, mut input, root) = installed_workspace()?;
    let snapshot = alpine_text::Buffer::new(&"x".repeat(MAX_DOCUMENT_BYTES)).snapshot();
    input.snapshot = snapshot.clone();
    let session = model.session.as_mut().ok_or("workspace")?;
    session.snapshot = snapshot.clone();
    session.synced_snapshot = snapshot;
    session.pending_change = true;
    for document_id in [2, 3] {
        session.pending_save = Some(PendingSave {
            buffer_revision: session.identity.buffer_revision,
            submitted: None,
        });
        let mut next = input.clone();
        next.path = root.join(format!("{document_id}.rs"));
        next.identity.document_id = document_id;
        session.park_and_activate(next, false)?;
    }
    assert_eq!(session.overlay_closes.len(), 2);
    assert_eq!(
        session.reserved_overlay_text_bytes(),
        MAX_OVERLAY_RETAINED_TEXT_BYTES
    );
    assert_eq!(
        session.retained_overlay_text_bytes(),
        MAX_OVERLAY_RETAINED_TEXT_BYTES
    );
    session.pending_save = Some(PendingSave {
        buffer_revision: session.identity.buffer_revision,
        submitted: None,
    });
    let mut replacement = input;
    replacement.path = root.join("replacement.rs");
    replacement.identity.document_id = 4;
    replacement.snapshot = alpine_text::Buffer::new("x").snapshot();
    assert!(matches!(
        session.park_and_activate(replacement.clone(), false),
        Err(RustDiagnosticsError::OverlayBudget)
    ));
    assert_eq!(session.identity.document_id, 3);
    assert!(matches!(
        model.preflight_workspace(&[replacement]),
        Err(RustDiagnosticsError::OverlayBudget)
    ));
    let session = model.session.as_mut().ok_or("workspace")?;
    session.reset_overlay_transport();
    assert!(session.overlay_closes.is_empty());
    assert_eq!(
        session.reserved_overlay_text_bytes(),
        MAX_DOCUMENT_BYTES * 2
    );
    assert!(session.pending_save.is_some());
    assert!(!model.shutdown().active);
    assert_eq!(model.snapshot().overlay_retained_text_bytes, 0);
    assert_eq!(model.snapshot().overlay_reserved_text_bytes, 0);
    std::fs::remove_dir_all(root)?;
    Ok(())
}
