//! Runtime contracts for the first owned Alpine Studio native surface.
//!
//! This crate intentionally contains only interface-level contracts for the first
//! native slice: deterministic lifecycle transitions, demand-driven frame admission,
//! input event batching, and non-panicking presentation outcomes. The goal is to
//! let implementation work evolve behind a stable contract.

use std::collections::VecDeque;

/// Identifier for an owned native surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SurfaceId {
    /// Stable numeric identity for a surface lifetime.
    value: u64,
}

impl SurfaceId {
    /// Creates a deterministic identity for a surface.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self { value }
    }

    /// Returns the raw identifier value.
    #[must_use]
    pub const fn value(self) -> u64 {
        self.value
    }
}

/// Publicly visible lifecycle for the run contract.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SurfaceState {
    /// Surface exists but has not yet entered an active run loop.
    #[default]
    Created,
    /// Surface is running and has accepted at least one frame in the active turn.
    Running,
    /// Surface is intentionally paused by visibility or workload policy.
    Paused,
    /// Shutdown requested and event loop will unwind after the in-flight frame.
    Closing,
    /// Surface has fully drained and is closed.
    Closed,
}

/// Surface visibility and power-policy hints used by the run contract.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Visibility {
    /// Surface is visible and can schedule frames.
    #[default]
    Visible,
    /// Surface exists but is hidden and should not render.
    Hidden,
    /// Surface is present but not visually relevant and should defer presenting.
    Occluded,
}

/// Failure modes that keep runtime behavior deterministic and avoid panics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeSurfaceError {
    /// Host does not support the requested surface operation.
    Unsupported,
    /// `run` was called while another run is active.
    AlreadyRunning,
    /// A surface is already closed.
    AlreadyClosed,
    /// Input queue exceeded configured capacity.
    InputOverflow,
    /// No dirty frame exists to process.
    NoFrameRequested,
}

/// A recoverable presentation status for frame submission.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresentationResult {
    /// Frame was presented and drained through the platform boundary.
    Presented,
    /// Surface was not eligible for presentation but state remained healthy.
    SkippedByPause,
    /// Device was interrupted and must be recovered through platform lifecycle.
    DeviceLost,
    /// Capability was not available on this host.
    UnsupportedCapability,
}

/// Reasons a frame can be considered dirty.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FrameDemandReason {
    /// First frame after startup, launch, or restore.
    #[default]
    Initial,
    /// Scene graph changed.
    SceneDirty,
    /// Input requires deterministic coalesced redraw.
    Input,
    /// Accessibility state or announcement changed.
    Accessibility,
    /// Visibility/power change required a reconciliation frame.
    Recovery,
}

/// One most-recent frame request per run loop turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameRequest {
    /// Monotonic revision from the caller.
    pub revision: u64,
    /// Why this revision is considered render-relevant.
    pub reason: FrameDemandReason,
    /// Whether any input is associated with this revision.
    pub has_input: bool,
}

/// Demand-driven coalescing policy: one latest frame per run turn.
#[derive(Debug, Default)]
pub struct FrameDemandQueue {
    latest: Option<FrameRequest>,
}

impl FrameDemandQueue {
    /// Creates a clean queue.
    #[must_use]
    pub const fn new() -> Self {
        Self { latest: None }
    }

    /// Enqueues or coalesces a request, keeping only the latest revision.
    pub fn request(&mut self, revision: u64, reason: FrameDemandReason, has_input: bool) {
        if self
            .latest
            .is_none_or(|existing| revision >= existing.revision)
        {
            self.latest = Some(FrameRequest {
                revision,
                reason,
                has_input,
            });
        }
    }

    /// Clears and returns the latest coalesced frame request if any.
    pub fn take_latest(&mut self) -> Option<FrameRequest> {
        self.latest.take()
    }

    /// Returns whether there is any pending frame work.
    #[must_use]
    pub const fn has_pending(&self) -> bool {
        self.latest.is_some()
    }
}

/// Input event payload accepted by the run loop.
#[derive(Clone, Debug, PartialEq)]
pub enum NativeInputEvent {
    /// Keyboard event with deterministic shape.
    Keyboard(KeyboardEvent),
    /// Pointer event with deterministic shape.
    Pointer(PointerEvent),
    /// Clipboard transfer event.
    Clipboard(ClipboardEvent),
    /// IME composition lifecycle.
    Ime(ImeEvent),
    /// Accessibility-focused announcement.
    Accessibility(AccessibilityEvent),
    /// Lifecycle and scheduler events.
    Lifecycle(LifecycleEvent),
}

