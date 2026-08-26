use std::fs;

use alpine_core::{LinearRgba, Size};
use alpine_platform_macos::{
    AccessibilityAction, AccessibilityActionResult, AccessibilityPayload, AccessibilityRequest,
    AccessibilityRequestId, EventTimestamp, ImeEvent, KeyState, Modifiers, ScrollPhase,
    SurfaceEvent,
};
use alpine_runtime::{Application, WorkerConfig};
use alpine_text::Transaction;
use serde_json::value::RawValue;

use super::{
    KEY_DELETE_BACKWARD, KEY_ESCAPE, KEY_RETURN, StudioApp, WorkspaceEditApplicationError,
    WorkspaceEditKind, WorkspaceEditPublicationRequest, accessibility, journal_path_for_session,
    rust_diagnostics::{
        RustDocumentInput, WorkspaceEditIdentity, WorkspaceEditPreparationOutput,
        tests::{diagnostics, fixture, mock_executable},
    },
    rust_navigation::local_file_uri,
    rust_workspace_edit::{PreparedWorkspaceEdit, WorkspaceEditError, WorkspaceEditProposal},
    tests::TestTextSystem,
};

fn prepared_insert(
    path: &std::path::Path,
    root: &std::path::Path,
    text: &str,
) -> Result<PreparedWorkspaceEdit, Box<dyn std::error::Error>> {
    let uri = local_file_uri(path);
    let raw = RawValue::from_string(
        serde_json::json!({
            "changes": {
                (uri): [{
                    "range": {
                        "start": {"line": 0, "character": 0},
                        "end": {"line": 0, "character": 0}
                    },
                    "newText": text
                }]
            }
        })
        .to_string(),
    )?;
    Ok(WorkspaceEditProposal::admit_rename(&raw, root)?.prepare()?)
}

