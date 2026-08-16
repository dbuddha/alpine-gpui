use std::fmt;

use crate::ValidatedFrame;

/// Admission state of one synchronous Direct Metal backend generation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackendState {
    /// New frames may be validated and submitted.
    Ready,
    /// The owner explicitly completed shutdown.
    Stopped,
    /// A terminal device failure invalidated this generation.
    DeviceLost,
}

impl fmt::Display for BackendState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ready => formatter.write_str("ready"),
            Self::Stopped => formatter.write_str("stopped"),
            Self::DeviceLost => formatter.write_str("device-lost"),
        }
    }
}

/// Monotonic identity for native objects created as one recovery generation.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct BackendGeneration(u64);

impl BackendGeneration {
    pub(crate) const INITIAL: Self = Self(1);

    pub(crate) const fn next(self) -> Option<Self> {
        match self.0.checked_add(1) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    /// Returns the nonzero generation sequence number.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Exact encoded and uploaded work observed during one frame attempt.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct FrameOperationUsage {
    pub(crate) draw_calls: usize,
    pub(crate) uploaded_bytes: usize,
}

/// Exact native resource use observed during one frame attempt.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[allow(
    clippy::struct_field_names,
    reason = "each value is a byte count with a distinct ownership meaning"
)]
pub(crate) struct FrameResourceUsage {
    pub(crate) allocated_bytes: usize,
    pub(crate) peak_retained_bytes: usize,
    pub(crate) current_retained_bytes: usize,
    pub(crate) readback_bytes: usize,
}

/// Terminal class used to balance accepted frame accounting.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AccountingOutcome {
    Completed,
    Failed,
    Cancelled,
}

/// Cumulative work and ownership evidence for one backend generation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BackendAccounting {
    generation: BackendGeneration,
    state: BackendState,
    render_calls: u128,
    admission_rejections: u128,
    validation_rejections: u128,
    accepted_frames: u128,
    completed_frames: u128,
    failed_frames: u128,
    cancelled_frames: u128,
    submitted_frames: u64,
    primitives: u128,
    omitted_primitives: u128,
    draw_calls: u128,
    uploaded_bytes: u128,
    allocated_bytes: u128,
    readback_bytes: u128,
    peak_retained_bytes: usize,
    current_retained_bytes: usize,
}

impl BackendAccounting {
    pub(crate) const fn new(generation: BackendGeneration) -> Self {
        Self {
            generation,
            state: BackendState::Ready,
            render_calls: 0,
            admission_rejections: 0,
            validation_rejections: 0,
            accepted_frames: 0,
            completed_frames: 0,
            failed_frames: 0,
            cancelled_frames: 0,
            submitted_frames: 0,
            primitives: 0,
            omitted_primitives: 0,
            draw_calls: 0,
            uploaded_bytes: 0,
            allocated_bytes: 0,
            readback_bytes: 0,
            peak_retained_bytes: 0,
            current_retained_bytes: 0,
        }
    }

    pub(crate) fn record_admission_rejection(&mut self) -> Result<(), ()> {
        self.record_rejection(true)
    }

    pub(crate) fn record_validation_rejection(&mut self) -> Result<(), ()> {
        self.record_rejection(false)
    }

    fn record_rejection(&mut self, admission: bool) -> Result<(), ()> {
        let mut next = *self;
        next.render_calls = next.render_calls.checked_add(1).ok_or(())?;
        if admission {
            next.admission_rejections = next.admission_rejections.checked_add(1).ok_or(())?;
        } else {
            next.validation_rejections = next.validation_rejections.checked_add(1).ok_or(())?;
        }
        *self = next;
        Ok(())
    }

    pub(crate) fn record_accepted(
        &mut self,
        frame: &ValidatedFrame,
        outcome: AccountingOutcome,
        committed: bool,
        operations: FrameOperationUsage,
        resources: FrameResourceUsage,
    ) -> Result<(), ()> {
        self.record_values(
            frame.consumed_primitives(),
            frame.omitted_primitives(),
            outcome,
            committed,
            operations,
            resources,
        )
    }

