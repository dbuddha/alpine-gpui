use std::{
    error::Error,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use super::*;

static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

struct TestRoot(PathBuf);

impl TestRoot {
    fn new() -> std::io::Result<Self> {
        let id = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("alpine-settings-app-{}-{id}", std::process::id()));
        fs::create_dir_all(&path)?;
        Ok(Self(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn production_application_admission_preserves_editor_and_workspace_state()
-> Result<(), Box<dyn Error>> {
    let root = TestRoot::new()?;
    fs::write(root.path().join("main.rs"), "fn main() {}\n")?;
    let settings_path = root.path().join(".alpine/settings.json");
    fs::create_dir_all(settings_path.parent().ok_or("missing settings parent")?)?;
    fs::write(
        &settings_path,
        br#"{"version":1,"editor":{"font_size":19,"tab_columns":2},"theme":{"caret":[0.2,0.4,0.6,1.0]}}"#,
    )?;
    let mut app = StudioApp::open_workspace(tests::TestTextSystem, root.path())?;
    app.settings_reload = settings::SettingsReload::explicit(None, Some(settings_path));
    let document_revision = app.runtime_document_revision;
    let workspace_revision = app.runtime_workspace_revision;
    let selection = app.selection;
    let active_tab = app.tabs.active_id()?;
    let workspace_root = app
        .workspace
        .as_ref()
        .map(|workspace| workspace.root().to_path_buf());
    let request = app
        .settings_reload
        .take_request()
        .ok_or("missing settings request")?;
    let effect = app.apply_settings_output(request.execute());
    assert!(effect.visual_changed);
    assert!((app.settings.active().editor.font_size - 19.0).abs() < f32::EPSILON);
    assert_eq!(app.settings.active().editor.tab_columns, 2);
    assert_eq!(app.runtime_document_revision, document_revision);
    assert_eq!(app.runtime_workspace_revision, workspace_revision);
    assert_eq!(app.selection, selection);
    assert_eq!(app.tabs.active_id()?, active_tab);
    assert_eq!(
        app.workspace
            .as_ref()
            .map(|workspace| workspace.root().to_path_buf()),
        workspace_root
    );
    assert_eq!(app.local_status, None);
    Ok(())
}

#[test]
fn command_reload_announces_but_startup_keymap_only_does_not_redraw() -> Result<(), Box<dyn Error>>
{
    let root = TestRoot::new()?;
    let path = root.path().join("settings.json");
    fs::write(
        &path,
        br#"{"version":1,"keymap":{"bindings":[{"physical_key":1,"modifiers":["command"],"action":"save_file","label":"Cmd+S"}]}}"#,
    )?;
    let mut app = StudioApp::new(tests::TestTextSystem)?;
    app.settings_reload = settings::SettingsReload::explicit(Some(path.clone()), None);
    let startup = app
        .settings_reload
        .take_request()
        .ok_or("missing startup request")?;
    assert_eq!(
        app.apply_settings_output(startup.execute()),
        EventEffect::default()
    );
    let command_context = app.command_context();
    assert!(app.command_palette.open(command_context)?);
    fs::write(
        &path,
        br#"{"version":1,"keymap":{"bindings":[{"physical_key":2,"modifiers":["command"],"action":"reload_settings","label":"Cmd+R"}]}}"#,
    )?;
    app.settings_reload.request(false)?;
    let palette_update = app
        .settings_reload
        .take_request()
        .ok_or("missing palette settings request")?;
    assert_eq!(
        app.apply_settings_output(palette_update.execute()),
        EventEffect::visual()
    );
    fs::write(
        &path,
        br#"{"version":1,"editor":{"font_size":20},"keymap":{"bindings":[{"physical_key":2,"modifiers":["command"],"action":"reload_settings","label":"Cmd+R"}]}}"#,
    )?;
    assert_eq!(
        app.dispatch_command(StudioCommand::ReloadSettings),
        EventEffect::visual()
    );
    let manual = app
        .settings_reload
        .take_request()
        .ok_or("missing manual request")?;
    assert_eq!(
        app.apply_settings_output(manual.execute()),
        EventEffect::visual()
    );
    assert!(matches!(app.local_status, Some(LocalStatus::Command(_))));
    Ok(())
}

#[test]
fn runtime_dispatch_submits_and_publishes_the_startup_settings_request()
-> Result<(), Box<dyn Error>> {
    let root = TestRoot::new()?;
    let path = root.path().join("settings.json");
    fs::write(&path, br#"{"version":1,"editor":{"font_size":19}}"#)?;
    let mut app = StudioApp::new(tests::TestTextSystem)?;
    app.settings_reload = settings::SettingsReload::explicit(Some(path), None);
    let viewport = Size::new(800.0, 600.0).ok_or("invalid viewport")?;
    let clear = LinearRgba::new(0.02, 0.02, 0.02, 1.0).ok_or("invalid clear color")?;
    let mut runtime = Application::new(app, viewport, clear, WorkerConfig::default())?;
    assert!(runtime.frame_if_dirty().is_some());
    assert!(runtime.frame_if_dirty().is_none());
    let _ = runtime.dispatch(&SurfaceEvent::Wake {
        timestamp: EventTimestamp::new(1),
    });
    assert_eq!(runtime.snapshot().worker().peak_queued_requests(), 1);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut timestamp = 2;
    loop {
        if runtime
            .dispatch(&SurfaceEvent::Wake {
                timestamp: EventTimestamp::new(timestamp),
            })
            .is_some()
        {
            break;
        }
        if std::time::Instant::now() >= deadline {
            return Err(format!(
                "timed out waiting for settings worker frame: {:?}",
                runtime.snapshot().worker()
            )
            .into());
        }
        timestamp = timestamp.checked_add(1).ok_or("wake timestamp exhausted")?;
        std::thread::yield_now();
    }
    assert_eq!(runtime.snapshot().worker().queued_requests(), 0);
    assert_eq!(runtime.snapshot().worker().queued_results(), 0);
    Ok(())
}

#[test]
fn settings_submission_retries_only_saturation_without_a_failure_banner()
-> Result<(), Box<dyn Error>> {
    let mut app = StudioApp::new(tests::TestTextSystem)?;
    app.settings_reload = settings::SettingsReload::explicit(None, None);
    app.settings_reload.request(true)?;
    let request = app
        .settings_reload
        .take_request()
        .ok_or("missing settings request")?;
    let generation = request.generation();
    assert!(
        !app.settings_reload
            .defer_submission(generation.saturating_add(1), true)
    );
    assert!(app.settings_reload.report().in_flight);
    assert_eq!(app.settings_reload.report().stale_results, 1);

    assert!(!app.apply_settings_submission_result(
        generation.saturating_add(1),
        true,
        Err(SubmitError::Closed)
    ));
    assert_eq!(app.local_status, None);
    assert!(app.settings_reload.report().in_flight);
    assert_eq!(app.settings_reload.report().stale_results, 2);

    assert!(!app.apply_settings_submission_result(
        generation,
        request.announce(),
        Err(SubmitError::Saturated)
    ));
    assert_eq!(app.local_status, None);
    assert!(!app.settings_reload.report().in_flight);
    assert!(app.settings_reload.report().pending);
    assert_eq!(app.settings_reload.report().failures, 0);
    let retry = app
        .settings_reload
        .take_request()
        .ok_or("missing settings retry")?;
    assert_eq!(retry.generation(), generation);
    assert!(retry.announce());
    assert_eq!(app.settings_reload.report().submissions, 2);

    assert!(app.apply_settings_submission_result(
        retry.generation(),
        retry.announce(),
        Err(SubmitError::Closed)
    ));
    assert_eq!(
        app.local_status,
        Some(LocalStatus::Command(Arc::from(
            "Settings reload failed: settings worker queue rejected reload"
        )))
    );
    assert_eq!(app.settings_reload.report().failures, 1);
    assert!(app.settings_reload.report().pending);

    let terminal_retry = app
        .settings_reload
        .take_request()
        .ok_or("missing terminal settings retry")?;
    app.local_status = None;
    assert!(app.apply_settings_submission_result(
        terminal_retry.generation(),
        terminal_retry.announce(),
        Err(SubmitError::SequenceExhausted)
    ));
    assert!(matches!(app.local_status, Some(LocalStatus::Command(_))));
    assert_eq!(app.settings_reload.report().failures, 2);
    Ok(())
}

#[test]
fn failed_application_reload_keeps_the_previous_settings_snapshot() -> Result<(), Box<dyn Error>> {
    let root = TestRoot::new()?;
    let path = root.path().join("settings.json");
    fs::write(&path, br#"{"version":1,"editor":{"font_size":17}}"#)?;
    let mut app = StudioApp::new(tests::TestTextSystem)?;
    app.settings_reload = settings::SettingsReload::explicit(Some(path.clone()), None);
    let accepted = app
        .settings_reload
        .take_request()
        .ok_or("missing accepted request")?;
    assert!(app.apply_settings_output(accepted.execute()).visual_changed);
    let previous = app.settings.active().clone();
    fs::write(&path, br#"{"version":99}"#)?;
    app.settings_reload.request(true)?;
    let rejected = app
        .settings_reload
        .take_request()
        .ok_or("missing rejected request")?;
    assert!(app.apply_settings_output(rejected.execute()).visual_changed);
    assert_eq!(app.settings.active(), &previous);
    assert!(matches!(app.local_status, Some(LocalStatus::Command(_))));
    Ok(())
}

#[test]
fn application_settings_admission_reports_every_terminal_result() -> Result<(), Box<dyn Error>> {
    let root = TestRoot::new()?;
    let path = root.path().join("settings.json");
    fs::write(&path, br#"{"version":1,"editor":{"font_size":17}}"#)?;
    let mut app = StudioApp::new(tests::TestTextSystem)?;
    app.settings_reload = settings::SettingsReload::explicit(Some(path.clone()), None);
    let accepted = app
        .settings_reload
        .take_request()
        .ok_or("missing accepted request")?;
    assert!(app.apply_settings_output(accepted.execute()).visual_changed);

    app.settings_reload.request(true)?;
    let unchanged = app
        .settings_reload
        .take_request()
        .ok_or("missing unchanged request")?;
    assert!(
        app.apply_settings_output(unchanged.execute())
            .visual_changed
    );
    assert!(matches!(app.local_status, Some(LocalStatus::Command(_))));

    app.settings_reload.request(false)?;
    let stale = app
        .settings_reload
        .take_request()
        .ok_or("missing stale request")?;
    app.settings_reload.request(false)?;
    assert_eq!(
        app.apply_settings_output(stale.execute()),
        EventEffect::default()
    );

    let rejected = app
        .settings_reload
        .take_request()
        .ok_or("missing rejected request")?;
    fs::write(&path, br#"{"version":1,"editor":{"font_size":0}}"#)?;
    assert!(app.apply_settings_output(rejected.execute()).visual_changed);
    assert!(matches!(app.local_status, Some(LocalStatus::Command(_))));

    app.settings_reload.exhaust_generation();
    assert!(app.request_settings_reload().visual_changed);
    assert!(matches!(app.local_status, Some(LocalStatus::Command(_))));

    app.local_status = None;
    record_project_settings_result(&mut app, Err::<bool, _>("settings generation exhausted"));
    assert_eq!(
        app.local_status,
        Some(LocalStatus::Command(Arc::from(
            "Settings project override failed: settings generation exhausted"
        )))
    );
    record_project_settings_result(&mut app, Ok::<bool, &str>(false));
    assert_eq!(
        app.local_status,
        Some(LocalStatus::Command(Arc::from(
            "Settings project override failed: settings generation exhausted"
        )))
    );
    Ok(())
}
