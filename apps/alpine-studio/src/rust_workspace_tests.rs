use std::{error::Error, path::PathBuf, sync::Arc};

use super::super::tests as diagnostic_tests;
use super::*;

#[path = "rust_closing_ownership_tests.rs"]
mod closing_ownership;

#[path = "rust_status_channel_tests.rs"]
mod status_channels;

#[path = "rust_overlay_ownership_tests.rs"]
mod overlay_ownership;

#[test]
fn workspace_contract_rosters_reject_independent_identity_and_duplicate_coordinates()
-> Result<(), Box<dyn Error>> {
    let (root, path, snapshot, identity) = diagnostic_tests::fixture();
    let input = RustDocumentInput::new(&path, &root, identity, snapshot);
    let mut other = input.clone();
    other.path = root.join("other.rs");
    other.identity.document_id += 1;
    assert_eq!(
        collect_workspace_inputs([input.clone(), other.clone()])?.len(),
        2
    );
    for coordinate in 0..5 {
        let mut invalid = other.clone();
        match coordinate {
            0 => invalid.workspace_root = root.parent().ok_or("parent")?.to_path_buf(),
            1 => invalid.identity.workspace_id += 1,
            2 => invalid.identity.workspace_revision += 1,
            3 => invalid.identity.document_id = input.identity.document_id,
            _ => invalid.path = input.path.clone(),
        }
        let result = collect_workspace_inputs([input.clone(), invalid.clone()]);
        if coordinate < 3 {
            assert!(matches!(result, Err(RustDiagnosticsError::InvalidIdentity)));
        } else {
            assert!(matches!(result, Err(RustDiagnosticsError::OverlayBudget)));
        }
        let starts = std::cell::Cell::new(0);
        let mut model = RustDiagnostics::default();
        let effect = model.sync_workspace(
            [input.clone(), invalid],
            Some(input.identity.document_id),
            |_| {
                starts.set(starts.get() + 1);
                Arc::new(|| {})
            },
        );
        assert!(effect.visual_changed);
        assert!(effect.continuation.is_none());
        assert!(!model.snapshot().active);
        assert_eq!(starts.get(), 0);
    }
    assert_eq!(std::fs::read_to_string(&path)?, input.snapshot.text());
    std::fs::remove_dir_all(root)?;

    // A valid replacement roster must retire foreign workspace authority,
    // even when its document path and buffer revision happen to be unchanged.
    for coordinate in 0..3 {
        let (mut model, mut input, root) = installed_workspace()?;
        assert!(model.snapshot().active);
        model.server_path = None;
        match coordinate {
            0 => input.workspace_root = root.parent().ok_or("parent")?.to_path_buf(),
            1 => input.identity.workspace_id += 1,
            _ => input.identity.workspace_revision += 1,
        }
        let wake_factories = std::cell::Cell::new(0);
        let effect =
            model.sync_workspace([input.clone()], Some(input.identity.document_id), |_| {
                wake_factories.set(wake_factories.get() + 1);
                Arc::new(|| {})
            });
        assert_eq!(wake_factories.get(), 0);
        assert!(effect.visual_changed);
        assert!(effect.continuation.is_none());
        assert!(
            model.session.is_none(),
            "foreign workspace coordinate {coordinate} retained the old session"
        );
        let released = model.snapshot();
        assert!(!released.active);
        assert_eq!(released.process_starts, 0);
        assert_eq!(released.overlay_documents, 0);
        assert_eq!(released.overlay_retained_text_bytes, 0);
        assert_eq!(released.overlay_reserved_text_bytes, 0);
        assert_eq!(std::fs::read_to_string(&input.path)?, input.snapshot.text());
        retire_inert_workspace(&mut model);
        std::fs::remove_dir_all(root)?;
    }
    Ok(())
}