    #[cfg(all(feature = "platform-spi", target_os = "macos", target_arch = "aarch64"))]
    pub(crate) fn record_presentation(
        &mut self,
        primitives: usize,
        omitted_primitives: usize,
        outcome: AccountingOutcome,
        committed: bool,
        operations: FrameOperationUsage,
        resources: FrameResourceUsage,
    ) -> Result<(), ()> {
        self.record_values(
            primitives,
            omitted_primitives,
            outcome,
            committed,
            operations,
            resources,
        )
    }

    fn record_values(
        &mut self,
        primitives: usize,
        omitted_primitives: usize,
        outcome: AccountingOutcome,
        committed: bool,
        operations: FrameOperationUsage,
        resources: FrameResourceUsage,
    ) -> Result<(), ()> {
        if outcome == AccountingOutcome::Cancelled
            && (operations != FrameOperationUsage::default()
                || resources != FrameResourceUsage::default())
        {
            return Err(());
        }
        let mut next = *self;
        next.render_calls = next.render_calls.checked_add(1).ok_or(())?;
        next.accepted_frames = next.accepted_frames.checked_add(1).ok_or(())?;
        match outcome {
            AccountingOutcome::Completed => {
                next.completed_frames = next.completed_frames.checked_add(1).ok_or(())?;
            }
            AccountingOutcome::Failed => {
                next.failed_frames = next.failed_frames.checked_add(1).ok_or(())?;
            }
            AccountingOutcome::Cancelled => {
                next.cancelled_frames = next.cancelled_frames.checked_add(1).ok_or(())?;
            }
        }
        if committed {
            next.submitted_frames = next.submitted_frames.checked_add(1).ok_or(())?;
        }
        next.primitives = next.primitives.checked_add(primitives as u128).ok_or(())?;
        next.omitted_primitives = next
            .omitted_primitives
            .checked_add(omitted_primitives as u128)
            .ok_or(())?;
        next.draw_calls = next
            .draw_calls
            .checked_add(operations.draw_calls as u128)
            .ok_or(())?;
        next.uploaded_bytes = next
            .uploaded_bytes
            .checked_add(operations.uploaded_bytes as u128)
            .ok_or(())?;
        next.allocated_bytes = next
            .allocated_bytes
            .checked_add(resources.allocated_bytes as u128)
            .ok_or(())?;
        next.readback_bytes = next
            .readback_bytes
            .checked_add(resources.readback_bytes as u128)
            .ok_or(())?;
        next.peak_retained_bytes = next.peak_retained_bytes.max(resources.peak_retained_bytes);
        next.current_retained_bytes = resources.current_retained_bytes;
        if !next.invariants_hold() {
            return Err(());
        }
        *self = next;
        Ok(())
    }

    #[cfg(kani)]
    pub(crate) fn record_symbolic(
        &mut self,
        outcome: AccountingOutcome,
        committed: bool,
        primitives: usize,
        omitted_primitives: usize,
        operations: FrameOperationUsage,
        resources: FrameResourceUsage,
    ) -> Result<(), ()> {
        self.record_values(
            primitives,
            omitted_primitives,
            outcome,
            committed,
            operations,
            resources,
        )
    }

    pub(crate) fn stop(&mut self) {
        self.state = BackendState::Stopped;
    }

    pub(crate) fn invalidate_device(&mut self) {
        self.state = BackendState::DeviceLost;
    }

    /// Returns this native-object generation.
    #[must_use]
    pub const fn generation(self) -> BackendGeneration {
        self.generation
    }

    /// Returns whether this generation admits work.
    #[must_use]
    pub const fn state(self) -> BackendState {
        self.state
    }

    /// Returns all render calls, including rejected calls.
    #[must_use]
    pub const fn render_calls(self) -> u128 {
        self.render_calls
    }

    /// Returns calls rejected because the backend was not ready.
    #[must_use]
    pub const fn admission_rejections(self) -> u128 {
        self.admission_rejections
    }

