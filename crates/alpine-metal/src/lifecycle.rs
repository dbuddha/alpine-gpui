use std::{error::Error, fmt};

/// Renderer admission and shutdown state for the synchronous MVP.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RendererState {
    /// New frame work may begin.
    Ready,
    /// New work is rejected while submitted work drains.
    ShuttingDown,
    /// All resources are drained and teardown is complete.
    Stopped,
}

/// One frame's abstract lifecycle state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrameState {
    /// No frame has begun.
    Idle,
    /// Validation and lowering succeeded.
    Lowered,
    /// Native encoding acquired the frame resource.
    Encoded,
    /// One native submission is in flight.
    Submitted,
    /// The submission completed successfully.
    Completed,
    /// The submission reached a terminal failure.
    Failed,
    /// Work was cancelled before submission.
    Cancelled,
}

/// Exclusive ownership state of the frame resource.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResourceState {
    /// No frame owns the resource.
    Free,
    /// Lowering or encoding exclusively owns the resource.
    Encoding,
    /// A committed command buffer exclusively owns the resource.
    InFlight,
}

/// Terminal classification visible at the safe boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrameOutcome {
    /// No terminal outcome exists.
    Pending,
    /// The frame completed successfully.
    Success,
    /// The frame failed after submission.
    Failure,
    /// The frame was cancelled before submission.
    Cancelled,
}

/// An action mapped from the AEP-0025 lifecycle model.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LifecycleAction {
    /// Validate and lower a new frame.
    BeginFrame,
    /// Finish native encoding.
    Encode,
    /// Commit exactly one command buffer.
    Submit,
    /// Record successful terminal completion.
    Complete,
    /// Record terminal submission failure.
    Fail,
    /// Record allocation or encoding failure before submission.
    FailBeforeSubmit,
    /// Cancel before submission.
    CancelBeforeSubmit,
    /// Stop admitting new frames.
    BeginShutdown,
    /// Complete teardown after drain.
    StopAfterDrain,
}

/// Pure single-frame transition state corresponding to `RendererLifecycle.tla`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameLifecycle {
    renderer: RendererState,
    frame: FrameState,
    resource: ResourceState,
    outcome: FrameOutcome,
    submit_count: u8,
    release_count: u8,
}

