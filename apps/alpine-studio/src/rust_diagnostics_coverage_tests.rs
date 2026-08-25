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
    assert!(stopped.begin_initialize(1));

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
        &mut generation,
        &mut identity,
        &mut stopped,
        &mut classes,
    ] {
        assert!(!model.shutdown().active);
    }

    for directory in [root, root_four, root_five, root_six, root_seven] {
        std::fs::remove_dir_all(directory)?;
    }
    Ok(())
}

#[test]
#[cfg_attr(
    miri,
    ignore = "native coverage enforces the 8 MiB message boundary without interpreting 16 MiB of Ropey construction"
)]
fn oversized_document_messages_fail_boundedly() -> Result<(), Box<dyn Error>> {
    let oversized = "x".repeat(8_388_609);

    let (mut oversized_open, _, root) = installed_model()?;
    let session = oversized_open.session.as_mut().ok_or("session")?;
    session.snapshot = alpine_text::Buffer::new(&oversized).snapshot();
    session.restart_count = MAX_RESTARTS_PER_DOCUMENT;
    assert!(oversized_open.open_document());

    let (mut oversized_change, _, root_two) = installed_model()?;
    let session = oversized_change.session.as_mut().ok_or("session")?;
    session.snapshot = alpine_text::Buffer::new(&oversized).snapshot();
    session.pending_change = true;
    session.restart_count = MAX_RESTARTS_PER_DOCUMENT;
    assert!(oversized_change.flush_change());

    assert!(!oversized_open.shutdown().active);
    assert!(!oversized_change.shutdown().active);
    std::fs::remove_dir_all(root)?;
    std::fs::remove_dir_all(root_two)?;
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

fn completion_result(text: &str) -> Result<Box<serde_json::value::RawValue>, Box<dyn Error>> {
    Ok(serde_json::value::RawValue::from_string(text.to_owned())?)
}

fn install_pending_completion(
    model: &mut RustDiagnostics,
    request_id: u32,
) -> Result<PendingCompletion, Box<dyn Error>> {
    let session = model.session.as_mut().ok_or("session")?;
    let pending = PendingCompletion {
        request_id,
        stamp: session.identity.request_stamp().ok_or("request stamp")?,
        identity: session.identity,
        process_epoch: session.process_epoch,
        lsp_version: session.lsp_version,
    };
    session.pending_completion = Some(pending);
    Ok(pending)
}

fn install_pending_navigation(
    model: &mut RustDiagnostics,
    request_id: u32,
    kind: NavigationRequestKind,
) -> Result<PendingNavigation, Box<dyn Error>> {
    let session = model.session.as_mut().ok_or("session")?;
    let pending = PendingNavigation {
        request_id,
        stamp: session.identity.request_stamp().ok_or("request stamp")?,
        kind,
        identity: session.identity,
        process_epoch: session.process_epoch,
        lsp_version: session.lsp_version,
    };
    session.pending_navigation = Some(pending);
    Ok(pending)
}

fn install_pending_symbols(
    model: &mut RustDiagnostics,
    request_id: u32,
    kind: SymbolRequestKind,
) -> Result<PendingSymbols, Box<dyn Error>> {
    let session = model.session.as_mut().ok_or("session")?;
    let query_revision = session
        .symbols
        .as_ref()
        .ok_or_else(|| format!("symbols for pending request {request_id}"))?
        .picker
        .query_revision();
    let pending = PendingSymbols {
        request_id,
        stamp: session.identity.request_stamp().ok_or("request stamp")?,
        kind,
        identity: session.identity,
        process_epoch: session.process_epoch,
        lsp_version: session.lsp_version,
        query_revision,
    };
    session.pending_symbols = Some(pending);
    Ok(pending)
}

fn hover_candidate() -> Result<NavigationCandidate, Box<dyn Error>> {
    let result = completion_result(r#"{"contents":"bounded hover"}"#)?;
    Ok(NavigationCandidate::Hover(Ok(Some(
        HoverContent::admit(&result)?.ok_or("hover")?,
    ))))
}

fn location_candidate(count: usize) -> Result<NavigationCandidate, Box<dyn Error>> {
    let locations = (0..count)
        .map(|index| {
            format!(
                r#"{{"uri":"file:///tmp/source-{index}.rs","range":{{"start":{{"line":0,"character":0}},"end":{{"line":0,"character":1}}}}}}"#
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let result = completion_result(&format!("[{locations}]"))?;
    Ok(NavigationCandidate::Locations(Ok(SourceLocations::admit(
        &result,
    )?)))
}

#[test]
fn navigation_request_guards_and_cancellation_errors_accumulate_visual_state()
-> Result<(), Box<dyn Error>> {
    let position = LspPosition::new(0, 0)?;
    let mut absent = RustDiagnostics::default();
    assert!(
        absent
            .request_navigation(NavigationRequestKind::Hover, position)
            .visual_changed
    );
    assert!(absent.status_message().is_some());
    assert!(
        !absent
            .request_navigation(NavigationRequestKind::Hover, position)
            .visual_changed
    );

    let (mut starting, _, root) = installed_model()?;
    starting.session.as_mut().ok_or("session")?.state = SessionState::Starting;
    assert!(
        starting
            .request_navigation(NavigationRequestKind::Definition, position)
            .visual_changed
    );
    assert!(starting.status_message().is_some());
    assert!(
        !starting
            .request_navigation(NavigationRequestKind::Definition, position)
            .visual_changed
    );

    let (mut invalid, input, root_three) = installed_model()?;
    invalid
        .session
        .as_mut()
        .ok_or("session")?
        .identity
        .document_revision = 0;
    assert!(
        invalid
            .request_navigation(NavigationRequestKind::References, position)
            .visual_changed
    );
    invalid.session.as_mut().ok_or("session")?.identity = input.identity;
    assert!(
        invalid
            .request_navigation(NavigationRequestKind::References, position)
            .visual_changed
    );

    let (mut changed, input, root_four) = installed_model()?;
    let _ = install_pending_navigation(&mut changed, 31, NavigationRequestKind::Hover)?;
    let mut newer = input;
    newer.identity.selection_revision += 1;
    let _ = changed.sync(Some(newer), |_| Arc::new(|| {}));
    assert_eq!(changed.snapshot().navigation_cancellations, 1);

    let (mut cancellation_error, _, root_two) = installed_model()?;
    let _ = install_pending_navigation(
        &mut cancellation_error,
        u32::MAX,
        NavigationRequestKind::Hover,
    )?;
    assert!(cancellation_error.cancel_navigation());
    assert!(cancellation_error.status_message().is_some());
    assert!(!cancellation_error.snapshot().navigation_pending());

    for model in [
        &mut starting,
        &mut invalid,
        &mut changed,
        &mut cancellation_error,
    ] {
        assert!(!model.shutdown().active);
    }
    for directory in [root, root_two, root_three, root_four] {
        std::fs::remove_dir_all(directory)?;
    }
    Ok(())
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one matrix keeps navigation methods, error admissions, observers, and bounded window edges discriminating"
)]
fn navigation_empty_error_observer_and_window_paths_are_discriminating()
-> Result<(), Box<dyn Error>> {
    assert_eq!(
        NavigationRequestKind::from_method("textDocument/hover"),
        Some(NavigationRequestKind::Hover)
    );
    assert_eq!(
        NavigationRequestKind::from_method("textDocument/definition"),
        Some(NavigationRequestKind::Definition)
    );
    assert_eq!(
        NavigationRequestKind::from_method("textDocument/references"),
        Some(NavigationRequestKind::References)
    );
    assert_eq!(NavigationRequestKind::from_method("test/echo"), None);
    assert_eq!(
        NavigationRequestKind::Hover.empty_status(),
        "No Rust hover information."
    );
    assert_eq!(
        NavigationRequestKind::References.empty_status(),
        "No Rust references found."
    );
    assert_eq!(NavigationRequestKind::Hover.label(), "Rust hover");
    assert_eq!(NavigationRequestKind::Definition.label(), "Rust definition");
    assert!(matches!(
        navigation_from_response(
            NavigationRequestKind::Hover,
            ResponseValue::error_for_test()
        ),
        NavigationCandidate::Hover(Err(NavigationError::Malformed))
    ));
    assert!(matches!(
        navigation_from_response(
            NavigationRequestKind::Definition,
            ResponseValue::error_for_test()
        ),
        NavigationCandidate::Locations(Err(NavigationError::Malformed))
    ));

    let (_, _, _, identity) = tests::fixture();
    let stamp = identity.request_stamp().ok_or("request stamp")?;
    let mut absent = RustDiagnostics::default();
    assert!(!absent.admit_navigation(1, stamp, NavigationRequestKind::Hover, hover_candidate()?));
    assert!(!absent.reject_stale_navigation(1));
    assert!(!absent.navigate_navigation(1));

    let (mut model, input, root) = installed_model()?;
    assert!(!model.admit_navigation(2, stamp, NavigationRequestKind::Hover, hover_candidate()?));
    let pending = install_pending_navigation(&mut model, 3, NavigationRequestKind::Hover)?;
    assert!(model.admit_navigation(
        pending.request_id,
        pending.stamp,
        pending.kind,
        NavigationCandidate::Hover(Ok(None))
    ));
    assert_eq!(
        model.status_message().as_deref(),
        Some("No Rust hover information.")
    );
    let pending = install_pending_navigation(&mut model, 4, NavigationRequestKind::References)?;
    assert!(model.admit_navigation(
        pending.request_id,
        pending.stamp,
        pending.kind,
        location_candidate(0)?
    ));
    assert_eq!(
        model.status_message().as_deref(),
        Some("No Rust references found.")
    );
    for (id, candidate) in [
        (
            5,
            NavigationCandidate::Hover(Err(NavigationError::Malformed)),
        ),
        (
            6,
            NavigationCandidate::Locations(Err(NavigationError::Malformed)),
        ),
    ] {
        let kind = if id == 5 {
            NavigationRequestKind::Hover
        } else {
            NavigationRequestKind::Definition
        };
        let pending = install_pending_navigation(&mut model, id, kind)?;
        assert_eq!(
            model.admit_navigation(pending.request_id, pending.stamp, pending.kind, candidate),
            id == 5
        );
        assert!(
            model
                .status_message()
                .is_some_and(|message| message.contains("Navigation"))
        );
    }

    let empty_hover = completion_result(r#"{"contents":[]}"#)?;
    assert_eq!(
        model.install_navigation_for_test(
            input.identity,
            NavigationRequestKind::Hover,
            &empty_hover
        ),
        Err(NavigationError::Malformed)
    );
    let empty_locations = completion_result("[]")?;
    assert_eq!(
        model.install_navigation_for_test(
            input.identity,
            NavigationRequestKind::Definition,
            &empty_locations
        ),
        Err(NavigationError::Malformed)
    );
    let mut wrong = input.identity;
    wrong.selection_revision += 1;
    assert_eq!(
        model.install_navigation_for_test(
            wrong,
            NavigationRequestKind::Hover,
            &completion_result(r#"{"contents":"hover"}"#)?
        ),
        Err(NavigationError::Malformed)
    );

    let hover = completion_result(r#"{"contents":"hover"}"#)?;
    model.install_navigation_for_test(input.identity, NavigationRequestKind::Hover, &hover)?;
    assert_eq!(model.navigation_visible_range(input.identity), None);
    assert!(model.navigation_row(input.identity, 0).is_none());
    assert_eq!(model.selected_source_location(input.identity), None);
    assert!(!model.navigate_navigation(1));

    let many = location_candidate(crate::rust_navigation::MAX_VISIBLE_SOURCE_LOCATIONS + 2)?;
    let NavigationCandidate::Locations(Ok(many)) = many else {
        return Err("locations candidate".into());
    };
    model.install_navigation_for_test(
        input.identity,
        NavigationRequestKind::Definition,
        &completion_result(&serde_json::to_string(
            &(0..crate::rust_navigation::MAX_VISIBLE_SOURCE_LOCATIONS + 2)
                .map(|index| {
                    serde_json::json!({
                        "uri": format!("file:///tmp/window-{index}.rs"),
                        "range": {
                            "start": { "line": 0, "character": 0 },
                            "end": { "line": 0, "character": 1 }
                        }
                    })
                })
                .collect::<Vec<_>>(),
        )?)?,
    )?;
    assert_eq!(
        many.locations().len(),
        crate::rust_navigation::MAX_VISIBLE_SOURCE_LOCATIONS + 2
    );
    assert!(!model.navigate_navigation(0));
    assert!(!model.navigate_navigation(-1));
    for _ in 0..=crate::rust_navigation::MAX_VISIBLE_SOURCE_LOCATIONS {
        assert!(model.navigate_navigation(1));
    }
    assert_eq!(
        model.navigation_visible_range(input.identity),
        Some(2..crate::rust_navigation::MAX_VISIBLE_SOURCE_LOCATIONS + 2)
    );
    assert!(model.navigate_navigation(isize::MIN));
    assert_eq!(model.navigation_visible_range(input.identity), Some(0..12));

    let pending = install_pending_navigation(&mut model, 7, NavigationRequestKind::Hover)?;
    assert!(!model.reject_stale_navigation(pending.request_id));
    assert!(!model.snapshot().navigation_pending());
    let _ = model.shutdown();
    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn navigation_admission_rejects_each_identity_axis_and_counts_exact_results()
-> Result<(), Box<dyn Error>> {
    let (mut model, input, root) = installed_model()?;

    let pending = install_pending_navigation(&mut model, 40, NavigationRequestKind::Hover)?;
    assert!(!model.admit_navigation(
        pending.request_id + 1,
        pending.stamp,
        pending.kind,
        hover_candidate()?
    ));

    let pending = install_pending_navigation(&mut model, 41, NavigationRequestKind::Hover)?;
    let mut other_identity = pending.identity;
    other_identity.selection_revision += 1;
    assert!(!model.admit_navigation(
        pending.request_id,
        other_identity.request_stamp().ok_or("other stamp")?,
        pending.kind,
        hover_candidate()?
    ));

    let pending = install_pending_navigation(&mut model, 42, NavigationRequestKind::Hover)?;
    assert!(!model.admit_navigation(
        pending.request_id,
        pending.stamp,
        NavigationRequestKind::Definition,
        hover_candidate()?
    ));

    let mut pending = install_pending_navigation(&mut model, 43, NavigationRequestKind::Hover)?;
    pending.identity.selection_revision += 1;
    model.session.as_mut().ok_or("session")?.pending_navigation = Some(pending);
    assert!(!model.admit_navigation(
        pending.request_id,
        pending.stamp,
        pending.kind,
        hover_candidate()?
    ));

    let mut pending = install_pending_navigation(&mut model, 44, NavigationRequestKind::Hover)?;
    pending.process_epoch += 1;
    model.session.as_mut().ok_or("session")?.pending_navigation = Some(pending);
    assert!(!model.admit_navigation(
        pending.request_id,
        pending.stamp,
        pending.kind,
        hover_candidate()?
    ));

    let mut pending = install_pending_navigation(&mut model, 45, NavigationRequestKind::Hover)?;
    pending.lsp_version += 1;
    model.session.as_mut().ok_or("session")?.pending_navigation = Some(pending);
    assert!(!model.admit_navigation(
        pending.request_id,
        pending.stamp,
        pending.kind,
        hover_candidate()?
    ));
    assert_eq!(model.snapshot().stale_navigation, 6);

    let pending = install_pending_navigation(&mut model, 46, NavigationRequestKind::Definition)?;
    assert!(model.admit_navigation(
        pending.request_id,
        pending.stamp,
        pending.kind,
        location_candidate(0)?
    ));
    assert!(!model.navigation_is_open(input.identity));
    assert_eq!(
        model.status_message().as_deref(),
        Some("No Rust definition found.")
    );

    let pending = install_pending_navigation(&mut model, 47, NavigationRequestKind::Definition)?;
    assert!(model.admit_navigation(
        pending.request_id,
        pending.stamp,
        pending.kind,
        location_candidate(1)?
    ));
    assert_eq!(model.snapshot().navigation_truncations, 0);

    let pending = install_pending_navigation(&mut model, 48, NavigationRequestKind::References)?;
    assert!(model.admit_navigation(
        pending.request_id,
        pending.stamp,
        pending.kind,
        location_candidate(crate::rust_navigation::MAX_SOURCE_LOCATIONS + 1)?
    ));
    assert_eq!(model.snapshot().navigation_truncations, 1);
    assert!(model.reject_stale_navigation(pending.request_id));
    assert!(!model.navigation_is_open(input.identity));

    let hover = completion_result(r#"{"contents":"identity"}"#)?;
    model.install_navigation_for_test(input.identity, NavigationRequestKind::Hover, &hover)?;
    model.session.as_mut().ok_or("session")?.process_epoch += 1;
    assert!(!model.navigation_is_open(input.identity));
    model.session.as_mut().ok_or("session")?.process_epoch -= 1;
    model.session.as_mut().ok_or("session")?.lsp_version += 1;
    assert!(!model.navigation_is_open(input.identity));

    assert!(!model.shutdown().active);
    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn completion_guards_navigation_and_admission_are_discriminating() -> Result<(), Box<dyn Error>> {
    let (root, _, snapshot, identity) = tests::fixture();
    let position = LspPosition::new(0, 0)?;
    let mut absent = RustDiagnostics::default();
    assert!(absent.request_completion(position).visual_changed);
    assert!(!absent.request_completion(position).visual_changed);
    assert!(!absent.navigate_completion(1));
    assert_eq!(absent.completion_visible_range(identity), None);
    assert!(absent.completion_row(identity, 0).is_none());
    assert_eq!(
        absent.take_selected_completion(identity, &snapshot, 0..0)?,
        None
    );
    let single = CompletionBatch::admit(&completion_result(
        r#"[{"label":"one","insertText":"one"}]"#,
    )?)?;
    let stamp = identity.request_stamp().ok_or("request stamp")?;
    assert!(!absent.admit_completion(1, stamp, Ok(single)));
    assert!(!absent.reject_stale_completion(1));

    let (mut model, input, root_two) = installed_model()?;
    model.session.as_mut().ok_or("session")?.state = SessionState::Starting;
    assert!(model.request_completion(position).visual_changed);
    assert!(!model.request_completion(position).visual_changed);
    model.session.as_mut().ok_or("session")?.state = SessionState::Open;
    model
        .session
        .as_mut()
        .ok_or("session")?
        .identity
        .document_revision = 0;
    assert!(model.request_completion(position).visual_changed);
    model.session.as_mut().ok_or("session")?.identity = input.identity;

    let many = (0..10)
        .map(|index| format!(r#"{{"label":"item-{index}"}}"#))
        .collect::<Vec<_>>()
        .join(",");
    model.install_completion_for_test(
        7,
        input.identity,
        &completion_result(&format!("[{many}]"))?,
    )?;
    for _ in 1..10 {
        assert!(model.navigate_completion(1));
    }
    assert!(!model.navigate_completion(1));
    assert!(model.navigate_completion(-9));
    let mut stale_identity = input.identity;
    stale_identity.selection_revision += 1;
    assert_eq!(model.completion_visible_range(stale_identity), None);
    assert!(model.completion_row(stale_identity, 0).is_none());
    assert_eq!(
        model.take_selected_completion(stale_identity, &input.snapshot, 0..0)?,
        None
    );
    assert_eq!(
        model.take_selected_completion(input.identity, &input.snapshot, 0..0)?,
        None
    );

    assert!(!model.shutdown().active);
    std::fs::remove_dir_all(root)?;
    std::fs::remove_dir_all(root_two)?;
    Ok(())
}

#[test]
fn completion_result_admission_rejects_every_stale_or_invalid_shape() -> Result<(), Box<dyn Error>>
{
    let (mut model, input, root) = installed_model()?;

    assert!(matches!(
        completion_batch_from_response(ResponseValue::error_for_test()),
        Err(CompletionError::Malformed)
    ));

    let empty = completion_result("null")?;
    let error = completion_result(r#"[{"label":"accepted"}]"#)?;
    let pending = install_pending_completion(&mut model, 9)?;
    assert!(model.admit_completion(
        pending.request_id,
        pending.stamp,
        Ok(CompletionBatch::admit(&empty)?)
    ));
    assert!(matches!(
        model.status_message().as_deref(),
        Some("No Rust completions.")
    ));

    let pending = install_pending_completion(&mut model, 10)?;
    assert!(model.admit_completion(
        pending.request_id,
        pending.stamp,
        Err(CompletionError::Malformed)
    ));

    let pending = install_pending_completion(&mut model, 11)?;
    assert!(!model.admit_completion(
        pending.request_id + 1,
        pending.stamp,
        Ok(CompletionBatch::admit(&error)?)
    ));
    assert!(!model.admit_completion(
        pending.request_id,
        pending.stamp,
        Ok(CompletionBatch::admit(&error)?)
    ));

    let pending = install_pending_completion(&mut model, 12)?;
    assert!(!model.reject_stale_completion(pending.request_id));

    assert_eq!(
        model.install_completion_for_test(8, input.identity, &empty),
        Err(RustDiagnosticsError::Completion(CompletionError::Malformed))
    );
    assert!(!model.shutdown().active);
    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn completion_cancellation_and_request_failures_are_bounded() -> Result<(), Box<dyn Error>> {
    let position = LspPosition::new(0, 0)?;
    let (mut changed, input, root) = installed_model()?;
    let _ = install_pending_completion(&mut changed, 21)?;
    let mut newer = input.clone();
    newer.identity.selection_revision += 1;
    assert_eq!(changed.snapshot().completion_cancellations, 0);
    let _ = changed.sync(Some(newer), |_| Arc::new(|| {}));
    assert_eq!(changed.snapshot().completion_cancellations, 1);

    let (mut cancellation_error, _, root_two) = installed_model()?;
    let _ = install_pending_completion(&mut cancellation_error, u32::MAX)?;
    assert!(cancellation_error.cancel_completion());
    assert!(cancellation_error.status_message().is_some());
    assert!(!cancellation_error.snapshot().completion_pending());

    let (mut saturated, _, root_three) = installed_model()?;
    assert!(saturated.request_completion(position).visual_changed);
    assert!(saturated.status_message().is_some());
    assert!(!saturated.snapshot().completion_pending());

    for model in [&mut changed, &mut cancellation_error, &mut saturated] {
        assert!(!model.shutdown().active);
    }
    for directory in [root, root_two, root_three] {
        std::fs::remove_dir_all(directory)?;
    }
    Ok(())
}

fn wait_for_running_peer(model: &mut RustDiagnostics) -> Result<LanguageWake, Box<dyn Error>> {
    let wake = model.current_wake_for_test().ok_or("language wake")?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        let _ = model.poll(wake);
        let running = model.session.as_ref().is_some_and(|session| {
            let snapshot = session.client.snapshot();
            snapshot.started && snapshot.peer.lifecycle() == crate::lsp_json::PeerLifecycle::Running
        });
        if running {
            return Ok(wake);
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    Err("timed out waiting for running language peer".into())
}

fn installed_process_model() -> Result<(RustDiagnostics, PathBuf), Box<dyn Error>> {
    let (root, path, snapshot, identity) = tests::fixture();
    let input = RustDocumentInput::new(&path, &root, identity, snapshot);
    let mut model = RustDiagnostics::with_server(tests::mock_executable());
    let effect = model.sync(Some(input), |_| Arc::new(|| {}));
    if !effect.visual_changed {
        return Err("production language session did not start".into());
    }
    Ok((model, root))
}

#[test]
#[cfg_attr(miri, ignore = "Miri cannot emulate child-process creation")]
fn successful_completion_request_clears_status_and_reports_visual_change()
-> Result<(), Box<dyn Error>> {
    let (mut model, root) = installed_process_model()?;
    let _wake = wait_for_running_peer(&mut model)?;
    let identity = model.session.as_ref().ok_or("session")?.identity;
    let hover = completion_result(r#"{"contents":"completion supersedes hover"}"#)?;
    model.install_navigation_for_test(identity, NavigationRequestKind::Hover, &hover)?;
    assert!(model.navigation_is_open(identity));
    let cancellation = model.request_completion(LspPosition::new(0, 0)?);
    assert!(cancellation.visual_changed);
    assert!(!model.navigation_is_open(identity));
    assert!(model.snapshot().completion_pending());
    let _ = model.cancel_completion();
    assert!(!model.snapshot().completion_pending());

    model.status = Some(Arc::from("old completion status"));
    let effect = model.request_completion(LspPosition::new(0, 0)?);
    assert!(effect.visual_changed);
    assert_eq!(model.status_message(), None);
    assert!(model.snapshot().completion_pending());
    assert_eq!(model.snapshot().completion_requests, 2);
    let _ = model.shutdown();
    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "Miri cannot emulate child-process creation")]
fn poll_ignores_non_completion_response_methods_even_with_matching_stamp()
-> Result<(), Box<dyn Error>> {
    let (mut model, root) = installed_process_model()?;
    let wake = wait_for_running_peer(&mut model)?;
    let mut diagnostics_observed = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        let _ = model.poll(wake);
        diagnostics_observed = model.snapshot().diagnostic_publications > 0;
        if diagnostics_observed {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(diagnostics_observed);
    let pending = {
        let session = model.session.as_mut().ok_or("session")?;
        let stamp = session.identity.request_stamp().ok_or("request stamp")?;
        let submitted = session.client.begin_request("test/echo", None, stamp)?;
        PendingCompletion {
            request_id: submitted.request_id,
            stamp,
            identity: session.identity,
            process_epoch: session.process_epoch,
            lsp_version: session.lsp_version,
        }
    };
    model.session.as_mut().ok_or("session")?.pending_completion = Some(pending);
    let status = model.status_message();
    let mut response_observed = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        let _ = model.poll(wake);
        response_observed = model
            .session
            .as_ref()
            .is_some_and(|session| session.client.snapshot().peer.pending_requests() == 0);
        if response_observed {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(response_observed);
    assert_eq!(
        model.session.as_ref().ok_or("session")?.pending_completion,
        Some(pending)
    );
    assert_eq!(model.status_message(), status);
    assert_eq!(model.snapshot().stale_completions, 0);
    let _ = model.shutdown();
    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn completion_admission_identity_axes_are_rejected_independently() -> Result<(), Box<dyn Error>> {
    let (mut model, input, root) = installed_model()?;
    let result = completion_result(r#"[{"label":"accepted"}]"#)?;

    let pending = install_pending_completion(&mut model, 50)?;
    let mut different_identity = pending.identity;
    different_identity.selection_revision += 1;
    let different_stamp = different_identity
        .request_stamp()
        .ok_or("different stamp")?;
    assert!(!model.admit_completion(
        pending.request_id,
        different_stamp,
        Ok(CompletionBatch::admit(&result)?)
    ));

    let mut pending = install_pending_completion(&mut model, 51)?;
    pending.identity.selection_revision += 1;
    model.session.as_mut().ok_or("session")?.pending_completion = Some(pending);
    assert!(!model.admit_completion(
        pending.request_id,
        pending.stamp,
        Ok(CompletionBatch::admit(&result)?)
    ));

    let mut pending = install_pending_completion(&mut model, 52)?;
    pending.process_epoch += 1;
    model.session.as_mut().ok_or("session")?.pending_completion = Some(pending);
    assert!(!model.admit_completion(
        pending.request_id,
        pending.stamp,
        Ok(CompletionBatch::admit(&result)?)
    ));

    let mut pending = install_pending_completion(&mut model, 53)?;
    pending.lsp_version += 1;
    model.session.as_mut().ok_or("session")?.pending_completion = Some(pending);
    assert!(!model.admit_completion(
        pending.request_id,
        pending.stamp,
        Ok(CompletionBatch::admit(&result)?)
    ));
    assert_eq!(model.snapshot().stale_completions, 4);
    assert_eq!(model.completion_visible_range(input.identity), None);
    let _ = model.shutdown();
    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn completion_observers_reject_process_version_and_language_identity_axes()
-> Result<(), Box<dyn Error>> {
    let (mut model, input, root) = installed_model()?;
    let result = completion_result(r#"[{"label":"accepted","insertText":"accepted"}]"#)?;

    for axis in 0..3 {
        model.install_completion_for_test(60 + axis, input.identity, &result)?;
        match axis {
            0 => {
                model
                    .session
                    .as_mut()
                    .ok_or("session")?
                    .completion
                    .as_mut()
                    .ok_or("completion")?
                    .process_epoch += 1;
            }
            1 => {
                model
                    .session
                    .as_mut()
                    .ok_or("session")?
                    .completion
                    .as_mut()
                    .ok_or("completion")?
                    .lsp_version += 1;
            }
            _ => {}
        }
        let mut observed_identity = input.identity;
        if axis == 2 {
            observed_identity.selection_revision += 1;
        }
        assert_eq!(model.completion_visible_range(observed_identity), None);
        assert!(model.completion_row(observed_identity, 0).is_none());
        assert_eq!(
            model.completion_accessibility_label(observed_identity),
            None
        );
        assert_eq!(
            model.take_selected_completion(observed_identity, &input.snapshot, 0..0)?,
            None
        );
    }
    assert_eq!(model.snapshot().stale_completions, 3);
    let _ = model.shutdown();
    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn completion_navigation_window_edges_and_truncation_counts_are_exact() -> Result<(), Box<dyn Error>>
{
    let (mut model, input, root) = installed_model()?;
    let ten = (0..10)
        .map(|index| format!(r#"{{"label":"item-{index}"}}"#))
        .collect::<Vec<_>>()
        .join(",");
    model.install_completion_for_test(
        70,
        input.identity,
        &completion_result(&format!("[{ten}]"))?,
    )?;
    assert_eq!(model.completion_visible_range(input.identity), Some(0..8));
    for _ in 0..7 {
        assert!(model.navigate_completion(1));
    }
    assert_eq!(model.completion_visible_range(input.identity), Some(0..8));
    assert!(model.navigate_completion(1));
    assert_eq!(model.completion_visible_range(input.identity), Some(1..9));
    assert!(model.navigate_completion(1));
    assert_eq!(model.completion_visible_range(input.identity), Some(2..10));
    assert!(model.navigate_completion(-1));
    assert_eq!(model.completion_visible_range(input.identity), Some(2..10));
    assert!(model.navigate_completion(-8));
    assert_eq!(model.completion_visible_range(input.identity), Some(0..8));

    let omitted = (0..=crate::rust_completion::MAX_COMPLETION_ITEMS)
        .map(|index| format!(r#"{{"label":"item-{index}"}}"#))
        .collect::<Vec<_>>()
        .join(",");
    let pending = install_pending_completion(&mut model, 71)?;
    assert!(model.admit_completion(
        pending.request_id,
        pending.stamp,
        Ok(CompletionBatch::admit(&completion_result(&format!(
            "[{omitted}]"
        ))?)?)
    ));
    assert_eq!(model.snapshot().completion_truncations, 1);
    let pending = install_pending_completion(&mut model, 72)?;
    assert!(model.admit_completion(
        pending.request_id,
        pending.stamp,
        Ok(CompletionBatch::admit(&completion_result(
            r#"[{"label":"not-truncated"}]"#
        )?)?)
    ));
    assert_eq!(model.snapshot().completion_truncations, 1);
    let _ = model.shutdown();
    std::fs::remove_dir_all(root)?;
    Ok(())
}

fn absent_symbol_guards_and_error_statuses_are_explicit() -> Result<(), Box<dyn Error>> {
    let (empty_root, _, _, empty_identity) = tests::fixture();
    let error_value = ResponseValue::error_for_test();
    assert_eq!(
        symbols_from_response(
            SymbolRequestKind::Document,
            error_value,
            "file:///tmp/main.rs"
        ),
        Err(SymbolError::Malformed)
    );
    let mut empty = RustDiagnostics::default();
    assert!(!empty.snapshot().symbol_pending());
    assert!(
        empty
            .open_symbols(SymbolRequestKind::Document)
            .visual_changed
    );
    assert_eq!(
        empty.open_symbols(SymbolRequestKind::Document),
        LanguageEffect::default()
    );
    assert_eq!(empty.issue_symbol_request(), LanguageEffect::default());
    assert!(!empty.cancel_symbols());
    assert_eq!(
        empty.commit_symbol_text(empty_identity, "x"),
        LanguageEffect::default()
    );
    assert_eq!(
        empty.delete_symbol_backward(empty_identity),
        LanguageEffect::default()
    );
    assert!(!empty.begin_symbol_composition(empty_identity));
    assert_eq!(
        empty.update_symbol_composition(empty_identity, "x", 0, 0),
        Err(SymbolError::InvalidComposition)
    );
    assert!(!empty.cancel_symbol_composition(empty_identity));
    assert!(
        empty
            .record_symbol_error(SymbolError::Malformed)
            .visual_changed
    );
    assert!(
        !empty
            .record_symbol_error(SymbolError::Malformed)
            .visual_changed
    );
    assert!(!empty.reject_stale_symbols(1));
    std::fs::remove_dir_all(empty_root)?;
    Ok(())
}

#[test]
fn symbol_guards_composition_and_error_statuses_are_explicit() -> Result<(), Box<dyn Error>> {
    absent_symbol_guards_and_error_statuses_are_explicit()?;
    let (mut model, input, root) = installed_model()?;
    model.session.as_mut().ok_or("session")?.state = SessionState::Starting;
    assert!(
        model
            .open_symbols(SymbolRequestKind::Document)
            .visual_changed
    );
    assert_eq!(
        model.open_symbols(SymbolRequestKind::Document),
        LanguageEffect::default()
    );
    model.session.as_mut().ok_or("session")?.state = SessionState::Open;
    model.session.as_mut().ok_or("session")?.symbols = None;
    assert_eq!(model.issue_symbol_request(), LanguageEffect::default());

    let result = completion_result(
        r#"[{"name":"main","kind":12,"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":4}},"selectionRange":{"start":{"line":0,"character":0},"end":{"line":0,"character":4}}}]"#,
    )?;
    model.install_symbols_for_test(input.identity, SymbolRequestKind::Document, &result)?;
    assert!(model.begin_symbol_composition(input.identity));
    assert!(!model.begin_symbol_composition(input.identity));
    assert_eq!(
        model.update_symbol_composition(input.identity, "x", 1, 0),
        Ok(true)
    );
    assert_eq!(
        model.update_symbol_composition(input.identity, "x", 1, 0),
        Ok(false)
    );
    assert!(model.cancel_symbol_composition(input.identity));
    assert!(!model.cancel_symbol_composition(input.identity));
    assert_eq!(
        model.commit_symbol_text(input.identity, ""),
        LanguageEffect::default()
    );
    assert!(
        model
            .commit_symbol_text(input.identity, "\n")
            .visual_changed
    );
    assert!(model.status_message().is_some());
    assert_eq!(
        model.delete_symbol_backward(input.identity),
        LanguageEffect::default()
    );

    let _pending = install_pending_symbols(&mut model, 900, SymbolRequestKind::Document)?;
    assert!(model.cancel_symbols());
    assert!(model.snapshot().symbol_cancellations <= 1);
    model.install_symbols_for_test(input.identity, SymbolRequestKind::Document, &result)?;
    let pending = install_pending_symbols(&mut model, 901, SymbolRequestKind::Document)?;
    let _ = model.issue_symbol_request();
    assert!(model.snapshot().symbol_cancellations <= 2);
    assert_ne!(pending.request_id, 0);

    let mut wrong_identity = input.identity;
    wrong_identity.selection_revision += 1;
    assert_eq!(
        model.install_symbols_for_test(wrong_identity, SymbolRequestKind::Document, &result),
        Err(SymbolError::Malformed)
    );
    let _ = model.shutdown();
    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn symbol_request_failure_cancellation_and_sync_paths_are_exact() -> Result<(), Box<dyn Error>> {
    let (mut failed, _, failed_root) = installed_model()?;
    let _ = failed
        .session
        .as_mut()
        .ok_or("failed session")?
        .client
        .shutdown();
    assert!(
        failed
            .open_symbols(SymbolRequestKind::Document)
            .visual_changed
    );
    assert!(failed.status_message().is_some());

    let (mut cancelling, _, cancelling_root) = installed_model()?;
    cancelling
        .session
        .as_mut()
        .ok_or("cancelling session")?
        .client
        .initialize_inert_for_test();
    assert!(
        cancelling
            .open_symbols(SymbolRequestKind::Document)
            .visual_changed
    );
    assert!(cancelling.snapshot().symbol_pending());
    assert!(cancelling.cancel_symbols());
    assert!(!cancelling.snapshot().symbol_pending());
    assert_eq!(cancelling.snapshot().symbol_cancellations, 1);

    let (mut syncing, input, syncing_root) = installed_model()?;
    syncing
        .session
        .as_mut()
        .ok_or("syncing session")?
        .client
        .initialize_inert_for_test();
    assert!(
        syncing
            .open_symbols(SymbolRequestKind::Document)
            .visual_changed
    );
    let mut changed = input;
    changed.identity.selection_revision = changed.identity.selection_revision.saturating_add(1);
    assert!(
        syncing
            .sync(Some(changed), |_| Arc::new(|| {}))
            .visual_changed
    );
    assert_eq!(syncing.snapshot().symbol_cancellations, 1);

    let _ = failed.shutdown();
    let _ = cancelling.shutdown();
    let _ = syncing.shutdown();
    std::fs::remove_dir_all(failed_root)?;
    std::fs::remove_dir_all(cancelling_root)?;
    std::fs::remove_dir_all(syncing_root)?;
    Ok(())
}

#[test]
fn symbol_cancellation_sources_and_error_merge_are_independent() -> Result<(), Box<dyn Error>> {
    let (mut model, input, root) = installed_model()?;
    model.install_completion_for_test(
        1,
        input.identity,
        &completion_result(r#"[{"label":"item"}]"#)?,
    )?;
    model.session.as_mut().ok_or("session")?.state = SessionState::Starting;
    model.status = Some(Arc::from("Rust analysis is not ready for navigation."));
    assert!(
        model
            .request_navigation(NavigationRequestKind::Hover, LspPosition::new(0, 0)?)
            .visual_changed
    );

    model.session.as_mut().ok_or("session")?.state = SessionState::Open;
    model.install_navigation_for_test(
        input.identity,
        NavigationRequestKind::Hover,
        &completion_result(r#"{"contents":"hover"}"#)?,
    )?;
    model.session.as_mut().ok_or("session")?.state = SessionState::Starting;
    model.status = Some(Arc::from("Rust analysis is not ready for symbols."));
    assert!(
        model
            .open_symbols(SymbolRequestKind::Document)
            .visual_changed
    );

    let symbols = completion_result(
        r#"[{"name":"main","kind":12,"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":4}},"selectionRange":{"start":{"line":0,"character":0},"end":{"line":0,"character":4}}}]"#,
    )?;
    model.session.as_mut().ok_or("session")?.state = SessionState::Open;
    model.install_symbols_for_test(input.identity, SymbolRequestKind::Document, &symbols)?;
    model.session.as_mut().ok_or("session")?.state = SessionState::Starting;
    assert!(
        model
            .open_symbols(SymbolRequestKind::Document)
            .visual_changed
    );

    model.session.as_mut().ok_or("session")?.state = SessionState::Open;
    model.install_symbols_for_test(input.identity, SymbolRequestKind::Document, &symbols)?;
    let _ = install_pending_symbols(&mut model, 777, SymbolRequestKind::Document)?;
    model.session.as_mut().ok_or("session")?.symbols = None;
    model.status = Some(Arc::from("before cancellation failure"));
    assert!(model.cancel_symbols());
    let _ = model.shutdown();
    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn symbol_identity_query_and_delete_observers_are_independent() -> Result<(), Box<dyn Error>> {
    let (mut model, input, root) = installed_model()?;
    let result = completion_result(
        r#"[{"name":"main","kind":12,"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":4}},"selectionRange":{"start":{"line":0,"character":0},"end":{"line":0,"character":4}}}]"#,
    )?;
    model.install_symbols_for_test(input.identity, SymbolRequestKind::Document, &result)?;
    let mut wrong_identity = input.identity;
    wrong_identity.selection_revision = wrong_identity.selection_revision.saturating_add(1);
    assert_eq!(model.symbol_display_text(wrong_identity), Ok(None));
    assert!(!model.begin_symbol_composition(wrong_identity));
    model
        .session
        .as_mut()
        .and_then(|session| session.symbols.as_mut())
        .ok_or("symbols")?
        .picker
        .commit_text("xy")?;
    assert_eq!(
        model.symbol_display_text(input.identity),
        Ok(Some("xy".to_owned()))
    );
    model
        .session
        .as_mut()
        .ok_or("session")?
        .client
        .initialize_inert_for_test();
    assert!(model.delete_symbol_backward(input.identity).visual_changed);
    assert_eq!(
        model.symbol_display_text(input.identity),
        Ok(Some("x".to_owned()))
    );
    let _ = model.shutdown();
    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn navigation_snapshot_preserves_pending_and_residency_by_result_kind() -> Result<(), Box<dyn Error>>
{
    let (mut model, input, root) = installed_model()?;
    let hover = completion_result(r#"{"contents":"retained hover"}"#)?;
    model.install_navigation_for_test(input.identity, NavigationRequestKind::Hover, &hover)?;
    let _ = install_pending_navigation(&mut model, 801, NavigationRequestKind::Hover)?;
    let hover_snapshot = model.snapshot();
    assert!(hover_snapshot.navigation_pending());
    assert!(hover_snapshot.hover_bytes > 0);
    assert_eq!(hover_snapshot.location_items, 0);
    assert_eq!(hover_snapshot.location_bytes, 0);

    let locations = completion_result(
        r#"[{"uri":"file:///tmp/main.rs","range":{"start":{"line":0,"character":0},"end":{"line":0,"character":1}}}]"#,
    )?;
    model.install_navigation_for_test(
        input.identity,
        NavigationRequestKind::Definition,
        &locations,
    )?;
    let _ = install_pending_navigation(&mut model, 802, NavigationRequestKind::Definition)?;
    let location_snapshot = model.snapshot();
    assert!(location_snapshot.navigation_pending());
    assert_eq!(location_snapshot.hover_bytes, 0);
    assert_eq!(location_snapshot.location_items, 1);
    assert!(location_snapshot.location_bytes > 0);
    let _ = model.shutdown();
    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
// One stateful sequence proves that each stale axis consumes exactly one pending request.
#[allow(clippy::too_many_lines)]
fn symbol_admission_rejects_stale_invalid_empty_truncated_and_oversized_results()
-> Result<(), Box<dyn Error>> {
    let (mut model, input, root) = installed_model()?;
    let result = completion_result(
        r#"[{"name":"main","kind":12,"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":4}},"selectionRange":{"start":{"line":0,"character":0},"end":{"line":0,"character":4}}}]"#,
    )?;
    let batch = SymbolBatch::admit(SymbolRequestKind::Document, &result, "file:///tmp/main.rs")?;
    let mut absent = RustDiagnostics::default();
    assert!(!absent.admit_symbols(
        1,
        input.identity.request_stamp().ok_or("absent stamp")?,
        SymbolRequestKind::Document,
        Ok(batch.clone())
    ));

    model.install_symbols_for_test(input.identity, SymbolRequestKind::Document, &result)?;
    let pending = install_pending_symbols(&mut model, 9, SymbolRequestKind::Document)?;
    let truncations = model.snapshot().symbol_truncations;
    assert!(model.admit_symbols(
        pending.request_id,
        pending.stamp,
        pending.kind,
        Ok(batch.clone())
    ));
    assert_eq!(model.snapshot().symbol_truncations, truncations);
    assert_eq!(model.status_message(), None);

    assert!(!model.admit_symbols(
        1,
        input.identity.request_stamp().ok_or("stamp")?,
        SymbolRequestKind::Document,
        Ok(batch.clone())
    ));

    model.install_symbols_for_test(input.identity, SymbolRequestKind::Document, &result)?;
    let pending = install_pending_symbols(&mut model, 10, SymbolRequestKind::Document)?;
    assert!(!model.admit_symbols(11, pending.stamp, pending.kind, Ok(batch.clone())));

    model.install_symbols_for_test(input.identity, SymbolRequestKind::Document, &result)?;
    let pending = install_pending_symbols(&mut model, 18, SymbolRequestKind::Document)?;
    assert!(!model.admit_symbols(
        pending.request_id,
        pending.stamp,
        SymbolRequestKind::Workspace,
        Ok(batch.clone())
    ));

    model.install_symbols_for_test(input.identity, SymbolRequestKind::Document, &result)?;
    let pending = install_pending_symbols(&mut model, 19, SymbolRequestKind::Document)?;
    model
        .session
        .as_mut()
        .ok_or("session")?
        .identity
        .selection_revision += 1;
    assert!(!model.admit_symbols(
        pending.request_id,
        pending.stamp,
        pending.kind,
        Ok(batch.clone())
    ));
    model.session.as_mut().ok_or("session")?.identity = input.identity;

    model.install_symbols_for_test(input.identity, SymbolRequestKind::Document, &result)?;
    let pending = install_pending_symbols(&mut model, 20, SymbolRequestKind::Document)?;
    model.session.as_mut().ok_or("session")?.process_epoch += 1;
    assert!(!model.admit_symbols(
        pending.request_id,
        pending.stamp,
        pending.kind,
        Ok(batch.clone())
    ));
    model.session.as_mut().ok_or("session")?.process_epoch -= 1;

    model.install_symbols_for_test(input.identity, SymbolRequestKind::Document, &result)?;
    let pending = install_pending_symbols(&mut model, 21, SymbolRequestKind::Document)?;
    model.session.as_mut().ok_or("session")?.lsp_version += 1;
    assert!(!model.admit_symbols(
        pending.request_id,
        pending.stamp,
        pending.kind,
        Ok(batch.clone())
    ));
    model.session.as_mut().ok_or("session")?.lsp_version -= 1;

    model.install_symbols_for_test(input.identity, SymbolRequestKind::Document, &result)?;
    let pending = install_pending_symbols(&mut model, 22, SymbolRequestKind::Document)?;
    model
        .session
        .as_mut()
        .and_then(|session| session.symbols.as_mut())
        .ok_or("symbols before query mutation")?
        .picker
        .commit_text("m")?;
    assert!(!model.admit_symbols(
        pending.request_id,
        pending.stamp,
        pending.kind,
        Ok(batch.clone())
    ));

    model.install_symbols_for_test(input.identity, SymbolRequestKind::Document, &result)?;
    let pending = install_pending_symbols(&mut model, 12, SymbolRequestKind::Document)?;
    model.session.as_mut().ok_or("session")?.symbols = None;
    assert!(!model.admit_symbols(
        pending.request_id,
        pending.stamp,
        pending.kind,
        Ok(batch.clone())
    ));

    model.install_symbols_for_test(input.identity, SymbolRequestKind::Document, &result)?;
    let pending = install_pending_symbols(&mut model, 13, SymbolRequestKind::Document)?;
    assert!(model.admit_symbols(
        pending.request_id,
        pending.stamp,
        pending.kind,
        Err(SymbolError::Malformed)
    ));

    model.install_symbols_for_test(input.identity, SymbolRequestKind::Document, &result)?;
    let pending = install_pending_symbols(&mut model, 14, SymbolRequestKind::Document)?;
    assert!(model.admit_symbols(
        pending.request_id,
        pending.stamp,
        pending.kind,
        Ok(SymbolBatch::oversized_for_test())
    ));

    model.install_symbols_for_test(input.identity, SymbolRequestKind::Document, &result)?;
    model
        .session
        .as_mut()
        .and_then(|session| session.symbols.as_mut())
        .ok_or("symbols before empty-match admission")?
        .picker
        .commit_text("absent")?;
    let pending = install_pending_symbols(&mut model, 15, SymbolRequestKind::Document)?;
    assert!(model.admit_symbols(
        pending.request_id,
        pending.stamp,
        pending.kind,
        Ok(batch.clone())
    ));
    assert!(matches!(
        model.status_message().as_deref(),
        Some("No Rust document symbols.")
    ));

    let values = (0..=(crate::rust_symbols::MAX_SYMBOL_ITEMS + 1))
        .map(|index| format!(
            r#"{{"name":"item-{index}","kind":12,"location":{{"uri":"file:///tmp/main.rs","range":{{"start":{{"line":0,"character":0}},"end":{{"line":0,"character":1}}}}}}}}"#
        ))
        .collect::<Vec<_>>()
        .join(",");
    let workspace = completion_result(&format!("[{values}]"))?;
    model.install_symbols_for_test(input.identity, SymbolRequestKind::Workspace, &workspace)?;
    let pending = install_pending_symbols(&mut model, 16, SymbolRequestKind::Workspace)?;
    let truncated = SymbolBatch::admit(
        SymbolRequestKind::Workspace,
        &workspace,
        "file:///tmp/main.rs",
    )?;
    assert_eq!(truncated.omitted(), 2);
    assert!(model.admit_symbols(
        pending.request_id,
        pending.stamp,
        pending.kind,
        Ok(truncated)
    ));
    assert!(
        model
            .status_message()
            .is_some_and(|message| message.contains("truncated"))
    );

    model.install_symbols_for_test(input.identity, SymbolRequestKind::Document, &result)?;
    let pending = install_pending_symbols(&mut model, 17, SymbolRequestKind::Document)?;
    assert!(!model.reject_stale_symbols(pending.request_id));
    assert!(!model.snapshot().symbol_pending());
    let _ = model.shutdown();
    std::fs::remove_dir_all(root)?;
    Ok(())
}