/// Modifier mask for keyboard input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Modifiers {
    /// Command / meta key state.
    pub command: bool,
    /// Control key state.
    pub control: bool,
    /// Option / alt key state.
    pub option: bool,
    /// Shift key state.
    pub shift: bool,
}

impl Default for Modifiers {
    fn default() -> Self {
        Self {
            command: false,
            control: false,
            option: false,
            shift: false,
        }
    }
}

/// Keyboard event with lifecycle-safe phases.
#[derive(Clone, Debug, PartialEq)]
pub struct KeyboardEvent {
    /// Raw key string used by app-specific mapping.
    pub key: String,
    /// Logical key state for deterministic sequencing.
    pub phase: KeyPhase,
    /// Modifier mask for deterministic policy matching.
    pub modifiers: Modifiers,
    /// Internal sequence for coalescing or replay.
    pub sequence: u64,
}

/// Pointer event state.
#[derive(Clone, Debug, PartialEq)]
pub struct PointerEvent {
    /// Pointer x position in logical surface coordinates.
    pub x: u32,
    /// Pointer y position in logical surface coordinates.
    pub y: u32,
    /// Button identifier or wheel axis marker.
    pub element: u16,
    /// Event kind and action.
    pub kind: PointerKind,
    /// Modifier mask for deterministic dispatch.
    pub modifiers: Modifiers,
}

/// Clipboard action kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipboardEvent {
    /// Clipboard action name.
    pub action: ClipboardAction,
    /// Text payload for non-empty clipboard actions.
    pub text: Option<String>,
}

/// IME lifecycle event.
#[derive(Clone, Debug, PartialEq)]
pub struct ImeEvent {
    /// Session identifier for composition tracking.
    pub session_id: u64,
    /// Composition operation.
    pub phase: ImePhase,
    /// Optional UTF-8 composition payload.
    pub text: Option<String>,
}

/// Accessibility announcement event.
#[derive(Clone, Debug, PartialEq)]
pub struct AccessibilityEvent {
    /// Announcement text for assistive technologies.
    pub announcement: String,
    /// Priority label for deterministic ordering.
    pub level: AccessibilityLevel,
}

/// Lifecycle / scheduler event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LifecycleEvent {
    /// Surface became visible.
    Visible,
    /// Surface became hidden.
    Hidden,
    /// Surface became occluded.
    Occluded,
    /// Input to close requested by the platform or user.
    CloseRequested,
    /// Surface woke from system suspension.
    Wake,
    /// Surface is now suspended.
    Sleep,
    /// Surface resized; zeros can pause rendering safely.
    Resize { width: u32, height: u32 },
}

/// Key transition phase.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyPhase {
    /// Press event.
    Pressed,
    /// Release event.
    Released,
    /// Repeat event.
    Repeat,
}

/// Pointer action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PointerKind {
    /// Pointer moved with no button change.
    Move,
    /// Down event with active button/axis.
    Down,
    /// Up event with active button/axis.
    Up,
    /// Wheel or trackpad scroll axis.
    Scroll,
}

/// Clipboard operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClipboardAction {
    /// Copy selection to clipboard.
    Copy,
    /// Paste selection from clipboard.
    Paste,
    /// Cut selection to clipboard.
    Cut,
}

/// IME composition phase.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImePhase {
    /// Composition started.
    Start,
    /// Composition updated.
    Update,
    /// Composition committed.
    Commit,
    /// Composition cancelled.
    Cancel,
}

/// Accessibility importance channel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AccessibilityLevel {
    /// No priority hint.
    Info,
    /// Soft preference for announcement ordering.
    Moderate,
    /// Assertive announcement requirement.
    Assertive,
}

/// Run turn data fed into host integration.
#[derive(Debug)]
pub struct RunTurn {
    /// Surface identity.
    pub surface_id: SurfaceId,
    /// Current visibility at turn start.
    pub visibility: Visibility,
    /// Frame request that won coalescing.
    pub frame: FrameRequest,
    /// Input events available at this run turn.
    pub events: Vec<NativeInputEvent>,
}

/// Callbacks decide whether to continue, yield, or close.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunDecision {
    /// Continue with this turn and keep the surface paused after drain.
    Continue,
    /// Continue and immediately re-run if a new frame is pending.
    ContinueAndDrill,
    /// Begin close and transition through shutdown.
    RequestShutdown,
}