    /// Returns calls rejected by pure validation.
    #[must_use]
    pub const fn validation_rejections(self) -> u128 {
        self.validation_rejections
    }

    /// Returns validated frames admitted to cancellation or native work.
    #[must_use]
    pub const fn accepted_frames(self) -> u128 {
        self.accepted_frames
    }

    /// Returns successfully completed frames.
    #[must_use]
    pub const fn completed_frames(self) -> u128 {
        self.completed_frames
    }

    /// Returns accepted frames that ended in failure.
    #[must_use]
    pub const fn failed_frames(self) -> u128 {
        self.failed_frames
    }

    /// Returns accepted frames cancelled before submission.
    #[must_use]
    pub const fn cancelled_frames(self) -> u128 {
        self.cancelled_frames
    }

    /// Returns command buffers committed by this generation.
    #[must_use]
    pub const fn submitted_frames(self) -> u64 {
        self.submitted_frames
    }

    /// Returns consumed primitive count across accepted frames.
    #[must_use]
    pub const fn primitives(self) -> u128 {
        self.primitives
    }

    /// Returns omitted primitive count across accepted frames.
    #[must_use]
    pub const fn omitted_primitives(self) -> u128 {
        self.omitted_primitives
    }

    /// Returns encoded draw-call count across accepted frames.
    #[must_use]
    pub const fn draw_calls(self) -> u128 {
        self.draw_calls
    }

    /// Returns bytes successfully copied into native upload buffers.
    #[must_use]
    pub const fn uploaded_bytes(self) -> u128 {
        self.uploaded_bytes
    }

    /// Returns native resource bytes allocated across accepted frames.
    #[must_use]
    pub const fn allocated_bytes(self) -> u128 {
        self.allocated_bytes
    }

    /// Returns native readback bytes allocated across accepted frames.
    #[must_use]
    pub const fn readback_bytes(self) -> u128 {
        self.readback_bytes
    }

    /// Returns the largest frame-local retained byte count.
    #[must_use]
    pub const fn peak_retained_bytes(self) -> usize {
        self.peak_retained_bytes
    }

    /// Returns bytes still retained by a completed synchronous call.
    #[must_use]
    pub const fn current_retained_bytes(self) -> usize {
        self.current_retained_bytes
    }

    /// Checks balanced terminal, submission, and ownership accounting.
    #[must_use]
    pub fn invariants_hold(self) -> bool {
        let Some(rejected_or_accepted) = self
            .admission_rejections
            .checked_add(self.validation_rejections)
            .and_then(|value| value.checked_add(self.accepted_frames))
        else {
            return false;
        };
        let Some(submitted_terminal_frames) = self.completed_frames.checked_add(self.failed_frames)
        else {
            return false;
        };
        let Some(terminal_frames) = submitted_terminal_frames.checked_add(self.cancelled_frames)
        else {
            return false;
        };
        rejected_or_accepted == self.render_calls
            && terminal_frames == self.accepted_frames
            && self.completed_frames <= u128::from(self.submitted_frames)
            && u128::from(self.submitted_frames) <= submitted_terminal_frames
            && self.current_retained_bytes == 0
    }

    #[cfg(all(test, not(all(target_os = "macos", target_arch = "aarch64"))))]
    pub(crate) fn exhaust_submission_sequence(&mut self) {
        let maximum = u128::from(u64::MAX);
        self.render_calls = maximum;
        self.accepted_frames = maximum;
        self.completed_frames = maximum;
        self.submitted_frames = u64::MAX;
    }

    #[cfg(test)]
    pub(crate) fn exhaust_render_sequence(&mut self) {
        self.render_calls = u128::MAX;
        self.admission_rejections = u128::MAX;
    }
}

#[cfg(test)]
mod tests {
    use alpine_core::{LinearRgba, Size};
    use alpine_scene::{SceneBuilder, SceneRevision};

    use super::{
        AccountingOutcome, BackendAccounting, BackendGeneration, FrameOperationUsage,
        FrameResourceUsage,
    };
    use crate::{OffscreenDescriptor, ValidatedFrame};

