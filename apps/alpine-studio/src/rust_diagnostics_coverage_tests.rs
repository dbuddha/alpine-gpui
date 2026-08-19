use std::{error::Error, path::Path, sync::Arc};

use crate::{
    lsp_client::LspClientPoll,
    lsp_process::{FailureKind, InputSequence, ProcessFailure, ProcessStage, StopReason},
};

use super::*;

fn installed_model() -> Result<(RustDiagnostics, RustDocumentInput, PathBuf), Box<dyn Error>> {
    let (root, path, snapshot, identity) = tests::fixture();
    let input = RustDocumentInput::new(&path, &root, identity, snapshot);
    let mut model = RustDiagnostics::default();
    model.install_for_test(
        input.clone(),
        &tests::diagnostics(&path, 1),
        tests::mock_executable(),
    )?;
    Ok((model, input, root))
}

#[test]
fn state_guards_and_admission_failures_are_discriminating() -> Result<(), Box<dyn Error>> {
    let (root, path, snapshot, identity) = tests::fixture();
    let input = RustDocumentInput::new(&path, &root, identity, snapshot);
    let target = Target {
        path: path.clone(),
        workspace_root: root.clone(),
    };

    let mut target_only = RustDiagnostics {
        target: Some(target),
        ..RustDiagnostics::default()
    };
    assert_eq!(
        target_only.sync(Some(input.clone()), |_| Arc::new(|| {})),
        LanguageEffect::default()
    );
    assert!(!target_only.begin_initialize(1));
    assert!(!target_only.open_document());
    assert!(!target_only.flush_change());
    assert!(!target_only.admit(Err(LanguageProtocolError::StaleDiagnostics)));
    assert_eq!(
        target_only.for_each_marker(identity, 0, 1, |_| Ok::<(), ()>(())),
        Ok(0)
    );
    assert!(target_only.restart_or_fail(RustDiagnosticsError::MissingServer));
    assert!(
        !target_only
            .fail(RustDiagnosticsError::MissingServer)
            .visual_changed
    );

    let mut missing = RustDiagnostics::default();
    assert!(
        missing
            .sync(Some(input.clone()), |_| Arc::new(|| {}))
            .visual_changed
    );
    assert!(matches!(
        missing.status_message().as_deref(),
        Some(message) if message.contains("MissingServer")
    ));

    let mut exhausted = RustDiagnostics::with_server(tests::mock_executable());
    exhausted.next_generation = u64::MAX;
    assert!(
        exhausted
            .sync(Some(input.clone()), |_| Arc::new(|| {}))
            .visual_changed
    );

    let mut invalid = RustDiagnostics::with_server(tests::mock_executable());
    let mut invalid_input = input.clone();
    invalid_input.identity.workspace_revision = 0;
    assert!(
        invalid
            .sync(Some(invalid_input), |_| Arc::new(|| {}))
            .visual_changed
    );

    let mut empty = RustDiagnostics::with_server(Path::new(""));
    assert!(empty.sync(Some(input), |_| Arc::new(|| {})).visual_changed);
    assert!(!empty.shutdown().active);
    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn installed_state_covers_selection_markers_versions_and_admission() -> Result<(), Box<dyn Error>> {
    let (mut model, input, root) = installed_model()?;
    let identity = input.identity;
    assert_eq!(
        model.for_each_marker(identity, usize::MAX, 8, |_| Ok::<(), ()>(())),
        Ok(0)
    );
    assert_eq!(
        model.for_each_marker(identity, 0, 8, |_| Err::<(), _>("visitor")),
        Err("visitor")
    );
    let document = LspDocument::from_file_path(&input.path, "rust", 1)?;
    let duplicate = DiagnosticBatch::admit(&tests::diagnostics(&input.path, 1), &document)?;
    assert!(!model.admit(Ok(duplicate)));
    assert!(model.admit(Err(LanguageProtocolError::StaleDiagnostics)));
    assert_eq!(model.snapshot().stale_diagnostics, 1);

    model.session.as_mut().ok_or("session")?.diagnostics = None;
    let duplicate = DiagnosticBatch::admit(&tests::diagnostics(&input.path, 1), &document)?;
    assert!(model.admit(Ok(duplicate)));
    let mut selection = input.clone();
    selection.identity.selection_revision += 1;
    assert_eq!(
        model.sync(Some(selection.clone()), |_| Arc::new(|| {})),
        LanguageEffect::default()
    );
    assert_eq!(
        model.for_each_marker(selection.identity, 0, 8, |_| Ok::<(), ()>(())),
        Ok(2)
    );
    model.session.as_mut().ok_or("session")?.diagnostics = None;
    assert_eq!(
        model.for_each_marker(selection.identity, 0, 8, |_| Ok::<(), ()>(())),
        Ok(0)
    );
    let generation = model.session.as_ref().ok_or("session")?.generation;
    assert_eq!(
        model.poll(LanguageWake {
            generation: generation + 1,
        }),
        LanguageEffect::default()
    );
    assert_eq!(model.snapshot().stale_wakes, 1);

    let session = model.session.as_mut().ok_or("session")?;
    session.lsp_version = i32::MAX;
    let mut newer = selection;
    newer.identity.buffer_revision += 1;
    assert!(model.sync(Some(newer), |_| Arc::new(|| {})).visual_changed);
    assert!(matches!(
        model.status_message().as_deref(),
        Some(message) if message.contains("VersionExhausted")
    ));
    assert!(!model.shutdown().active);
    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn lifecycle_failures_and_poll_classes_are_bounded() -> Result<(), Box<dyn Error>> {
    let (mut begin, _, root) = installed_model()?;
    assert!(begin.begin_initialize(1));

    let (mut oversized_open, _, root_two) = installed_model()?;
    let oversized = "x".repeat(8_388_609);
    let session = oversized_open.session.as_mut().ok_or("session")?;
    session.snapshot = alpine_text::Buffer::new(&oversized).snapshot();
    session.restart_count = MAX_RESTARTS_PER_DOCUMENT;
    assert!(oversized_open.open_document());

    let (mut oversized_change, _, root_three) = installed_model()?;
    let session = oversized_change.session.as_mut().ok_or("session")?;
    session.snapshot = alpine_text::Buffer::new(&oversized).snapshot();
    session.pending_change = true;
    session.restart_count = MAX_RESTARTS_PER_DOCUMENT;
    assert!(oversized_change.flush_change());

    let (mut generation, _, root_four) = installed_model()?;
    generation
        .session
        .as_mut()
        .ok_or("session")?
        .process_generation = u64::MAX;
    assert!(generation.restart_or_fail(RustDiagnosticsError::MissingServer));

    let (mut identity, _, root_five) = installed_model()?;
    identity
        .session
        .as_mut()
        .ok_or("session")?
        .identity
        .workspace_revision = 0;
    assert!(identity.restart_or_fail(RustDiagnosticsError::MissingServer));

    let (mut stopped, _, root_six) = installed_model()?;
    let _ = stopped.session.as_mut().ok_or("session")?.client.shutdown();
    assert!(stopped.restart_or_fail(RustDiagnosticsError::MissingServer));

    let (mut classes, _, root_seven) = installed_model()?;
    let mut visual = false;
    assert!(!classes.apply_poll(
        LspClientPoll::Protocol {
            frames: 1,
            body_bytes: 1
        },
        &mut visual
    ));
    assert!(!classes.apply_poll(LspClientPoll::Stderr { bytes: 1 }, &mut visual));
    assert!(!classes.apply_poll(LspClientPoll::Stopped(StopReason::Restart), &mut visual));
    classes.session.as_mut().ok_or("session")?.state = SessionState::Starting;
    classes.session.as_mut().ok_or("session")?.pending_change = true;
    assert!(!classes.apply_poll(
        LspClientPoll::InputRejected {
            sequence: InputSequence::for_test(1),
            failure: ProcessFailure {
                stage: ProcessStage::Input,
                kind: FailureKind::QueueSaturated,
                raw_os_error: None,
            },
        },
        &mut visual,
    ));
    assert!(visual);
    assert!(!classes.session.as_ref().ok_or("session")?.pending_change);
    classes.session.as_mut().ok_or("session")?.state = SessionState::Open;
    assert!(!classes.apply_poll(
        LspClientPoll::InputRejected {
            sequence: InputSequence::for_test(2),
            failure: ProcessFailure {
                stage: ProcessStage::Input,
                kind: FailureKind::QueueSaturated,
                raw_os_error: None,
            },
        },
        &mut visual,
    ));
    assert!(classes.session.as_ref().ok_or("session")?.pending_change);
    assert!(classes.apply_poll(
        LspClientPoll::Exited {
            success: false,
            code: Some(7),
        },
        &mut visual,
    ));

    for model in [
        &mut begin,
        &mut oversized_open,
        &mut oversized_change,
        &mut generation,
        &mut identity,
        &mut stopped,
        &mut classes,
    ] {
        assert!(!model.shutdown().active);
    }

    for directory in [
        root, root_two, root_three, root_four, root_five, root_six, root_seven,
    ] {
        std::fs::remove_dir_all(directory)?;
    }
    Ok(())
}

#[test]
fn visual_accumulation_continuation_and_restart_counts_are_exact() -> Result<(), Box<dyn Error>> {
    for (current, observed, expected) in [
        (false, false, false),
        (false, true, true),
        (true, false, true),
        (true, true, true),
    ] {
        let mut actual = current;
        merge_visual_changed(&mut actual, observed);
        assert_eq!(actual, expected);
    }
    assert_eq!(continuation_for_queued_events(0, 9), None);
    assert_eq!(
        continuation_for_queued_events(1, 9),
        Some(LanguageWake { generation: 9 })
    );

    let (mut model, _, root) = installed_model()?;
    assert!(model.restart_or_fail(RustDiagnosticsError::MissingServer));
    assert_eq!(model.session.as_ref().ok_or("session")?.restart_count, 1);
    assert_eq!(model.snapshot().restarts, 1);
    assert!(!model.restart_or_fail(RustDiagnosticsError::MissingServer));
    assert_eq!(model.session.as_ref().ok_or("session")?.restart_count, 2);
    assert_eq!(model.snapshot().restarts, 2);
    assert!(!model.shutdown().active);
    std::fs::remove_dir_all(root)?;
    Ok(())
}
