use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use alpine_platform_macos::{EventTimestamp, ImeEvent, InputEpoch, SurfaceEvent};

use super::open_paths::RestoredLaunch;
use super::tests::TestTextSystem;
use super::*;

type TestResult = Result<(), Box<dyn std::error::Error>>;

static NEXT_SCRATCH: AtomicU64 = AtomicU64::new(1);

struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new(label: &str) -> std::io::Result<Self> {
        let root = std::env::temp_dir().join(format!(
            "alpine-open-paths-{label}-{}-{}",
            std::process::id(),
            NEXT_SCRATCH.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root)?;
        Ok(Self {
            root: fs::canonicalize(root)?,
        })
    }

    fn file(&self, name: &str, text: &str) -> std::io::Result<PathBuf> {
        let path = self.root.join(name);
        fs::write(&path, text)?;
        Ok(path)
    }

    fn folder(&self, name: &str) -> std::io::Result<PathBuf> {
        let path = self.root.join(name);
        fs::create_dir_all(&path)?;
        Ok(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn opened(paths: &[&Path], dropped: usize) -> SurfaceEvent {
    SurfaceEvent::OpenPaths {
        timestamp: EventTimestamp::new(1),
        paths: paths.iter().map(|path| path.to_path_buf()).collect(),
        dropped,
    }
}

fn status(app: &EditorApp) -> &str {
    app.local_status.as_ref().map_or("", LocalStatus::message)
}

fn single_pane() -> session::SessionPanes {
    session::SessionPanes {
        nodes: [
            session::SessionNode::Leaf { pane: 0 },
            session::SessionNode::Empty,
            session::SessionNode::Empty,
            session::SessionNode::Empty,
            session::SessionNode::Empty,
            session::SessionNode::Empty,
            session::SessionNode::Empty,
        ],
        panes: [
            Some(session::SessionPane {
                tab: 0,
                view: DocumentViewState::default(),
            }),
            None,
            None,
            None,
        ],
        active_pane: 0,
    }
}

// A previous session: a workspace with two clean tabs, the first active.
fn restored_session(workspace: &Scratch) -> Result<EditorApp, Box<dyn std::error::Error>> {
    let state = session_state(workspace)?;
    Ok(EditorApp::from_session(TestTextSystem, state).map_err(|error| error.to_string())?)
}

fn session_state(workspace: &Scratch) -> std::io::Result<session::SessionState> {
    let alpha = workspace.file("alpha.rs", "alpha\n")?;
    let beta = workspace.file("beta.rs", "beta\n")?;
    Ok(session::SessionState {
        workspace: Some(workspace.root.clone()),
        tabs: vec![
            session::SessionTab {
                path: Some(alpha),
                view: DocumentViewState::default(),
            },
            session::SessionTab {
                path: Some(beta),
                view: DocumentViewState::default(),
            },
        ],
        active_tab: 0,
        panes: single_pane(),
        file_tree: session::SessionFileTree::default(),
    })
}

#[test]
fn two_opened_files_become_two_tabs_and_the_last_is_focused() -> TestResult {
    let scratch = Scratch::new("tabs")?;
    let base = scratch.file("base.rs", "base\n")?;
    let first = scratch.file("first.rs", "first\n")?;
    let second = scratch.file("second.rs", "second\n")?;
    let mut app = EditorApp::open_file(TestTextSystem, &base)?;

    let effect = app.handle_event(&opened(&[&first, &second], 0));

    assert!(effect.document_identity_advanced);
    assert_eq!(app.tabs.len(), 3);
    assert_eq!(app.tabs.path_at(1), Some(first.as_path()));
    assert_eq!(app.tabs.path_at(2), Some(second.as_path()));
    assert_eq!(app.tabs.active_index(), 2);
    assert_eq!(app.buffer().snapshot().text(), "second\n");
    assert!(app.local_status.is_none());

    // An already open file is activated, not opened twice.
    app.handle_event(&opened(&[&first], 0));
    assert_eq!(app.tabs.len(), 3);
    assert_eq!(app.tabs.active_index(), 1);
    assert_eq!(app.buffer().snapshot().text(), "first\n");
    Ok(())
}

#[test]
fn an_opened_folder_becomes_the_workspace_before_files_open() -> TestResult {
    let scratch = Scratch::new("folder")?;
    let base = scratch.file("base.rs", "base\n")?;
    let project = scratch.folder("project")?;
    let inside = scratch.file("project/main.rs", "fn main() {}\n")?;
    let mut app = EditorApp::open_file(TestTextSystem, &base)?;

    // The folder comes first even when listed last, or it would close main.rs.
    let effect = app.handle_event(&opened(&[&inside, &project], 0));

    assert!(effect.document_identity_advanced);
    assert_eq!(
        app.workspace.as_ref().map(Workspace::root),
        Some(project.as_path())
    );
    assert!(app.file_tree.is_active());
    assert_eq!(app.tabs.len(), 2);
    assert_eq!(app.tabs.path_at(0), None);
    assert_eq!(app.tabs.path_at(1), Some(inside.as_path()));
    assert_eq!(app.tabs.active_index(), 1);
    assert!(app.local_status.is_none());

    // A new workspace closes every tab, so unsaved text refuses it.
    app.handle_event(&SurfaceEvent::Ime {
        timestamp: EventTimestamp::new(2),
        input_epoch: InputEpoch::INITIAL,
        event: ImeEvent::Committed("x".into()),
    });
    let other = scratch.folder("other")?;
    app.handle_event(&opened(&[&other], 0));
    assert_eq!(
        app.workspace.as_ref().map(Workspace::root),
        Some(project.as_path())
    );
    assert_eq!(app.tabs.len(), 2);
    assert!(app.document.is_dirty());
    let other = other.to_string_lossy();
    assert!(status(&app).contains(&*other), "{}", status(&app));
    Ok(())
}

#[test]
fn a_missing_path_sets_a_status_and_keeps_the_open_document() -> TestResult {
    let scratch = Scratch::new("missing")?;
    let base = scratch.file("base.rs", "base\n")?;
    let good = scratch.file("good.rs", "good\n")?;
    let missing = scratch.root.join("missing.rs");
    let gone = scratch.root.join("gone.rs");
    let mut app = EditorApp::open_file(TestTextSystem, &base)?;

    let effect = app.handle_event(&opened(&[&missing], 0));

    assert!(effect.visual_changed);
    assert!(!effect.document_identity_advanced);
    assert_eq!(app.tabs.len(), 1);
    assert_eq!(app.tabs.path_at(0), Some(base.as_path()));
    assert_eq!(app.buffer().snapshot().text(), "base\n");
    assert_eq!(app.workspace_failures, 1);
    assert!(status(&app).contains("missing.rs"), "{}", status(&app));

    // A later success in the same batch must not hide the failure.
    app.handle_event(&opened(&[&gone, &good], 2));
    assert_eq!(app.tabs.len(), 2);
    assert_eq!(app.tabs.path_at(1), Some(good.as_path()));
    assert_eq!(app.workspace_failures, 2);
    assert!(status(&app).contains("gone.rs"), "{}", status(&app));
    assert!(status(&app).contains("2 more"), "{}", status(&app));
    Ok(())
}

#[test]
fn opened_paths_at_a_restored_launch_win_over_the_session() -> TestResult {
    let workspace = Scratch::new("session")?;
    let elsewhere = Scratch::new("elsewhere")?;
    let gamma = elsewhere.file("gamma.rs", "gamma\n")?;

    // run() marks a restored launch; paths then match an explicit launch.
    let mut launched = restored_session(&workspace)?;
    launched.restored_launch = RestoredLaunch::Pending;
    // A tab never loaded holds no unsaved text.
    assert_eq!(launched.tabs.is_deferred(1), Ok(true));
    let effect = launched.handle_event(&opened(&[&gamma], 0));
    let mut explicit =
        ExplicitPathTarget::open(&gamma, ExplicitPathKind::Any)?.into_app(TestTextSystem)?;
    assert!(effect.document_identity_advanced);
    assert!(launched.workspace.is_none());
    assert_eq!(launched.tabs.len(), 1);
    assert_eq!(launched.buffer().snapshot().text(), "gamma\n");
    assert_eq!(
        launched
            .capture_session()
            .map_err(|error| format!("{error:?}"))?,
        explicit
            .capture_session()
            .map_err(|error| format!("{error:?}"))?
    );

    // Only the first batch replaces; a second one adds tabs.
    let delta = elsewhere.file("delta.rs", "delta\n")?;
    launched.handle_event(&opened(&[&delta], 0));
    assert_eq!(launched.tabs.len(), 2);

    // Without the launch mark the session stays and the file is added.
    let mut warm = restored_session(&workspace)?;
    warm.handle_event(&opened(&[&gamma], 0));
    assert_eq!(
        warm.workspace.as_ref().map(Workspace::root),
        Some(workspace.root.as_path())
    );
    assert_eq!(warm.tabs.len(), 3);

    // Input ends the launch window.
    let mut touched = restored_session(&workspace)?;
    touched.restored_launch = RestoredLaunch::Pending;
    touched.handle_event(&SurfaceEvent::Scroll {
        timestamp: EventTimestamp::new(1),
        delta_x: 0.0,
        delta_y: -1.0,
        phase: alpine_platform_macos::ScrollPhase::Changed,
        precise: true,
        modifiers: alpine_platform_macos::Modifiers::default(),
    });
    touched.handle_event(&opened(&[&gamma], 0));
    assert_eq!(touched.tabs.len(), 3);

    // Unsaved text is never replaced: the file joins the restored tabs.
    let mut dirty = restored_session(&workspace)?;
    dirty.handle_event(&SurfaceEvent::Ime {
        timestamp: EventTimestamp::new(1),
        input_epoch: InputEpoch::INITIAL,
        event: ImeEvent::Committed("x".into()),
    });
    assert!(dirty.document.is_dirty());
    dirty.restored_launch = RestoredLaunch::Pending;
    dirty.handle_event(&opened(&[&gamma], 0));
    assert_eq!(dirty.tabs.len(), 3);
    assert_eq!(
        dirty.workspace.as_ref().map(Workspace::root),
        Some(workspace.root.as_path())
    );
    let alpha = dirty.tabs.document_at(0, &dirty.document)?;
    assert!(alpha.is_dirty());

    // A launch batch where nothing opens keeps the session.
    let mut failed = restored_session(&workspace)?;
    failed.restored_launch = RestoredLaunch::Pending;
    failed.handle_event(&opened(&[&elsewhere.root.join("missing.rs")], 0));
    assert_eq!(failed.tabs.len(), 2);
    assert!(failed.workspace.is_some());
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn the_bundle_opens_text_and_folders_without_becoming_the_default() -> TestResult {
    const SCRIPT: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../scripts/build-alpine-editor-app.sh"
    ));
    let start = SCRIPT
        .find("cat > \"$contents/Info.plist\" <<EOF\n")
        .ok_or("Info.plist heredoc")?;
    let body = &SCRIPT[start..];
    let body = &body[body.find('\n').ok_or("heredoc start")? + 1..];
    let body = &body[..body.find("\nEOF\n").ok_or("heredoc end")?];
    let plist = body.replace("$workspace_version", "0.0.0");
    assert!(!plist.contains('$'), "unexpanded shell text in Info.plist");

    let scratch = Scratch::new("plist")?;
    let path = scratch.file("Info.plist", &plist)?;
    let lint = std::process::Command::new("/usr/bin/plutil")
        .arg("-lint")
        .arg(&path)
        .output()?;
    assert!(
        lint.status.success(),
        "{}",
        String::from_utf8_lossy(&lint.stdout)
    );
    let extract = std::process::Command::new("/usr/bin/plutil")
        .args(["-extract", "CFBundleDocumentTypes", "json", "-o", "-"])
        .arg(&path)
        .output()?;
    assert!(extract.status.success(), "no CFBundleDocumentTypes");
    let types: serde_json::Value = serde_json::from_slice(&extract.stdout)?;
    let types = types.as_array().ok_or("document types array")?;
    let mut declared = Vec::new();
    let mut extensions = Vec::new();
    for entry in types {
        assert_eq!(entry["CFBundleTypeRole"], "Editor");
        assert_eq!(entry["LSHandlerRank"], "Alternate");
        let content = entry["LSItemContentTypes"].as_array();
        let listed = entry["CFBundleTypeExtensions"].as_array();
        // Launch Services ignores extensions in an entry that lists types.
        assert!(content.is_some() != listed.is_some(), "{entry}");
        declared.extend(
            content
                .into_iter()
                .flatten()
                .filter_map(serde_json::Value::as_str),
        );
        extensions.extend(
            listed
                .into_iter()
                .flatten()
                .filter_map(serde_json::Value::as_str),
        );
    }
    // Dynamic UTIs (.rs, .toml) and .ts as MPEG-TS need extensions to match.
    extensions.sort_unstable();
    let mut expected = [
        "rs", "toml", "lock", "py", "ts", "tsx", "js", "jsx", "mjs", "cjs", "json", "md", "c", "h",
        "cc", "cpp", "hpp", "java", "go", "sh", "yaml", "yml", "txt",
    ];
    expected.sort_unstable();
    assert_eq!(extensions, expected);
    declared.sort_unstable();
    assert_eq!(
        declared,
        [
            "public.folder",
            "public.plain-text",
            "public.source-code",
            "public.text"
        ]
    );
    Ok(())
}

#[test]
fn losing_focus_ends_the_launch_window_and_passive_events_do_not() -> TestResult {
    let workspace = Scratch::new("focus")?;
    let elsewhere = Scratch::new("focus-elsewhere")?;
    let gamma = elsewhere.file("gamma.rs", "gamma\n")?;
    let timestamp = EventTimestamp::new(1);
    let passive = [
        SurfaceEvent::Wake { timestamp },
        SurfaceEvent::Resize {
            timestamp,
            extent: alpine_platform_macos::SurfaceExtent::new(800.0, 600.0, 2.0)?,
        },
        SurfaceEvent::Pointer {
            timestamp,
            action: PointerAction::Moved,
            position: Point::new(10.0, 10.0).ok_or("point")?,
            button: alpine_platform_macos::PointerButton::None,
            modifiers: alpine_platform_macos::Modifiers::default(),
        },
        SurfaceEvent::Focus {
            timestamp,
            input_epoch: InputEpoch::INITIAL,
            focused: true,
        },
    ];
    for event in passive {
        let mut app = restored_session(&workspace)?;
        app.restored_launch = RestoredLaunch::Pending;
        app.handle_event(&event);
        assert_eq!(app.restored_launch, RestoredLaunch::Pending, "{event:?}");
    }

    // The user moved on; a later Dock drop adds tabs to the session.
    let mut app = restored_session(&workspace)?;
    app.restored_launch = RestoredLaunch::Pending;
    app.handle_event(&SurfaceEvent::Focus {
        timestamp,
        input_epoch: InputEpoch::INITIAL,
        focused: false,
    });
    assert_eq!(app.restored_launch, RestoredLaunch::Settled);
    app.handle_event(&opened(&[&gamma], 0));
    assert_eq!(app.tabs.len(), 3);
    assert!(app.workspace.is_some());
    Ok(())
}

#[test]
fn the_recovery_warning_survives_opened_paths() -> TestResult {
    let workspace = Scratch::new("recovered")?;
    let elsewhere = Scratch::new("recovered-elsewhere")?;
    let gamma = elsewhere.file("gamma.rs", "gamma\n")?;
    let state = recovery::RecoveryState {
        session: session_state(&workspace)?,
        documents: vec![recovery::RecoveredDocument {
            tab: 0,
            base: "alpha\n".into(),
            local: "alpha edited\n".into(),
        }],
    };
    let mut app = EditorApp::from_recovery(TestTextSystem, state).map_err(|e| e.to_string())?;
    let Some(LocalStatus::Recovery(warning)) = app.local_status.clone() else {
        return Err("restore set no recovery warning".into());
    };
    app.restored_launch = RestoredLaunch::Pending;

    // Unsaved text keeps the session; the opened file's success restores it.
    app.handle_event(&opened(&[&gamma], 0));
    assert_eq!(app.tabs.len(), 3);
    assert_eq!(
        app.local_status,
        Some(LocalStatus::Recovery(Arc::clone(&warning)))
    );

    // A failure joins the warning instead of replacing it.
    app.handle_event(&opened(&[&elsewhere.root.join("missing.rs")], 0));
    let Some(LocalStatus::Recovery(combined)) = app.local_status.clone() else {
        return Err(format!("warning replaced: {:?}", app.local_status).into());
    };
    assert!(combined.starts_with(&*warning), "{combined}");
    assert!(combined.contains("missing.rs"), "{combined}");
    Ok(())
}

#[test]
fn a_pending_recovery_counts_as_unsaved_text() -> TestResult {
    let workspace = Scratch::new("pending")?;
    let elsewhere = Scratch::new("pending-elsewhere")?;
    let gamma = elsewhere.file("gamma.rs", "gamma\n")?;
    let mut app = restored_session(&workspace)?;
    app.pending_recovery = vec![
        None,
        Some(recovery::RecoveredDocument {
            tab: 1,
            base: "beta\n".into(),
            local: "beta edited\n".into(),
        }),
    ];
    app.restored_launch = RestoredLaunch::Pending;
    app.handle_event(&opened(&[&gamma], 0));
    assert_eq!(app.tabs.len(), 3);
    assert!(app.workspace.is_some());
    Ok(())
}

#[test]
fn every_event_kind_has_a_validation_counter_slot() {
    let open = opened(&[], 0);
    assert_eq!(
        usize::try_from(surface_event_kind(&open)).ok(),
        Some(SURFACE_EVENT_KINDS)
    );
}
