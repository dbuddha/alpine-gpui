//! Real Studio process composition through production AppKit accessibility.

use std::{
    cell::{Cell, RefCell},
    fs,
    path::Path,
    rc::Rc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use alpine_platform_macos::{
    AccessibilityRole, CloseDisposition, EventTimestamp, NativeSurface, SurfaceDescriptor,
    SurfaceEvent, SurfaceLifecycle, SurfaceOperation, native_validation as platform_validation,
};
use alpine_runtime::{Application, WorkerConfig};

use super::{
    COMMAND_SHIFT_MODIFIERS, DEFAULT_SCALE, FONT_FAMILY, KEY_E, KEY_P, PresentationEvidenceMode,
    StudioApp, StudioError, WINDOW_HEIGHT, WINDOW_WIDTH, Workspace, event_handler, keyboard_event,
    parse_presentation_evidence_mode,
};

const MAX_WORKER_TURNS: u64 = 1_024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OmittedStep {
    Open,
    Edit,
    Action,
    Save,
    Close,
}

impl OmittedStep {
    fn from_environment() -> Result<Option<Self>, Box<dyn std::error::Error>> {
        let Ok(value) = std::env::var("ALPINE_STUDIO_NATIVE_ACCESSIBILITY_OMIT") else {
            return Ok(None);
        };
        match value.as_str() {
            "open" => Ok(Some(Self::Open)),
            "edit" => Ok(Some(Self::Edit)),
            "action" => Ok(Some(Self::Action)),
            "save" => Ok(Some(Self::Save)),
            "close" => Ok(Some(Self::Close)),
            _ => {
                Err(format!("unsupported native accessibility omission control: {value:?}").into())
            }
        }
    }
}

/// Handle-free evidence from the real Studio native accessibility process journey.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeStudioAccessibilityEvidence {
    tree_actions: usize,
    tab_actions: usize,
    command_actions: usize,
    diagnostic_actions: usize,
    query_frames: u64,
    maximum_action_frames: u64,
    persisted_bytes: usize,
    released_owner_classes: usize,
}

impl NativeStudioAccessibilityEvidence {
    /// Returns accepted file-tree accessibility actions.
    #[must_use]
    pub const fn tree_actions(self) -> usize {
        self.tree_actions
    }
    /// Returns accepted tab accessibility actions.
    #[must_use]
    pub const fn tab_actions(self) -> usize {
        self.tab_actions
    }
    /// Returns accepted command-palette accessibility actions.
    #[must_use]
    pub const fn command_actions(self) -> usize {
        self.command_actions
    }
    /// Returns accepted diagnostic accessibility actions.
    #[must_use]
    pub const fn diagnostic_actions(self) -> usize {
        self.diagnostic_actions
    }
    /// Returns frames submitted by stable accessibility queries.
    #[must_use]
    pub const fn query_frames(self) -> u64 {
        self.query_frames
    }
    /// Returns the greatest frame delta from one accepted visible action.
    #[must_use]
    pub const fn maximum_action_frames(self) -> u64 {
        self.maximum_action_frames
    }
    /// Returns exact saved UTF-8 bytes.
    #[must_use]
    pub const fn persisted_bytes(self) -> usize {
        self.persisted_bytes
    }
    /// Returns native owner classes released exactly once.
    #[must_use]
    pub const fn released_owner_classes(self) -> usize {
        self.released_owner_classes
    }
}