impl Default for FrameLifecycle {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameLifecycle {
    /// Creates a ready renderer with no active frame or resource owner.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            renderer: RendererState::Ready,
            frame: FrameState::Idle,
            resource: ResourceState::Free,
            outcome: FrameOutcome::Pending,
            submit_count: 0,
            release_count: 0,
        }
    }

    /// Applies one modeled action atomically.
    ///
    /// # Errors
    ///
    /// Returns [`TransitionError`] without changing state when the action is not
    /// enabled in the current lifecycle state.
    pub fn apply(&mut self, action: LifecycleAction) -> Result<(), TransitionError> {
        let before = *self;
        if !before.invariants_hold() {
            return Err(TransitionError {
                action,
                renderer: before.renderer,
                frame: before.frame,
                resource: before.resource,
            });
        }
        let enabled = match action {
            LifecycleAction::BeginFrame => self.begin_frame(),
            LifecycleAction::Encode => self.encode(),
            LifecycleAction::Submit => self.submit(),
            LifecycleAction::Complete => self.complete(),
            LifecycleAction::Fail => self.fail(),
            LifecycleAction::FailBeforeSubmit => self.fail_before_submit(),
            LifecycleAction::CancelBeforeSubmit => self.cancel_before_submit(),
            LifecycleAction::BeginShutdown => self.begin_shutdown(),
            LifecycleAction::StopAfterDrain => self.stop_after_drain(),
        };
        if enabled {
            debug_assert!(self.invariants_hold());
            Ok(())
        } else {
            *self = before;
            Err(TransitionError {
                action,
                renderer: before.renderer,
                frame: before.frame,
                resource: before.resource,
            })
        }
    }

    /// Returns the renderer state.
    #[must_use]
    pub const fn renderer(self) -> RendererState {
        self.renderer
    }

    /// Returns the frame state.
    #[must_use]
    pub const fn frame(self) -> FrameState {
        self.frame
    }

    /// Returns the resource owner state.
    #[must_use]
    pub const fn resource(self) -> ResourceState {
        self.resource
    }

    /// Returns the terminal outcome classification.
    #[must_use]
    pub const fn outcome(self) -> FrameOutcome {
        self.outcome
    }

    /// Returns the observed submission count.
    #[must_use]
    pub const fn submit_count(self) -> u8 {
        self.submit_count
    }

    /// Returns the observed terminal resource-release count.
    #[must_use]
    pub const fn release_count(self) -> u8 {
        self.release_count
    }

    /// Checks every binding invariant represented by the finite model.
    #[must_use]
    pub const fn invariants_hold(self) -> bool {
        let frame_is_consistent = match self.frame {
            FrameState::Idle => matches!(
                (
                    self.resource,
                    self.outcome,
                    self.submit_count,
                    self.release_count
                ),
                (ResourceState::Free, FrameOutcome::Pending, 0, 0)
            ),
            FrameState::Lowered | FrameState::Encoded => matches!(
                (
                    self.resource,
                    self.outcome,
                    self.submit_count,
                    self.release_count
                ),
                (ResourceState::Encoding, FrameOutcome::Pending, 0, 0)
            ),
            FrameState::Submitted => matches!(
                (
                    self.resource,
                    self.outcome,
                    self.submit_count,
                    self.release_count
                ),
                (ResourceState::InFlight, FrameOutcome::Pending, 1, 0)
            ),
            FrameState::Completed => matches!(
                (
                    self.resource,
                    self.outcome,
                    self.submit_count,
                    self.release_count
                ),
                (ResourceState::Free, FrameOutcome::Success, 1, 1)
            ),
            FrameState::Failed => {
                matches!(
                    (self.resource, self.outcome, self.release_count),
                    (ResourceState::Free, FrameOutcome::Failure, 1)
                ) && self.submit_count <= 1
            }
            FrameState::Cancelled => matches!(
                (
                    self.resource,
                    self.outcome,
                    self.submit_count,
                    self.release_count
                ),
                (ResourceState::Free, FrameOutcome::Cancelled, 0, 1)
            ),
        };
        let renderer_is_consistent = match self.renderer {
            RendererState::Ready => true,
            RendererState::ShuttingDown => matches!(
                self.frame,
                FrameState::Idle
                    | FrameState::Submitted
                    | FrameState::Completed
                    | FrameState::Failed
                    | FrameState::Cancelled
            ),
            RendererState::Stopped => matches!(
                self.frame,
                FrameState::Idle
                    | FrameState::Completed
                    | FrameState::Failed
                    | FrameState::Cancelled
            ),
        };
        frame_is_consistent && renderer_is_consistent
    }

    fn begin_frame(&mut self) -> bool {
        if !matches!(
            (self.renderer, self.frame),
            (RendererState::Ready, FrameState::Idle)
        ) {
            return false;
        }
        self.frame = FrameState::Lowered;
        self.resource = ResourceState::Encoding;
        true
    }

    fn encode(&mut self) -> bool {
        if !matches!(
            (self.renderer, self.frame),
            (RendererState::Ready, FrameState::Lowered)
        ) {
            return false;
        }
        self.frame = FrameState::Encoded;
        true
    }

    fn submit(&mut self) -> bool {
        if !matches!(
            (self.renderer, self.frame),
            (RendererState::Ready, FrameState::Encoded)
        ) {
            return false;
        }
        self.frame = FrameState::Submitted;
        self.resource = ResourceState::InFlight;
        self.submit_count = 1;
        true
    }

    fn complete(&mut self) -> bool {
        if !matches!(
            (self.renderer, self.frame),
            (
                RendererState::Ready | RendererState::ShuttingDown,
                FrameState::Submitted
            )
        ) {
            return false;
        }
        self.frame = FrameState::Completed;
        self.resource = ResourceState::Free;
        self.release_count = 1;
        self.outcome = FrameOutcome::Success;
        true
    }

    fn fail(&mut self) -> bool {
        if !matches!(
            (self.renderer, self.frame),
            (
                RendererState::Ready | RendererState::ShuttingDown,
                FrameState::Submitted
            )
        ) {
            return false;
        }
        self.frame = FrameState::Failed;
        self.resource = ResourceState::Free;
        self.release_count = 1;
        self.outcome = FrameOutcome::Failure;
        true
    }

    fn fail_before_submit(&mut self) -> bool {
        if !matches!(
            (self.renderer, self.frame),
            (
                RendererState::Ready,
                FrameState::Lowered | FrameState::Encoded
            )
        ) {
            return false;
        }
        self.frame = FrameState::Failed;
        self.resource = ResourceState::Free;
        self.release_count = 1;
        self.outcome = FrameOutcome::Failure;
        true
    }

    fn cancel_before_submit(&mut self) -> bool {
        if !matches!(
            (self.renderer, self.frame),
            (
                RendererState::Ready,
                FrameState::Lowered | FrameState::Encoded
            )
        ) {
            return false;
        }
        self.frame = FrameState::Cancelled;
        self.resource = ResourceState::Free;
        self.release_count = 1;
        self.outcome = FrameOutcome::Cancelled;
        true
    }

    fn begin_shutdown(&mut self) -> bool {
        if !matches!(
            (self.renderer, self.frame),
            (
                RendererState::Ready,
                FrameState::Idle
                    | FrameState::Submitted
                    | FrameState::Completed
                    | FrameState::Failed
                    | FrameState::Cancelled
            )
        ) {
            return false;
        }
        self.renderer = RendererState::ShuttingDown;
        true
    }

    fn stop_after_drain(&mut self) -> bool {
        if !matches!(
            (self.renderer, self.frame),
            (
                RendererState::ShuttingDown,
                FrameState::Idle
                    | FrameState::Completed
                    | FrameState::Failed
                    | FrameState::Cancelled
            )
        ) {
            return false;
        }
        self.renderer = RendererState::Stopped;
        true
    }
}

