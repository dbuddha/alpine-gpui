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
    AccessibilityRole, CloseDisposition, EventTimestamp, InputEpoch, NativeSurface,
    SurfaceDescriptor, SurfaceEvent, SurfaceLifecycle, SurfaceOperation, SurfaceSnapshot,
    native_validation as platform_validation,
};
use alpine_runtime::{Application, WorkerConfig};

use super::{
    COMMAND_SHIFT_MODIFIERS, DEFAULT_SCALE, FONT_FAMILY, KEY_E, KEY_P, PresentationEvidenceMode,
    StudioApp, StudioError, WINDOW_HEIGHT, WINDOW_WIDTH, Workspace, event_handler, keyboard_event,
    parse_presentation_evidence_mode,
};

const MAX_WORKER_TURNS: u64 = 1_024;
const MAX_TERMINAL_DRAINS: u8 = 8;
// Hosted Metal runners can take several seconds to make an already submitted
// command terminal. This is a correctness watchdog, not a frame-performance
// budget; the submission count and ownership drain remain independently bound.
const FRAME_TERMINAL_TIMEOUT: Duration = Duration::from_secs(10);
const FRAME_TERMINAL_POLL: Duration = Duration::from_millis(100);
const DIAGNOSTIC_READY_TIMEOUT: Duration = Duration::from_secs(10);
const DIAGNOSTIC_READY_POLL: Duration = Duration::from_millis(5);
const MAX_LANGUAGE_TRACE_BYTES: u64 = 4_096;
const MAX_OMISSION_ERROR_BYTES: usize = 512;
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

    const fn expected_failure(self) -> &'static str {
        match self {
            Self::Open => {
                "native accessibility omission confirmed: step=open stage=workspace-file-open"
            }
            Self::Edit => {
                "native accessibility omission confirmed: step=edit stage=document-unchanged-before-save"
            }
            Self::Action => {
                "native accessibility omission confirmed: step=action stage=diagnostic-action"
            }
            Self::Save => "native accessibility omission confirmed: step=save stage=persisted-save",
            Self::Close => "native accessibility omission confirmed: step=close stage=final-close",
        }
    }
}

fn bounded_omission_error(value: &str) -> &str {
    let mut end = value.len().min(MAX_OMISSION_ERROR_BYTES);
    while !value.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    &value[..end]
}

fn omission_failure_with_cause(
    step: OmittedStep,
    cause: &dyn std::fmt::Display,
) -> Box<dyn std::error::Error> {
    let cause = cause.to_string();
    format!(
        "{}; cause={}",
        step.expected_failure(),
        bounded_omission_error(&cause)
    )
    .into()
}

