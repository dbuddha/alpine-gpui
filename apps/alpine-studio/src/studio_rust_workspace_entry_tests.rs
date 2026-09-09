use std::{
    rc::Rc,
    time::{Duration, Instant},
};

use super::*;

#[derive(Clone, Copy, Debug)]
struct EntryObservation {
    diagnostics: rust_diagnostics::RustDiagnosticsSnapshot,
    deferred: [Option<bool>; 4],
}

impl EntryObservation {
    fn capture(app: &StudioApp) -> Self {
        Self {
            diagnostics: app.rust_diagnostics.snapshot(),
            deferred: std::array::from_fn(|index| app.tabs.is_deferred(index).ok()),
        }
    }
}

// The runtime retains exclusive ownership of the real application. The observer
// copies bounded values only; it cannot mutate state or service worker results.
struct ObservedStudio {
    app: StudioApp,
    observation: Rc<Cell<EntryObservation>>,
}

impl alpine_runtime::AppDelegate for ObservedStudio {
    type WorkerOutput = StudioWorkerOutput;

    fn event(&mut self, event: &SurfaceEvent, context: &mut AppContext<'_, Self::WorkerOutput>) {
        self.app.event(event, context);
        self.observation.set(EntryObservation::capture(&self.app));
    }

    fn worker_result(
        &mut self,
        token: alpine_runtime::WorkToken,
        result: Self::WorkerOutput,
        context: &mut AppContext<'_, Self::WorkerOutput>,
    ) {
        self.app.worker_result(token, result, context);
        self.observation.set(EntryObservation::capture(&self.app));
    }

    fn frame(&mut self, context: alpine_runtime::WindowContext) -> Scene {
        let scene = self.app.frame(context);
        self.observation.set(EntryObservation::capture(&self.app));
        scene
    }
}

fn restored_entry_app(
    fixture: &TestWorkspace,
    with_workspace: bool,
) -> Result<StudioApp, Box<dyn std::error::Error>> {
    fixture.write("main.rs", "fn broken( {\n")?;
    fixture.write("notes.txt", "plain text\n")?;
    fixture.create_dir("sub")?;
    fixture.write("sub/other.rs", "fn other() {}\n")?;
    fixture.write("lazy.rs", "fn lazy() {}\n")?;
    let root = fs::canonicalize(fixture.path())?;
    let state = session::SessionState {
        workspace: with_workspace.then_some(root.clone()),
        tabs: ["main.rs", "notes.txt", "sub/other.rs", "lazy.rs"]
            .into_iter()
            .map(|path| session::SessionTab {
                path: Some(root.join(path)),
                view: DocumentViewState::default(),
            })
            .collect(),
        active_tab: 0,
        panes: three_visible_panes(),
        file_tree: session::SessionFileTree::default(),
    };
    let mut app =
        StudioApp::from_session(TestTextSystem, state).map_err(|error| error.to_string())?;
    app.rust_diagnostics = RustDiagnostics::with_server(rust_diagnostics::tests::mock_executable());
    Ok(app)
}

fn three_visible_panes() -> session::SessionPanes {
    use session::{SessionAxis, SessionNode, SessionPane, SessionPanes};

    SessionPanes {
        nodes: [
            SessionNode::Split {
                axis: SessionAxis::Columns,
                first: 1,
                second: 2,
            },
            SessionNode::Leaf { pane: 0 },
            SessionNode::Split {
                axis: SessionAxis::Rows,
                first: 3,
                second: 4,
            },
            SessionNode::Leaf { pane: 1 },
            SessionNode::Leaf { pane: 2 },
            SessionNode::Empty,
            SessionNode::Empty,
        ],
        panes: [
            Some(SessionPane {
                tab: 0,
                view: DocumentViewState::default(),
            }),
            Some(SessionPane {
                tab: 1,
                view: DocumentViewState::default(),
            }),
            Some(SessionPane {
                tab: 2,
                view: DocumentViewState::default(),
            }),
            None,
        ],
        active_pane: 0,
    }
}

fn await_entry_diagnostics(
    runtime: &mut Application<ObservedStudio>,
    observation: &Cell<EntryObservation>,
) -> Result<EntryObservation, Box<dyn std::error::Error>> {
    let started = Instant::now();
    let mut timestamp = 4_000_u64;
    loop {
        let _frame = runtime.dispatch(&SurfaceEvent::Wake {
            timestamp: EventTimestamp::new(timestamp),
        });
        let current = observation.get();
        if current.diagnostics.active
            && current.diagnostics.diagnostic_items > 0
            && !current.diagnostics.overlay_write_pending
        {
            return Ok(current);
        }
        if started.elapsed() >= rust_diagnostics::tests::PRODUCT_DIAGNOSTIC_READINESS {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                format!("restored Studio diagnostics never became ready: {current:?}"),
            )
            .into());
        }
        timestamp = timestamp
            .checked_add(1)
            .ok_or("restored Studio fixture timestamp exhausted")?;
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn assert_restored_entry_roster(
    with_workspace: bool,
    expected_overlays: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    let fixture = TestWorkspace::new()?;
    let app = restored_entry_app(&fixture, with_workspace)?;
    let initial = EntryObservation::capture(&app);
    let expected_deferred = [Some(false), Some(false), Some(false), Some(true)];
    assert_eq!(initial.deferred, expected_deferred);
    assert_eq!(initial.diagnostics.process_starts, 0);
    let observation = Rc::new(Cell::new(initial));
    let delegate = ObservedStudio {
        app,
        observation: Rc::clone(&observation),
    };
    let clear = LinearRgba::new(0.02, 0.02, 0.02, 1.0).ok_or(SurfaceError::invariant(
        alpine_platform_macos::SurfaceOperation::Application,
    ))?;
    let mut runtime = Application::new(delegate, viewport()?, clear, WorkerConfig::default())?;
    let admitted = await_entry_diagnostics(&mut runtime, &observation)?;

    // Roster identity is an assertion after readiness, not a polling predicate
    // that turns an incorrect admission into an unexplained timeout.
    assert_eq!(admitted.diagnostics.overlay_documents, expected_overlays);
    assert_eq!(admitted.diagnostics.process_starts, 1);
    assert_eq!(admitted.diagnostics.restarts, 0);
    assert_eq!(admitted.deferred, expected_deferred);
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "Miri cannot emulate child-process creation")]
fn standalone_entry_excludes_loaded_non_rust_and_other_parent_documents()
-> Result<(), Box<dyn std::error::Error>> {
    assert_restored_entry_roster(false, 1)
}

#[test]
#[cfg_attr(miri, ignore = "Miri cannot emulate child-process creation")]
fn workspace_entry_includes_loaded_nested_rust_but_leaves_deferred_tabs_unloaded()
-> Result<(), Box<dyn std::error::Error>> {
    assert_restored_entry_roster(true, 2)
}