/// Runs one real workspace, Studio runtime, AppKit accessibility, LSP, save,
/// dirty-close, and teardown journey.
///
/// # Errors
///
/// Returns a structured construction, worker, native selector, save, language,
/// or teardown failure from the production-composed validation process.
pub fn qualify_studio_accessibility_process()
-> Result<NativeStudioAccessibilityEvidence, Box<dyn std::error::Error>> {
    if std::env::var_os("ALPINE_RUST_ANALYZER").is_none() {
        return Err("ALPINE_RUST_ANALYZER is required for native Studio qualification".into());
    }
    let omitted_step = OmittedStep::from_environment()?;
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let root = std::env::temp_dir().join(format!(
        "alpine-studio-native-accessibility-{}-{nonce}",
        std::process::id()
    ));
    let source = root.join("src");
    fs::create_dir_all(&source)?;
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"native_accessibility_fixture\"\nversion = \"0.0.0\"\nedition = \"2024\"\n",
    )?;
    let main_path = source.join("main.rs");
    let lib_path = source.join("lib.rs");
    fs::write(&main_path, "fn main() {\n    broken();\n}\n")?;
    fs::write(&lib_path, "pub fn library() {}\n")?;
    let result = qualify_workspace(&root, &main_path, &lib_path, omitted_step);
    let cleanup = fs::remove_dir_all(root);
    match (result, cleanup) {
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(Box::new(error)),
        (Ok(evidence), Ok(())) => Ok(evidence),
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "one process journey preserves workspace, semantic, document, frame, file, and owner identity"
)]
fn qualify_workspace(
    root: &Path,
    main_path: &Path,
    _lib_path: &Path,
    omitted_step: Option<OmittedStep>,
) -> Result<NativeStudioAccessibilityEvidence, Box<dyn std::error::Error>> {
    let workspace = Workspace::open_root(root)?;
    let mut text_system = alpine_text_layout::CoreTextSystem::new();
    text_system.register_font(FONT_FAMILY, "Menlo-Regular")?;
    let mut delegate = StudioApp::from_workspace(text_system, workspace)?;
    delegate.prime_workspace_launch()?;
    let clear = alpine_core::LinearRgba::new(0.02, 0.02, 0.02, 1.0).ok_or(StudioError::Runtime(
        alpine_runtime::RuntimeError::Surface(alpine_platform_macos::SurfaceError::validation(
            SurfaceOperation::Validation,
        )),
    ))?;
    let viewport = alpine_core::Size::new(WINDOW_WIDTH, WINDOW_HEIGHT).ok_or(
        StudioError::Runtime(alpine_runtime::RuntimeError::Surface(
            alpine_platform_macos::SurfaceError::validation(SurfaceOperation::Validation),
        )),
    )?;
    let descriptor = SurfaceDescriptor::new(
        "Alpine Studio native accessibility process",
        f64::from(WINDOW_WIDTH),
        f64::from(WINDOW_HEIGHT),
        f64::from(DEFAULT_SCALE),
    )?;
    let mut application = Application::new(delegate, viewport, clear, WorkerConfig::default())?;
    let surface = platform_validation::new_surface(&descriptor)
        .map_err(|error| format!("native accessibility surface construction failed: {error}"))?;
    let initial_frame = application
        .frame_if_dirty()
        .ok_or("Studio did not build its initial accessibility frame")?;
    let (scene, clear) = initial_frame.into_parts();
    let _revision = surface
        .request_frame(scene, clear)
        .map_err(|error| format!("initial native accessibility frame request failed: {error}"))?;
    surface
        .show()
        .map_err(|error| format!("native accessibility surface show failed: {error}"))?;
    let evidence_mode = presentation_evidence_mode()?;
    if evidence_mode.requires_surface_configuration(surface.snapshot().is_presentation_visible()) {
        platform_validation::inject_surface_configuration(
            &surface,
            f64::from(WINDOW_WIDTH),
            f64::from(WINDOW_HEIGHT),
            f64::from(DEFAULT_SCALE),
            0,
            true,
        )?;
    }
    await_frame_terminal(&surface, Duration::from_secs(5))
        .map_err(|error| format!("initial native accessibility frame failed: {error}"))?;

    let state = Rc::new(RefCell::new(application));
    let mut timestamp = 10_u64;
    let mut tree_actions = 0_usize;
    let mut tab_actions = 0_usize;
    let mut command_actions = 0_usize;
    let mut diagnostic_actions = 0_usize;
    dispatch(
        &surface,
        &state,
        &[keyboard_event(
            timestamp,
            KEY_E,
            "e",
            COMMAND_SHIFT_MODIFIERS,
        )],
    )
    .map_err(|error| format!("initial file-tree focus dispatch failed: {error}"))?;
    timestamp = timestamp.saturating_add(1);

    let src_label = wait_for_label_suffix(&surface, &state, &mut timestamp, "src")?;
    let mut maximum_action_frames =
        activate(&surface, &state, AccessibilityRole::ListItem, &src_label)?;
    tree_actions = tree_actions.saturating_add(1);
    let lib_label = wait_for_label_suffix(&surface, &state, &mut timestamp, "lib.rs")?;
    let main_label = wait_for_label_suffix(&surface, &state, &mut timestamp, "main.rs")?;
    maximum_action_frames = maximum_action_frames.max(activate(
        &surface,
        &state,
        AccessibilityRole::ListItem,
        &lib_label,
    )?);
    tree_actions = tree_actions.saturating_add(1);
    if omitted_step != Some(OmittedStep::Open) {
        maximum_action_frames = maximum_action_frames.max(activate(
            &surface,
            &state,
            AccessibilityRole::ListItem,
            &main_label,
        )?);
        tree_actions = tree_actions.saturating_add(1);
    }

    maximum_action_frames = maximum_action_frames.max(activate(
        &surface,
        &state,
        AccessibilityRole::Tab,
        "lib.rs",
    )?);
    tab_actions = tab_actions.saturating_add(1);
    maximum_action_frames = maximum_action_frames.max(activate(
        &surface,
        &state,
        AccessibilityRole::Tab,
        "main.rs",
    )?);
    tab_actions = tab_actions.saturating_add(1);
    dispatch(
        &surface,
        &state,
        &[keyboard_event(
            timestamp,
            KEY_E,
            "e",
            COMMAND_SHIFT_MODIFIERS,
        )],
    )
    .map_err(|error| format!("diagnostic admission dispatch failed: {error}"))?;
    timestamp = timestamp.saturating_add(1);

    let diagnostic_label = wait_for_label_prefix(
        &surface,
        &state,
        &mut timestamp,
        "diagnostic severity 1 on line 1",
    )?;
    let stable_before = surface.snapshot().submission_count();
    let tree =
        platform_validation::inspect_native_accessibility_tree(&surface, event_handler(&state))
            .map_err(|error| format!("stable native tree query failed: {error}"))?;
    let stable_after = surface.snapshot().submission_count();
    let query_frames = stable_after.saturating_sub(stable_before);
    assert_eq!(query_frames, 0);
    assert_eq!(tree.focused_nodes(), 1);
    assert!(tree.nodes().len() <= alpine_platform_macos::MAX_ACCESSIBILITY_NODES);
    assert!(tree.nodes().iter().all(|node| {
        node.current()
            && node.bounded_screen_frame()
            && !node.identifier().is_empty()
            && node.semantic_id() != 0
    }));
    assert!(tree.nodes().iter().any(|node| node.role() == "AXTextArea"));
    assert!(tree.nodes().iter().any(|node| node.label() == "main.rs"));
    assert!(
        tree.nodes()
            .iter()
            .any(|node| node.label() == diagnostic_label.as_ref())
    );

    if omitted_step != Some(OmittedStep::Action) {
        maximum_action_frames = maximum_action_frames.max(activate(
            &surface,
            &state,
            AccessibilityRole::ListItem,
            &diagnostic_label,
        )?);
        diagnostic_actions = diagnostic_actions.saturating_add(1);
    }
    if omitted_step != Some(OmittedStep::Edit) {
        platform_validation::commit_native_text(&surface, "// alpine\n", event_handler(&state))
            .map_err(|error| format!("first native editor text commit failed: {error}"))?;
        await_frame_terminal(&surface, Duration::from_millis(100))
            .map_err(|error| format!("first native editor text frame failed: {error}"))?;
    }
    timestamp = timestamp.saturating_add(1);

    if omitted_step != Some(OmittedStep::Save) {
        maximum_action_frames = maximum_action_frames.max(open_palette_and_activate(
            &surface,
            &state,
            &mut timestamp,
            "File: Save",
        )?);
        command_actions = command_actions.saturating_add(1);
    }
    let persisted = fs::read(main_path)?;
    if !persisted.starts_with(b"// alpine\n") {
        return Err("required native edit and save did not preserve the expected prefix".into());
    }

    platform_validation::commit_native_text(&surface, "dirty", event_handler(&state))
        .map_err(|error| format!("dirty native editor text commit failed: {error}"))?;
    await_frame_terminal(&surface, Duration::from_millis(100))
        .map_err(|error| format!("dirty native editor text frame failed: {error}"))?;
    timestamp = timestamp.saturating_add(1);
    let observer = surface.observer();
    let (closed, disposition, close_frame) = replay_close(&surface, &state)
        .map_err(|error| format!("dirty-close native replay failed: {error}"))?;
    assert!(!closed);
    assert_eq!(disposition, CloseDisposition::Cancel);
    assert!(close_frame);
    await_frame_terminal(&surface, Duration::from_millis(100))
        .map_err(|error| format!("dirty-close native frame failed: {error}"))?;
    assert_eq!(observer.lifecycle(), SurfaceLifecycle::Live);
    let blocked =
        platform_validation::inspect_native_accessibility_tree(&surface, event_handler(&state))
            .map_err(|error| format!("dirty-close native tree query failed: {error}"))?;
    assert!(blocked.nodes().iter().any(|node| {
        node.role() == "AXStaticText" && node.label() == "Save changes before closing."
    }));
    maximum_action_frames = maximum_action_frames.max(open_palette_and_activate(
        &surface,
        &state,
        &mut timestamp,
        "File: Save",
    )?);
    command_actions = command_actions.saturating_add(1);
    let persisted = fs::read(main_path)?;
    assert!(
        persisted
            .windows("dirty".len())
            .any(|bytes| bytes == b"dirty")
    );
    if omitted_step == Some(OmittedStep::Close) {
        return Err("required final native close was omitted".into());
    }
    let (closed, disposition, close_frame) = replay_close(&surface, &state)
        .map_err(|error| format!("final native close replay failed: {error}"))?;
    if !closed || disposition != CloseDisposition::Allow || close_frame {
        let lifecycle = observer.lifecycle();
        let status =
            if lifecycle == SurfaceLifecycle::Live && disposition == CloseDisposition::Cancel {
                inspect(&surface, &state)
                    .map(|tree| {
                        tree.nodes()
                            .iter()
                            .filter(|node| node.role() == "AXStaticText")
                            .map(|node| Box::<str>::from(node.label()))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_else(|error| vec![format!("status query failed: {error}").into()])
            } else {
                Vec::new()
            };
        return Err(format!(
            "final native close rejected: closed={closed} disposition={disposition:?} frame={close_frame} lifecycle={lifecycle:?} status={status:?}"
        )
        .into());
    }
    assert_eq!(observer.lifecycle(), SurfaceLifecycle::Closing);
    assert!(state.borrow().snapshot().is_shutting_down());
    assert!(maximum_action_frames <= 1);
    let observed_actions = (
        tree_actions,
        tab_actions,
        command_actions,
        diagnostic_actions,
    );
    if observed_actions != (3, 2, 2, 1) {
        return Err(format!(
            "native accessibility action count mismatch: observed={observed_actions:?} expected=(3, 2, 2, 1)"
        )
        .into());
    }

    drop(state);
    let owners = platform_validation::close_with_owner_evidence(surface)
        .map_err(|error| format!("native owner drain failed after accepted close: {error}"))?;
    assert_eq!(owners.active(), [0; 10]);
    assert_eq!(owners.release_order_violations(), 0);
    let expected = [1, 1, 1, 1, 1, 1, 1, 1, 1, 0];
    if owners.acquired() != expected || owners.released() != expected {
        return Err(format!(
            "native accessibility owner release mismatch: acquired={:?} released={:?} active={:?}",
            owners.acquired(),
            owners.released(),
            owners.active()
        )
        .into());
    }
    Ok(NativeStudioAccessibilityEvidence {
        tree_actions,
        tab_actions,
        command_actions,
        diagnostic_actions,
        query_frames,
        maximum_action_frames,
        persisted_bytes: persisted.len(),
        released_owner_classes: owners
            .released()
            .iter()
            .filter(|released| **released == 1)
            .count(),
    })
}

fn dispatch(
    surface: &NativeSurface,
    state: &Rc<RefCell<Application<StudioApp>>>,
    events: &[SurfaceEvent],
) -> Result<(), alpine_platform_macos::SurfaceError> {
    let frame_requested = Rc::new(Cell::new(false));
    let observed_frame = Rc::clone(&frame_requested);
    let mut handler = event_handler(state);
    platform_validation::replay_callback_surface_events(surface, events, move |event| {
        let response = handler(event);
        observed_frame.set(observed_frame.get() || response.frame().is_some());
        response
    })?;
    if frame_requested.get() {
        return await_frame_terminal(surface, Duration::from_millis(100));
    }
    if let Some(error) = surface.take_error()? {
        return Err(error);
    }
    let snapshot = surface.snapshot();
    if snapshot.occupied_frame_slots() != 0 || snapshot.submitted_frame_slots() != 0 {
        return Err(alpine_platform_macos::SurfaceError::validation(
            SurfaceOperation::Validation,
        ));
    }
    Ok(())
}

fn await_frame_terminal(
    surface: &NativeSurface,
    timeout: Duration,
) -> Result<(), alpine_platform_macos::SurfaceError> {
    if matches!(
        presentation_evidence_mode()?,
        PresentationEvidenceMode::HostedDirect
    ) {
        platform_validation::inject_post_commit_observation(surface, None, 1.0)?;
    }
    platform_validation::run_until_frame_terminal(surface, timeout);
    if let Some(error) = surface.take_error()? {
        return Err(error);
    }
    let snapshot = surface.snapshot();
    if snapshot.occupied_frame_slots() != 0 || snapshot.submitted_frame_slots() != 0 {
        return Err(alpine_platform_macos::SurfaceError::validation(
            SurfaceOperation::Validation,
        ));
    }
    Ok(())
}

fn presentation_evidence_mode()
-> Result<PresentationEvidenceMode, alpine_platform_macos::SurfaceError> {
    let value = std::env::var_os("ALPINE_PRESENTATION_EVIDENCE_MODE");
    parse_presentation_evidence_mode(value.as_deref()).ok_or_else(|| {
        alpine_platform_macos::SurfaceError::validation(SurfaceOperation::Validation)
    })
}

fn replay_close(
    surface: &NativeSurface,
    state: &Rc<RefCell<Application<StudioApp>>>,
) -> Result<(bool, CloseDisposition, bool), alpine_platform_macos::SurfaceError> {
    let observed = Rc::new(RefCell::new((CloseDisposition::NotRequested, false)));
    let callback_observed = Rc::clone(&observed);
    let callback_state = Rc::clone(state);
    let closed = platform_validation::replay_close_with_handler(surface, move |event| {
        let close_requested = matches!(event, SurfaceEvent::CloseRequested { .. });
        let response = callback_state.try_borrow_mut().map_or_else(
            |_| alpine_platform_macos::SurfaceResponse::default(),
            |mut application| application.dispatch_with_response(&event),
        );
        if close_requested {
            *callback_observed.borrow_mut() =
                (response.close_disposition(), response.frame().is_some());
        }
        response
    })?;
    let (disposition, frame) = *observed.borrow();
    Ok((closed, disposition, frame))
}

fn inspect(
    surface: &NativeSurface,
    state: &Rc<RefCell<Application<StudioApp>>>,
) -> Result<
    alpine_platform_macos::native_validation::NativeAccessibilityTreeEvidence,
    alpine_platform_macos::SurfaceError,
> {
    platform_validation::inspect_native_accessibility_tree(surface, event_handler(state))
}

fn wait_for_label_suffix(
    surface: &NativeSurface,
    state: &Rc<RefCell<Application<StudioApp>>>,
    timestamp: &mut u64,
    suffix: &str,
) -> Result<Box<str>, Box<dyn std::error::Error>> {
    wait_for_label(surface, state, timestamp, |label| label.ends_with(suffix))
        .map_err(|error| format!("waiting for native label suffix {suffix:?}: {error}").into())
}

fn wait_for_label_prefix(
    surface: &NativeSurface,
    state: &Rc<RefCell<Application<StudioApp>>>,
    timestamp: &mut u64,
    prefix: &str,
) -> Result<Box<str>, Box<dyn std::error::Error>> {
    wait_for_label(surface, state, timestamp, |label| label.starts_with(prefix))
        .map_err(|error| format!("waiting for native label prefix {prefix:?}: {error}").into())
}

fn wait_for_label(
    surface: &NativeSurface,
    state: &Rc<RefCell<Application<StudioApp>>>,
    timestamp: &mut u64,
    predicate: impl Fn(&str) -> bool,
) -> Result<Box<str>, Box<dyn std::error::Error>> {
    for _ in 0..MAX_WORKER_TURNS {
        dispatch(
            surface,
            state,
            &[SurfaceEvent::Wake {
                timestamp: EventTimestamp::new(*timestamp),
            }],
        )
        .map_err(|error| format!("worker wake at timestamp {} failed: {error}", *timestamp))?;
        *timestamp = timestamp.saturating_add(1);
        let tree = inspect(surface, state)
            .map_err(|error| format!("native tree refresh after worker wake failed: {error}"))?;
        if let Some(node) = tree.nodes().iter().find(|node| predicate(node.label())) {
            return Ok(Box::from(node.label()));
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    Err("bounded Studio accessibility label did not become visible".into())
}

fn activate(
    surface: &NativeSurface,
    state: &Rc<RefCell<Application<StudioApp>>>,
    role: AccessibilityRole,
    label: &str,
) -> Result<u64, Box<dyn std::error::Error>> {
    let before = surface.snapshot().submission_count();
    let response_summary = Rc::new(RefCell::new(String::from("callback not reached")));
    let observed_summary = Rc::clone(&response_summary);
    let callback_state = Rc::clone(state);
    let evidence = platform_validation::activate_named_native_accessibility_node(
        surface,
        role,
        label,
        move |event| {
            let response = callback_state.try_borrow_mut().map_or_else(
                |_| alpine_platform_macos::SurfaceResponse::default(),
                |mut application| application.dispatch_with_response(&event),
            );
            *observed_summary.borrow_mut() = format!(
                "frame={} clipboard={} close={:?} accessibility_kind={:?}",
                response.frame().is_some(),
                response.clipboard_write().is_some(),
                response.close_disposition(),
                response.accessibility_response().map(|value| value.kind())
            );
            response
        },
    )
    .map_err(|error| {
        format!(
            "native accessibility dispatch failed for role={role:?} label={label:?}: {error}; response={}",
            response_summary.borrow()
        )
    })?;
    await_frame_terminal(surface, Duration::from_millis(100)).map_err(|error| {
        format!(
            "native accessibility action frame failed for role={role:?} label={label:?}: {error}"
        )
    })?;
    let after = surface.snapshot().submission_count();
    if !evidence.selector_allowed() || !evidence.accepted() || evidence.dispatch_failed() {
        return Err(format!(
            "native accessibility action failed: role={role:?} label={label:?} native_role={:?} native_label={:?} identifier={:?} selector_allowed={} accepted={} current_after={} dispatch_failed={}",
            evidence.role(),
            evidence.label(),
            evidence.identifier(),
            evidence.selector_allowed(),
            evidence.accepted(),
            evidence.current_after_action(),
            evidence.dispatch_failed()
        )
        .into());
    }
    assert_eq!(evidence.label(), label);
    assert!(!evidence.identifier().is_empty());
    assert_ne!(evidence.semantic_id(), 0);
    let frames = after.saturating_sub(before);
    assert!(frames <= 1);
    Ok(frames)
}

fn open_palette_and_activate(
    surface: &NativeSurface,
    state: &Rc<RefCell<Application<StudioApp>>>,
    timestamp: &mut u64,
    label: &str,
) -> Result<u64, Box<dyn std::error::Error>> {
    dispatch(
        surface,
        state,
        &[keyboard_event(
            *timestamp,
            KEY_P,
            "p",
            COMMAND_SHIFT_MODIFIERS,
        )],
    )
    .map_err(|error| format!("command-palette open dispatch failed: {error}"))?;
    *timestamp = timestamp.saturating_add(1);
    let tree = inspect(surface, state)?;
    assert!(tree.nodes().iter().any(|node| {
        node.role() == "AXGroup" && node.label() == "Command palette" && node.focused()
    }));
    activate(surface, state, AccessibilityRole::ListItem, label)
}