    #[allow(clippy::expect_used, reason = "fixed test values must remain valid")]
    fn frame() -> ValidatedFrame {
        let viewport = Size::new(1.0, 1.0).expect("fixture viewport must be valid");
        let scene = SceneBuilder::new(SceneRevision::new(1), viewport).finish();
        let clear = LinearRgba::new(0.0, 0.0, 0.0, 0.0).expect("fixture clear must be valid");
        ValidatedFrame::new(
            &scene,
            OffscreenDescriptor::new(1, 1, 1.0, clear).expect("fixture descriptor must be valid"),
        )
        .expect("fixture frame must be valid")
    }

    #[test]
    fn balances_rejections_and_every_terminal_outcome() -> Result<(), Box<dyn std::error::Error>> {
        let frame = frame();
        let mut accounting = BackendAccounting::new(BackendGeneration::INITIAL);
        accounting
            .record_admission_rejection()
            .map_err(|()| "admission overflow")?;
        accounting
            .record_validation_rejection()
            .map_err(|()| "validation overflow")?;
        for (outcome, committed) in [
            (AccountingOutcome::Completed, true),
            (AccountingOutcome::Failed, true),
            (AccountingOutcome::Failed, false),
            (AccountingOutcome::Cancelled, false),
        ] {
            let has_native_work = outcome != AccountingOutcome::Cancelled;
            accounting
                .record_accepted(
                    &frame,
                    outcome,
                    committed,
                    FrameOperationUsage {
                        draw_calls: usize::from(has_native_work),
                        uploaded_bytes: usize::from(has_native_work) * 32,
                    },
                    FrameResourceUsage {
                        allocated_bytes: usize::from(has_native_work) * 512,
                        peak_retained_bytes: usize::from(has_native_work) * 512,
                        current_retained_bytes: 0,
                        readback_bytes: usize::from(has_native_work) * 256,
                    },
                )
                .map_err(|()| "frame overflow")?;
        }

        assert!(accounting.invariants_hold());
        assert_eq!(accounting.render_calls(), 6);
        assert_eq!(accounting.admission_rejections(), 1);
        assert_eq!(accounting.validation_rejections(), 1);
        assert_eq!(accounting.accepted_frames(), 4);
        assert_eq!(accounting.completed_frames(), 1);
        assert_eq!(accounting.failed_frames(), 2);
        assert_eq!(accounting.cancelled_frames(), 1);
        assert_eq!(accounting.submitted_frames(), 2);
        assert_eq!(accounting.draw_calls(), 3);
        assert_eq!(accounting.uploaded_bytes(), 96);
        assert_eq!(accounting.allocated_bytes(), 1_536);
        assert_eq!(accounting.readback_bytes(), 768);
        assert_eq!(accounting.peak_retained_bytes(), 512);
        assert_eq!(accounting.current_retained_bytes(), 0);
        Ok(())
    }

    #[test]
    fn generation_is_monotonic_and_detects_exhaustion() {
        assert_eq!(BackendGeneration::INITIAL.get(), 1);
        assert_eq!(
            BackendGeneration::INITIAL
                .next()
                .map(BackendGeneration::get),
            Some(2)
        );
        assert_eq!(BackendGeneration(u64::MAX).next(), None);
    }

    #[test]
    fn exposes_every_counter_and_terminal_state_without_losing_invariants()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut accounting = BackendAccounting::new(BackendGeneration::INITIAL);
        assert_eq!(accounting.admission_rejections(), 0);
        assert_eq!(accounting.validation_rejections(), 0);
        assert_eq!(accounting.completed_frames(), 0);
        assert_eq!(accounting.cancelled_frames(), 0);
        assert_eq!(accounting.primitives(), 0);
        assert_eq!(accounting.omitted_primitives(), 0);
        assert_eq!(accounting.draw_calls(), 0);
        assert_eq!(accounting.uploaded_bytes(), 0);
        accounting
            .record_values(
                2,
                1,
                AccountingOutcome::Completed,
                true,
                FrameOperationUsage {
                    draw_calls: 1,
                    uploaded_bytes: 32,
                },
                FrameResourceUsage {
                    allocated_bytes: 768,
                    peak_retained_bytes: 768,
                    current_retained_bytes: 0,
                    readback_bytes: 256,
                },
            )
            .map_err(|()| "frame overflow")?;