/// Requires one native omission child to fail for its exact requested step.
///
/// The child-process harness uses this before publishing a success marker, so
/// unrelated startup, renderer, language, filesystem, or lifecycle failures
/// cannot masquerade as an accepted omission control.
///
/// # Errors
///
/// Returns bounded evidence when the requested step is unknown or the observed
/// root error is not the canonical failure for that step.
#[doc(hidden)]
pub fn validate_native_accessibility_omission_failure(
    requested: &str,
    observed: &str,
) -> Result<(), Box<str>> {
    let step =
        OmittedStep::from_value(requested).map_err(|error| error.to_string().into_boxed_str())?;
    let expected = step.expected_failure();
    let matched = if step == OmittedStep::Open {
        observed
            .strip_prefix(expected)
            .and_then(|suffix| suffix.strip_prefix("; cause="))
            .is_some_and(|cause| {
                !cause.is_empty()
                    && cause.len() <= MAX_OMISSION_ERROR_BYTES
                    && cause == bounded_omission_error(cause)
            })
    } else {
        observed == expected
    };
    if matched {
        Ok(())
    } else {
        Err(format!(
            "native accessibility omission {requested:?} expected {expected:?}, observed {:?}",
            bounded_omission_error(observed)
        )
        .into())
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
    let initial_frame_baseline = surface.snapshot();
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
    await_frame_terminal(
        &surface,
        &state,
        initial_frame_baseline,
        FRAME_TERMINAL_TIMEOUT,
    )
    .map_err(|error| format!("initial native accessibility frame failed: {error}"))?;

    let mut timestamp = 10_u64;
    dispatch(
        &surface,
        &state,
        &[SurfaceEvent::Focus {
            timestamp: EventTimestamp::new(timestamp),
            input_epoch: InputEpoch::INITIAL,
            focused: true,
        }],
    )
    .map_err(|error| format!("initial native accessibility focus dispatch failed: {error}"))?;
    timestamp = timestamp.saturating_add(1);
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
    let main_tab_frames = match activate(&surface, &state, AccessibilityRole::Tab, "main.rs") {
        Ok(frames) => frames,
        Err(error) if omitted_step == Some(OmittedStep::Open) => {
            return Err(omission_failure_with_cause(OmittedStep::Open, &error));
        }
        Err(error) => return Err(error),
    };
    maximum_action_frames = maximum_action_frames.max(main_tab_frames);
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

    if omitted_step == Some(OmittedStep::Action) {
        return Err(OmittedStep::Action.expected_failure().into());
    }
    maximum_action_frames = maximum_action_frames.max(activate(
        &surface,
        &state,
        AccessibilityRole::ListItem,
        &diagnostic_label,
    )?);
    diagnostic_actions = diagnostic_actions.saturating_add(1);
    let dispatch_failure_control_marker =
        require_dispatch_failure(&surface, &state, &diagnostic_label)?;
    let first_edit_baseline = surface.snapshot();
    if omitted_step == Some(OmittedStep::Edit) {
        require_frame_quiescence(&surface)
            .map_err(|error| format!("omitted native edit emitted frame work: {error}"))?;
        let after_omission = surface.snapshot();
        if after_omission.submission_count() != first_edit_baseline.submission_count() {
            return Err(format!(
                "omitted native edit changed submission count: before={} after={}",
                first_edit_baseline.submission_count(),
                after_omission.submission_count()
            )
            .into());
        }
        let persisted = fs::read(main_path)?;
        if persisted.starts_with(b"// alpine\n") {
            return Err("omitted native edit changed persisted document bytes".into());
        }
        return Err(OmittedStep::Edit.expected_failure().into());
    }
    platform_validation::commit_native_text(&surface, "// alpine\n", event_handler(&state))
        .map_err(|error| format!("first native editor text commit failed: {error}"))?;
    await_frame_terminal(
        &surface,
        &state,
        first_edit_baseline,
        FRAME_TERMINAL_TIMEOUT,
    )
    .map_err(|error| format!("first native editor text frame failed: {error}"))?;
    timestamp = timestamp.saturating_add(1);

    if omitted_step != Some(OmittedStep::Save) {
        let lost_epoch = relinquish_native_focus(&surface, &state, &mut timestamp)?;
        if platform_validation::input_focus_state(&surface) != (lost_epoch, false) {
            return Err("native focus-loss control was not retained for restoration".into());
        }
        let save_frames =
            open_palette_and_activate(&surface, &state, &mut timestamp, "File: Save")?;
        maximum_action_frames = maximum_action_frames.max(save_frames);
        command_actions = command_actions.saturating_add(1);
    }
    let persisted = fs::read(main_path)?;
    if !persisted.starts_with(b"// alpine\n") {
        if omitted_step == Some(OmittedStep::Save) {
            return Err(OmittedStep::Save.expected_failure().into());
        }
        return Err("required native edit and save did not preserve the expected prefix".into());
    }
    if omitted_step == Some(OmittedStep::Close) {
        require_frame_quiescence(&surface)
            .map_err(|error| format!("omitted final close emitted frame work: {error}"))?;
        if surface.observer().lifecycle() != SurfaceLifecycle::Live
            || state.borrow().snapshot().is_shutting_down()
        {
            return Err("omitted final close changed the live lifecycle state".into());
        }
        return Err(OmittedStep::Close.expected_failure().into());
    }

    let dirty_edit_baseline = surface.snapshot();
    platform_validation::commit_native_text(&surface, "dirty", event_handler(&state))
        .map_err(|error| format!("dirty native editor text commit failed: {error}"))?;
    await_frame_terminal(
        &surface,
        &state,
        dirty_edit_baseline,
        FRAME_TERMINAL_TIMEOUT,
    )
    .map_err(|error| format!("dirty native editor text frame failed: {error}"))?;
    timestamp = timestamp.saturating_add(1);
    let observer = surface.observer();
    let dirty_close_baseline = surface.snapshot();
    let (closed, disposition, close_frame) = replay_close(&surface, &state)
        .map_err(|error| format!("dirty-close native replay failed: {error}"))?;
    assert!(!closed);
    assert_eq!(disposition, CloseDisposition::Cancel);
    assert!(close_frame);
    await_frame_terminal(
        &surface,
        &state,
        dirty_close_baseline,
        FRAME_TERMINAL_TIMEOUT,
    )
    .map_err(|error| format!("dirty-close native frame failed: {error}"))?;
    assert_eq!(observer.lifecycle(), SurfaceLifecycle::Live);
    let blocked =
        platform_validation::inspect_native_accessibility_tree(&surface, event_handler(&state))
            .map_err(|error| format!("dirty-close native tree query failed: {error}"))?;
    assert!(blocked.nodes().iter().any(|node| {
        node.role() == "AXStaticText"
            && node.label() == "Save changes in main.rs with Command-S before closing."
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
    if !owner_release_matches(owners.acquired(), owners.released(), expected) {
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
    let initial = surface.snapshot();
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
        await_frame_terminal(surface, state, initial, FRAME_TERMINAL_TIMEOUT)?;
    } else {
        require_frame_quiescence(surface)?;
    }
    Ok(frame_requested)
}

#[cfg_attr(test, mutants::skip)] // Native snapshot I/O is composed in process E2E; the complete policy truth table is tested below.
fn require_frame_quiescence(
    surface: &NativeSurface,
) -> Result<(), alpine_platform_macos::SurfaceError> {
    if let Some(error) = surface.take_error()? {
        return Err(error);
    }
    let snapshot = surface.snapshot();
    if !frame_quiescent(
        snapshot.occupied_frame_slots(),
        snapshot.submitted_frame_slots(),
        snapshot.display_link_paused(),
    ) {
        return Err(alpine_platform_macos::SurfaceError::validation(
            SurfaceOperation::Validation,
        ));
    }
    Ok(())
}

fn await_frame_terminal(
    surface: &NativeSurface,
    state: &Rc<RefCell<Application<StudioApp>>>,
    initial: SurfaceSnapshot,
    timeout: Duration,
) -> Result<(), Box<dyn std::error::Error>> {
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

const fn frame_quiescent(
    occupied_slots: u8,
    submitted_slots: u8,
    display_link_paused: bool,
) -> bool {
    occupied_slots == 0 && submitted_slots == 0 && display_link_paused
}

fn should_arm_hosted_observation(
    evidence_mode: PresentationEvidenceMode,
    submitted_slots: u8,
    armed_at_submission: Option<u64>,
    submission_count: u64,
) -> bool {
    matches!(evidence_mode, PresentationEvidenceMode::HostedDirect)
        && submitted_slots > 0
        && armed_at_submission != Some(submission_count)
}

/// Returns whether one failed hosted child is the exact retryable command stall.
///
/// The retry belongs to the outer process harness. It never changes frame
/// ownership, the correctness deadline, physical evidence, or terminal state.
#[must_use]
pub fn hosted_terminal_stall_retry_allowed(
    evidence_mode: &str,
    attempt: u8,
    status_succeeded: bool,
    stdout: &str,
    stderr: &str,
    language_trace_complete: bool,
) -> bool {
    evidence_mode == "hosted-direct"
        && attempt == 0
        && !status_succeeded
        && stdout.is_empty()
        && language_trace_complete
        && stderr.contains("dirty-close native frame failed")
        && stderr.contains("frame-terminal correctness-timeout failed after 1 observed submissions")
        && stderr
            .contains("frame ownership did not become terminal before the correctness deadline")
        && stderr.contains("current=(occupied=1 submitted=1 paused=false")
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

fn owner_release_matches(acquired: [u64; 10], released: [u64; 10], expected: [u64; 10]) -> bool {
    acquired == expected && released == expected
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
    let mut wake_turns = 0_u64;
    let mut frame_wakes = 0_u64;
    let mut tree_inspections = 0_u64;
    let mut inspected_semantic_revision = 0_u64;
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
        let language = crate::native_validation_language_evidence();
        let authority_ready = if diagnostic_authority_ready(language) {
            let server_trace = language_server_phase_trace();
            diagnostic_qualification_ready(true, completed_language_server_phases(&server_trace))
        } else {
            false
        };
        if diagnostic_tree_inspection_required(
            frame_requested,
            authority_ready,
            language.semantic_revision,
            inspected_semantic_revision,
        ) {
            tree_inspections = tree_inspections.saturating_add(1);
            let tree = inspect(surface, state).map_err(|error| {
                format!(
                    "waiting for native label prefix {prefix:?}: native tree refresh after worker wake failed: {error}"
                )
            })?;
            let observed_semantic_revision = tree.revision().semantic();
            if semantic_revision_regressed(observed_semantic_revision, inspected_semantic_revision)
            {
                return Err(format!(
                    "waiting for native label prefix {prefix:?}: semantic revision regressed from {inspected_semantic_revision} to {observed_semantic_revision}"
                )
                .into());
            }
            inspected_semantic_revision = observed_semantic_revision;
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
                "waiting for native label prefix {prefix:?}: label did not become visible before the {DIAGNOSTIC_READY_TIMEOUT:?} correctness deadline; polling=(wake_turns={wake_turns} frame_wakes={frame_wakes} tree_inspections={tree_inspections} inspected_semantic_revision={inspected_semantic_revision}) surface=(occupied={} submitted={} paused={} submissions={} terminal={:?}) worker=(queued_requests={} queued_results={} dropped_results={} panicked_jobs={}) external=(current_items={} admitted={} drained={} full={} disconnected={} shutting_down={} sequence_exhausted={}) language={language:?} server=(completed_phases={completed_server_phases}/{} next_phase={next_server_phase:?} trace={server_trace:?})",
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct LanguageServerTraceEvidence {
    attempts: usize,
    completed_phases: usize,
}

fn trace_process_id(line: &str, prefix: &str) -> Option<u32> {
    let value = line.strip_prefix(prefix)?.parse::<u32>().ok()?;
    (value > 0).then_some(value)
}

const fn semantic_revision_regressed(observed: u64, previous: u64) -> bool {
    observed < previous
}

fn language_server_trace_evidence(trace: &str) -> Result<LanguageServerTraceEvidence, Box<str>> {
    const ATTEMPT_PHASES: [&str; 7] = [
        "wrapper-invoked",
        "process-spawned",
        "initialize-received",
        "initialize-responded",
        "initialized-received",
        "did-open-received",
        "diagnostics-written",
    ];
    let mut lines = trace.lines();
    if lines.next() != Some("qualification-child") {
        return Err("language trace must start with qualification-child".into());
    }
    let mut attempts = 0_usize;
    let mut process_id = 0_u32;
    let mut completed_attempt_phases = 0_usize;
    for line in lines {
        if let Some(next_process_id) = trace_process_id(line, "wrapper-invoked:") {
            attempts = attempts
                .checked_add(1)
                .ok_or_else(|| Box::<str>::from("language trace attempt count overflow"))?;
            process_id = next_process_id;
            completed_attempt_phases = 1;
            continue;
        }
        if attempts == 0 {
            return Err(format!("language trace phase {line:?} has no owning attempt").into());
        }
        if completed_attempt_phases >= ATTEMPT_PHASES.len() {
            return Err(format!("language trace has trailing phase {line:?}").into());
        }
        if completed_attempt_phases == 1 {
            let Some(spawned_process_id) = trace_process_id(line, "process-spawned:") else {
                return Err(format!(
                    "language trace expected PID-bound process spawn, observed {line:?}"
                )
                .into());
            };
            if spawned_process_id != process_id {
                return Err(format!(
                    "language trace wrapper PID {process_id} did not own process PID {spawned_process_id}"
                )
                .into());
            }
        } else if line != ATTEMPT_PHASES[completed_attempt_phases] {
            return Err(format!(
                "language trace expected {:?}, observed {line:?}",
                ATTEMPT_PHASES[completed_attempt_phases]
            )
            .into());
        }
        completed_attempt_phases = completed_attempt_phases.saturating_add(1);
    }
    if attempts == 0 {
        return Err("language trace contains no process attempt".into());
    }
    Ok(LanguageServerTraceEvidence {
        attempts,
        completed_phases: completed_attempt_phases.saturating_add(1),
    })
}

#[doc(hidden)]
pub fn validate_native_language_startup_prefix(trace: &str) -> Result<(), Box<str>> {
    let mut phases = trace.lines();
    if phases.next() == Some("qualification-child") && phases.next().is_none() {
        return Ok(());
    }
    let _evidence = language_server_trace_evidence(trace)?;
    Ok(())
}

#[doc(hidden)]
pub fn validate_native_language_startup_trace(trace: &str) -> Result<(), Box<str>> {
    let evidence = language_server_trace_evidence(trace)?;
    if evidence.attempts != 1 {
        Err(format!(
            "language startup used {} processes instead of one workspace process",
            evidence.attempts
        )
        .into())
    } else if evidence.completed_phases == REQUIRED_LANGUAGE_PHASES.len() {
        Ok(())
    } else {
        Err(format!(
            "final language attempt was incomplete: attempts={} completed_phases={}/{}",
            evidence.attempts,
            evidence.completed_phases,
            REQUIRED_LANGUAGE_PHASES.len()
        )
        .into())
    }
}

fn completed_language_server_phases(trace: &str) -> usize {
    language_server_trace_evidence(trace).map_or(0, |evidence| evidence.completed_phases)
}

const fn diagnostic_qualification_ready(
    authority_ready: bool,
    completed_server_phases: usize,
) -> bool {
    authority_ready && completed_server_phases == REQUIRED_LANGUAGE_PHASES.len()
}

fn diagnostic_authority_ready(evidence: crate::NativeValidationLanguageEvidence) -> bool {
    evidence.active
        && evidence.sync_calls > 0
        && evidence.wake_callbacks > 0
        && language_handoff_recovered(evidence)
        && (evidence.foreground_results > 0 || evidence.latch_polls > 0)
        && evidence.generation > 0
        && evidence.process_epoch > 0
        && evidence.lsp_version > 0
        && evidence.submitted_inputs >= 3
        && evidence.written_inputs >= 3
        && evidence.input_saturations == 0
        && evidence.process_starts == 1
        && evidence.polls > 0
        && evidence.diagnostic_publications > 0
        && evidence.diagnostic_items > 0
        && stale_language_wakes_are_bounded(evidence)
        && evidence.restarts == 0
        && evidence.document_switches > 0
        && evidence.invalidations > 0
        && evidence.frame_builds > 1
        && evidence.semantic_revision > 0
}

fn stale_language_wakes_are_bounded(evidence: crate::NativeValidationLanguageEvidence) -> bool {
    evidence
        .foreground_results
        .checked_add(evidence.latch_polls)
        .is_some_and(|observed_wakes| evidence.stale_wakes <= observed_wakes)
}

fn language_handoff_recovered(evidence: crate::NativeValidationLanguageEvidence) -> bool {
    let Some(classified) = evidence
        .external_admissions
        .checked_add(evidence.external_full)
        .and_then(|value| value.checked_add(evidence.external_disconnected))
        .and_then(|value| value.checked_add(evidence.external_shutting_down))
        .and_then(|value| value.checked_add(evidence.external_sequence_exhausted))
    else {
        return false;
    };
    classified != 0
        && classified == evidence.latch_publications
        && evidence.external_disconnected == 0
        && evidence.external_shutting_down == 0
        && evidence.external_sequence_exhausted == 0
        && evidence.published_wake_generation == evidence.generation
        && evidence.observed_wake_generation == evidence.generation
        && evidence.pending_wake_generation == 0
}

fn diagnostic_wait_expired(elapsed: Duration) -> bool {
    elapsed >= DIAGNOSTIC_READY_TIMEOUT
}

const fn diagnostic_tree_inspection_required(
    frame_requested: bool,
    authority_ready: bool,
    semantic_revision: u64,
    inspected_semantic_revision: u64,
) -> bool {
    authority_ready && (frame_requested || semantic_revision > inspected_semantic_revision)
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
    let action_baseline = surface.snapshot();
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
        await_frame_terminal(surface, state, action_baseline, FRAME_TERMINAL_TIMEOUT).map_err(
            |error| {
            format!(
                "native accessibility action frame failed for role={role:?} label={label:?}: {error}"
            )
            },
        )?;
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
    let action_baseline = surface.snapshot();
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
        await_frame_terminal(surface, state, action_baseline, FRAME_TERMINAL_TIMEOUT)?;
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

        assert!(!diagnostic_tree_inspection_required(false, false, 0, 0));
        assert!(!diagnostic_tree_inspection_required(true, false, 2, 1));
        assert!(diagnostic_tree_inspection_required(false, true, 2, 1));
        assert!(diagnostic_tree_inspection_required(true, true, 2, 2));
        assert!(!diagnostic_tree_inspection_required(false, true, 2, 2));
    }

    #[test]
    fn language_server_phase_evidence_requires_the_exact_ordered_prefix() {
        assert_eq!(
            trace_process_id("wrapper-invoked:41", "wrapper-invoked:"),
            Some(41)
        );
        assert_eq!(
            trace_process_id("wrapper-invoked:0", "wrapper-invoked:"),
            None
        );
        assert_eq!(
            trace_process_id("wrapper-invoked:x", "wrapper-invoked:"),
            None
        );
        assert!(!semantic_revision_regressed(2, 1));
        assert!(!semantic_revision_regressed(2, 2));
        assert!(semantic_revision_regressed(1, 2));
        let trace = "qualification-child\nwrapper-invoked:41\nprocess-spawned:41\ninitialize-received\ninitialize-responded\ninitialized-received\ndid-open-received\ndiagnostics-written\n";
        assert_eq!(
            completed_language_server_phases(trace),
            REQUIRED_LANGUAGE_PHASES.len()
        );
        assert_eq!(validate_native_language_startup_trace(trace), Ok(()));
        assert_eq!(validate_native_language_startup_prefix(trace), Ok(()));
        assert_eq!(
            validate_native_language_startup_prefix(
                "qualification-child\nwrapper-invoked:41\nprocess-spawned:41\ninitialize-received\n"
            ),
            Ok(())
        );
        assert_eq!(
            validate_native_language_startup_prefix("qualification-child\n"),
            Ok(())
        );
        let switched = "qualification-child\nwrapper-invoked:41\nprocess-spawned:41\ninitialize-received\nwrapper-invoked:73\nprocess-spawned:73\ninitialize-received\ninitialize-responded\ninitialized-received\ndid-open-received\ndiagnostics-written\n";
        assert_eq!(
            completed_language_server_phases(switched),
            REQUIRED_LANGUAGE_PHASES.len()
        );
        assert!(validate_native_language_startup_trace(switched).is_err());
        assert_eq!(
            completed_language_server_phases(
                "qualification-child\nwrapper-invoked:41\nprocess-spawned:73\n"
            ),
            0
        );
        assert!(validate_native_language_startup_trace(
            "qualification-child\nwrapper-invoked:41\nprocess-spawned:41\ninitialize-responded\n"
        )
        .is_err());
        assert!(
            validate_native_language_startup_prefix(
                "qualification-child\nwrapper-invoked:41\nprocess-spawned:73\n"
            )
            .is_err()
        );
        assert!(
            validate_native_language_startup_prefix(
                "qualification-child\nwrapper-invoked:41\ninitialize-received\n"
            )
            .is_err()
        );
        assert!(
            validate_native_language_startup_trace("qualification-child\nwrapper-invoked:41\n")
                .is_err()
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
            external_full: 0,
            external_disconnected: 0,
            external_shutting_down: 0,
            external_sequence_exhausted: 0,
            published_wake_generation: 1,
            observed_wake_generation: 1,
            pending_wake_generation: 0,
            latch_polls: 0,
            foreground_results: 1,
            invalidations: 1,
            active: true,
            generation: 1,
            process_epoch: 1,
            lsp_version: 1,
            process_queued_events: 0,
            process_starts: 1,
            submitted_inputs: 3,
            written_inputs: 3,
            input_saturations: 0,
            polls: 1,
            diagnostic_publications: 1,
            diagnostic_items: 1,
            stale_wakes: 0,
            restarts: 0,
            document_switches: 1,
            frame_builds: 2,
            semantic_revision: 1,
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
                latch_publications: 2,
                ..valid
            },
            crate::NativeValidationLanguageEvidence {
                external_disconnected: 1,
                latch_publications: 2,
                ..valid
            },
            crate::NativeValidationLanguageEvidence {
                external_shutting_down: 1,
                latch_publications: 2,
                ..valid
            },
            crate::NativeValidationLanguageEvidence {
                external_sequence_exhausted: 1,
                latch_publications: 2,
                ..valid
            },
            crate::NativeValidationLanguageEvidence {
                published_wake_generation: 2,
                ..valid
            },
            crate::NativeValidationLanguageEvidence {
                observed_wake_generation: 2,
                ..valid
            },
            crate::NativeValidationLanguageEvidence {
                pending_wake_generation: 1,
                ..valid
            },
            crate::NativeValidationLanguageEvidence {
                foreground_results: 0,
                latch_polls: 0,
                ..valid
            },
            crate::NativeValidationLanguageEvidence {
                generation: 0,
                published_wake_generation: 0,
                observed_wake_generation: 0,
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
                stale_wakes: 2,
                ..valid
            },
            crate::NativeValidationLanguageEvidence {
                foreground_results: u64::MAX,
                latch_polls: 1,
                ..valid
            },
            crate::NativeValidationLanguageEvidence {
                restarts: 1,
                ..valid
            },
            crate::NativeValidationLanguageEvidence {
                document_switches: 0,
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
            crate::NativeValidationLanguageEvidence {
                semantic_revision: 0,
                ..valid
            },
        ] {
            assert!(!diagnostic_authority_ready(invalid));
        }
        assert!(diagnostic_authority_ready(
            crate::NativeValidationLanguageEvidence {
                latch_publications: 2,
                external_full: 1,
                ..valid
            }
        ));
        assert!(diagnostic_authority_ready(
            crate::NativeValidationLanguageEvidence {
                external_admissions: 0,
                external_full: 1,
                foreground_results: 0,
                latch_polls: 1,
                ..valid
            }
        ));
        assert!(diagnostic_authority_ready(
            crate::NativeValidationLanguageEvidence {
                stale_wakes: 1,
                ..valid
            }
        ));
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

        assert!(frame_quiescent(0, 0, true));
        assert!(!frame_quiescent(1, 0, true));
        assert!(!frame_quiescent(0, 1, true));
        assert!(!frame_quiescent(0, 0, false));

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

        let expected = [1_u64; 10];
        assert!(owner_release_matches(expected, expected, expected));
        let mut wrong_acquired = expected;
        wrong_acquired[0] = 0;
        assert!(!owner_release_matches(wrong_acquired, expected, expected));
        let mut wrong_released = expected;
        wrong_released[9] = 0;
        assert!(!owner_release_matches(expected, wrong_released, expected));

        assert!(is_close_status_role("AXStaticText"));
        assert!(!is_close_status_role("AXButton"));
    }

    #[test]
    fn hosted_observation_requires_mode_in_flight_slots_and_a_new_submission() {
        assert!(should_arm_hosted_observation(
            PresentationEvidenceMode::HostedDirect,
            1,
            None,
            4
        ));
        assert!(!should_arm_hosted_observation(
            PresentationEvidenceMode::Physical,
            1,
            None,
            4
        ));
        assert!(!should_arm_hosted_observation(
            PresentationEvidenceMode::HostedDirect,
            0,
            None,
            4
        ));
        assert!(!should_arm_hosted_observation(
            PresentationEvidenceMode::HostedDirect,
            1,
            Some(4),
            4
        ));
        assert!(!should_arm_hosted_observation(
            PresentationEvidenceMode::HostedDirect,
            0,
            None,
            18
        ));
        assert!(should_arm_hosted_observation(
            PresentationEvidenceMode::HostedDirect,
            1,
            None,
            19
        ));
    }

    #[test]
    fn hosted_terminal_retry_requires_every_exact_stall_fact() {
        let stall = "dirty-close native frame failed: frame-terminal correctness-timeout failed after 1 observed submissions: frame ownership did not become terminal before the correctness deadline; current=(occupied=1 submitted=1 paused=false submissions=19)";
        assert!(hosted_terminal_stall_retry_allowed(
            "hosted-direct",
            0,
            false,
            "",
            stall,
            true
        ));
        assert!(!hosted_terminal_stall_retry_allowed(
            "physical", 0, false, "", stall, true
        ));
        assert!(!hosted_terminal_stall_retry_allowed(
            "hosted-direct",
            1,
            false,
            "",
            stall,
            true
        ));
        assert!(!hosted_terminal_stall_retry_allowed(
            "hosted-direct",
            0,
            true,
            "",
            stall,
            true
        ));
        assert!(!hosted_terminal_stall_retry_allowed(
            "hosted-direct",
            0,
            false,
            "unexpected output",
            stall,
            true
        ));
        assert!(!hosted_terminal_stall_retry_allowed(
            "hosted-direct",
            0,
            false,
            "",
            stall,
            false
        ));
        for non_stall in [
            stall.replace("dirty-close", "initial"),
            stall.replace("correctness-timeout", "event-loop"),
            stall.replace("occupied=1", "occupied=0"),
        ] {
            assert!(!hosted_terminal_stall_retry_allowed(
                "hosted-direct",
                0,
                false,
                "",
                &non_stall,
                true
            ));
        }
    }

    #[test]
    fn omission_failure_requires_the_exact_named_step_and_bounds_diagnostics() {
        for step in [
            OmittedStep::Open,
            OmittedStep::Edit,
            OmittedStep::Action,
            OmittedStep::Save,
            OmittedStep::Close,
        ] {
            let requested = match step {
                OmittedStep::Open => "open",
                OmittedStep::Edit => "edit",
                OmittedStep::Action => "action",
                OmittedStep::Save => "save",
                OmittedStep::Close => "close",
            };
            let observed = if step == OmittedStep::Open {
                format!(
                    "{}; cause=command activation rejected",
                    step.expected_failure()
                )
            } else {
                step.expected_failure().to_owned()
            };
            assert!(validate_native_accessibility_omission_failure(requested, &observed).is_ok());
            assert!(
                validate_native_accessibility_omission_failure(
                    requested,
                    "unrelated native failure"
                )
                .is_err()
            );
        }
        assert!(
            validate_native_accessibility_omission_failure(
                "edit",
                OmittedStep::Save.expected_failure()
            )
            .is_err()
        );
        assert!(
            validate_native_accessibility_omission_failure(
                "unknown",
                OmittedStep::Open.expected_failure()
            )
            .is_err()
        );
        assert!(
            validate_native_accessibility_omission_failure(
                "open",
                OmittedStep::Open.expected_failure()
            )
            .is_err()
        );
        assert!(
            validate_native_accessibility_omission_failure(
                "edit",
                OmittedStep::Edit.expected_failure()
            )
            .is_ok()
        );
        assert!(
            validate_native_accessibility_omission_failure(
                "edit",
                &format!(
                    "{}; cause=unrelated failure",
                    OmittedStep::Edit.expected_failure()
                )
            )
            .is_err()
        );
        let oversized_open = format!(
            "{}; cause={}",
            OmittedStep::Open.expected_failure(),
            "x".repeat(MAX_OMISSION_ERROR_BYTES + 1)
        );
        assert!(validate_native_accessibility_omission_failure("open", &oversized_open).is_err());

        let oversized = "x".repeat(MAX_OMISSION_ERROR_BYTES * 2);
        let error = validate_native_accessibility_omission_failure("edit", &oversized)
            .expect_err("an unrelated oversized error must fail closed");
        assert!(error.len() < MAX_OMISSION_ERROR_BYTES + 256);
        assert!(!error.contains(&"x".repeat(MAX_OMISSION_ERROR_BYTES + 1)));
    }
}

fn open_palette_and_activate(
    surface: &NativeSurface,
    state: &Rc<RefCell<Application<StudioApp>>>,
    timestamp: &mut u64,
    label: &str,
) -> Result<u64, Box<dyn std::error::Error>> {
    let (input_epoch, native_focused) = platform_validation::input_focus_state(surface);
    if !native_focused {
        platform_validation::set_input_focus_state(surface, input_epoch, true);
    }
    if platform_validation::input_focus_state(surface) != (input_epoch, true) {
        return Err("command-palette native focus restoration failed".into());
    }
    dispatch(
        surface,
        state,
        &[SurfaceEvent::Focus {
            timestamp: EventTimestamp::new(*timestamp),
            input_epoch,
            focused: true,
        }],
    )
    .map_err(|error| format!("command-palette focus dispatch failed: {error}"))?;
    *timestamp = timestamp.saturating_add(1);
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

fn relinquish_native_focus(
    surface: &NativeSurface,
    state: &Rc<RefCell<Application<StudioApp>>>,
    timestamp: &mut u64,
) -> Result<InputEpoch, Box<dyn std::error::Error>> {
    let (current_epoch, _) = platform_validation::input_focus_state(surface);
    let lost_epoch = current_epoch
        .checked_next()
        .ok_or("native focus epoch exhausted during accessibility qualification")?;
    platform_validation::set_input_focus_state(surface, lost_epoch, false);
    if platform_validation::input_focus_state(surface) != (lost_epoch, false) {
        return Err("native focus-loss control was not established".into());
    }
    dispatch(
        surface,
        state,
        &[SurfaceEvent::Focus {
            timestamp: EventTimestamp::new(*timestamp),
            input_epoch: lost_epoch,
            focused: false,
        }],
    )
    .map_err(|error| format!("native focus-loss control dispatch failed: {error}"))?;
    *timestamp = timestamp.saturating_add(1);
    Ok(lost_epoch)
}