#[test]
fn workspace_contract_pending_writer_preserves_exact_snapshot_and_owner_accounting()
-> Result<(), Box<dyn Error>> {
    let (mut model, mut input, root) = installed_workspace()?;
    model.initialize_installed_transport_for_test()?;
    let mut other = input.clone();
    other.path = root.join("other.rs");
    other.identity.document_id += 1;
    other.snapshot = alpine_text::Buffer::new("pub fn other() -> u8 { 7 }\n").snapshot();
    let old_active_bytes = input.snapshot.len_bytes();
    let old_parked_bytes = other.snapshot.len_bytes();
    let session = model.session.as_mut().ok_or("workspace")?;
    assert!(session.update_parked(other.clone())?);
    session.parked[0].opened = true;
    // An outstanding writer keeps both old server snapshots alive while real
    // foreground roster reconciliation admits the two shortened buffers.
    session.overlay_write = Some(InputSequence::for_test(7));
    for (document, replacement) in [(&mut input, "a"), (&mut other, "bb")] {
        let mut buffer = alpine_text::Buffer::new(&document.snapshot.text());
        let mut transaction = alpine_text::Transaction::new(buffer.revision());
        transaction.replace(0..document.snapshot.len_bytes(), replacement)?;
        let _ = buffer.apply(transaction)?;
        document.snapshot = buffer.snapshot();
        document.identity.buffer_revision = buffer.revision().get();
    }
    let roster = [input.clone(), other.clone()];
    let _ = model.sync_workspace(roster.clone(), Some(input.identity.document_id), |_| {
        Arc::new(|| {})
    });
    let session = model.session.as_ref().ok_or("workspace lost")?;
    assert_eq!(session.overlay_write, Some(InputSequence::for_test(7)));
    assert_eq!(session.snapshot.text(), "a");
    assert_eq!(session.parked[0].snapshot.text(), "bb");
    assert_eq!(session.synced_snapshot.len_bytes(), old_active_bytes);
    assert_eq!(
        session.parked[0].synced_snapshot.len_bytes(),
        old_parked_bytes
    );
    assert_eq!(
        session.retained_overlay_text_bytes(),
        1 + old_active_bytes + 2 + old_parked_bytes
    );
    assert_eq!(
        session.reserved_overlay_text_bytes(),
        2 * old_active_bytes + 2 * old_parked_bytes
    );
    assert_eq!(
        model.snapshot().overlay_retained_text_bytes,
        1 + old_active_bytes + 2 + old_parked_bytes
    );
    assert!(session.overlay_contents_match(&roster));
    for index in 0..2 {
        let mut changed = roster.clone();
        changed[index].path = root.join("renamed.rs");
        assert!(!session.overlay_contents_match(&changed));
        changed = roster.clone();
        changed[index].identity.buffer_revision += 1;
        assert!(!session.overlay_contents_match(&changed));
        changed = roster.clone();
        changed[index].identity.selection_revision += 1;
        assert!(session.overlay_contents_match(&changed));
    }
    for (document_id, path, expected) in [
        (
            input.identity.document_id,
            input.path.clone(),
            2 * old_active_bytes,
        ),
        (
            other.identity.document_id,
            other.path.clone(),
            2 * old_parked_bytes,
        ),
        (input.identity.document_id, root.join("new.rs"), 6),
        (other.identity.document_id, root.join("new.rs"), 6),
        (99, input.path.clone(), 6),
        (99, other.path.clone(), 6),
        (input.identity.document_id, other.path.clone(), 6),
        (other.identity.document_id, input.path.clone(), 6),
    ] {
        let mut next = input.clone();
        next.identity.document_id = document_id;
        next.identity.buffer_revision += 1;
        next.path = path;
        next.snapshot = alpine_text::Buffer::new("xyz").snapshot();
        assert_eq!(session.prospective_reservation(&next), expected);
    }
    retire_inert_workspace(&mut model);
    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
#[cfg_attr(
    miri,
    ignore = "large logical-byte fixture; small arithmetic is covered separately"
)]
fn workspace_contract_budget_boundary_is_atomic_and_foreign_workspaces_release_reservations()
-> Result<(), Box<dyn Error>> {
    let (mut model, input, root) = installed_workspace()?;
    let full = alpine_text::Buffer::new(&"x".repeat(MAX_DOCUMENT_BYTES)).snapshot();
    let mut last_buffer =
        alpine_text::Buffer::new(&"x".repeat(MAX_DOCUMENT_BYTES - input.snapshot.len_bytes()));
    let mut roster = vec![input.clone()];
    let session = model.session.as_mut().ok_or("workspace")?;
    for document_id in 2..=4 {
        let mut next = input.clone();
        next.path = root.join(format!("{document_id}.rs"));
        next.identity.document_id = document_id;
        next.snapshot = if document_id == 4 {
            last_buffer.snapshot()
        } else {
            full.clone()
        };
        assert!(session.update_parked(next.clone())?);
        roster.push(next);
    }
    // COW snapshots intentionally share allocations. These exact logical-byte
    // reservations are not physical footprint or allocator-residency evidence.
    assert_eq!(
        session.reserved_overlay_text_bytes(),
        MAX_OVERLAY_RETAINED_TEXT_BYTES
    );
    assert_eq!(
        session.retained_overlay_text_bytes(),
        MAX_OVERLAY_RETAINED_TEXT_BYTES
    );
    assert!(session.check_overlay_budget(0).is_ok());
    for additional in [1, usize::MAX] {
        assert!(matches!(
            session.check_overlay_budget(additional),
            Err(RustDiagnosticsError::OverlayBudget)
        ));
    }
    let mut extra = input.clone();
    extra.identity.document_id = 5;
    extra.path = root.join("extra.rs");
    extra.snapshot = alpine_text::Buffer::new("x").snapshot();
    assert!(matches!(
        session.update_parked(extra.clone()),
        Err(RustDiagnosticsError::OverlayBudget)
    ));
    let last_bytes = last_buffer.snapshot().len_bytes();
    let mut transaction = alpine_text::Transaction::new(last_buffer.revision());
    transaction.replace(last_bytes..last_bytes, "x")?;
    let _ = last_buffer.apply(transaction)?;
    let mut growing = roster[3].clone();
    growing.snapshot = last_buffer.snapshot();
    growing.identity.buffer_revision = last_buffer.revision().get();
    assert!(matches!(
        session.update_parked(growing),
        Err(RustDiagnosticsError::OverlayBudget)
    ));
    assert_eq!(session.parked.len(), 3);
    assert_eq!(session.parked[2].snapshot.len_bytes(), last_bytes);
    assert_eq!(session.parked[2].identity, roster[3].identity);
    assert_eq!(session.parked[2].lsp_version, 1);
    assert_eq!(
        session.reserved_overlay_text_bytes(),
        MAX_OVERLAY_RETAINED_TEXT_BYTES
    );
    assert!(model.preflight_workspace(&roster).is_ok());
    // Shrinking current inputs cannot release the same workspace's older
    // synced snapshots while its writer still owns them.
    for document in &mut roster {
        document.snapshot = extra.snapshot.clone();
        document.identity.buffer_revision += 1;
    }
    assert!(model.preflight_workspace(&roster).is_ok());
    roster.push(extra);
    assert!(matches!(
        model.preflight_workspace(&roster),
        Err(RustDiagnosticsError::OverlayBudget)
    ));
    for coordinate in 0..3 {
        let mut replacement = roster.clone();
        for document in &mut replacement {
            match coordinate {
                0 => document.workspace_root = root.parent().ok_or("parent")?.to_path_buf(),
                1 => document.identity.workspace_id += 1,
                _ => document.identity.workspace_revision += 1,
            }
        }
        assert!(model.preflight_workspace(&replacement).is_ok());
    }
    assert_eq!(model.snapshot().overlay_documents, 4);
    assert_eq!(
        model.snapshot().overlay_reserved_text_bytes,
        MAX_OVERLAY_RETAINED_TEXT_BYTES
    );
    assert_eq!(std::fs::read_to_string(&input.path)?, input.snapshot.text());
    retire_inert_workspace(&mut model);
    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn saved_notifications_reject_each_stale_owner_and_preserve_newer_saves()
-> Result<(), Box<dyn Error>> {
    let (mut model, mut input, root) = installed_workspace()?;
    model.initialize_installed_transport_for_test()?;
    model.session.as_mut().ok_or("workspace")?.state = SessionState::Initializing;
    for identity in [
        LanguageIdentity {
            workspace_id: input.identity.workspace_id + 1,
            ..input.identity
        },
        LanguageIdentity {
            workspace_revision: input.identity.workspace_revision + 1,
            ..input.identity
        },
        LanguageIdentity {
            document_id: 99,
            ..input.identity
        },
    ] {
        let _ = model.record_saved_document(identity);
        assert!(
            model
                .session
                .as_ref()
                .ok_or("workspace")?
                .pending_save
                .is_none()
        );
    }
    let mut buffer = alpine_text::Buffer::new(&input.snapshot.text());
    for text in ["// first\n", "// second\n"] {
        let mut transaction = alpine_text::Transaction::new(buffer.revision());
        transaction.replace(0..0, text)?;
        let _ = buffer.apply(transaction)?;
    }
    input.snapshot = buffer.snapshot();
    input.identity.buffer_revision = buffer.revision().get();
    let _ = model.sync_workspace([input.clone()], Some(input.identity.document_id), |_| {
        Arc::new(|| {})
    });
    let older = LanguageIdentity {
        buffer_revision: input.identity.buffer_revision - 1,
        ..input.identity
    };
    let _ = model.record_saved_document(older);
    assert!(
        model
            .session
            .as_ref()
            .ok_or("workspace")?
            .pending_save
            .is_none()
    );
    // Save delivery may precede the end-of-event overlay reconciliation. Keep
    // its newest revision until that reconciliation admits the matching text.
    let next = LanguageIdentity {
        buffer_revision: input.identity.buffer_revision + 1,
        ..input.identity
    };
    let _ = model.record_saved_document(next);
    let _ = model.record_saved_document(input.identity);
    assert_eq!(
        model
            .session
            .as_ref()
            .ok_or("workspace")?
            .pending_save
            .as_ref()
            .ok_or("save")?
            .buffer_revision,
        next.buffer_revision,
    );
    let newest = LanguageIdentity {
        buffer_revision: next.buffer_revision + 1,
        ..next
    };
    let _ = model.record_saved_document(newest);
    assert_eq!(
        model
            .session
            .as_ref()
            .ok_or("workspace")?
            .pending_save
            .as_ref()
            .ok_or("save")?
            .buffer_revision,
        newest.buffer_revision,
    );
    retire_inert_workspace(&mut model);
    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn saved_notifications_wait_for_text_and_survive_only_owned_acknowledgements()
-> Result<(), Box<dyn Error>> {
    let (mut model, input, root) = installed_workspace()?;
    model.initialize_installed_transport_for_test()?;
    let mut saved = input.identity;
    saved.buffer_revision += 1;
    let before = model.snapshot().process_submitted_inputs;
    let _ = model.record_saved_document(saved);
    assert_eq!(model.snapshot().process_submitted_inputs, before);
    let session = model.session.as_mut().ok_or("workspace")?;
    assert_eq!(
        session
            .pending_save
            .as_ref()
            .ok_or("pending save")?
            .buffer_revision,
        1
    );
    session.identity.buffer_revision = 1;
    session.pending_change = true;
    assert!(session.flush_overlay()?);
    let changed = session.overlay_write.ok_or("change sequence")?;
    assert!(
        session
            .pending_save
            .as_ref()
            .ok_or("save")?
            .submitted
            .is_none()
    );
    assert!(!session.flush_overlay()?);
    assert!(session.acknowledge_overlay(changed));
    assert!(session.flush_overlay()?);
    let saving = session.overlay_write.ok_or("save sequence")?;
    assert_ne!(saving, changed);
    assert_eq!(
        session.pending_save.as_ref().ok_or("save")?.submitted,
        Some(saving)
    );
    assert!(!session.acknowledge_overlay(changed));
    assert!(session.pending_save.is_some());
    // A second successful save while the first write is pending must survive
    // acknowledgement of that earlier save, even at the same text revision.
    let _ = model.record_saved_document(saved);
    let session = model.session.as_mut().ok_or("workspace")?;
    assert!(session.acknowledge_overlay(saving));
    assert!(session.pending_save.is_some());
    assert!(session.flush_overlay()?);
    let repeated = session.overlay_write.ok_or("repeated save sequence")?;
    assert!(session.acknowledge_overlay(repeated));
    assert!(session.pending_save.is_none());
    assert!(!session.flush_overlay()?);
    assert_eq!(model.snapshot().process_submitted_inputs, before + 3);
    retire_inert_workspace(&mut model);
    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn saved_notifications_coalesce_follow_tabs_retry_and_reject_foreign_owners()
-> Result<(), Box<dyn Error>> {
    let (mut model, input, root) = installed_workspace()?;
    model.initialize_installed_transport_for_test()?;
    model.session.as_mut().ok_or("workspace")?.state = SessionState::Initializing;
    for _ in 0..10_000 {
        let _ = model.record_saved_document(input.identity);
    }
    let before = model.snapshot().process_submitted_inputs;
    let mut foreign = input.identity;
    foreign.workspace_revision += 1;
    let _ = model.record_saved_document(foreign);
    foreign = input.identity;
    foreign.document_id = 99;
    let _ = model.record_saved_document(foreign);
    assert_eq!(model.snapshot().process_submitted_inputs, before);
    let mut next = input.clone();
    next.path = root.join("b.rs");
    next.identity.document_id = 2;
    let session = model.session.as_mut().ok_or("workspace")?;
    session.park_and_activate(next, true)?;
    assert!(session.pending_save.is_none());
    assert!(session.parked[0].pending_save.is_some());
    session.state = SessionState::Open;
    assert!(session.flush_overlay()?);
    let opened = session.overlay_write.ok_or("open")?;
    assert!(session.acknowledge_overlay(opened));
    assert!(session.flush_overlay()?);
    let saved = session.overlay_write.ok_or("saved")?;
    assert_eq!(
        session.parked[0]
            .pending_save
            .as_ref()
            .ok_or("parked save")?
            .submitted,
        Some(saved)
    );
    session.reset_overlay_transport();
    assert!(
        session.parked[0]
            .pending_save
            .as_ref()
            .ok_or("retry")?
            .submitted
            .is_none()
    );
    assert!(!session.acknowledge_overlay(saved));
    assert!(session.parked[0].pending_save.is_some());
    session.retain_overlays(&[2])?;
    assert!(session.parked.is_empty());
    assert!(session.pending_save.is_none());
    retire_inert_workspace(&mut model);
    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn reservation_covers_writer_acknowledgement_and_coalesced_updates() {
    // All small current/synced/new length combinations, independent of rope
    // sharing, prove that admitting a change reserves its later writer copy.
    for current in 0..16 {
        for synced in 0..16 {
            for next in 0..16 {
                let reserved = snapshot_reservation(current, synced)
                    + 2 * overlay_growth(current, synced, next);
                assert_eq!(reserved, 2 * current.max(synced).max(next));
                assert!(reserved >= next + synced);
                assert!(reserved >= 2 * next);
                for coalesced in 0..16 {
                    let reserved = snapshot_reservation(next, synced)
                        + 2 * overlay_growth(next, synced, coalesced);
                    assert!(reserved >= coalesced + synced);
                    assert!(reserved >= 2 * coalesced);
                }
            }
        }
    }
}

#[test]
fn reservation_does_not_wrap_and_growth_uses_the_larger_snapshot() {
    assert_eq!(snapshot_reservation(usize::MAX, 0), usize::MAX);
    assert_eq!(snapshot_reservation(0, usize::MAX), usize::MAX);
    assert_eq!(overlay_growth(4, 12, 10), 0);
    assert_eq!(overlay_growth(4, 12, 13), 1);
    assert_eq!(overlay_growth(12, 4, 13), 1);
    assert_eq!(overlay_growth(0, 0, usize::MAX), usize::MAX);
}

// These fixtures install an inert client or reject work before any process starts.
// They cannot answer an LSP shutdown request. Real-server tests own that protocol
// gate; model fixtures must release ownership without waiting for an absent peer.
fn retire_inert_workspace(model: &mut RustDiagnostics) {
    assert_eq!(
        model.snapshot().process_starts,
        0,
        "inert cleanup must not replace a real-server shutdown gate"
    );
    let _ = model.stop();
    assert!(model.session.is_none());
    let released = model.snapshot();
    assert!(!released.active);
    assert_eq!(released.overlay_documents, 0);
    assert_eq!(released.overlay_retained_text_bytes, 0);
    assert_eq!(released.overlay_reserved_text_bytes, 0);
}

fn installed_workspace() -> Result<(RustDiagnostics, RustDocumentInput, PathBuf), Box<dyn Error>> {
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
fn inactive_workspace_reconciliation_is_quiet_after_the_view_transition()
-> Result<(), Box<dyn Error>> {
    let (mut model, input, root) = installed_workspace()?;
    let initial_status = model.status_message().ok_or("initial diagnostic status")?;
    let first = model.sync_workspace([input.clone()], None, |_| Arc::new(|| {}));
    assert!(first.visual_changed);
    assert!(model.status_message().is_none());
    assert_eq!(model.status.as_deref(), Some(initial_status.as_ref()));
    for _ in 0..100 {
        let next = model.sync_workspace([input.clone()], None, |_| Arc::new(|| {}));
        assert!(!next.visual_changed);
        assert!(next.continuation.is_none());
        assert!(model.status_message().is_none());
        let session = model.session.as_ref().ok_or("workspace lost")?;
        assert!(!session.active_view);
        assert!(session.document_opened);
        assert!(!session.workspace_ready());
    }
    assert_eq!(model.snapshot().document_switches, 0);
    assert!(
        model
            .sync_workspace([input], Some(1), |_| Arc::new(|| {}))
            .visual_changed
    );
    assert!(model.session.as_ref().ok_or("workspace")?.workspace_ready());
    assert_eq!(
        model.status_message().as_deref(),
        Some(initial_status.as_ref())
    );
    retire_inert_workspace(&mut model);
    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn acknowledgement_and_every_pending_overlay_guard_request_readiness() -> Result<(), Box<dyn Error>>
{
    let (mut model, input, root) = installed_workspace()?;
    let session = model.session.as_mut().ok_or("workspace")?;
    assert!(session.workspace_ready());
    session.overlay_write = Some(InputSequence::for_test(7));
    assert!(!session.workspace_ready());
    assert!(!session.acknowledge_overlay(InputSequence::for_test(6)));
    assert!(session.overlay_write.is_some());
    assert!(session.acknowledge_overlay(InputSequence::for_test(7)));
    assert!(!session.acknowledge_overlay(InputSequence::for_test(7)));
    assert!(session.workspace_ready());
    session.pending_change = true;
    assert!(!session.workspace_ready());
    session.pending_change = false;
    session.document_opened = false;
    assert!(!session.workspace_ready());
    session.document_opened = true;
    session.overlay_closes.push(ClosingDocument {
        document: session.document.clone(),
        opened: true,
        pending_save: None,
        pending_text: None,
    });
    assert!(!session.workspace_ready());
    session.overlay_closes.clear();
    let mut parked = ParkedDocument::new(input)?;
    parked.identity.document_id = 2;
    session.parked.push(parked);
    assert!(!session.workspace_ready());
    session.parked[0].opened = true;
    assert!(session.workspace_ready());
    session.parked[0].pending_change = true;
    assert!(!session.workspace_ready());
    session.parked[0].pending_change = false;
    session.state = SessionState::Starting;
    assert!(!session.workspace_ready());
    session.state = SessionState::Open;
    session.active_view = false;
    assert!(!session.workspace_ready());
    session.active_view = true;
    assert!(session.workspace_ready());
    retire_inert_workspace(&mut model);
    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn parked_edits_keep_versions_and_reject_stale_diagnostics() -> Result<(), Box<dyn Error>> {
    let (root, path, snapshot, identity) = diagnostic_tests::fixture();
    let mut input = RustDocumentInput::new(&path, &root, identity, snapshot);
    let mut parked = ParkedDocument::new(input.clone())?;
    parked.opened = true;
    let mut buffer = alpine_text::Buffer::new("fn main() {}\n");
    let mut transaction = alpine_text::Transaction::new(buffer.revision());
    transaction.replace(0..0, "// changed while inactive\n")?;
    let _ = buffer.apply(transaction)?;
    input.snapshot = buffer.snapshot();
    input.identity.buffer_revision = buffer.revision().get();
    assert!(parked.update(input.clone())?);
    assert_eq!(parked.lsp_version, 2);
    assert!(parked.pending_change);
    assert!(!parked.update(input)?);
    assert!(
        crate::lsp_language::DiagnosticBatch::admit(
            &diagnostic_tests::diagnostics(&path, 1),
            &parked.document
        )
        .is_err()
    );
    let admitted = crate::lsp_language::DiagnosticBatch::admit(
        &diagnostic_tests::diagnostics(&path, 2),
        &parked.document,
    )?;
    assert_eq!(admitted.document_version(), Some(2));
    assert_eq!(parked.snapshot.text(), buffer.snapshot().text());
    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn roster_bounds_close_and_restart_keep_explicit_ownership() -> Result<(), Box<dyn Error>> {
    let (mut model, input, root) = installed_workspace()?;
    let session = model.session.as_mut().ok_or("workspace")?;
    for document_id in 2..=32 {
        let mut next = input.clone();
        next.path = root.join(format!("{document_id}.rs"));
        next.identity.document_id = document_id;
        assert!(session.update_parked(next)?);
    }
    assert_eq!(session.parked.len() + 1, MAX_OVERLAY_DOCUMENTS);
    let mut excess = input.clone();
    excess.path = root.join("excess.rs");
    excess.identity.document_id = 33;
    assert!(matches!(
        session.update_parked(excess),
        Err(RustDiagnosticsError::OverlayBudget)
    ));
    for document in &mut session.parked {
        document.opened = true;
    }
    session.retain_overlays(&[1, 2])?;
    assert_eq!(session.parked.len(), 1);
    assert_eq!(session.overlay_closes.len(), 30);
    assert_eq!(session.parked[0].identity.document_id, 2);
    session.overlay_write = Some(InputSequence::for_test(9));
    let text = session.parked[0].snapshot.text();
    session.reset_overlay_transport();
    assert!(session.overlay_write.is_none());
    assert!(session.overlay_closes.is_empty());
    assert!(!session.document_opened);
    assert!(!session.parked[0].opened);
    assert!(session.parked[0].diagnostics.is_none());
    assert_eq!(session.parked[0].snapshot.text(), text);
    assert!(!session.workspace_ready());
    retire_inert_workspace(&mut model);
    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn duplicate_rosters_fail_closed_without_changing_editor_text() -> Result<(), Box<dyn Error>> {
    let (mut model, input, root) = installed_workspace()?;
    let snapshot = input.snapshot.clone();
    let effect = model.sync_workspace([input.clone(), input], Some(1), |_| Arc::new(|| {}));
    assert!(effect.visual_changed);
    assert!(!model.snapshot().active);
    assert!(
        model
            .status_message()
            .is_some_and(|message| message.contains("OverlayBudget"))
    );
    assert_eq!(snapshot.text(), "fn main() {}\n");
    assert_eq!(
        std::fs::read_to_string(root.join("main.rs"))?,
        snapshot.text()
    );
    retire_inert_workspace(&mut model);
    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn view_only_revision_changes_preserve_current_text_diagnostics() -> Result<(), Box<dyn Error>> {
    let (mut model, mut input, root) = installed_workspace()?;
    let before_selection = input.identity.selection_revision;
    let _ = model.sync_workspace([input.clone()], None, |_| Arc::new(|| {}));
    input.identity.document_revision += 2;
    let _ = model.sync_workspace([input.clone()], Some(1), |_| Arc::new(|| {}));
    assert_eq!(input.identity.selection_revision, before_selection);
    let mut markers = 0;
    let count = model.for_each_marker(input.identity, 0, 1, |_| {
        markers += 1;
        Ok::<(), ()>(())
    });
    assert_eq!(count, Ok(1));
    assert_eq!(markers, 1);
    assert_eq!(model.snapshot().document_switches, 0);
    retire_inert_workspace(&mut model);
    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn full_roster_replacement_releases_the_removed_active_owner_before_admission()
-> Result<(), Box<dyn Error>> {
    let (mut model, input, root) = installed_workspace()?;
    let mut retained = Vec::new();
    let session = model.session.as_mut().ok_or("workspace")?;
    for document_id in 2..=32 {
        let mut next = input.clone();
        next.path = root.join(format!("{document_id}.rs"));
        next.identity.document_id = document_id;
        let _ = session.update_parked(next.clone())?;
        retained.push(next);
    }
    for parked in &mut session.parked {
        parked.opened = true;
    }
    // A real pending writer prevents flushing; it does not prevent bounded
    // roster reconciliation while that acknowledgement is outstanding.
    session.overlay_write = Some(InputSequence::for_test(7));
    let generation = session.generation;
    let removed_uri = session.document.uri().to_owned();
    let mut replacement = input.clone();
    replacement.path = root.join("replacement.rs");
    replacement.identity.document_id = 33;
    retained.push(replacement.clone());
    assert_eq!(retained.len(), MAX_OVERLAY_DOCUMENTS);
    let _ = model.sync_workspace(retained.clone(), Some(33), |_| Arc::new(|| {}));
    let session = model
        .session
        .as_ref()
        .ok_or("valid replacement removed workspace")?;
    assert_eq!(session.generation, generation);
    assert_eq!(session.identity.document_id, 33);
    assert_eq!(session.parked.len() + 1, MAX_OVERLAY_DOCUMENTS);
    assert!(
        session
            .parked
            .iter()
            .all(|document| document.identity.document_id != 1)
    );
    assert_eq!(session.overlay_closes.len(), 1);
    assert_eq!(session.overlay_closes[0].document.uri(), removed_uri);
    assert_eq!(session.overlay_write, Some(InputSequence::for_test(7)));
    assert!(!session.workspace_ready());
    retained.push(input);
    let _ = model.sync_workspace(retained, Some(33), |_| Arc::new(|| {}));
    assert!(!model.snapshot().active);
    assert!(
        model
            .status_message()
            .is_some_and(|message| message.contains("OverlayBudget"))
    );
    retire_inert_workspace(&mut model);
    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
#[cfg_attr(
    miri,
    ignore = "large logical-byte fixture; small reservation arithmetic is covered separately"
)]
fn aggregate_limit_is_preflighted_and_repeated_rejection_is_quiet() -> Result<(), Box<dyn Error>> {
    let (root, path, _, identity) = diagnostic_tests::fixture();
    let snapshot = alpine_text::Buffer::new(&"x".repeat(MAX_DOCUMENT_BYTES)).snapshot();
    let input = RustDocumentInput::new(&path, &root, identity, snapshot);
    let mut documents = Vec::new();
    for document_id in 1..=4 {
        let mut document = input.clone();
        document.path = root.join(format!("{document_id}.rs"));
        document.identity.document_id = document_id;
        documents.push(document);
    }
    let mut model = RustDiagnostics::default();
    // Three admitted current/synced pairs exactly fill the 48-MiB logical
    // reservation; a fourth must be refused before a server factory runs.
    assert!(model.preflight_workspace(&documents[..3]).is_ok());
    assert!(matches!(
        model.preflight_workspace(&documents),
        Err(RustDiagnosticsError::OverlayBudget)
    ));
    let starts = std::cell::Cell::new(0);
    for attempt in 0..8 {
        let effect = model.sync_workspace(documents.clone(), Some(1), |_| {
            starts.set(starts.get() + 1);
            Arc::new(|| {})
        });
        assert_eq!(effect.visual_changed, attempt == 0);
        assert!(effect.continuation.is_none());
        assert!(!model.snapshot().active);
        assert!(
            model
                .status_message()
                .is_some_and(|message| message.contains("OverlayBudget"))
        );
    }
    assert_eq!(starts.get(), 0);
    assert_eq!(model.next_generation, 0);
    assert_eq!(input.snapshot.len_bytes(), MAX_DOCUMENT_BYTES);
    assert_eq!(std::fs::read_to_string(&path)?, "fn main() {}\n");
    retire_inert_workspace(&mut model);
    std::fs::remove_dir_all(root)?;
    Ok(())
}