/// A rejected action and the state in which it was attempted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransitionError {
    action: LifecycleAction,
    renderer: RendererState,
    frame: FrameState,
    resource: ResourceState,
}

impl TransitionError {
    /// Returns the rejected action.
    #[must_use]
    pub const fn action(self) -> LifecycleAction {
        self.action
    }

    /// Returns the renderer state before rejection.
    #[must_use]
    pub const fn renderer(self) -> RendererState {
        self.renderer
    }

    /// Returns the frame state before rejection.
    #[must_use]
    pub const fn frame(self) -> FrameState {
        self.frame
    }

    /// Returns the resource state before rejection.
    #[must_use]
    pub const fn resource(self) -> ResourceState {
        self.resource
    }
}

impl fmt::Display for TransitionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "action {:?} is disabled in {:?}/{:?}/{:?}",
            self.action, self.renderer, self.frame, self.resource
        )
    }
}

impl Error for TransitionError {}

#[cfg(test)]
mod tests {
    use super::{
        FrameLifecycle, FrameOutcome, FrameState, LifecycleAction, RendererState, ResourceState,
    };

    const RENDERER_STATES: [RendererState; 3] = [
        RendererState::Ready,
        RendererState::ShuttingDown,
        RendererState::Stopped,
    ];
    const FRAME_STATES: [FrameState; 7] = [
        FrameState::Idle,
        FrameState::Lowered,
        FrameState::Encoded,
        FrameState::Submitted,
        FrameState::Completed,
        FrameState::Failed,
        FrameState::Cancelled,
    ];
    const RESOURCE_STATES: [ResourceState; 3] = [
        ResourceState::Free,
        ResourceState::Encoding,
        ResourceState::InFlight,
    ];
    const OUTCOMES: [FrameOutcome; 4] = [
        FrameOutcome::Pending,
        FrameOutcome::Success,
        FrameOutcome::Failure,
        FrameOutcome::Cancelled,
    ];
    const ACTIONS: [LifecycleAction; 9] = [
        LifecycleAction::BeginFrame,
        LifecycleAction::Encode,
        LifecycleAction::Submit,
        LifecycleAction::Complete,
        LifecycleAction::Fail,
        LifecycleAction::FailBeforeSubmit,
        LifecycleAction::CancelBeforeSubmit,
        LifecycleAction::BeginShutdown,
        LifecycleAction::StopAfterDrain,
    ];