#[test]
#[cfg_attr(
    miri,
    ignore = "Studio construction reaches macOS signpost FFI unsupported by Miri; lower workspace-edit layers remain interpreted"
)]
fn production_publication_queues_commits_admits_and_undoes_active_document()
-> Result<(), Box<dyn std::error::Error>> {
    let (root, path, _, _) = fixture();
    let mut app = StudioApp::open_file(TestTextSystem, &path)?;
    let snapshot = app.buffer().snapshot();
    let original = snapshot.text();
    let language_identity = app.language_identity();
    let session_path = root.join("session-v2.json");
    app.session_path = Some(session_path.clone());
    app.rust_diagnostics.install_for_test(
        RustDocumentInput::new(&path, &root, language_identity, snapshot),
        &diagnostics(&path, 1),
        mock_executable(),
    )?;
    let language = app.rust_diagnostics.snapshot();
    let identity = WorkspaceEditIdentity::for_test(
        language_identity,
        language.process_epoch,
        language.lsp_version,
        71,
        WorkspaceEditKind::Rename,
    );
    let uri = local_file_uri(&path);
    let raw = RawValue::from_string(
        serde_json::json!({
            "changes": {
                (uri): [{
                    "range": {
                        "start": {"line": 0, "character": 0},
                        "end": {"line": 0, "character": 0}
                    },
                    "newText": "renamed_"
                }]
            }
        })
        .to_string(),
    )?;
    let prepared = WorkspaceEditProposal::admit_rename(&raw, &root)?.prepare()?;
    app.workspace_edits
        .install_preview_for_test(identity, prepared)?;

    assert!(
        app.handle_workspace_edit_key(KEY_RETURN, true)
            .is_some_and(|effect| !effect.visual_changed)
    );
    assert!(app.workspace_edits.preview().is_some());
    assert!(
        app.handle_workspace_edit_key(KEY_RETURN, false)
            .is_some_and(|effect| effect.visual_changed)
    );
    let (queued_identity, prepared) = app
        .workspace_edits
        .take_queued_publication()
        .ok_or("publication was not queued")?;
    assert_eq!(queued_identity, identity);
    let journal = journal_path_for_session(&session_path)?;
    let output =
        WorkspaceEditPublicationRequest::new(identity, journal.clone(), prepared).execute();
    assert!(
        app.apply_workspace_edit_publication_output(output)
            .document_changed
    );

    let expected = format!("renamed_{original}");
    assert_eq!(fs::read_to_string(&path)?, expected);
    assert_eq!(app.buffer().snapshot().text(), expected);
    assert!(!app.document.is_dirty());
    assert!(app.buffer_mut().undo()?);
    assert_eq!(app.buffer().snapshot().text(), original);
    assert!(!journal.exists());
    drop(app);
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
#[cfg_attr(
    miri,
    ignore = "Studio construction reaches macOS signpost FFI unsupported by Miri; lower workspace-edit layers remain interpreted"
)]
fn failed_publication_retains_preview_and_freezes_every_mutating_input()
-> Result<(), Box<dyn std::error::Error>> {
    let (root, path, _, _) = fixture();
    let mut app = StudioApp::open_file(TestTextSystem, &path)?;
    let snapshot = app.buffer().snapshot();
    let original = snapshot.text();
    let language_identity = app.language_identity();
    let session_path = root.join("session-v2.json");
    app.session_path = Some(session_path.clone());
    app.rust_diagnostics.install_for_test(
        RustDocumentInput::new(&path, &root, language_identity, snapshot),
        &diagnostics(&path, 1),
        mock_executable(),
    )?;
    let language = app.rust_diagnostics.snapshot();
    let identity = WorkspaceEditIdentity::for_test(
        language_identity,
        language.process_epoch,
        language.lsp_version,
        72,
        WorkspaceEditKind::Rename,
    );
    let prepared = prepared_insert(&path, &root, "renamed_")?;
    app.workspace_edits
        .install_preview_for_test(identity, prepared)?;
    assert!(app.queue_workspace_edit_publication().visual_changed);
    let (queued_identity, prepared) = app
        .workspace_edits
        .take_queued_publication()
        .ok_or("publication was not queued")?;

    let before = app.buffer().snapshot().text();
    assert!(
        app.handle_workspace_edit_key(KEY_RETURN, false)
            .is_some_and(|effect| !effect.document_changed)
    );
    let keyboard = SurfaceEvent::Keyboard {
        timestamp: EventTimestamp::new(1),
        state: KeyState::Down,
        physical_key: 8,
        logical_key: "c".into(),
        modifiers: Modifiers::from_bits(Modifiers::COMMAND),
        repeat: false,
    };
    assert!(!app.handle_event(&keyboard).document_changed);
    let scroll = SurfaceEvent::Scroll {
        timestamp: EventTimestamp::new(2),
        delta_x: 0.0,
        delta_y: 80.0,
        phase: ScrollPhase::Changed,
        precise: true,
        modifiers: Modifiers::default(),
    };
    assert!(!app.handle_event(&scroll).visual_changed);
    assert!(app.handle_close_request().cancel_close);
    let observed = app.accessibility_snapshot()?.revision();
    let action = AccessibilityAction::set_selection(observed, 0, 0);
    let request = AccessibilityRequest::action(AccessibilityRequestId::new(1), action)?;
    let (response, effect) = accessibility::respond(&mut app, &request);
    assert!(!effect.visual_changed);
    assert!(matches!(
        response.result(),
        Ok(AccessibilityPayload::Action(
            AccessibilityActionResult::Unchanged
        ))
    ));
    assert_eq!(app.buffer().snapshot().text(), before);

    fs::write(&path, "external_change")?;
    let journal = journal_path_for_session(&session_path)?;
    let output = WorkspaceEditPublicationRequest::new(queued_identity, journal, prepared).execute();
    let effect = app.apply_workspace_edit_publication_output(output);
    assert!(effect.visual_changed);
    assert!(!effect.document_changed);
    assert!(app.workspace_edits.preview().is_some());
    assert_eq!(app.buffer().snapshot().text(), original);
    assert_eq!(fs::read_to_string(&path)?, "external_change");
    fs::write(&path, original)?;
    drop(app);
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
#[cfg_attr(
    miri,
    ignore = "Studio construction reaches macOS signpost FFI unsupported by Miri; lower workspace-edit layers remain interpreted"
)]
fn publication_admits_committed_bytes_into_an_inactive_loaded_tab()
-> Result<(), Box<dyn std::error::Error>> {
    let (root, first_path, _, _) = fixture();
    let first_original = fs::read_to_string(&first_path)?;
    let second_path = root.join("second.rs");
    fs::write(&second_path, "fn second() {}\n")?;
    let second_path = fs::canonicalize(second_path)?;
    let mut app = StudioApp::open_file(TestTextSystem, &first_path)?;
    app.open_workspace_path(&second_path, None)?;
    let second_original = app.buffer().snapshot().text();
    let snapshot = app.buffer().snapshot();
    let language_identity = app.language_identity();
    let session_path = root.join("session-v2.json");
    app.session_path = Some(session_path.clone());
    app.rust_diagnostics.install_for_test(
        RustDocumentInput::new(&second_path, &root, language_identity, snapshot),
        &diagnostics(&second_path, 1),
        mock_executable(),
    )?;
    let language = app.rust_diagnostics.snapshot();
    let identity = WorkspaceEditIdentity::for_test(
        language_identity,
        language.process_epoch,
        language.lsp_version,
        73,
        WorkspaceEditKind::Rename,
    );
    let prepared = prepared_insert(&first_path, &root, "inactive_")?;
    app.workspace_edits
        .install_preview_for_test(identity, prepared)?;
    assert!(app.queue_workspace_edit_publication().visual_changed);
    let (queued_identity, prepared) = app
        .workspace_edits
        .take_queued_publication()
        .ok_or("publication was not queued")?;
    let output = WorkspaceEditPublicationRequest::new(
        queued_identity,
        journal_path_for_session(&session_path)?,
        prepared,
    )
    .execute();
    let effect = app.apply_workspace_edit_publication_output(output);
    assert!(!effect.document_changed);
    assert_eq!(app.buffer().snapshot().text(), second_original);
    let inactive = app
        .tabs
        .inactive_document_for_path(&first_path)
        .ok_or("inactive document was not retained")?;
    assert_eq!(
        inactive.buffer().snapshot().text(),
        format!("inactive_{first_original}")
    );
    assert!(!inactive.is_dirty());
    drop(app);
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
#[cfg_attr(
    miri,
    ignore = "Studio construction reaches macOS signpost FFI unsupported by Miri; lower workspace-edit layers remain interpreted"
)]
fn locally_diverged_inactive_tab_rejects_publication_before_disk_mutation()
-> Result<(), Box<dyn std::error::Error>> {
    let (root, first_path, _, _) = fixture();
    let first_original = fs::read_to_string(&first_path)?;
    let second_path = root.join("second.rs");
    fs::write(&second_path, "fn second() {}\n")?;
    let second_path = fs::canonicalize(second_path)?;
    let mut app = StudioApp::open_file(TestTextSystem, &first_path)?;
    app.open_workspace_path(&second_path, None)?;
    let snapshot = app.buffer().snapshot();
    let language_identity = app.language_identity();
    app.session_path = Some(root.join("session-v2.json"));
    app.rust_diagnostics.install_for_test(
        RustDocumentInput::new(&second_path, &root, language_identity, snapshot),
        &diagnostics(&second_path, 1),
        mock_executable(),
    )?;
    let language = app.rust_diagnostics.snapshot();
    let identity = WorkspaceEditIdentity::for_test(
        language_identity,
        language.process_epoch,
        language.lsp_version,
        74,
        WorkspaceEditKind::Rename,
    );
    let prepared = prepared_insert(&first_path, &root, "rejected_")?;
    let inactive = app
        .tabs
        .inactive_document_mut_for_path(&first_path)
        .ok_or("inactive document was not retained")?;
    let mut transaction = Transaction::new(inactive.buffer().revision());
    transaction.replace(0..0, "local_")?;
    inactive.buffer_mut().apply(transaction)?;
    app.workspace_edits
        .install_preview_for_test(identity, prepared)?;
    let effect = app.queue_workspace_edit_publication();
    assert!(effect.visual_changed);
    assert!(app.workspace_edits.take_queued_publication().is_none());
    assert_eq!(fs::read_to_string(&first_path)?, first_original);
    assert!(
        app.tabs
            .inactive_document_for_path(&first_path)
            .is_some_and(super::StudioDocument::is_dirty)
    );
    drop(app);
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
#[cfg_attr(
    miri,
    ignore = "Studio construction reaches macOS signpost FFI unsupported by Miri; lower workspace-edit layers remain interpreted"
)]
fn rust_rename_and_format_commands_use_bounded_workspace_edit_lifecycle()
-> Result<(), Box<dyn std::error::Error>> {
    let (root, path, _, _) = fixture();
    let mut app = StudioApp::open_file(TestTextSystem, &path)?;
    let snapshot = app.buffer().snapshot();
    let language_identity = app.language_identity();
    app.rust_diagnostics.install_for_test(
        RustDocumentInput::new(&path, &root, language_identity, snapshot),
        &diagnostics(&path, 1),
        mock_executable(),
    )?;

    assert!(app.open_rust_rename().visual_changed);
    assert!(app.workspace_edits.is_rename_input());
    assert!(
        app.handle_workspace_edit_ime(&ImeEvent::Started)
            .visual_changed
    );
    assert!(
        app.handle_workspace_edit_ime(&ImeEvent::Updated {
            text: "ré".into(),
            selected_start_utf16: 1,
            selected_length_utf16: 1,
        })
        .visual_changed
    );
    assert!(
        app.handle_workspace_edit_ime(&ImeEvent::Cancelled)
            .visual_changed
    );
    assert!(
        app.handle_workspace_edit_key(0, false)
            .is_some_and(|effect| !effect.visual_changed)
    );
    assert!(app.workspace_edits.commit_text("renamed")?);
    assert!(
        app.handle_workspace_edit_key(KEY_DELETE_BACKWARD, true)
            .is_some_and(|effect| !effect.visual_changed)
    );
    assert!(app.workspace_edits.is_rename_input());
    assert!(
        app.handle_workspace_edit_key(KEY_DELETE_BACKWARD, false)
            .is_some_and(|effect| effect.visual_changed)
    );
    assert!(app.workspace_edits.commit_text("d")?);
    assert!(
        app.handle_workspace_edit_key(KEY_RETURN, true)
            .is_some_and(|effect| !effect.visual_changed)
    );
    assert!(app.workspace_edits.is_rename_input());
    assert!(
        app.handle_workspace_edit_key(KEY_RETURN, false)
            .is_some_and(|effect| effect.visual_changed)
    );
    assert!(!app.rust_diagnostics.snapshot().workspace_edit_pending);
    assert!(!app.workspace_edits.is_open());
    assert!(!app.cancel_workspace_edit_panel().visual_changed);
    assert!(!app.rust_diagnostics.snapshot().workspace_edit_pending);

    let _ = app.trigger_rust_formatting();
    assert!(!app.rust_diagnostics.snapshot().workspace_edit_pending);
    assert!(!app.workspace_edits.is_open());

    assert!(app.open_rust_rename().visual_changed);
    assert!(
        app.handle_workspace_edit_key(KEY_ESCAPE, true)
            .is_some_and(|effect| effect.visual_changed)
    );
    assert!(!app.workspace_edits.is_open());
    assert!(
        !app.handle_workspace_edit_ime(&ImeEvent::Started)
            .visual_changed
    );
    assert!(
        !WorkspaceEditApplicationError::MissingPersistence
            .to_string()
            .is_empty()
    );
    let mut formatting_app = StudioApp::open_file(TestTextSystem, &path)?;
    let formatting_snapshot = formatting_app.buffer().snapshot();
    let formatting_identity = formatting_app.language_identity();
    formatting_app.rust_diagnostics.install_for_test(
        RustDocumentInput::new(&path, &root, formatting_identity, formatting_snapshot),
        &diagnostics(&path, 1),
        mock_executable(),
    )?;
    assert!(formatting_app.trigger_rust_formatting().visual_changed);
    drop(app);
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
#[cfg_attr(
    miri,
    ignore = "Studio construction reaches macOS signpost FFI unsupported by Miri; lower workspace-edit layers remain interpreted"
)]
fn preparation_outcomes_publish_only_current_nonempty_previews()
-> Result<(), Box<dyn std::error::Error>> {
    let (root, path, _, _) = fixture();
    let mut app = StudioApp::open_file(TestTextSystem, &path)?;
    let snapshot = app.buffer().snapshot();
    let language_identity = app.language_identity();
    app.rust_diagnostics.install_for_test(
        RustDocumentInput::new(&path, &root, language_identity, snapshot),
        &diagnostics(&path, 1),
        mock_executable(),
    )?;
    let language = app.rust_diagnostics.snapshot();
    let identity = WorkspaceEditIdentity::for_test(
        language_identity,
        language.process_epoch,
        language.lsp_version,
        75,
        WorkspaceEditKind::Rename,
    );
    let other = WorkspaceEditIdentity::for_test(
        language_identity,
        language.process_epoch,
        language.lsp_version,
        76,
        WorkspaceEditKind::Rename,
    );
    let prepared = prepared_insert(&path, &root, "preview_")?;

    app.workspace_edits.wait(WorkspaceEditKind::Rename)?;
    app.workspace_edits.preparation_started(identity)?;
    let ignored = app.apply_workspace_edit_output(WorkspaceEditPreparationOutput {
        identity: other,
        wire_bytes: 1,
        result: Ok(prepared.clone()),
    });
    assert!(!ignored.visual_changed);

    let preview = app.apply_workspace_edit_output(WorkspaceEditPreparationOutput {
        identity,
        wire_bytes: 23,
        result: Ok(prepared.clone()),
    });
    assert!(preview.visual_changed);
    assert!(app.workspace_edits.preview().is_some());
    assert!(
        app.handle_workspace_edit_key(KEY_ESCAPE, true)
            .is_some_and(|effect| effect.visual_changed)
    );

    app.workspace_edits.wait(WorkspaceEditKind::Rename)?;
    app.workspace_edits.preparation_started(identity)?;
    let empty = app.apply_workspace_edit_output(WorkspaceEditPreparationOutput {
        identity,
        wire_bytes: 2,
        result: Ok(prepared.publication_fixture_for_test(0, true)),
    });
    assert!(empty.visual_changed);
    assert!(!app.workspace_edits.is_open());

    app.workspace_edits.wait(WorkspaceEditKind::Rename)?;
    app.workspace_edits.preparation_started(identity)?;
    let malformed = app.apply_workspace_edit_output(WorkspaceEditPreparationOutput {
        identity,
        wire_bytes: 3,
        result: Err(WorkspaceEditError::Malformed),
    });
    assert!(malformed.visual_changed);
    assert!(!app.workspace_edits.is_open());
    drop(app);
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
#[cfg_attr(
    miri,
    ignore = "Studio construction reaches macOS signpost FFI unsupported by Miri; lower workspace-edit layers remain interpreted"
)]
#[allow(clippy::too_many_lines)]
fn runtime_workspace_edit_handoffs_invalidate_and_admit_production_frames()
-> Result<(), Box<dyn std::error::Error>> {
    let (root, path, _, _) = fixture();
    let viewport = Size::new(900.0, 600.0).ok_or("viewport")?;
    let clear = LinearRgba::new(0.02, 0.02, 0.02, 1.0).ok_or("clear")?;

    let mut preparation_app = StudioApp::open_file(TestTextSystem, &path)?;
    let snapshot = preparation_app.buffer().snapshot();
    let language_identity = preparation_app.language_identity();
    preparation_app.rust_diagnostics.install_for_test(
        RustDocumentInput::new(&path, &root, language_identity, snapshot),
        &diagnostics(&path, 1),
        mock_executable(),
    )?;
    let language = preparation_app.rust_diagnostics.snapshot();
    let preparation_identity = WorkspaceEditIdentity::for_test(
        language_identity,
        language.process_epoch,
        language.lsp_version,
        81,
        WorkspaceEditKind::Rename,
    );
    let uri = local_file_uri(&path);
    let raw = RawValue::from_string(
        serde_json::json!({
            "changes": {
                (uri.clone()): [{
                    "range": {
                        "start": {"line": 0, "character": 0},
                        "end": {"line": 0, "character": 0}
                    },
                    "newText": "runtime_"
                }]
            }
        })
        .to_string(),
    )?;
    preparation_app
        .workspace_edits
        .wait(WorkspaceEditKind::Rename)?;
    preparation_app
        .rust_diagnostics
        .stage_workspace_edit_preparation_for_test(preparation_identity, &root, &uri, &raw)?;
    let mut preparation_runtime =
        Application::new(preparation_app, viewport, clear, WorkerConfig::default())?;
    preparation_runtime
        .frame_if_dirty()
        .ok_or("initial preparation frame")?;
    assert!(
        preparation_runtime
            .dispatch(&SurfaceEvent::Wake {
                timestamp: EventTimestamp::new(80),
            })
            .is_some()
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while preparation_runtime.snapshot().worker().queued_results() == 0 {
        if std::time::Instant::now() >= deadline {
            return Err("workspace edit preparation worker timed out".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert!(
        preparation_runtime
            .dispatch(&SurfaceEvent::Wake {
                timestamp: EventTimestamp::new(81),
            })
            .is_some()
    );
    drop(preparation_runtime);

    let mut publication_app = StudioApp::open_file(TestTextSystem, &path)?;
    let snapshot = publication_app.buffer().snapshot();
    let language_identity = publication_app.language_identity();
    publication_app.rust_diagnostics.install_for_test(
        RustDocumentInput::new(&path, &root, language_identity, snapshot),
        &diagnostics(&path, 1),
        mock_executable(),
    )?;
    let language = publication_app.rust_diagnostics.snapshot();
    let publication_identity = WorkspaceEditIdentity::for_test(
        language_identity,
        language.process_epoch,
        language.lsp_version,
        82,
        WorkspaceEditKind::Rename,
    );
    publication_app.workspace_edits.install_preview_for_test(
        publication_identity,
        prepared_insert(&path, &root, "publish_")?,
    )?;
    assert!(
        publication_app
            .workspace_edits
            .queue_publication(language_identity, &language)?
    );
    let mut publication_runtime =
        Application::new(publication_app, viewport, clear, WorkerConfig::default())?;
    publication_runtime
        .frame_if_dirty()
        .ok_or("initial publication frame")?;
    assert!(
        publication_runtime
            .dispatch(&SurfaceEvent::Wake {
                timestamp: EventTimestamp::new(82),
            })
            .is_some()
    );
    drop(publication_runtime);
    fs::remove_dir_all(root)?;
    Ok(())
}
