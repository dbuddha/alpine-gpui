//! Portable, allocation-free lifecycle contracts for Alpine platform owners.
//!
//! This crate contains no native handles. Platform implementations translate
//! operating-system events into [`PresentationAction`] values and enact the
//! explicit directives returned by [`PresentationState::apply`].

#![no_std]

use core::{error::Error, fmt};

/// Application ownership state for one native surface.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ApplicationState {
    /// New invalidations and frame attempts are accepted.
    Running,
    /// New work is rejected while committed work drains.
    Stopping,
    /// Pacing is invalid and no frame resource remains owned.
    Stopped,
}

/// Desired lifecycle state of one layer-bound display link.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum DisplayLinkState {
    /// The link exists but produces no callbacks.
    Paused,
    /// The link may produce callbacks for pending dirty work.
    Running,
    /// The link was invalidated and cannot be resumed.
    Invalid,
}

/// One frame attempt's portable phase.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum PresentationPhase {
    /// No attempt owns prepared or native frame work.
    Idle,
    /// An immutable scene revision and surface epoch were captured.
    Prepared,
    /// A display callback transferred one drawable to the attempt.
    Encoding,
    /// One command buffer was committed.
    Submitted,
    /// The callback drawable's direct presentation method was called.
    PresentCalled,
}

/// Exclusive resource ownership represented by the portable state.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum PresentationResource {
    /// No callback drawable or committed frame is owned.
    Free,
    /// One callback drawable is exclusively owned before commit.
    Drawable,
    /// One committed frame remains in flight.
    InFlight,
}

/// Terminal classification for the most recent attempt.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum PresentationOutcome {
    /// No terminal result is currently recorded.
    None,
    /// The attempt presented the current revision and surface epoch.
    Presented,
    /// The attempt terminated but was no longer current or was cancelled.
    Superseded,
    /// The attempt terminated with a classified failure.
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum CommitEligibility {
    NotCommitted,
    Eligible,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PostconditionMode {
    Check,
    #[cfg(test)]
    InjectInvalidLink,
}

/// Monotonic identity of the newest requested immutable scene.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct PresentationRevision(u64);

impl PresentationRevision {
    /// The initial revision before any invalidation.
    pub const INITIAL: Self = Self(0);

    /// Returns the persisted integer identity.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    const fn next(self) -> Option<Self> {
        match self.0.checked_add(1) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }
}

/// Monotonic identity of native size, scale, and display configuration.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct SurfaceEpoch(u64);

impl SurfaceEpoch {
    /// The initial surface epoch.
    pub const INITIAL: Self = Self(0);

    /// Returns the persisted integer identity.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    const fn next(self) -> Option<Self> {
        match self.0.checked_add(1) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }
}

/// Opaque identity correlating every event in one frame attempt.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct FrameToken {
    attempt: u64,
    revision: PresentationRevision,
    epoch: SurfaceEpoch,
}

impl FrameToken {
    /// Returns the monotonic attempt identity.
    #[must_use]
    pub const fn attempt(self) -> u64 {
        self.attempt
    }

    /// Returns the immutable scene revision captured by preparation.
    #[must_use]
    pub const fn revision(self) -> PresentationRevision {
        self.revision
    }

    /// Returns the surface epoch captured by preparation.
    #[must_use]
    pub const fn epoch(self) -> SurfaceEpoch {
        self.epoch
    }
}

/// Directive that the native owner must enact on its display link.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DisplayLinkDirective {
    /// No display-link mutation is required.
    None,
    /// Resume the existing paused link.
    Resume,
    /// Pause the existing running link.
    Pause,
    /// Permanently invalidate the link before native teardown.
    Invalidate,
}

/// Terminal portable evidence for one attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AttemptEvidence {
    requested_revision: PresentationRevision,
    frame_revision: PresentationRevision,
    frame_epoch: SurfaceEpoch,
    outcome: PresentationOutcome,
    submission_count: u8,
    present_call_count: u8,
    eligible_at_commit: bool,
}

impl AttemptEvidence {
    /// Returns the newest requested revision when the attempt terminated.
    #[must_use]
    pub const fn requested_revision(self) -> PresentationRevision {
        self.requested_revision
    }

    /// Returns the revision captured by the attempt.
    #[must_use]
    pub const fn frame_revision(self) -> PresentationRevision {
        self.frame_revision
    }

    /// Returns the surface epoch captured by the attempt.
    #[must_use]
    pub const fn frame_epoch(self) -> SurfaceEpoch {
        self.frame_epoch
    }

    /// Returns the terminal classification.
    #[must_use]
    pub const fn outcome(self) -> PresentationOutcome {
        self.outcome
    }

    /// Returns the number of command commits for the attempt.
    #[must_use]
    pub const fn submission_count(self) -> u8 {
        self.submission_count
    }

    /// Returns the number of direct presentation calls for the attempt.
    #[must_use]
    pub const fn present_call_count(self) -> u8 {
        self.present_call_count
    }

    /// Returns whether the attempt was current and eligible at commit.
    #[must_use]
    pub const fn eligible_at_commit(self) -> bool {
        self.eligible_at_commit
    }
}

/// Observable event produced by one successful transition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PresentationEvent {
    /// An idempotent visibility or size update changed nothing.
    Unchanged,
    /// A new scene revision became dirty.
    Invalidated(PresentationRevision),
    /// Native size, scale, or display identity advanced.
    SurfaceAdvanced(SurfaceEpoch),
    /// Visibility changed.
    VisibilityChanged(bool),
    /// Nonzero physical size eligibility changed.
    SizeEligibilityChanged(bool),
    /// Pacing entered the running state.
    PacingResumed,
    /// A frame token captured the newest revision and epoch.
    Prepared(FrameToken),
    /// One callback drawable entered exclusive encoding ownership.
    UpdateBegan(FrameToken),
    /// Stale precommit work was released.
    StaleDiscarded(AttemptEvidence),
    /// One command buffer was committed.
    Submitted(FrameToken),
    /// The callback drawable received one direct presentation call.
    PresentCalled(FrameToken),
    /// An attempt reached one terminal result.
    Terminal(AttemptEvidence),
    /// Shutdown began and committed work must drain.
    ShutdownDraining,
    /// Shutdown completed without committed work to drain.
    Stopped,
}

/// Native side effects and evidence returned by a successful transition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PresentationTransition {
    event: PresentationEvent,
    display_link: DisplayLinkDirective,
}

impl PresentationTransition {
    /// Returns the event accepted by the portable state machine.
    #[must_use]
    pub const fn event(self) -> PresentationEvent {
        self.event
    }

    /// Returns the display-link operation required at the native boundary.
    #[must_use]
    pub const fn display_link(self) -> DisplayLinkDirective {
        self.display_link
    }
}

