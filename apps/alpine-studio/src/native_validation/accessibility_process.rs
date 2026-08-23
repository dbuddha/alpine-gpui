//! Real Studio process composition through production AppKit accessibility.

use std::{
    cell::{Cell, RefCell},
    ffi::OsStr,
    fs,
    io::{Read as _, Write as _},
    path::Path,
    rc::Rc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
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
const MAX_TERMINAL_DRAINS: u8 = 8;
const FRAME_TERMINAL_TIMEOUT: Duration = Duration::from_secs(5);
const FRAME_TERMINAL_POLL: Duration = Duration::from_millis(100);
const DIAGNOSTIC_READY_TIMEOUT: Duration = Duration::from_secs(10);
const DIAGNOSTIC_READY_POLL: Duration = Duration::from_millis(5);
const MAX_LANGUAGE_TRACE_BYTES: u64 = 4_096;
const REQUIRED_LANGUAGE_PHASES: [&str; 8] = [
    "qualification-child",
    "wrapper-invoked",
    "process-spawned",
    "initialize-received",
    "initialize-responded",
    "initialized-received",
    "did-open-received",
    "diagnostics-written",
];
const MISMATCH_CONTROL_MARKER: u64 = 0xA11C_E551;
const DISPATCH_FAILURE_CONTROL_MARKER: u64 = 0xD15F_A11E;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OmittedStep {
    Open,
    Edit,
    Action,
    Save,
    Close,
}

impl OmittedStep {
    fn from_value(value: &str) -> Result<Self, Box<dyn std::error::Error>> {
        match value {
            "open" => Ok(Self::Open),
            "edit" => Ok(Self::Edit),
            "action" => Ok(Self::Action),
            "save" => Ok(Self::Save),
            "close" => Ok(Self::Close),
            _ => {
                Err(format!("unsupported native accessibility omission control: {value:?}").into())
            }
        }
    }

    fn from_environment() -> Result<Option<Self>, Box<dyn std::error::Error>> {
        let Ok(value) = std::env::var("ALPINE_STUDIO_NATIVE_ACCESSIBILITY_OMIT") else {
            return Ok(None);
        };
        Self::from_value(&value).map(Some)
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
    mismatch_control_marker: u64,
    dispatch_failure_control_marker: u64,
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
    /// Returns the completed exact-role mismatch control marker.
    #[must_use]
    pub const fn mismatch_control_marker(self) -> u64 {
        self.mismatch_control_marker
    }
    /// Returns the completed failed-refresh dispatch control marker.
    #[must_use]
    pub const fn dispatch_failure_control_marker(self) -> u64 {
        self.dispatch_failure_control_marker
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
    let server = std::env::var_os("ALPINE_RUST_ANALYZER")
        .ok_or("ALPINE_RUST_ANALYZER is required for native Studio qualification")?;
    record_qualification_child_phase(&server)?;
    crate::reset_native_validation_language_evidence();
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

fn record_qualification_child_phase(server: &OsStr) -> Result<(), Box<dyn std::error::Error>> {
    let wrapper = fs::read_to_string(server)?;
    if !qualification_wrapper_valid(&wrapper) {
        return Err("native Studio qualification received an unexpected language wrapper".into());
    }
    let expected_process = std::env::var_os("ALPINE_STUDIO_NATIVE_PROCESS_EXE")
        .ok_or("ALPINE_STUDIO_NATIVE_PROCESS_EXE is required for native Studio qualification")?;
    if fs::canonicalize(expected_process)? != fs::canonicalize(std::env::current_exe()?)? {
        return Err("native Studio qualification process identity mismatch".into());
    }
    let trace = std::env::var_os("ALPINE_STUDIO_NATIVE_LSP_TRACE")
        .ok_or("ALPINE_STUDIO_NATIVE_LSP_TRACE is required for native Studio qualification")?;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(trace)?;
    writeln!(file, "qualification-child")?;
    Ok(())
}

fn qualification_wrapper_valid(wrapper: &str) -> bool {
    wrapper.contains("wrapper-invoked") && wrapper.contains("ALPINE_STUDIO_NATIVE_LSP_SERVER")
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
    let state = Rc::new(RefCell::new(application));
    await_frame_terminal(&surface, &state, FRAME_TERMINAL_TIMEOUT)
        .map_err(|error| format!("initial native accessibility frame failed: {error}"))?;

    let mut timestamp = 10_u64;
    let mut tree_actions = 0_usize;
    let mut tab_actions = 0_usize;
    let mut command_actions = 0_usize;
    let mut diagnostic_actions = 0_usize;
    let mut dispatch_failure_control_marker = 0_u64;
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
    let (lib_label, main_label) =
        wait_for_label_suffix_pair(&surface, &state, &mut timestamp, "lib.rs", "main.rs")?;
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
    let language = crate::native_validation_language_evidence();
    let server_trace = language_server_phase_trace();
    let completed_server_phases = completed_language_server_phases(&server_trace);
    if !diagnostic_qualification_ready(
        diagnostic_authority_ready(language),
        completed_server_phases,
    ) {
        return Err(format!(
            "native diagnostic label lacked exact production language authority: language={language:?} server=(completed_phases={completed_server_phases}/{} trace={server_trace:?})",
            REQUIRED_LANGUAGE_PHASES.len()
        )
        .into());
    }
    let stable_before = surface.snapshot().submission_count();
    let tree =
        platform_validation::inspect_native_accessibility_tree(&surface, event_handler(&state))
            .map_err(|error| format!("stable native tree query failed: {error}"))?;
    let stable_after = surface.snapshot().submission_count();
    let query_frames = stable_after.saturating_sub(stable_before);
    let accessor_focused_nodes = tree.nodes().iter().filter(|node| node.focused()).count();
    let selected_nodes = tree.nodes().iter().filter(|node| node.selected()).count();
    let activate_allowed_nodes = tree
        .nodes()
        .iter()
        .filter(|node| node.activate_allowed())
        .count();
    assert_eq!(query_frames, 0);
    assert_eq!(accessor_focused_nodes, tree.focused_nodes());
    assert_eq!(tree.focused_nodes(), 1);
    assert!(selected_nodes > 0);
    assert!(selected_nodes < tree.nodes().len());
    assert!(activate_allowed_nodes > 0);
    assert!(activate_allowed_nodes < tree.nodes().len());
    assert!(tree.nodes().len() <= alpine_platform_macos::MAX_ACCESSIBILITY_NODES);
    assert!(tree.nodes().iter().all(|node| {
        node.current()
            && node.bounded_screen_frame()
            && !node.identifier().is_empty()
            && node.semantic_id() != 0
            && node
                .identifier()
                .rsplit_once('.')
                .and_then(|(_, semantic_id)| semantic_id.parse::<u64>().ok())
                == Some(node.semantic_id())
    }));
    assert!(tree.nodes().iter().any(|node| node.role() == "AXTextArea"));
    assert!(tree.nodes().iter().any(|node| node.label() == "main.rs"));
    assert!(
        tree.nodes()
            .iter()
            .any(|node| node.label() == diagnostic_label.as_ref())
    );
    let mismatch_control_marker =
        reject_mismatched_activation(&surface, &state, &diagnostic_label)?;

    if omitted_step != Some(OmittedStep::Action) {
        maximum_action_frames = maximum_action_frames.max(activate(
            &surface,
            &state,
            AccessibilityRole::ListItem,
            &diagnostic_label,
        )?);
        diagnostic_actions = diagnostic_actions.saturating_add(1);
        dispatch_failure_control_marker =
            require_dispatch_failure(&surface, &state, &diagnostic_label)?;
    }
    if omitted_step != Some(OmittedStep::Edit) {
        platform_validation::commit_native_text(&surface, "// alpine\n", event_handler(&state))
            .map_err(|error| format!("first native editor text commit failed: {error}"))?;
        await_frame_terminal(&surface, &state, FRAME_TERMINAL_TIMEOUT)
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
    await_frame_terminal(&surface, &state, FRAME_TERMINAL_TIMEOUT)
        .map_err(|error| format!("dirty native editor text frame failed: {error}"))?;
    timestamp = timestamp.saturating_add(1);
    let observer = surface.observer();
    let (closed, disposition, close_frame) = replay_close(&surface, &state)
        .map_err(|error| format!("dirty-close native replay failed: {error}"))?;
    assert!(!closed);
    assert_eq!(disposition, CloseDisposition::Cancel);
    assert!(close_frame);
    await_frame_terminal(&surface, &state, FRAME_TERMINAL_TIMEOUT)
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
    if !final_close_succeeded(closed, disposition, close_frame) {
        let lifecycle = observer.lifecycle();
        let status = if should_inspect_rejected_close(lifecycle, disposition) {
            inspect(&surface, &state)
                .map(|tree| {
                    tree.nodes()
                        .iter()
                        .filter(|node| is_close_status_role(node.role()))
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
    if !negative_control_markers_match(mismatch_control_marker, dispatch_failure_control_marker) {
        return Err(format!(
            "native accessibility negative-control mismatch: role_marker={mismatch_control_marker:#x} dispatch_marker={dispatch_failure_control_marker:#x}"
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
        mismatch_control_marker,
        dispatch_failure_control_marker,
    })
}

fn dispatch(
    surface: &NativeSurface,
    state: &Rc<RefCell<Application<StudioApp>>>,
    events: &[SurfaceEvent],
) -> Result<bool, Box<dyn std::error::Error>> {
    let frame_requested = Rc::new(Cell::new(false));
    let observed_frame = Rc::clone(&frame_requested);
    let mut handler = event_handler(state);
    platform_validation::replay_callback_surface_events(surface, events, move |event| {
        let response = handler(event);
        observed_frame.set(observed_frame.get() || response.frame().is_some());
        response
    })?;
    let frame_requested = frame_requested.get();
    if frame_requested {
        await_frame_terminal(surface, state, FRAME_TERMINAL_TIMEOUT)?;
    } else {
        require_frame_quiescence(surface)?;
    }
    Ok(frame_requested)
}

fn require_frame_quiescence(
    surface: &NativeSurface,
) -> Result<(), alpine_platform_macos::SurfaceError> {
    if let Some(error) = surface.take_error()? {
        return Err(error);
    }
    let snapshot = surface.snapshot();
    if snapshot.occupied_frame_slots() != 0
        || snapshot.submitted_frame_slots() != 0
        || !snapshot.display_link_paused()
    {
        return Err(alpine_platform_macos::SurfaceError::validation(
            SurfaceOperation::Validation,
        ));
    }
    Ok(())
}

fn await_frame_terminal(
    surface: &NativeSurface,
    state: &Rc<RefCell<Application<StudioApp>>>,
    timeout: Duration,
) -> Result<(), Box<dyn std::error::Error>> {
    let initial = surface.snapshot();
    let failure = |phase: &str,
                   observed_submissions: u64,
                   error: &dyn std::fmt::Display|
     -> Box<dyn std::error::Error> {
        let current = surface.snapshot();
        format!(
                "frame-terminal {phase} failed after {observed_submissions} observed submissions: {error}; initial=(occupied={} submitted={} paused={} submissions={} presented={} qualified={} failed={} cancelled={} superseded={} terminal={:?}) current=(occupied={} submitted={} paused={} submissions={} presented={} qualified={} failed={} cancelled={} superseded={} terminal={:?})",
                initial.occupied_frame_slots(),
                initial.submitted_frame_slots(),
                initial.display_link_paused(),
                initial.submission_count(),
                initial.presented_count(),
                initial.qualified_presented_count(),
                initial.failed_count(),
                initial.cancelled_count(),
                initial.superseded_count(),
                initial.last_terminal(),
                current.occupied_frame_slots(),
                current.submitted_frame_slots(),
                current.display_link_paused(),
                current.submission_count(),
                current.presented_count(),
                current.qualified_presented_count(),
                current.failed_count(),
                current.cancelled_count(),
                current.superseded_count(),
                current.last_terminal(),
            )
            .into()
    };
    let evidence_mode =
        presentation_evidence_mode().map_err(|error| failure("evidence-mode", 0, &error))?;
    let started = std::time::Instant::now();
    let mut armed_at_submission = None;
    loop {
        let surfaced = surface
            .take_error()
            .map_err(|error| failure("surface-error-read", 0, &error))?;
        if let Some(error) = surfaced {
            return Err(failure("latched-surface-error", 0, &error));
        }
        let snapshot = surface.snapshot();
        let observed_submissions = u64::from(initial.submitted_frame_slots()).saturating_add(
            snapshot
                .submission_count()
                .saturating_sub(initial.submission_count()),
        );
        if frame_drain_bound_exceeded(observed_submissions) {
            return Err(failure(
                "frame-drain-bound",
                observed_submissions,
                &"submitted frame count exceeded the bounded drain contract",
            ));
        }
        if frame_terminal_ready(
            observed_submissions,
            snapshot.occupied_frame_slots(),
            snapshot.submitted_frame_slots(),
            snapshot.display_link_paused(),
        ) {
            return Ok(());
        }
        let elapsed = started.elapsed();
        if elapsed >= timeout {
            return Err(failure(
                "correctness-timeout",
                observed_submissions,
                &"frame ownership did not become terminal before the correctness deadline",
            ));
        }
        if should_arm_hosted_observation(
            evidence_mode,
            snapshot.submitted_frame_slots(),
            armed_at_submission,
            snapshot.submission_count(),
        ) {
            platform_validation::inject_post_commit_observation(surface, None, 1.0)
                .map_err(|error| failure("hosted-observation", observed_submissions, &error))?;
            armed_at_submission = Some(snapshot.submission_count());
        }
        let poll = FRAME_TERMINAL_POLL.min(timeout.saturating_sub(elapsed));
        platform_validation::run_until_frame_terminal_with_handler(
            surface,
            poll,
            event_handler(state),
        )
        .map_err(|error| failure("event-loop", observed_submissions, &error))?;
    }
}

const fn frame_drain_bound_exceeded(observed_submissions: u64) -> bool {
    observed_submissions > MAX_TERMINAL_DRAINS as u64
}

const fn frame_terminal_ready(
    observed_submissions: u64,
    occupied_slots: u8,
    submitted_slots: u8,
    display_link_paused: bool,
) -> bool {
    observed_submissions > 0 && occupied_slots == 0 && submitted_slots == 0 && display_link_paused
}

fn should_arm_hosted_observation(
    evidence_mode: PresentationEvidenceMode,
    submitted_slots: u8,
    armed_at_submission: Option<u64>,
    submission_count: u64,
) -> bool {
    matches!(evidence_mode, PresentationEvidenceMode::HostedDirect)
        && submitted_slots == 0
        && armed_at_submission != Some(submission_count)
}

const fn action_frame_bound_exceeded(frames: u64) -> bool {
    frames > 1
}

const fn accessibility_action_succeeded(
    selector_allowed: bool,
    accepted: bool,
    dispatch_failed: bool,
) -> bool {
    selector_allowed && accepted && !dispatch_failed
}

const fn final_close_succeeded(
    closed: bool,
    disposition: CloseDisposition,
    returned_frame: bool,
) -> bool {
    closed && matches!(disposition, CloseDisposition::Allow) && !returned_frame
}

const fn should_inspect_rejected_close(
    lifecycle: SurfaceLifecycle,
    disposition: CloseDisposition,
) -> bool {
    matches!(lifecycle, SurfaceLifecycle::Live) && matches!(disposition, CloseDisposition::Cancel)
}

const fn negative_control_markers_match(role_marker: u64, dispatch_marker: u64) -> bool {
    role_marker == MISMATCH_CONTROL_MARKER && dispatch_marker == DISPATCH_FAILURE_CONTROL_MARKER
}

fn is_close_status_role(role: &str) -> bool {
    role == "AXStaticText"
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

fn wait_for_label_suffix_pair(
    surface: &NativeSurface,
    state: &Rc<RefCell<Application<StudioApp>>>,
    timestamp: &mut u64,
    first_suffix: &str,
    second_suffix: &str,
) -> Result<(Box<str>, Box<str>), Box<dyn std::error::Error>> {
    for _ in 0..MAX_WORKER_TURNS {
        dispatch(
            surface,
            state,
            &[SurfaceEvent::Wake {
                timestamp: EventTimestamp::new(*timestamp),
            }],
        )
        .map_err(|error| format!("paired-label worker wake failed: {error}"))?;
        *timestamp = timestamp.saturating_add(1);
        let tree = inspect(surface, state).map_err(|error| {
            format!("native tree refresh after paired-label wake failed: {error}")
        })?;
        let first = tree
            .nodes()
            .iter()
            .find(|node| node.label().ends_with(first_suffix));
        let second = tree
            .nodes()
            .iter()
            .find(|node| node.label().ends_with(second_suffix));
        if let (Some(first), Some(second)) = (first, second) {
            return Ok((Box::from(first.label()), Box::from(second.label())));
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    Err(format!(
        "bounded Studio accessibility labels with suffixes {first_suffix:?} and {second_suffix:?} did not become visible together"
    )
    .into())
}

fn wait_for_label_prefix(
    surface: &NativeSurface,
    state: &Rc<RefCell<Application<StudioApp>>>,
    timestamp: &mut u64,
    prefix: &str,
) -> Result<Box<str>, Box<dyn std::error::Error>> {
    let started = Instant::now();
    let mut first_inspection = true;
    let mut wake_turns = 0_u64;
    let mut frame_wakes = 0_u64;
    let mut tree_inspections = 0_u64;
    loop {
        let frame_requested = dispatch(
            surface,
            state,
            &[SurfaceEvent::Wake {
                timestamp: EventTimestamp::new(*timestamp),
            }],
        )
        .map_err(|error| {
            format!(
                "waiting for native label prefix {prefix:?}: worker wake at timestamp {} failed: {error}",
                *timestamp
            )
        })?;
        wake_turns = wake_turns.saturating_add(1);
        frame_wakes = frame_wakes.saturating_add(u64::from(frame_requested));
        *timestamp = timestamp.saturating_add(1);
        if diagnostic_tree_inspection_required(first_inspection, frame_requested) {
            first_inspection = false;
            tree_inspections = tree_inspections.saturating_add(1);
            let tree = inspect(surface, state).map_err(|error| {
                format!(
                    "waiting for native label prefix {prefix:?}: native tree refresh after worker wake failed: {error}"
                )
            })?;
            if let Some(node) = tree
                .nodes()
                .iter()
                .find(|node| node.label().starts_with(prefix))
            {
                return Ok(Box::from(node.label()));
            }
        }
        if diagnostic_wait_expired(started.elapsed()) {
            let runtime = state.borrow().snapshot();
            let worker = runtime.worker();
            let external = runtime.external();
            let surface = surface.snapshot();
            let language = crate::native_validation_language_evidence();
            let server_trace = language_server_phase_trace();
            let completed_server_phases = completed_language_server_phases(&server_trace);
            let next_server_phase = REQUIRED_LANGUAGE_PHASES.get(completed_server_phases);
            return Err(format!(
                "waiting for native label prefix {prefix:?}: label did not become visible before the {DIAGNOSTIC_READY_TIMEOUT:?} correctness deadline; polling=(wake_turns={wake_turns} frame_wakes={frame_wakes} tree_inspections={tree_inspections}) surface=(occupied={} submitted={} paused={} submissions={} terminal={:?}) worker=(queued_requests={} queued_results={} dropped_results={} panicked_jobs={}) external=(current_items={} admitted={} drained={} full={} disconnected={} shutting_down={} sequence_exhausted={}) language={language:?} server=(completed_phases={completed_server_phases}/{} next_phase={next_server_phase:?} trace={server_trace:?})",
                surface.occupied_frame_slots(),
                surface.submitted_frame_slots(),
                surface.display_link_paused(),
                surface.submission_count(),
                surface.last_terminal(),
                worker.queued_requests(),
                worker.queued_results(),
                worker.dropped_results(),
                worker.panicked_jobs(),
                external.current_items(),
                external.admitted(),
                external.drained(),
                external.full(),
                external.disconnected(),
                external.shutting_down(),
                external.sequence_exhausted(),
                REQUIRED_LANGUAGE_PHASES.len(),
            )
            .into());
        }
        std::thread::sleep(DIAGNOSTIC_READY_POLL);
    }
}

fn language_server_phase_trace() -> Box<str> {
    let Some(path) = std::env::var_os("ALPINE_STUDIO_NATIVE_LSP_TRACE") else {
        return Box::from("<language trace path unavailable>");
    };
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) => return format!("<language trace unavailable: {error}>").into(),
    };
    let mut bytes = Vec::with_capacity(MAX_LANGUAGE_TRACE_BYTES as usize);
    let read = file
        .take(MAX_LANGUAGE_TRACE_BYTES.saturating_add(1))
        .read_to_end(&mut bytes);
    if let Err(error) = read {
        return format!("<language trace read failed: {error}>").into();
    }
    if !language_trace_size_within_bound(bytes.len()) {
        return Box::from("<language trace exceeded bounded evidence size>");
    }
    String::from_utf8(bytes).map_or_else(
        |error| format!("<language trace was not UTF-8: {error}>").into(),
        Box::from,
    )
}

const fn language_trace_size_within_bound(bytes: usize) -> bool {
    bytes <= MAX_LANGUAGE_TRACE_BYTES as usize
}

fn completed_language_server_phases(trace: &str) -> usize {
    REQUIRED_LANGUAGE_PHASES
        .iter()
        .zip(trace.lines())
        .take_while(|(expected, observed)| **expected == *observed)
        .count()
}

const fn diagnostic_qualification_ready(
    authority_ready: bool,
    completed_server_phases: usize,
) -> bool {
    authority_ready && completed_server_phases == REQUIRED_LANGUAGE_PHASES.len()
}

const fn diagnostic_authority_ready(evidence: crate::NativeValidationLanguageEvidence) -> bool {
    evidence.active
        && evidence.sync_calls > 0
        && evidence.wake_callbacks > 0
        && evidence.latch_publications > 0
        && evidence.external_admissions > 0
        && evidence.external_rejections == 0
        && evidence.foreground_results > 0
        && evidence.generation > 0
        && evidence.process_epoch > 0
        && evidence.lsp_version > 0
        && evidence.submitted_inputs >= 3
        && evidence.written_inputs >= 3
        && evidence.input_saturations == 0
        && evidence.polls > 0
        && evidence.diagnostic_publications > 0
        && evidence.diagnostic_items > 0
        && evidence.stale_wakes == 0
        && evidence.restarts == 0
        && evidence.invalidations > 0
        && evidence.frame_builds > 1
}

fn diagnostic_wait_expired(elapsed: Duration) -> bool {
    elapsed >= DIAGNOSTIC_READY_TIMEOUT
}

const fn diagnostic_tree_inspection_required(
    first_inspection: bool,
    frame_requested: bool,
) -> bool {
    first_inspection || frame_requested
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
    let response_summary = Rc::new(RefCell::new(String::from("callback not reached")));
    let observed_summary = Rc::clone(&response_summary);
    let response_frames = Rc::new(Cell::new(0_u64));
    let observed_frames = Rc::clone(&response_frames);
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
            if response.frame().is_some() {
                observed_frames.set(observed_frames.get().saturating_add(1));
            }
            response
        },
    )
    .map_err(|error| {
        format!(
            "native accessibility dispatch failed for role={role:?} label={label:?}: {error}; response={}",
            response_summary.borrow()
        )
    })?;
    if !accessibility_action_succeeded(
        evidence.selector_allowed(),
        evidence.accepted(),
        evidence.dispatch_failed(),
    ) {
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
    let frames = response_frames.get();
    if action_frame_bound_exceeded(frames) {
        return Err(format!(
            "native accessibility action returned {frames} frames for role={role:?} label={label:?}"
        )
        .into());
    }
    if frames == 0 {
        require_frame_quiescence(surface).map_err(|error| {
            format!(
                "native accessibility no-frame action failed for role={role:?} label={label:?}: {error}"
            )
        })?;
    } else {
        await_frame_terminal(surface, state, FRAME_TERMINAL_TIMEOUT).map_err(|error| {
            format!(
                "native accessibility action frame failed for role={role:?} label={label:?}: {error}"
            )
        })?;
    }
    assert_eq!(evidence.label(), label);
    assert!(!evidence.role().is_empty());
    assert!(!evidence.identifier().is_empty());
    assert_ne!(evidence.semantic_id(), 0);
    assert_eq!(
        evidence
            .identifier()
            .rsplit_once('.')
            .and_then(|(_, semantic_id)| semantic_id.parse::<u64>().ok()),
        Some(evidence.semantic_id())
    );
    Ok(frames)
}

fn reject_mismatched_activation(
    surface: &NativeSurface,
    state: &Rc<RefCell<Application<StudioApp>>>,
    label: &str,
) -> Result<u64, Box<dyn std::error::Error>> {
    let result = platform_validation::activate_named_native_accessibility_node(
        surface,
        AccessibilityRole::CodeEditor,
        label,
        event_handler(state),
    );
    assert!(
        result.is_err(),
        "a current label paired with the wrong semantic role must fail exact named activation"
    );
    require_frame_quiescence(surface)
        .map_err(|error| format!("mismatched native activation emitted work: {error}"))?;
    Ok(MISMATCH_CONTROL_MARKER)
}

fn require_dispatch_failure(
    surface: &NativeSurface,
    state: &Rc<RefCell<Application<StudioApp>>>,
    label: &str,
) -> Result<u64, Box<dyn std::error::Error>> {
    let action_observed = Rc::new(Cell::new(false));
    let callback_action_observed = Rc::clone(&action_observed);
    let response_frames = Rc::new(Cell::new(0_u64));
    let callback_response_frames = Rc::clone(&response_frames);
    let callback_state = Rc::clone(state);
    let evidence = platform_validation::activate_named_native_accessibility_node(
        surface,
        AccessibilityRole::ListItem,
        label,
        move |event| {
            let reject_post_action_refresh = callback_action_observed.get()
                && matches!(
                    event,
                    SurfaceEvent::Accessibility { ref request, .. }
                        if request.kind()
                            != alpine_platform_macos::AccessibilityRequestKind::Action
                );
            if reject_post_action_refresh {
                return alpine_platform_macos::SurfaceResponse::default();
            }
            let action = matches!(
                event,
                SurfaceEvent::Accessibility { ref request, .. }
                    if request.kind() == alpine_platform_macos::AccessibilityRequestKind::Action
            );
            let response = callback_state.try_borrow_mut().map_or_else(
                |_| alpine_platform_macos::SurfaceResponse::default(),
                |mut application| application.dispatch_with_response(&event),
            );
            if response.frame().is_some() {
                callback_response_frames.set(callback_response_frames.get().saturating_add(1));
            }
            if action {
                callback_action_observed.set(true);
            }
            response
        },
    )?;
    assert!(action_observed.get());
    assert!(evidence.selector_allowed());
    assert!(!evidence.accepted());
    assert!(evidence.dispatch_failed());
    assert!(evidence.current_after_action());
    assert_eq!(evidence.label(), label);
    assert!(!evidence.role().is_empty());
    assert!(!evidence.identifier().is_empty());
    assert_ne!(evidence.semantic_id(), 0);
    assert_eq!(
        evidence
            .identifier()
            .rsplit_once('.')
            .and_then(|(_, semantic_id)| semantic_id.parse::<u64>().ok()),
        Some(evidence.semantic_id())
    );
    let frames = response_frames.get();
    if action_frame_bound_exceeded(frames) {
        return Err(format!(
            "dispatch-failure control returned {frames} frames for label={label:?}"
        )
        .into());
    }
    if frames == 0 {
        require_frame_quiescence(surface)?;
    } else {
        await_frame_terminal(surface, state, FRAME_TERMINAL_TIMEOUT)?;
    }
    let recovered = inspect(surface, state)?;
    assert!(recovered.nodes().iter().any(|node| {
        node.label() == label && node.current() && node.semantic_id() == evidence.semantic_id()
    }));
    Ok(DISPATCH_FAILURE_CONTROL_MARKER)
}

#[cfg(test)]
mod process_contract_tests {
    use super::*;

    #[test]
    fn omission_values_preserve_every_required_step_and_reject_unknown_values()
    -> Result<(), Box<dyn std::error::Error>> {
        assert_eq!(OmittedStep::from_value("open")?, OmittedStep::Open);
        assert_eq!(OmittedStep::from_value("edit")?, OmittedStep::Edit);
        assert_eq!(OmittedStep::from_value("action")?, OmittedStep::Action);
        assert_eq!(OmittedStep::from_value("save")?, OmittedStep::Save);
        assert_eq!(OmittedStep::from_value("close")?, OmittedStep::Close);
        assert!(OmittedStep::from_value("unknown").is_err());
        Ok(())
    }

    #[test]
    fn process_evidence_accessors_preserve_nondefault_values() {
        let evidence = NativeStudioAccessibilityEvidence {
            tree_actions: 2,
            tab_actions: 3,
            command_actions: 4,
            diagnostic_actions: 5,
            query_frames: 6,
            maximum_action_frames: 7,
            persisted_bytes: 8,
            released_owner_classes: 9,
            mismatch_control_marker: 10,
            dispatch_failure_control_marker: 11,
        };
        assert_eq!(evidence.tree_actions(), 2);
        assert_eq!(evidence.tab_actions(), 3);
        assert_eq!(evidence.command_actions(), 4);
        assert_eq!(evidence.diagnostic_actions(), 5);
        assert_eq!(evidence.query_frames(), 6);
        assert_eq!(evidence.maximum_action_frames(), 7);
        assert_eq!(evidence.persisted_bytes(), 8);
        assert_eq!(evidence.released_owner_classes(), 9);
        assert_eq!(evidence.mismatch_control_marker(), 10);
        assert_eq!(evidence.dispatch_failure_control_marker(), 11);
    }

    #[test]
    fn diagnostic_wait_expires_at_the_exact_correctness_deadline() {
        assert!(!diagnostic_wait_expired(
            DIAGNOSTIC_READY_TIMEOUT.saturating_sub(Duration::from_millis(1))
        ));
        assert!(diagnostic_wait_expired(DIAGNOSTIC_READY_TIMEOUT));
        assert!(diagnostic_wait_expired(
            DIAGNOSTIC_READY_TIMEOUT.saturating_add(Duration::from_millis(1))
        ));

        assert!(diagnostic_tree_inspection_required(true, false));
        assert!(diagnostic_tree_inspection_required(true, true));
        assert!(diagnostic_tree_inspection_required(false, true));
        assert!(!diagnostic_tree_inspection_required(false, false));
    }

    #[test]
    fn language_server_phase_evidence_requires_the_exact_ordered_prefix() {
        let mut trace = String::new();
        for (index, phase) in REQUIRED_LANGUAGE_PHASES.iter().enumerate() {
            assert_eq!(completed_language_server_phases(&trace), index);
            trace.push_str(phase);
            trace.push('\n');
        }
        assert_eq!(
            completed_language_server_phases(&trace),
            REQUIRED_LANGUAGE_PHASES.len()
        );
        assert_eq!(
            completed_language_server_phases(
                "qualification-child\nwrapper-invoked\nprocess-spawned\ninitialize-responded\ninitialize-received\n"
            ),
            3
        );
        assert_eq!(completed_language_server_phases("unknown\n"), 0);
    }

    #[test]
    fn process_language_ownership_requires_every_independent_boundary() {
        assert!(qualification_wrapper_valid(
            "wrapper-invoked ALPINE_STUDIO_NATIVE_LSP_SERVER"
        ));
        assert!(!qualification_wrapper_valid("wrapper-invoked"));
        assert!(!qualification_wrapper_valid(
            "ALPINE_STUDIO_NATIVE_LSP_SERVER"
        ));
        assert!(!qualification_wrapper_valid("unrelated wrapper"));

        assert!(diagnostic_qualification_ready(
            true,
            REQUIRED_LANGUAGE_PHASES.len()
        ));
        assert!(!diagnostic_qualification_ready(
            false,
            REQUIRED_LANGUAGE_PHASES.len()
        ));
        assert!(!diagnostic_qualification_ready(
            true,
            REQUIRED_LANGUAGE_PHASES.len().saturating_sub(1)
        ));

        assert!(language_trace_size_within_bound(0));
        assert!(language_trace_size_within_bound(
            MAX_LANGUAGE_TRACE_BYTES as usize
        ));
        assert!(!language_trace_size_within_bound(
            MAX_LANGUAGE_TRACE_BYTES as usize + 1
        ));
    }

    #[test]
    fn diagnostic_authority_requires_every_independent_production_phase() {
        let valid = crate::NativeValidationLanguageEvidence {
            sync_calls: 1,
            wake_callbacks: 1,
            latch_publications: 1,
            external_admissions: 1,
            external_rejections: 0,
            latch_polls: 0,
            foreground_results: 1,
            invalidations: 1,
            active: true,
            generation: 1,
            process_epoch: 1,
            lsp_version: 1,
            process_queued_events: 0,
            submitted_inputs: 3,
            written_inputs: 3,
            input_saturations: 0,
            polls: 1,
            diagnostic_publications: 1,
            diagnostic_items: 1,
            stale_wakes: 0,
            restarts: 0,
            frame_builds: 2,
        };
        assert!(diagnostic_authority_ready(valid));
        for invalid in [
            crate::NativeValidationLanguageEvidence {
                active: false,
                ..valid
            },
            crate::NativeValidationLanguageEvidence {
                sync_calls: 0,
                ..valid
            },
            crate::NativeValidationLanguageEvidence {
                wake_callbacks: 0,
                ..valid
            },
            crate::NativeValidationLanguageEvidence {
                latch_publications: 0,
                ..valid
            },
            crate::NativeValidationLanguageEvidence {
                external_admissions: 0,
                ..valid
            },
            crate::NativeValidationLanguageEvidence {
                external_rejections: 1,
                ..valid
            },
            crate::NativeValidationLanguageEvidence {
                foreground_results: 0,
                ..valid
            },
            crate::NativeValidationLanguageEvidence {
                generation: 0,
                ..valid
            },
            crate::NativeValidationLanguageEvidence {
                process_epoch: 0,
                ..valid
            },
            crate::NativeValidationLanguageEvidence {
                lsp_version: 0,
                ..valid
            },
            crate::NativeValidationLanguageEvidence {
                submitted_inputs: 2,
                ..valid
            },
            crate::NativeValidationLanguageEvidence {
                written_inputs: 2,
                ..valid
            },
            crate::NativeValidationLanguageEvidence {
                input_saturations: 1,
                ..valid
            },
            crate::NativeValidationLanguageEvidence { polls: 0, ..valid },
            crate::NativeValidationLanguageEvidence {
                diagnostic_publications: 0,
                ..valid
            },
            crate::NativeValidationLanguageEvidence {
                diagnostic_items: 0,
                ..valid
            },
            crate::NativeValidationLanguageEvidence {
                stale_wakes: 1,
                ..valid
            },
            crate::NativeValidationLanguageEvidence {
                restarts: 1,
                ..valid
            },
            crate::NativeValidationLanguageEvidence {
                invalidations: 0,
                ..valid
            },
            crate::NativeValidationLanguageEvidence {
                frame_builds: 1,
                ..valid
            },
        ] {
            assert!(!diagnostic_authority_ready(invalid));
        }
    }

    #[test]
    fn frame_drain_predicates_require_every_independent_condition() {
        assert!(!frame_drain_bound_exceeded(8));
        assert!(frame_drain_bound_exceeded(9));

        assert!(frame_terminal_ready(1, 0, 0, true));
        assert!(!frame_terminal_ready(0, 0, 0, true));
        assert!(!frame_terminal_ready(1, 1, 0, true));
        assert!(!frame_terminal_ready(1, 0, 1, true));
        assert!(!frame_terminal_ready(1, 0, 0, false));

        assert!(!action_frame_bound_exceeded(0));
        assert!(!action_frame_bound_exceeded(1));
        assert!(action_frame_bound_exceeded(2));

        assert!(accessibility_action_succeeded(true, true, false));
        assert!(!accessibility_action_succeeded(false, true, false));
        assert!(!accessibility_action_succeeded(true, false, false));
        assert!(!accessibility_action_succeeded(true, true, true));

        assert!(final_close_succeeded(true, CloseDisposition::Allow, false));
        assert!(!final_close_succeeded(
            false,
            CloseDisposition::Allow,
            false
        ));
        assert!(!final_close_succeeded(
            true,
            CloseDisposition::Cancel,
            false
        ));
        assert!(!final_close_succeeded(true, CloseDisposition::Allow, true));

        assert!(should_inspect_rejected_close(
            SurfaceLifecycle::Live,
            CloseDisposition::Cancel
        ));
        assert!(!should_inspect_rejected_close(
            SurfaceLifecycle::Closing,
            CloseDisposition::Cancel
        ));
        assert!(!should_inspect_rejected_close(
            SurfaceLifecycle::Live,
            CloseDisposition::Allow
        ));

        assert!(negative_control_markers_match(
            MISMATCH_CONTROL_MARKER,
            DISPATCH_FAILURE_CONTROL_MARKER
        ));
        assert!(!negative_control_markers_match(
            MISMATCH_CONTROL_MARKER + 1,
            DISPATCH_FAILURE_CONTROL_MARKER
        ));
        assert!(!negative_control_markers_match(
            MISMATCH_CONTROL_MARKER,
            DISPATCH_FAILURE_CONTROL_MARKER + 1
        ));

        assert!(is_close_status_role("AXStaticText"));
        assert!(!is_close_status_role("AXButton"));
    }

    #[test]
    fn hosted_observation_requires_mode_empty_slots_and_a_new_submission() {
        assert!(should_arm_hosted_observation(
            PresentationEvidenceMode::HostedDirect,
            0,
            None,
            4
        ));
        assert!(!should_arm_hosted_observation(
            PresentationEvidenceMode::Physical,
            0,
            None,
            4
        ));
        assert!(!should_arm_hosted_observation(
            PresentationEvidenceMode::HostedDirect,
            1,
            None,
            4
        ));
        assert!(!should_arm_hosted_observation(
            PresentationEvidenceMode::HostedDirect,
            0,
            Some(4),
            4
        ));
    }
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