    fn expected_invariants(state: FrameLifecycle) -> bool {
        let frame_is_consistent = match state.frame {
            FrameState::Idle => matches!(
                (
                    state.resource,
                    state.outcome,
                    state.submit_count,
                    state.release_count
                ),
                (ResourceState::Free, FrameOutcome::Pending, 0, 0)
            ),
            FrameState::Lowered | FrameState::Encoded => matches!(
                (
                    state.resource,
                    state.outcome,
                    state.submit_count,
                    state.release_count
                ),
                (ResourceState::Encoding, FrameOutcome::Pending, 0, 0)
            ),
            FrameState::Submitted => matches!(
                (
                    state.resource,
                    state.outcome,
                    state.submit_count,
                    state.release_count
                ),
                (ResourceState::InFlight, FrameOutcome::Pending, 1, 0)
            ),
            FrameState::Completed => matches!(
                (
                    state.resource,
                    state.outcome,
                    state.submit_count,
                    state.release_count
                ),
                (ResourceState::Free, FrameOutcome::Success, 1, 1)
            ),
            FrameState::Failed => {
                matches!(
                    (state.resource, state.outcome, state.release_count),
                    (ResourceState::Free, FrameOutcome::Failure, 1)
                ) && state.submit_count <= 1
            }
            FrameState::Cancelled => matches!(
                (
                    state.resource,
                    state.outcome,
                    state.submit_count,
                    state.release_count
                ),
                (ResourceState::Free, FrameOutcome::Cancelled, 0, 1)
            ),
        };
        let renderer_is_consistent = match state.renderer {
            RendererState::Ready => true,
            RendererState::ShuttingDown => matches!(
                state.frame,
                FrameState::Idle
                    | FrameState::Submitted
                    | FrameState::Completed
                    | FrameState::Failed
                    | FrameState::Cancelled
            ),
            RendererState::Stopped => matches!(
                state.frame,
                FrameState::Idle
                    | FrameState::Completed
                    | FrameState::Failed
                    | FrameState::Cancelled
            ),
        };
        frame_is_consistent && renderer_is_consistent
    }

    #[allow(clippy::too_many_lines)]
    fn expected_transition(
        mut state: FrameLifecycle,
        action: LifecycleAction,
    ) -> Option<FrameLifecycle> {
        let enabled = match action {
            LifecycleAction::BeginFrame => matches!(
                (state.renderer, state.frame, state.resource),
                (RendererState::Ready, FrameState::Idle, ResourceState::Free)
            ),
            LifecycleAction::Encode => matches!(
                (state.renderer, state.frame, state.resource),
                (
                    RendererState::Ready,
                    FrameState::Lowered,
                    ResourceState::Encoding
                )
            ),
            LifecycleAction::Submit => matches!(
                (
                    state.renderer,
                    state.frame,
                    state.resource,
                    state.submit_count
                ),
                (
                    RendererState::Ready,
                    FrameState::Encoded,
                    ResourceState::Encoding,
                    0
                )
            ),
            LifecycleAction::Complete | LifecycleAction::Fail => matches!(
                (state.renderer, state.frame, state.resource),
                (
                    RendererState::Ready | RendererState::ShuttingDown,
                    FrameState::Submitted,
                    ResourceState::InFlight
                )
            ),
            LifecycleAction::FailBeforeSubmit => matches!(
                (state.renderer, state.frame, state.resource),
                (
                    RendererState::Ready,
                    FrameState::Lowered | FrameState::Encoded,
                    ResourceState::Encoding
                )
            ),
            LifecycleAction::CancelBeforeSubmit => matches!(
                (state.renderer, state.frame, state.resource),
                (
                    RendererState::Ready,
                    FrameState::Lowered | FrameState::Encoded,
                    ResourceState::Encoding
                )
            ),
            LifecycleAction::BeginShutdown => matches!(
                (state.renderer, state.frame),
                (
                    RendererState::Ready,
                    FrameState::Idle
                        | FrameState::Submitted
                        | FrameState::Completed
                        | FrameState::Failed
                        | FrameState::Cancelled
                )
            ),
            LifecycleAction::StopAfterDrain => matches!(
                (state.renderer, state.frame, state.resource),
                (
                    RendererState::ShuttingDown,
                    FrameState::Idle
                        | FrameState::Completed
                        | FrameState::Failed
                        | FrameState::Cancelled,
                    ResourceState::Free
                )
            ),
        };
        if !enabled {
            return None;
        }

        match action {
            LifecycleAction::BeginFrame => {
                state.frame = FrameState::Lowered;
                state.resource = ResourceState::Encoding;
            }
            LifecycleAction::Encode => state.frame = FrameState::Encoded,
            LifecycleAction::Submit => {
                state.frame = FrameState::Submitted;
                state.resource = ResourceState::InFlight;
                state.submit_count = 1;
            }
            LifecycleAction::Complete => {
                state.frame = FrameState::Completed;
                state.resource = ResourceState::Free;
                state.release_count = 1;
                state.outcome = FrameOutcome::Success;
            }
            LifecycleAction::Fail | LifecycleAction::FailBeforeSubmit => {
                state.frame = FrameState::Failed;
                state.resource = ResourceState::Free;
                state.release_count = 1;
                state.outcome = FrameOutcome::Failure;
            }
            LifecycleAction::CancelBeforeSubmit => {
                state.frame = FrameState::Cancelled;
                state.resource = ResourceState::Free;
                state.release_count = 1;
                state.outcome = FrameOutcome::Cancelled;
            }
            LifecycleAction::BeginShutdown => state.renderer = RendererState::ShuttingDown,
            LifecycleAction::StopAfterDrain => state.renderer = RendererState::Stopped,
        }
        Some(state)
    }