/// `run` return shape separates shutdown from no-frame states.
#[derive(Debug, PartialEq)]
pub enum RunOutcome {
    /// No frame requested so run exits cleanly.
    NoFrame {
        /// Pending queue was empty.
        reason: NativeSurfaceError,
        /// Pending event count at clean exit.
        queued_inputs: usize,
    },
    /// A frame was processed.
    FrameProcessed {
        /// Revision consumed by the callback.
        revision: u64,
        /// Reason carried on the processed frame.
        reason: FrameDemandReason,
        /// Input events processed in this turn.
        event_count: usize,
        /// Final surface state after callback.
        post_state: SurfaceState,
    },
    /// Surface is closed and fully drained.
    Closed {
        /// Last processed revision before closure.
        last_revision: u64,
        /// Events still queued at closure time.
        queued_inputs: usize,
    },
}

/// Configuration constants for safe first-mile behavior.
#[derive(Clone, Debug)]
pub struct NativeSurfaceConfig {
    /// Maximum queued input events.
    pub max_input_events: usize,
}

impl Default for NativeSurfaceConfig {
    fn default() -> Self {
        Self {
            max_input_events: 1024,
        }
    }
}

/// Owned runtime contract for one native surface.
#[derive(Debug)]
pub struct NativeSurface {
    id: SurfaceId,
    state: SurfaceState,
    visibility: Visibility,
    width: u32,
    height: u32,
    frame_queue: FrameDemandQueue,
    input_events: VecDeque<NativeInputEvent>,
    config: NativeSurfaceConfig,
    shutdown_requested: bool,
    last_revision: u64,
}

impl NativeSurface {
    /// Creates a default surface contract with deterministic configuration.
    #[must_use]
    pub fn new(id: SurfaceId) -> Self {
        Self {
            id,
            state: SurfaceState::default(),
            visibility: Visibility::default(),
            width: 0,
            height: 0,
            frame_queue: FrameDemandQueue::default(),
            input_events: VecDeque::new(),
            config: NativeSurfaceConfig::default(),
            shutdown_requested: false,
            last_revision: 0,
        }
    }

    /// Returns the current lifecycle state.
    #[must_use]
    pub const fn state(&self) -> SurfaceState {
        self.state
    }

    /// Returns whether the surface has been closed.
    #[must_use]
    pub const fn is_closed(&self) -> bool {
        self.state == SurfaceState::Closed
    }

    /// Returns current visibility hint.
    #[must_use]
    pub const fn visibility(&self) -> Visibility {
        self.visibility
    }

    /// Returns the native size in logical pixels.
    #[must_use]
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Updates visibility and emits deterministic pause/continue transitions.
    pub fn set_visibility(&mut self, visibility: Visibility) {
        self.visibility = visibility;
        self.state = match visibility {
            Visibility::Visible => {
                if self.shutdown_requested || self.state == SurfaceState::Closed {
                    self.state
                } else {
                    SurfaceState::Paused
                }
            }
            Visibility::Hidden | Visibility::Occluded => SurfaceState::Paused,
        };
    }