/// Action applied to one portable presentation owner.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PresentationAction {
    /// Publish a newer immutable scene revision.
    Invalidate,
    /// Advance the native surface epoch and request replacement work.
    AdvanceSurfaceEpoch,
    /// Set whether the native surface is visible and unobscured.
    SetVisible(bool),
    /// Set whether the physical drawable extent is nonzero.
    SetSized(bool),
    /// Acknowledge that the native owner resumed its display link.
    Resume,
    /// Capture the newest dirty revision and epoch.
    Prepare,
    /// Transfer one callback drawable to the prepared attempt.
    BeginUpdate(FrameToken),
    /// Release prepared or encoding work after it becomes stale.
    DiscardStale(FrameToken),
    /// Commit one command buffer for current work.
    Submit(FrameToken),
    /// Call the callback drawable's direct presentation method once.
    CallPresent(FrameToken),
    /// Correlate presentation completion and release in-flight ownership.
    CompletePresentation(FrameToken),
    /// Record active encoding, command, or presentation failure.
    FailActive(FrameToken),
    /// Invalidate pacing and stop or begin draining.
    BeginShutdown,
    /// Finish teardown after committed work drains.
    StopAfterDrain,
}

/// Payload-free action identity used by structured errors.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PresentationActionKind {
    /// [`PresentationAction::Invalidate`].
    Invalidate,
    /// [`PresentationAction::AdvanceSurfaceEpoch`].
    AdvanceSurfaceEpoch,
    /// [`PresentationAction::SetVisible`].
    SetVisible,
    /// [`PresentationAction::SetSized`].
    SetSized,
    /// [`PresentationAction::Resume`].
    Resume,
    /// [`PresentationAction::Prepare`].
    Prepare,
    /// [`PresentationAction::BeginUpdate`].
    BeginUpdate,
    /// [`PresentationAction::DiscardStale`].
    DiscardStale,
    /// [`PresentationAction::Submit`].
    Submit,
    /// [`PresentationAction::CallPresent`].
    CallPresent,
    /// [`PresentationAction::CompletePresentation`].
    CompletePresentation,
    /// [`PresentationAction::FailActive`].
    FailActive,
    /// [`PresentationAction::BeginShutdown`].
    BeginShutdown,
    /// [`PresentationAction::StopAfterDrain`].
    StopAfterDrain,
}

impl PresentationAction {
    const fn kind(self) -> PresentationActionKind {
        match self {
            Self::Invalidate => PresentationActionKind::Invalidate,
            Self::AdvanceSurfaceEpoch => PresentationActionKind::AdvanceSurfaceEpoch,
            Self::SetVisible(_) => PresentationActionKind::SetVisible,
            Self::SetSized(_) => PresentationActionKind::SetSized,
            Self::Resume => PresentationActionKind::Resume,
            Self::Prepare => PresentationActionKind::Prepare,
            Self::BeginUpdate(_) => PresentationActionKind::BeginUpdate,
            Self::DiscardStale(_) => PresentationActionKind::DiscardStale,
            Self::Submit(_) => PresentationActionKind::Submit,
            Self::CallPresent(_) => PresentationActionKind::CallPresent,
            Self::CompletePresentation(_) => PresentationActionKind::CompletePresentation,
            Self::FailActive(_) => PresentationActionKind::FailActive,
            Self::BeginShutdown => PresentationActionKind::BeginShutdown,
            Self::StopAfterDrain => PresentationActionKind::StopAfterDrain,
        }
    }
}

/// Stable reason that a transition was rejected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransitionErrorKind {
    /// The action is disabled in the current lifecycle phase.
    ActionDisabled,
    /// The supplied token does not identify the active attempt.
    TokenMismatch,
    /// The requested revision or epoch is no longer current.
    AttemptStale,
    /// A monotonic identity reached `u64::MAX`.
    SequenceExhausted,
    /// The state did not satisfy its binding invariants.
    InvariantViolation,
}

/// Structured transition failure that exposes no native objects.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransitionError {
    kind: TransitionErrorKind,
    action: PresentationActionKind,
    application: ApplicationState,
    phase: PresentationPhase,
}

impl TransitionError {
    /// Returns the stable rejection category.
    #[must_use]
    pub const fn kind(self) -> TransitionErrorKind {
        self.kind
    }

    /// Returns the rejected action identity.
    #[must_use]
    pub const fn action(self) -> PresentationActionKind {
        self.action
    }

    /// Returns the application state observed before rejection.
    #[must_use]
    pub const fn application(self) -> ApplicationState {
        self.application
    }

    /// Returns the presentation phase observed before rejection.
    #[must_use]
    pub const fn phase(self) -> PresentationPhase {
        self.phase
    }
}

impl fmt::Display for TransitionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "presentation action {:?} rejected in {:?}/{:?}: {:?}",
            self.action, self.application, self.phase, self.kind
        )
    }
}

impl Error for TransitionError {}

/// Allocation-free portable state for one native presentation surface.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct PresentationState {
    application: ApplicationState,
    link: DisplayLinkState,
    visible: bool,
    sized: bool,
    dirty: bool,
    requested_revision: PresentationRevision,
    presented_revision: PresentationRevision,
    surface_epoch: SurfaceEpoch,
    phase: PresentationPhase,
    frame_token: Option<FrameToken>,
    resource: PresentationResource,
    submission_count: u8,
    present_call_count: u8,
    commit_eligibility: CommitEligibility,
    outcome: PresentationOutcome,
    next_attempt: u64,
}

impl Default for PresentationState {
    fn default() -> Self {
        Self::new()
    }
}