    fn apply_all(lifecycle: &mut FrameLifecycle, actions: &[LifecycleAction]) {
        for action in actions {
            assert_eq!(lifecycle.apply(*action), Ok(()));
            assert!(lifecycle.invariants_hold());
        }
    }

    #[test]
    fn lifecycle_action_sequence_companion() {
        let mut success = FrameLifecycle::default();
        assert_eq!(success.release_count(), 0);
        apply_all(
            &mut success,
            &[
                LifecycleAction::BeginFrame,
                LifecycleAction::Encode,
                LifecycleAction::Submit,
                LifecycleAction::Complete,
                LifecycleAction::BeginShutdown,
                LifecycleAction::StopAfterDrain,
            ],
        );
        assert_eq!(success.renderer(), RendererState::Stopped);
        assert_eq!(success.frame(), FrameState::Completed);
        assert_eq!(success.resource(), ResourceState::Free);
        assert_eq!(success.outcome(), FrameOutcome::Success);
        assert_eq!(success.submit_count(), 1);
        assert_eq!(success.release_count(), 1);

        let mut failure = FrameLifecycle::new();
        apply_all(
            &mut failure,
            &[
                LifecycleAction::BeginFrame,
                LifecycleAction::Encode,
                LifecycleAction::Submit,
                LifecycleAction::BeginShutdown,
                LifecycleAction::Fail,
                LifecycleAction::StopAfterDrain,
            ],
        );
        assert_eq!(failure.frame(), FrameState::Failed);
        assert_eq!(failure.outcome(), FrameOutcome::Failure);

        let mut cancelled = FrameLifecycle::new();
        apply_all(
            &mut cancelled,
            &[
                LifecycleAction::BeginFrame,
                LifecycleAction::CancelBeforeSubmit,
                LifecycleAction::BeginShutdown,
                LifecycleAction::StopAfterDrain,
            ],
        );
        assert_eq!(cancelled.frame(), FrameState::Cancelled);
        assert_eq!(cancelled.outcome(), FrameOutcome::Cancelled);
        assert_eq!(cancelled.submit_count(), 0);
    }

    #[test]
    fn deliberately_inconsistent_state_fails_invariants() {
        let broken = FrameLifecycle {
            renderer: RendererState::Ready,
            frame: FrameState::Submitted,
            resource: ResourceState::Free,
            outcome: FrameOutcome::Pending,
            submit_count: 0,
            release_count: 0,
        };
        assert!(!broken.invariants_hold());
    }