    /// Updates logical size. A zero-size surface is treated as hidden for run-time
    /// scheduling and presentation.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
        if width == 0 || height == 0 {
            self.state = SurfaceState::Paused;
        }
    }

    /// Requests a deterministic frame with latest-wins coalescing.
    pub fn request_frame(&mut self, revision: u64, reason: FrameDemandReason, has_input: bool) {
        self.frame_queue.request(revision, reason, has_input);
    }

    /// Enqueues a deterministic input event for the next frame drain.
    pub fn post_event(&mut self, event: NativeInputEvent) -> Result<(), NativeSurfaceError> {
        if self.input_events.len() >= self.config.max_input_events {
            return Err(NativeSurfaceError::InputOverflow);
        }

        self.input_events.push_back(event);
        Ok(())
    }

    /// Triggers close with a clean shutdown sequence.
    pub fn request_shutdown(&mut self) {
        self.shutdown_requested = true;
    }

    /// Signals platform-level unsupported state without panicking.
    pub fn unsupported_state_error() -> NativeSurfaceError {
        NativeSurfaceError::Unsupported
    }

    /// Returns one latest revision and consumes all matching input for that turn.
    pub fn run<F>(&mut self, mut on_turn: F) -> Result<RunOutcome, NativeSurfaceError>
    where
        F: FnMut(RunTurn) -> RunDecision,
    {
        if self.state == SurfaceState::Closed {
            return Err(NativeSurfaceError::AlreadyClosed);
        }
        if self.state == SurfaceState::Running {
            return Err(NativeSurfaceError::AlreadyRunning);
        }

        self.state = SurfaceState::Running;

        if self.shutdown_requested {
            self.state = SurfaceState::Closing;
        }

        if matches!(self.visibility, Visibility::Hidden | Visibility::Occluded)
            || self.width == 0
            || self.height == 0
        {
            self.state = SurfaceState::Paused;
            let queued_inputs = self.input_events.len();
            if self.shutdown_requested {
                self.state = SurfaceState::Closed;
                return Ok(RunOutcome::Closed {
                    last_revision: self.last_revision,
                    queued_inputs,
                });
            }

            return Ok(RunOutcome::NoFrame {
                reason: NativeSurfaceError::NoFrameRequested,
                queued_inputs,
            });
        }

        let frame = match self.frame_queue.take_latest() {
            Some(frame) => frame,
            None => {
                self.state = SurfaceState::Paused;
                return Ok(RunOutcome::NoFrame {
                    reason: NativeSurfaceError::NoFrameRequested,
                    queued_inputs: self.input_events.len(),
                });
            }
        };

        let events = self.input_events.drain(..).collect::<Vec<_>>();
        let event_count = events.len();
        let turn = RunTurn {
            surface_id: self.id,
            visibility: self.visibility,
            frame,
            events,
        };

        let decision = on_turn(turn);
        self.last_revision = frame.revision;

        match decision {
            RunDecision::Continue => {
                self.state = SurfaceState::Paused;
            }
            RunDecision::ContinueAndDrill => {
                if self.frame_queue.has_pending() {
                    self.state = SurfaceState::Running;
                } else {
                    self.state = SurfaceState::Paused;
                }
            }
            RunDecision::RequestShutdown => {
                self.shutdown_requested = true;
                self.state = SurfaceState::Closing;
            }
        }

        if self.shutdown_requested {
            self.state = SurfaceState::Closed;
            return Ok(RunOutcome::Closed {
                last_revision: self.last_revision,
                queued_inputs: self.input_events.len(),
            });
        }

        Ok(RunOutcome::FrameProcessed {
            revision: self.last_revision,
            reason: frame.reason,
            event_count,
            post_state: self.state,
        })
    }

    /// Deterministic presentation classification for recovery and adaptation gates.
    pub fn present_result(&self, status: PresentationResult) -> Result<(), NativeSurfaceError> {
        match status {
            PresentationResult::Presented | PresentationResult::SkippedByPause => Ok(()),
            PresentationResult::DeviceLost | PresentationResult::UnsupportedCapability => {
                Err(NativeSurfaceError::Unsupported)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press_b_key(sequence: u64) -> NativeInputEvent {
        NativeInputEvent::Keyboard(KeyboardEvent {
            key: "B".to_string(),
            phase: KeyPhase::Pressed,
            modifiers: Modifiers::default(),
            sequence,
        })
    }

    #[test]
    fn keeps_only_latest_frame_request() {
        let mut surface = NativeSurface::new(SurfaceId::new(1));
        surface.request_frame(1, FrameDemandReason::Initial, false);
        surface.request_frame(7, FrameDemandReason::Input, true);
        surface.request_frame(3, FrameDemandReason::SceneDirty, true);
        assert_eq!(
            surface.frame_queue.take_latest(),
            Some(FrameRequest {
                revision: 7,
                reason: FrameDemandReason::Input,
                has_input: true,
            })
        );
    }

    #[test]
    fn run_drains_events_and_closes_cleanly() {
        let mut surface = NativeSurface::new(SurfaceId::new(2));
        surface.resize(1440, 1024);
        surface.request_frame(11, FrameDemandReason::SceneDirty, true);
        assert_eq!(surface.post_event(press_b_key(1)), Ok(()));

        let outcome = surface.run(|turn| {
            assert_eq!(turn.frame.revision, 11);
            assert_eq!(turn.events.len(), 1);
            assert_eq!(turn.surface_id, SurfaceId::new(2));
            RunDecision::RequestShutdown
        });

        assert_eq!(
            outcome,
            Ok(RunOutcome::Closed {
                last_revision: 11,
                queued_inputs: 0,
            })
        );
    }

    #[test]
    fn hidden_state_pauses_without_render() {
        let mut surface = NativeSurface::new(SurfaceId::new(3));
        surface.resize(800, 600);
        surface.set_visibility(Visibility::Hidden);
        surface.request_frame(1, FrameDemandReason::Accessibility, false);

        let outcome = surface.run(|_| RunDecision::Continue);
        assert_eq!(
            outcome,
            Ok(RunOutcome::NoFrame {
                reason: NativeSurfaceError::NoFrameRequested,
                queued_inputs: 0,
            })
        );
        assert_eq!(surface.state(), SurfaceState::Paused);
    }

    #[test]
    fn input_queue_rejects_when_full() {
        let mut surface = NativeSurface::new(SurfaceId::new(4));
        surface.config.max_input_events = 1;
        assert_eq!(surface.post_event(press_b_key(1)), Ok(()));
        assert_eq!(surface.post_event(press_b_key(2)), Err(NativeSurfaceError::InputOverflow));
    }
}