impl PresentationState {
    /// Creates a running, clean, hidden, zero-sized, and paused surface.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            application: ApplicationState::Running,
            link: DisplayLinkState::Paused,
            visible: false,
            sized: false,
            dirty: false,
            requested_revision: PresentationRevision::INITIAL,
            presented_revision: PresentationRevision::INITIAL,
            surface_epoch: SurfaceEpoch::INITIAL,
            phase: PresentationPhase::Idle,
            frame_token: None,
            resource: PresentationResource::Free,
            submission_count: 0,
            present_call_count: 0,
            commit_eligibility: CommitEligibility::NotCommitted,
            outcome: PresentationOutcome::None,
            next_attempt: 0,
        }
    }

    /// Applies one action atomically and returns required native effects.
    ///
    /// # Errors
    ///
    /// Returns a structured error and restores the exact original state when
    /// the action is disabled, stale, exhausted, or violates an invariant.
    pub fn apply(
        &mut self,
        action: PresentationAction,
    ) -> Result<PresentationTransition, TransitionError> {
        self.apply_checked(action, PostconditionMode::Check)
    }

    fn apply_checked(
        &mut self,
        action: PresentationAction,
        postcondition: PostconditionMode,
    ) -> Result<PresentationTransition, TransitionError> {
        let before = *self;
        let action_kind = action.kind();
        if !before.invariants_hold() {
            return Err(before.error(action_kind, TransitionErrorKind::InvariantViolation));
        }
        let result = self.apply_inner(action);
        #[cfg(test)]
        if matches!(postcondition, PostconditionMode::InjectInvalidLink) && result.is_ok() {
            self.link = DisplayLinkState::Invalid;
        }
        #[cfg(not(test))]
        let _ = postcondition;
        match result {
            Ok(transition) if self.invariants_hold() => Ok(transition),
            Ok(_) => {
                *self = before;
                Err(before.error(action_kind, TransitionErrorKind::InvariantViolation))
            }
            Err(kind) => {
                *self = before;
                Err(before.error(action_kind, kind))
            }
        }
    }

    /// Returns the application lifecycle state.
    #[must_use]
    pub const fn application(self) -> ApplicationState {
        self.application
    }

    /// Returns the desired display-link state.
    #[must_use]
    pub const fn display_link(self) -> DisplayLinkState {
        self.link
    }

    /// Returns whether the surface is visible and unobscured.
    #[must_use]
    pub const fn is_visible(self) -> bool {
        self.visible
    }

    /// Returns whether the physical drawable extent is nonzero.
    #[must_use]
    pub const fn is_sized(self) -> bool {
        self.sized
    }

    /// Returns whether a newer current frame remains required.
    #[must_use]
    pub const fn is_dirty(self) -> bool {
        self.dirty
    }

    /// Returns the newest requested scene revision.
    #[must_use]
    pub const fn requested_revision(self) -> PresentationRevision {
        self.requested_revision
    }

    /// Returns the newest revision qualified as currently presented.
    #[must_use]
    pub const fn presented_revision(self) -> PresentationRevision {
        self.presented_revision
    }

    /// Returns the current surface epoch.
    #[must_use]
    pub const fn surface_epoch(self) -> SurfaceEpoch {
        self.surface_epoch
    }

    /// Returns the active frame phase.
    #[must_use]
    pub const fn phase(self) -> PresentationPhase {
        self.phase
    }

    /// Returns the active frame token, if an attempt exists.
    #[must_use]
    pub const fn active_token(self) -> Option<FrameToken> {
        self.frame_token
    }

    /// Returns the current exclusive frame-resource owner.
    #[must_use]
    pub const fn resource(self) -> PresentationResource {
        self.resource
    }

    /// Returns the latest terminal outcome.
    #[must_use]
    pub const fn outcome(self) -> PresentationOutcome {
        self.outcome
    }

    /// Returns the current attempt's command-commit count.
    #[must_use]
    pub const fn submission_count(self) -> u8 {
        self.submission_count
    }

    /// Returns the current attempt's direct presentation-call count.
    #[must_use]
    pub const fn present_call_count(self) -> u8 {
        self.present_call_count
    }

    /// Returns whether current conditions request display-link resumption.
    #[must_use]
    pub const fn needs_resume(self) -> bool {
        matches!(self.application, ApplicationState::Running)
            && matches!(self.link, DisplayLinkState::Paused)
            && self.visible
            && self.sized
            && self.dirty
    }

    /// Checks every executable invariant mapped from AEP 0064.
    #[must_use]
    pub const fn invariants_hold(self) -> bool {
        let link_owned = match self.application {
            ApplicationState::Running => !matches!(self.link, DisplayLinkState::Invalid),
            ApplicationState::Stopping | ApplicationState::Stopped => {
                matches!(self.link, DisplayLinkState::Invalid)
            }
        };
        let running_link_is_eligible = !matches!(self.link, DisplayLinkState::Running)
            || (self.visible && self.sized && self.dirty);
        let resource_matches_phase = match self.phase {
            PresentationPhase::Idle | PresentationPhase::Prepared => {
                matches!(self.resource, PresentationResource::Free)
            }
            PresentationPhase::Encoding => {
                matches!(self.resource, PresentationResource::Drawable)
            }
            PresentationPhase::Submitted | PresentationPhase::PresentCalled => {
                matches!(self.resource, PresentationResource::InFlight)
            }
        };
        let active_has_token =
            matches!(self.phase, PresentationPhase::Idle) || self.frame_token.is_some();
        let token_is_bounded = match self.frame_token {
            Some(token) => {
                token.attempt > 0
                    && token.attempt <= self.next_attempt
                    && token.revision.0 <= self.requested_revision.0
                    && token.epoch.0 <= self.surface_epoch.0
            }
            None => matches!(self.phase, PresentationPhase::Idle),
        };
        let ordered_counts = self.submission_count <= 1
            && self.present_call_count <= 1
            && self.present_call_count <= self.submission_count;
        let phase_counts_are_ordered = match self.phase {
            PresentationPhase::Prepared | PresentationPhase::Encoding => self.submission_count == 0,
            PresentationPhase::Submitted => {
                self.submission_count == 1 && self.present_call_count == 0
            }
            PresentationPhase::PresentCalled => {
                self.submission_count == 1 && self.present_call_count == 1
            }
            PresentationPhase::Idle => true,
        };
        let committed_is_eligible = !matches!(
            self.phase,
            PresentationPhase::Submitted | PresentationPhase::PresentCalled
        ) || (self.submission_count == 1
            && matches!(self.commit_eligibility, CommitEligibility::Eligible));
        let token_qualifies_presentation = match self.frame_token {
            Some(token) => {
                token.revision.0 == self.requested_revision.0
                    && token.revision.0 == self.presented_revision.0
                    && token.epoch.0 == self.surface_epoch.0
            }
            None => false,
        };
        let current_presentation = !matches!(self.outcome, PresentationOutcome::Presented)
            || (matches!(self.phase, PresentationPhase::Idle)
                && token_qualifies_presentation
                && self.submission_count == 1
                && self.present_call_count == 1
                && matches!(self.commit_eligibility, CommitEligibility::Eligible));
        let clean_idle_is_paused = !(matches!(self.application, ApplicationState::Running)
            && matches!(self.phase, PresentationPhase::Idle)
            && !self.dirty)
            || matches!(self.link, DisplayLinkState::Paused);
        let stopped_is_drained = !matches!(self.application, ApplicationState::Stopped)
            || matches!(self.phase, PresentationPhase::Idle);
        let terminal_is_idle = matches!(self.outcome, PresentationOutcome::None)
            || matches!(self.phase, PresentationPhase::Idle);
        let presented_is_monotonic = self.presented_revision.0 <= self.requested_revision.0;

        link_owned
            && running_link_is_eligible
            && resource_matches_phase
            && active_has_token
            && token_is_bounded
            && ordered_counts
            && phase_counts_are_ordered
            && committed_is_eligible
            && current_presentation
            && clean_idle_is_paused
            && stopped_is_drained
            && terminal_is_idle
            && presented_is_monotonic
    }

    fn apply_inner(
        &mut self,
        action: PresentationAction,
    ) -> Result<PresentationTransition, TransitionErrorKind> {
        match action {
            PresentationAction::Invalidate => self.invalidate(),
            PresentationAction::AdvanceSurfaceEpoch => self.advance_surface_epoch(),
            PresentationAction::SetVisible(visible) => self.set_visible(visible),
            PresentationAction::SetSized(sized) => self.set_sized(sized),
            PresentationAction::Resume => self.resume(),
            PresentationAction::Prepare => self.prepare(),
            PresentationAction::BeginUpdate(token) => self.begin_update(token),
            PresentationAction::DiscardStale(token) => self.discard_stale(token),
            PresentationAction::Submit(token) => self.submit(token),
            PresentationAction::CallPresent(token) => self.call_present(token),
            PresentationAction::CompletePresentation(token) => self.complete_presentation(token),
            PresentationAction::FailActive(token) => self.fail_active(token),
            PresentationAction::BeginShutdown => self.begin_shutdown(),
            PresentationAction::StopAfterDrain => self.stop_after_drain(),
        }
    }

    fn invalidate(&mut self) -> Result<PresentationTransition, TransitionErrorKind> {
        self.require_running()?;
        self.requested_revision = self
            .requested_revision
            .next()
            .ok_or(TransitionErrorKind::SequenceExhausted)?;
        self.dirty = true;
        self.outcome = PresentationOutcome::None;
        Ok(self.transition(
            PresentationEvent::Invalidated(self.requested_revision),
            self.resume_directive(),
        ))
    }

    fn advance_surface_epoch(&mut self) -> Result<PresentationTransition, TransitionErrorKind> {
        self.require_running()?;
        self.surface_epoch = self
            .surface_epoch
            .next()
            .ok_or(TransitionErrorKind::SequenceExhausted)?;
        self.dirty = true;
        self.outcome = PresentationOutcome::None;
        Ok(self.transition(
            PresentationEvent::SurfaceAdvanced(self.surface_epoch),
            self.resume_directive(),
        ))
    }

    fn set_visible(
        &mut self,
        visible: bool,
    ) -> Result<PresentationTransition, TransitionErrorKind> {
        self.require_running()?;
        if self.visible == visible {
            return Ok(self.transition(PresentationEvent::Unchanged, self.resume_directive()));
        }
        self.visible = visible;
        let directive = self.reconcile_eligibility();
        Ok(self.transition(PresentationEvent::VisibilityChanged(visible), directive))
    }

    fn set_sized(&mut self, sized: bool) -> Result<PresentationTransition, TransitionErrorKind> {
        self.require_running()?;
        if self.sized == sized {
            return Ok(self.transition(PresentationEvent::Unchanged, self.resume_directive()));
        }
        self.sized = sized;
        let directive = self.reconcile_eligibility();
        Ok(self.transition(PresentationEvent::SizeEligibilityChanged(sized), directive))
    }

    fn resume(&mut self) -> Result<PresentationTransition, TransitionErrorKind> {
        if !self.needs_resume() {
            return Err(TransitionErrorKind::ActionDisabled);
        }
        self.link = DisplayLinkState::Running;
        Ok(self.transition(
            PresentationEvent::PacingResumed,
            DisplayLinkDirective::Resume,
        ))
    }

    fn prepare(&mut self) -> Result<PresentationTransition, TransitionErrorKind> {
        if !matches!(
            (self.link, self.phase),
            (DisplayLinkState::Running, PresentationPhase::Idle)
        ) {
            return Err(TransitionErrorKind::ActionDisabled);
        }
        self.next_attempt = self
            .next_attempt
            .checked_add(1)
            .ok_or(TransitionErrorKind::SequenceExhausted)?;
        let token = FrameToken {
            attempt: self.next_attempt,
            revision: self.requested_revision,
            epoch: self.surface_epoch,
        };
        self.frame_token = Some(token);
        self.phase = PresentationPhase::Prepared;
        self.resource = PresentationResource::Free;
        self.submission_count = 0;
        self.present_call_count = 0;
        self.commit_eligibility = CommitEligibility::NotCommitted;
        self.outcome = PresentationOutcome::None;
        Ok(self.transition(
            PresentationEvent::Prepared(token),
            DisplayLinkDirective::None,
        ))
    }

    fn begin_update(
        &mut self,
        token: FrameToken,
    ) -> Result<PresentationTransition, TransitionErrorKind> {
        self.require_token(token)?;
        if !matches!(self.application, ApplicationState::Running)
            || !matches!(self.phase, PresentationPhase::Prepared)
            || !self.attempt_is_current()
        {
            return Err(if self.attempt_is_current() {
                TransitionErrorKind::ActionDisabled
            } else {
                TransitionErrorKind::AttemptStale
            });
        }
        self.phase = PresentationPhase::Encoding;
        self.resource = PresentationResource::Drawable;
        Ok(self.transition(
            PresentationEvent::UpdateBegan(token),
            DisplayLinkDirective::None,
        ))
    }

    fn discard_stale(
        &mut self,
        token: FrameToken,
    ) -> Result<PresentationTransition, TransitionErrorKind> {
        self.require_token(token)?;
        if !matches!(
            self.phase,
            PresentationPhase::Prepared | PresentationPhase::Encoding
        ) {
            return Err(TransitionErrorKind::ActionDisabled);
        }
        if self.attempt_is_current() {
            return Err(TransitionErrorKind::ActionDisabled);
        }
        self.phase = PresentationPhase::Idle;
        self.resource = PresentationResource::Free;
        self.outcome = PresentationOutcome::Superseded;
        let directive = self.reconcile_eligibility();
        Ok(self.transition(
            PresentationEvent::StaleDiscarded(self.attempt_evidence()),
            directive,
        ))
    }

    fn submit(&mut self, token: FrameToken) -> Result<PresentationTransition, TransitionErrorKind> {
        self.require_token(token)?;
        if !matches!(self.phase, PresentationPhase::Encoding) || self.submission_count != 0 {
            return Err(TransitionErrorKind::ActionDisabled);
        }
        if !self.attempt_is_current() {
            return Err(TransitionErrorKind::AttemptStale);
        }
        self.phase = PresentationPhase::Submitted;
        self.resource = PresentationResource::InFlight;
        self.submission_count = 1;
        self.commit_eligibility = CommitEligibility::Eligible;
        Ok(self.transition(
            PresentationEvent::Submitted(token),
            DisplayLinkDirective::None,
        ))
    }

    fn call_present(
        &mut self,
        token: FrameToken,
    ) -> Result<PresentationTransition, TransitionErrorKind> {
        self.require_token(token)?;
        if !matches!(self.phase, PresentationPhase::Submitted) {
            return Err(TransitionErrorKind::ActionDisabled);
        }
        self.phase = PresentationPhase::PresentCalled;
        self.present_call_count = 1;
        Ok(self.transition(
            PresentationEvent::PresentCalled(token),
            DisplayLinkDirective::None,
        ))
    }

    fn complete_presentation(
        &mut self,
        token: FrameToken,
    ) -> Result<PresentationTransition, TransitionErrorKind> {
        self.require_token(token)?;
        if !matches!(self.phase, PresentationPhase::PresentCalled) {
            return Err(TransitionErrorKind::ActionDisabled);
        }
        self.phase = PresentationPhase::Idle;
        self.resource = PresentationResource::Free;
        if self.attempt_is_current() {
            self.outcome = PresentationOutcome::Presented;
            self.presented_revision = token.revision;
            self.dirty = false;
        } else {
            self.outcome = PresentationOutcome::Superseded;
        }
        let directive = self.reconcile_eligibility();
        Ok(self.transition(
            PresentationEvent::Terminal(self.attempt_evidence()),
            directive,
        ))
    }

    fn fail_active(
        &mut self,
        token: FrameToken,
    ) -> Result<PresentationTransition, TransitionErrorKind> {
        self.require_token(token)?;
        if !matches!(
            self.phase,
            PresentationPhase::Encoding
                | PresentationPhase::Submitted
                | PresentationPhase::PresentCalled
        ) {
            return Err(TransitionErrorKind::ActionDisabled);
        }
        self.phase = PresentationPhase::Idle;
        self.resource = PresentationResource::Free;
        self.outcome = PresentationOutcome::Failed;
        let directive = if matches!(self.application, ApplicationState::Running) {
            let was_running = matches!(self.link, DisplayLinkState::Running);
            self.link = DisplayLinkState::Paused;
            if was_running {
                DisplayLinkDirective::Pause
            } else {
                DisplayLinkDirective::None
            }
        } else {
            DisplayLinkDirective::None
        };
        Ok(self.transition(
            PresentationEvent::Terminal(self.attempt_evidence()),
            directive,
        ))
    }

    fn begin_shutdown(&mut self) -> Result<PresentationTransition, TransitionErrorKind> {
        self.require_running()?;
        self.link = DisplayLinkState::Invalid;
        self.dirty = false;
        let event = if matches!(
            self.phase,
            PresentationPhase::Submitted | PresentationPhase::PresentCalled
        ) {
            self.application = ApplicationState::Stopping;
            PresentationEvent::ShutdownDraining
        } else {
            if matches!(
                self.phase,
                PresentationPhase::Prepared | PresentationPhase::Encoding
            ) {
                self.outcome = PresentationOutcome::Superseded;
            }
            self.application = ApplicationState::Stopped;
            self.phase = PresentationPhase::Idle;
            self.resource = PresentationResource::Free;
            PresentationEvent::Stopped
        };
        Ok(self.transition(event, DisplayLinkDirective::Invalidate))
    }

    fn stop_after_drain(&mut self) -> Result<PresentationTransition, TransitionErrorKind> {
        if !matches!(
            (self.application, self.phase),
            (ApplicationState::Stopping, PresentationPhase::Idle)
        ) {
            return Err(TransitionErrorKind::ActionDisabled);
        }
        self.application = ApplicationState::Stopped;
        Ok(self.transition(PresentationEvent::Stopped, DisplayLinkDirective::None))
    }

    const fn require_running(&self) -> Result<(), TransitionErrorKind> {
        if matches!(self.application, ApplicationState::Running) {
            Ok(())
        } else {
            Err(TransitionErrorKind::ActionDisabled)
        }
    }

    fn require_token(&self, token: FrameToken) -> Result<(), TransitionErrorKind> {
        if self.frame_token == Some(token) {
            Ok(())
        } else {
            Err(TransitionErrorKind::TokenMismatch)
        }
    }

    fn attempt_is_current(&self) -> bool {
        matches!(self.application, ApplicationState::Running)
            && self.visible
            && self.sized
            && self.dirty
            && self.frame_token.is_some_and(|token| {
                token.revision == self.requested_revision && token.epoch == self.surface_epoch
            })
    }

    const fn resume_directive(self) -> DisplayLinkDirective {
        if self.needs_resume() {
            DisplayLinkDirective::Resume
        } else {
            DisplayLinkDirective::None
        }
    }

    fn reconcile_eligibility(&mut self) -> DisplayLinkDirective {
        if matches!(self.application, ApplicationState::Running)
            && self.visible
            && self.sized
            && self.dirty
        {
            self.resume_directive()
        } else if matches!(self.link, DisplayLinkState::Running) {
            self.link = DisplayLinkState::Paused;
            DisplayLinkDirective::Pause
        } else {
            DisplayLinkDirective::None
        }
    }

    fn attempt_evidence(&self) -> AttemptEvidence {
        let token = self.frame_token.unwrap_or(FrameToken {
            attempt: 0,
            revision: PresentationRevision::INITIAL,
            epoch: SurfaceEpoch::INITIAL,
        });
        AttemptEvidence {
            requested_revision: self.requested_revision,
            frame_revision: token.revision,
            frame_epoch: token.epoch,
            outcome: self.outcome,
            submission_count: self.submission_count,
            present_call_count: self.present_call_count,
            eligible_at_commit: matches!(self.commit_eligibility, CommitEligibility::Eligible),
        }
    }

    const fn transition(
        self,
        event: PresentationEvent,
        display_link: DisplayLinkDirective,
    ) -> PresentationTransition {
        debug_assert!(self.invariants_hold());
        PresentationTransition {
            event,
            display_link,
        }
    }

    const fn error(
        self,
        action: PresentationActionKind,
        kind: TransitionErrorKind,
    ) -> TransitionError {
        TransitionError {
            kind,
            action,
            application: self.application,
            phase: self.phase,
        }
    }
}