    #[test]
    fn exhaustive_state_oracle_checks_invariants_and_actions() {
        for renderer in RENDERER_STATES {
            for frame in FRAME_STATES {
                for resource in RESOURCE_STATES {
                    for outcome in OUTCOMES {
                        for submit_count in 0..=2 {
                            for release_count in 0..=2 {
                                let state = FrameLifecycle {
                                    renderer,
                                    frame,
                                    resource,
                                    outcome,
                                    submit_count,
                                    release_count,
                                };
                                let expected_valid = expected_invariants(state);
                                assert_eq!(state.invariants_hold(), expected_valid, "{state:?}");
                                for action in ACTIONS {
                                    let expected = expected_valid
                                        .then(|| expected_transition(state, action))
                                        .flatten();
                                    let mut actual = state;
                                    let result = actual.apply(action);
                                    assert_eq!(
                                        result.is_ok(),
                                        expected.is_some(),
                                        "{state:?} {action:?}"
                                    );
                                    assert_eq!(
                                        actual,
                                        expected.unwrap_or(state),
                                        "{state:?} {action:?}"
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn invalid_actions_are_atomic_and_descriptive() -> Result<(), Box<dyn std::error::Error>> {
        let actions = [
            LifecycleAction::Encode,
            LifecycleAction::Submit,
            LifecycleAction::Complete,
            LifecycleAction::Fail,
            LifecycleAction::CancelBeforeSubmit,
            LifecycleAction::StopAfterDrain,
        ];
        for action in actions {
            let mut lifecycle = FrameLifecycle::new();
            let before = lifecycle;
            let error = lifecycle.apply(action);
            assert_eq!(lifecycle, before);
            let error = error.err().ok_or("action must be rejected")?;
            assert_eq!(error.action(), action);
            assert_eq!(error.renderer(), RendererState::Ready);
            assert_eq!(error.frame(), FrameState::Idle);
            assert_eq!(error.resource(), ResourceState::Free);
            assert!(!error.to_string().is_empty());
        }

        let mut shutdown = FrameLifecycle::new();
        shutdown.apply(LifecycleAction::BeginShutdown)?;
        let before = shutdown;
        assert!(shutdown.apply(LifecycleAction::BeginFrame).is_err());
        assert_eq!(shutdown, before);

        assert!(shutdown.apply(LifecycleAction::Complete).is_err());
        assert!(shutdown.apply(LifecycleAction::Fail).is_err());
        shutdown.apply(LifecycleAction::StopAfterDrain)?;
        assert!(shutdown.apply(LifecycleAction::Complete).is_err());
        assert!(shutdown.apply(LifecycleAction::Fail).is_err());

        let mut lowering = FrameLifecycle::new();
        lowering.apply(LifecycleAction::BeginFrame)?;
        assert!(lowering.apply(LifecycleAction::BeginShutdown).is_err());

        let mut encoded = lowering;
        encoded.apply(LifecycleAction::Encode)?;
        assert!(encoded.apply(LifecycleAction::BeginShutdown).is_err());

        let mut completed_after_shutdown = FrameLifecycle::new();
        apply_all(
            &mut completed_after_shutdown,
            &[
                LifecycleAction::BeginFrame,
                LifecycleAction::Encode,
                LifecycleAction::Submit,
                LifecycleAction::BeginShutdown,
                LifecycleAction::Complete,
            ],
        );
        assert_eq!(completed_after_shutdown.outcome(), FrameOutcome::Success);

        let mut failed_while_ready = FrameLifecycle::new();
        apply_all(
            &mut failed_while_ready,
            &[
                LifecycleAction::BeginFrame,
                LifecycleAction::Encode,
                LifecycleAction::Submit,
                LifecycleAction::Fail,
            ],
        );
        assert_eq!(failed_while_ready.outcome(), FrameOutcome::Failure);

        let mut failed_before_submit = FrameLifecycle::new();
        apply_all(
            &mut failed_before_submit,
            &[
                LifecycleAction::BeginFrame,
                LifecycleAction::Encode,
                LifecycleAction::FailBeforeSubmit,
                LifecycleAction::BeginShutdown,
                LifecycleAction::StopAfterDrain,
            ],
        );
        assert_eq!(failed_before_submit.outcome(), FrameOutcome::Failure);
        assert_eq!(failed_before_submit.submit_count(), 0);
        assert_eq!(failed_before_submit.release_count(), 1);
        Ok(())
    }
}