        assert_eq!(accounting.generation().get(), 1);
        assert_eq!(accounting.state(), super::BackendState::Ready);
        assert_eq!(accounting.primitives(), 2);
        assert_eq!(accounting.omitted_primitives(), 1);
        assert_eq!(accounting.draw_calls(), 1);
        assert_eq!(accounting.uploaded_bytes(), 32);
        assert_eq!(accounting.allocated_bytes(), 768);
        assert_eq!(accounting.readback_bytes(), 256);
        assert_eq!(accounting.peak_retained_bytes(), 768);
        assert_eq!(accounting.current_retained_bytes(), 0);
        assert_eq!(accounting.state().to_string(), "ready");

        let mut invalid_retention = accounting;
        invalid_retention.current_retained_bytes = 1;
        assert_eq!(invalid_retention.current_retained_bytes(), 1);
        assert!(!invalid_retention.invariants_hold());

        accounting.stop();
        assert_eq!(accounting.state().to_string(), "stopped");
        accounting.invalidate_device();
        assert_eq!(accounting.state().to_string(), "device-lost");
        assert!(accounting.invariants_hold());
        Ok(())
    }

    #[test]
    fn rejects_counter_overflow_and_every_unbalanced_relation() {
        let mut accounting = BackendAccounting::new(BackendGeneration::INITIAL);
        accounting.render_calls = u128::MAX;
        assert_eq!(accounting.record_admission_rejection(), Err(()));
        assert_eq!(accounting.record_validation_rejection(), Err(()));
        assert_eq!(accounting.render_calls, u128::MAX);
        assert_eq!(accounting.admission_rejections, 0);
        assert_eq!(accounting.validation_rejections, 0);

        let mut invalid = BackendAccounting::new(BackendGeneration::INITIAL);
        invalid.render_calls = 1;
        assert!(!invalid.invariants_hold());
        invalid.accepted_frames = 1;
        assert!(!invalid.invariants_hold());
        invalid.completed_frames = 1;
        invalid.current_retained_bytes = 1;
        assert!(!invalid.invariants_hold());
        invalid.current_retained_bytes = 0;
        invalid.submitted_frames = 1;
        assert!(invalid.invariants_hold());

        invalid.admission_rejections = u128::MAX;
        invalid.validation_rejections = 1;
        assert!(!invalid.invariants_hold());
        invalid.admission_rejections = 0;
        invalid.validation_rejections = 0;
        invalid.completed_frames = u128::MAX;
        invalid.failed_frames = 1;
        assert!(!invalid.invariants_hold());
        invalid.failed_frames = 0;
        invalid.cancelled_frames = 1;
        assert!(!invalid.invariants_hold());

        let mut atomic = BackendAccounting::new(BackendGeneration::INITIAL);
        let before = atomic;
        assert_eq!(
            atomic.record_values(
                0,
                0,
                AccountingOutcome::Completed,
                true,
                FrameOperationUsage::default(),
                FrameResourceUsage {
                    current_retained_bytes: 1,
                    ..FrameResourceUsage::default()
                },
            ),
            Err(())
        );
        assert_eq!(atomic, before);

        let mut cancellation = BackendAccounting::new(BackendGeneration::INITIAL);
        let before = cancellation;
        assert_eq!(
            cancellation.record_values(
                1,
                0,
                AccountingOutcome::Cancelled,
                false,
                FrameOperationUsage {
                    draw_calls: 1,
                    uploaded_bytes: 32,
                },
                FrameResourceUsage::default(),
            ),
            Err(())
        );
        assert_eq!(cancellation, before);
    }
}