#[cfg(kani)]
mod proofs;

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::string::ToString;
    use std::vec::Vec;

    use super::{
        ApplicationState, DisplayLinkDirective, FrameToken, PresentationAction, PresentationEvent,
        PresentationOutcome, PresentationPhase, PresentationResource, PresentationState,
        TransitionErrorKind,
    };

    fn apply(
        state: &mut PresentationState,
        action: PresentationAction,
    ) -> Result<PresentationEvent, &'static str> {
        state
            .apply(action)
            .map(super::PresentationTransition::event)
            .map_err(|_| "test transition failed")
    }

    fn make_eligible(state: &mut PresentationState) -> Result<(), &'static str> {
        apply(state, PresentationAction::SetVisible(true))?;
        apply(state, PresentationAction::SetSized(true))?;
        let transition = state.apply(PresentationAction::Invalidate);
        assert_eq!(
            transition.map(super::PresentationTransition::display_link),
            Ok(DisplayLinkDirective::Resume)
        );
        apply(state, PresentationAction::Resume)?;
        Ok(())
    }

    fn prepare(state: &mut PresentationState) -> Result<FrameToken, &'static str> {
        let event = apply(state, PresentationAction::Prepare)?;
        let PresentationEvent::Prepared(token) = event else {
            return Err("expected prepared event");
        };
        Ok(token)
    }

    #[test]
    fn model_trace_replay_qualifies_only_a_current_presented_frame() -> Result<(), &'static str> {
        let mut state = PresentationState::new();
        make_eligible(&mut state)?;
        let token = prepare(&mut state)?;
        apply(&mut state, PresentationAction::BeginUpdate(token))?;
        apply(&mut state, PresentationAction::Submit(token))?;
        apply(&mut state, PresentationAction::CallPresent(token))?;
        let event = apply(&mut state, PresentationAction::CompletePresentation(token))?;

        let PresentationEvent::Terminal(evidence) = event else {
            return Err("expected terminal evidence");
        };
        assert_eq!(evidence.outcome(), PresentationOutcome::Presented);
        assert_eq!(evidence.submission_count(), 1);
        assert_eq!(evidence.present_call_count(), 1);
        assert!(evidence.eligible_at_commit());
        assert_eq!(state.presented_revision(), state.requested_revision());
        assert_eq!(state.submission_count(), 1);
        assert_eq!(state.present_call_count(), 1);
        assert!(!state.is_dirty());
        assert!(state.invariants_hold());
        Ok(())
    }

    #[test]
    fn coalesces_invalidations_and_discards_precommit_stale_work() -> Result<(), &'static str> {
        let mut state = PresentationState::new();
        make_eligible(&mut state)?;
        apply(&mut state, PresentationAction::Invalidate)?;
        apply(&mut state, PresentationAction::Invalidate)?;
        let newest = state.requested_revision();
        let token = prepare(&mut state)?;
        assert_eq!(token.revision(), newest);
        apply(&mut state, PresentationAction::BeginUpdate(token))?;
        apply(&mut state, PresentationAction::AdvanceSurfaceEpoch)?;
        assert_eq!(state.surface_epoch().get(), 1);

        let before = state;
        let error = state.apply(PresentationAction::Submit(token));
        assert_eq!(
            error.map_err(super::TransitionError::kind),
            Err(TransitionErrorKind::AttemptStale)
        );
        assert_eq!(state, before);

        let event = apply(&mut state, PresentationAction::DiscardStale(token))?;
        let PresentationEvent::StaleDiscarded(evidence) = event else {
            return Err("expected stale evidence");
        };
        assert_eq!(evidence.outcome(), PresentationOutcome::Superseded);
        assert_eq!(state.resource(), PresentationResource::Free);
        assert!(state.is_dirty());
        Ok(())
    }

    #[test]
    fn deliberately_faulty_stale_qualification_is_detected() -> Result<(), &'static str> {
        let mut state = PresentationState::new();
        make_eligible(&mut state)?;
        let token = prepare(&mut state)?;
        apply(&mut state, PresentationAction::BeginUpdate(token))?;
        apply(&mut state, PresentationAction::Submit(token))?;
        apply(&mut state, PresentationAction::CallPresent(token))?;
        apply(&mut state, PresentationAction::Invalidate)?;
        let event = apply(&mut state, PresentationAction::CompletePresentation(token))?;

        let PresentationEvent::Terminal(evidence) = event else {
            return Err("expected terminal evidence");
        };
        assert_eq!(evidence.outcome(), PresentationOutcome::Superseded);
        assert_ne!(state.presented_revision(), state.requested_revision());
        assert!(state.is_dirty());

        let mut faulty = state;
        faulty.outcome = PresentationOutcome::Presented;
        assert!(!faulty.invariants_hold());
        Ok(())
    }

    #[test]
    fn invalid_actions_are_atomic_and_descriptive() -> Result<(), &'static str> {
        let mut state = PresentationState::new();
        let before = state;
        let error = state.apply(PresentationAction::Prepare);
        assert_eq!(
            error.map_err(super::TransitionError::kind),
            Err(TransitionErrorKind::ActionDisabled)
        );
        assert_eq!(state, before);

        make_eligible(&mut state)?;
        let token = prepare(&mut state)?;
        let wrong = FrameToken {
            attempt: token.attempt() + 1,
            revision: token.revision(),
            epoch: token.epoch(),
        };
        let before = state;
        let error = state.apply(PresentationAction::BeginUpdate(wrong));
        assert_eq!(
            error.map_err(super::TransitionError::kind),
            Err(TransitionErrorKind::TokenMismatch)
        );
        assert_eq!(state, before);
        let message = error.err().map(|value| value.to_string());
        assert!(message.is_some_and(|value| value.contains("TokenMismatch")));
        Ok(())
    }

    #[test]
    fn accessors_and_sequence_exhaustion_are_stable() -> Result<(), &'static str> {
        let state = PresentationState::default();
        assert!(core::mem::size_of::<PresentationState>() <= 128);
        assert!(core::mem::size_of::<FrameToken>() <= 24);
        assert_eq!(state.application(), ApplicationState::Running);
        assert_eq!(state.display_link(), super::DisplayLinkState::Paused);
        assert!(!state.is_visible());
        assert!(!state.is_sized());
        assert!(!state.is_dirty());
        assert_eq!(state.requested_revision().get(), 0);
        assert_eq!(state.presented_revision().get(), 0);
        assert_eq!(state.surface_epoch().get(), 0);
        assert_eq!(state.phase(), PresentationPhase::Idle);
        assert_eq!(state.active_token(), None);
        assert_eq!(state.resource(), PresentationResource::Free);
        assert_eq!(state.outcome(), PresentationOutcome::None);
        assert_eq!(state.submission_count(), 0);
        assert_eq!(state.present_call_count(), 0);
        assert!(!state.needs_resume());

        let mut active = state;
        make_eligible(&mut active)?;
        assert!(active.is_visible());
        assert!(active.is_sized());
        assert_eq!(active.requested_revision().get(), 1);
        let first = prepare(&mut active)?;
        assert_eq!(first.attempt(), 1);
        apply(&mut active, PresentationAction::BeginUpdate(first))?;
        let event = apply(&mut active, PresentationAction::FailActive(first))?;
        if let PresentationEvent::Terminal(evidence) = event {
            assert_eq!(evidence.submission_count(), 0);
            assert_eq!(evidence.present_call_count(), 0);
            assert!(!evidence.eligible_at_commit());
        } else {
            assert!(matches!(event, PresentationEvent::Terminal(_)));
        }
        assert_eq!(active.submission_count(), 0);
        assert_eq!(active.present_call_count(), 0);
        apply(&mut active, PresentationAction::Resume)?;
        let second = prepare(&mut active)?;
        assert_eq!(second.attempt(), 2);

        let mut exhausted_revision = state;
        exhausted_revision.requested_revision = super::PresentationRevision(u64::MAX);
        let error = exhausted_revision.apply(PresentationAction::Invalidate);
        let Err(error) = error else {
            return Err("revision exhaustion unexpectedly succeeded");
        };
        assert_eq!(error.kind(), TransitionErrorKind::SequenceExhausted);
        assert_eq!(error.action(), super::PresentationActionKind::Invalidate);
        assert_eq!(error.application(), ApplicationState::Running);
        assert_eq!(error.phase(), PresentationPhase::Idle);

        let mut exhausted_epoch = state;
        exhausted_epoch.surface_epoch = super::SurfaceEpoch(u64::MAX);
        let error = exhausted_epoch.apply(PresentationAction::AdvanceSurfaceEpoch);
        assert_eq!(
            error.map_err(super::TransitionError::kind),
            Err(TransitionErrorKind::SequenceExhausted)
        );

        let mut exhausted_attempt = state;
        make_eligible(&mut exhausted_attempt)?;
        exhausted_attempt.next_attempt = u64::MAX;
        let error = exhausted_attempt.apply(PresentationAction::Prepare);
        assert_eq!(
            error.map_err(super::TransitionError::kind),
            Err(TransitionErrorKind::SequenceExhausted)
        );

        let mut corrupt = state;
        corrupt.link = super::DisplayLinkState::Running;
        let error = corrupt.apply(PresentationAction::Invalidate);
        assert_eq!(
            error.map_err(super::TransitionError::kind),
            Err(TransitionErrorKind::InvariantViolation)
        );

        let mut injected = state;
        let before = injected;
        let error = injected.apply_checked(
            PresentationAction::SetVisible(false),
            super::PostconditionMode::InjectInvalidLink,
        );
        assert_eq!(
            error.map_err(super::TransitionError::kind),
            Err(TransitionErrorKind::InvariantViolation)
        );
        assert_eq!(injected, before);

        let mut stopped = state;
        apply(&mut stopped, PresentationAction::BeginShutdown)?;
        let error = stopped.apply(PresentationAction::Invalidate);
        assert_eq!(
            error.map_err(super::TransitionError::kind),
            Err(TransitionErrorKind::ActionDisabled)
        );
        Ok(())
    }

    #[test]
    fn each_lifecycle_guard_rejects_its_wrong_phase() -> Result<(), &'static str> {
        let mut state = PresentationState::new();
        apply(&mut state, PresentationAction::SetVisible(true))?;
        apply(&mut state, PresentationAction::SetSized(true))?;
        apply(&mut state, PresentationAction::Invalidate)?;

        let before = state;
        assert!(state.apply(PresentationAction::Prepare).is_err());
        assert_eq!(state, before);

        apply(&mut state, PresentationAction::Resume)?;
        let token = prepare(&mut state)?;
        let before = state;
        assert!(state.apply(PresentationAction::Prepare).is_err());
        assert_eq!(state, before);
        assert!(state.apply(PresentationAction::CallPresent(token)).is_err());
        assert_eq!(state, before);
        assert!(
            state
                .apply(PresentationAction::CompletePresentation(token))
                .is_err()
        );
        assert_eq!(state, before);
        assert!(state.apply(PresentationAction::FailActive(token)).is_err());
        assert_eq!(state, before);

        apply(&mut state, PresentationAction::BeginUpdate(token))?;
        apply(&mut state, PresentationAction::Submit(token))?;
        apply(&mut state, PresentationAction::CallPresent(token))?;
        let before = state;
        assert!(state.apply(PresentationAction::CallPresent(token)).is_err());
        assert_eq!(state, before);

        let mut not_draining = PresentationState::new();
        assert!(
            not_draining
                .apply(PresentationAction::StopAfterDrain)
                .is_err()
        );
        assert_eq!(not_draining, PresentationState::new());
        Ok(())
    }

    #[test]
    fn every_invariant_clause_rejects_a_discriminating_state() -> Result<(), &'static str> {
        let mut eligible = PresentationState::new();
        make_eligible(&mut eligible)?;

        for corrupt in [
            PresentationState {
                visible: false,
                ..eligible
            },
            PresentationState {
                sized: false,
                ..eligible
            },
            PresentationState {
                dirty: false,
                ..eligible
            },
            PresentationState {
                resource: PresentationResource::Drawable,
                ..eligible
            },
            PresentationState {
                submission_count: 2,
                ..eligible
            },
            PresentationState {
                present_call_count: 2,
                ..eligible
            },
            PresentationState {
                submission_count: 0,
                present_call_count: 1,
                ..eligible
            },
        ] {
            assert!(!corrupt.invariants_hold());
        }

        let token = prepare(&mut eligible)?;
        let prepared = eligible;
        for corrupt in [
            PresentationState {
                frame_token: None,
                ..prepared
            },
            PresentationState {
                frame_token: Some(FrameToken {
                    attempt: 0,
                    ..token
                }),
                ..prepared
            },
            PresentationState {
                frame_token: Some(FrameToken {
                    attempt: prepared.next_attempt + 1,
                    ..token
                }),
                ..prepared
            },
            PresentationState {
                frame_token: Some(FrameToken {
                    revision: super::PresentationRevision(prepared.requested_revision().get() + 1),
                    ..token
                }),
                ..prepared
            },
            PresentationState {
                frame_token: Some(FrameToken {
                    epoch: super::SurfaceEpoch(prepared.surface_epoch().get() + 1),
                    ..token
                }),
                ..prepared
            },
            PresentationState {
                submission_count: 1,
                ..prepared
            },
        ] {
            assert!(!corrupt.invariants_hold());
        }

        Ok(())
    }

    #[test]
    fn committed_phase_invariants_reject_discriminating_counts() -> Result<(), &'static str> {
        let mut submitted = PresentationState::new();
        make_eligible(&mut submitted)?;
        let token = prepare(&mut submitted)?;
        apply(&mut submitted, PresentationAction::BeginUpdate(token))?;
        apply(&mut submitted, PresentationAction::Submit(token))?;
        assert!(
            !PresentationState {
                submission_count: 0,
                ..submitted
            }
            .invariants_hold()
        );
        assert!(
            !PresentationState {
                present_call_count: 1,
                ..submitted
            }
            .invariants_hold()
        );
        assert!(
            !PresentationState {
                commit_eligibility: super::CommitEligibility::NotCommitted,
                ..submitted
            }
            .invariants_hold()
        );

        apply(&mut submitted, PresentationAction::CallPresent(token))?;
        assert!(
            !PresentationState {
                submission_count: 0,
                ..submitted
            }
            .invariants_hold()
        );
        assert!(
            !PresentationState {
                present_call_count: 0,
                ..submitted
            }
            .invariants_hold()
        );

        Ok(())
    }

    #[test]
    fn terminal_invariants_reject_discriminating_state() -> Result<(), &'static str> {
        let mut submitted = PresentationState::new();
        make_eligible(&mut submitted)?;
        let token = prepare(&mut submitted)?;
        let prepared = submitted;
        apply(&mut submitted, PresentationAction::BeginUpdate(token))?;
        apply(&mut submitted, PresentationAction::Submit(token))?;
        apply(&mut submitted, PresentationAction::CallPresent(token))?;
        apply(
            &mut submitted,
            PresentationAction::CompletePresentation(token),
        )?;
        let presented = submitted;
        for corrupt in [
            PresentationState {
                frame_token: None,
                ..presented
            },
            PresentationState {
                requested_revision: super::PresentationRevision(
                    presented.requested_revision().get() + 1,
                ),
                ..presented
            },
            PresentationState {
                presented_revision: super::PresentationRevision(0),
                ..presented
            },
            PresentationState {
                surface_epoch: super::SurfaceEpoch(presented.surface_epoch().get() + 1),
                ..presented
            },
            PresentationState {
                submission_count: 0,
                ..presented
            },
            PresentationState {
                present_call_count: 0,
                ..presented
            },
            PresentationState {
                commit_eligibility: super::CommitEligibility::NotCommitted,
                ..presented
            },
        ] {
            assert!(!corrupt.invariants_hold());
        }

        assert!(
            !PresentationState {
                link: super::DisplayLinkState::Running,
                ..PresentationState::new()
            }
            .invariants_hold()
        );
        assert!(
            !PresentationState {
                application: ApplicationState::Stopped,
                link: super::DisplayLinkState::Invalid,
                phase: PresentationPhase::Submitted,
                resource: PresentationResource::InFlight,
                submission_count: 1,
                commit_eligibility: super::CommitEligibility::Eligible,
                frame_token: Some(token),
                ..prepared
            }
            .invariants_hold()
        );
        assert!(
            !PresentationState {
                outcome: PresentationOutcome::Failed,
                ..prepared
            }
            .invariants_hold()
        );
        assert!(
            !PresentationState {
                presented_revision: super::PresentationRevision(1),
                ..PresentationState::new()
            }
            .invariants_hold()
        );
        Ok(())
    }

    #[test]
    fn every_shutdown_phase_releases_or_drains() -> Result<(), &'static str> {
        for target in [
            PresentationPhase::Idle,
            PresentationPhase::Prepared,
            PresentationPhase::Encoding,
            PresentationPhase::Submitted,
            PresentationPhase::PresentCalled,
        ] {
            let mut state = PresentationState::new();
            make_eligible(&mut state)?;
            let token = if matches!(target, PresentationPhase::Idle) {
                None
            } else {
                let token = prepare(&mut state)?;
                if matches!(
                    target,
                    PresentationPhase::Encoding
                        | PresentationPhase::Submitted
                        | PresentationPhase::PresentCalled
                ) {
                    apply(&mut state, PresentationAction::BeginUpdate(token))?;
                }
                if matches!(
                    target,
                    PresentationPhase::Submitted | PresentationPhase::PresentCalled
                ) {
                    apply(&mut state, PresentationAction::Submit(token))?;
                }
                if matches!(target, PresentationPhase::PresentCalled) {
                    apply(&mut state, PresentationAction::CallPresent(token))?;
                }
                Some(token)
            };
            apply(&mut state, PresentationAction::BeginShutdown)?;
            if matches!(state.application(), ApplicationState::Stopping) {
                let token = token.ok_or("committed phase must have a token")?;
                if matches!(state.phase(), PresentationPhase::Submitted) {
                    apply(&mut state, PresentationAction::CallPresent(token))?;
                }
                apply(&mut state, PresentationAction::CompletePresentation(token))?;
                apply(&mut state, PresentationAction::StopAfterDrain)?;
            }
            assert_eq!(state.application(), ApplicationState::Stopped);
            assert_eq!(state.resource(), PresentationResource::Free);
            assert!(state.invariants_hold());
        }
        Ok(())
    }

    #[test]
    fn exhaustive_reachable_state_and_action_oracle() {
        let mut frontier = BTreeSet::from([PresentationState::new()]);
        let mut observed = frontier.clone();
        for _ in 0..10 {
            let mut next = BTreeSet::new();
            for state in frontier {
                for action in actions_for(state) {
                    let mut candidate = state;
                    let result = candidate.apply(action);
                    if result.is_ok() {
                        assert!(candidate.invariants_hold());
                        if observed.insert(candidate) {
                            next.insert(candidate);
                        }
                    } else {
                        assert_eq!(candidate, state);
                    }
                }
            }
            if next.is_empty() {
                break;
            }
            frontier = next;
        }
        assert!(
            observed
                .iter()
                .any(|state| { matches!(state.outcome(), PresentationOutcome::Presented) })
        );
        assert!(
            observed
                .iter()
                .any(|state| { matches!(state.outcome(), PresentationOutcome::Superseded) })
        );
        assert!(
            observed
                .iter()
                .any(|state| { matches!(state.application(), ApplicationState::Stopped) })
        );
    }

    fn actions_for(state: PresentationState) -> Vec<PresentationAction> {
        let mut actions = Vec::from([
            PresentationAction::Invalidate,
            PresentationAction::AdvanceSurfaceEpoch,
            PresentationAction::SetVisible(false),
            PresentationAction::SetVisible(true),
            PresentationAction::SetSized(false),
            PresentationAction::SetSized(true),
            PresentationAction::Resume,
            PresentationAction::Prepare,
            PresentationAction::BeginShutdown,
            PresentationAction::StopAfterDrain,
        ]);
        if let Some(token) = state.active_token() {
            actions.extend([
                PresentationAction::BeginUpdate(token),
                PresentationAction::DiscardStale(token),
                PresentationAction::Submit(token),
                PresentationAction::CallPresent(token),
                PresentationAction::CompletePresentation(token),
                PresentationAction::FailActive(token),
            ]);
        }
        actions
    }
}
