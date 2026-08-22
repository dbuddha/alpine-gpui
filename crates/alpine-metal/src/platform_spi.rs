//! Target-only bridge between Alpine's native macOS owner and Metal backend.
//!
//! This module is enabled only by the workspace platform implementation. It is
//! not an application contract and does not exist on portable targets.

use alpine_renderer::FrameReport;
use alpine_scene::Scene;
use objc2::{rc::Retained, runtime::ProtocolObject};
use objc2_metal::{MTLDevice, MTLDrawable, MTLTexture};

use crate::{
    FrameLifecycle, InitializationError, LifecycleAction, MetalBackend, OffscreenDescriptor,
    RenderError,
    accounting::{AccountingOutcome, FrameOperationUsage, FrameResourceUsage},
    submission::{DrawableRenderAttempt, NativeDrawableAttempt},
};

/// Retained Metal device shared by one layer and its renderer generation.
pub type NativeDevice = Retained<ProtocolObject<dyn MTLDevice>>;

/// One callback-drawable attempt with facts separated from terminal success.
#[must_use]
pub struct DrawableAttempt {
    committed: bool,
    present_called: bool,
    result: Result<FrameReport, RenderError>,
}

/// Stable index of one of the three native presentation-resource slots.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DrawableSlot(u8);

impl DrawableSlot {
    /// Creates a slot index in the range `0..3`.
    #[must_use]
    pub const fn new(index: u8) -> Option<Self> {
        if index < 3 { Some(Self(index)) } else { None }
    }

    /// Returns the zero-based native slot index.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }
}

/// Opaque ownership token for one committed callback drawable.
#[derive(Clone, Copy)]
#[must_use]
pub struct DrawableSubmission {
    native: crate::native::NativePresentationId,
    sequence: u64,
    primitives: usize,
    omitted_primitives: usize,
}

/// Result of one split-phase callback submission attempt.
#[must_use]
pub enum DrawableSubmitAttempt {
    /// Validation or native setup failed before a command became in flight.
    Rejected(DrawableAttempt),
    /// One command was committed and directly presented without a GPU wait.
    Submitted(DrawableSubmission),
}

/// Non-blocking main-thread observation of one submitted drawable.
#[must_use]
pub enum DrawableCompletionPoll {
    /// The completion handler has not published a terminal record yet.
    Pending,
    /// The exact submission reached a consumed terminal result.
    Complete(DrawableAttempt),
}

/// Handle-free reusable presentation-resource accounting.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PresentationSnapshot(crate::native::NativePresentationSnapshot);

impl PresentationSnapshot {
    /// Returns the fixed native slot count.
    #[must_use]
    pub const fn capacity(self) -> u8 {
        3
    }

    /// Returns slots retaining committed command ownership.
    #[must_use]
    pub const fn occupied_slots(self) -> u8 {
        self.0.occupied_slots
    }

    /// Returns shared upload bytes retained across all three slots.
    #[must_use]
    pub const fn current_upload_bytes(self) -> usize {
        self.0.current_upload_bytes
    }

    /// Returns the largest simultaneous upload retention observed.
    #[must_use]
    pub const fn peak_upload_bytes(self) -> usize {
        self.0.peak_upload_bytes
    }

    /// Returns current upload retention for each stable slot index.
    #[must_use]
    pub const fn slot_upload_bytes(self) -> [usize; 3] {
        self.0.slot_upload_bytes
    }

    /// Returns peak upload retention for each stable slot index.
    #[must_use]
    pub const fn slot_peak_upload_bytes(self) -> [usize; 3] {
        self.0.slot_peak_upload_bytes
    }

    /// Returns successful native upload-buffer allocations.
    #[must_use]
    pub const fn upload_allocations(self) -> u64 {
        self.0.upload_allocations
    }

    /// Returns upload buffers released by pressure or sustained disuse.
    #[must_use]
    pub const fn upload_trims(self) -> u64 {
        self.0.upload_trims
    }
}

impl DrawableAttempt {
    pub(crate) fn from_native(attempt: DrawableRenderAttempt) -> Self {
        Self {
            committed: attempt.committed,
            present_called: attempt.present_called,
            result: attempt.result,
        }
    }

    /// Returns whether one command buffer was committed.
    #[must_use]
    pub const fn committed(&self) -> bool {
        self.committed
    }

    /// Returns whether the callback drawable received one direct present call.
    #[must_use]
    pub const fn present_called(&self) -> bool {
        self.present_called
    }

    /// Returns terminal command evidence or the classified render failure.
    pub fn into_result(self) -> Result<FrameReport, RenderError> {
        self.result
    }
}

/// Initializes one backend generation from the exact device installed on its
/// corresponding `CAMetalLayer`.
///
/// # Errors
///
/// Returns the ordinary stage-classified Direct Metal initialization error.
pub fn new_backend_with_device(device: NativeDevice) -> Result<MetalBackend, InitializationError> {
    crate::native::new_backend_with_device(device).map(MetalBackend::from_platform_parts)
}

/// Uses a capability-relaxed backend only for native validation hosts.
///
/// # Errors
///
/// Returns a stage-classified initialization error when real Metal objects
/// cannot be created.
#[cfg(any(test, alpine_native_validation))]
pub fn new_validation_backend_with_device(
    device: NativeDevice,
) -> Result<MetalBackend, InitializationError> {
    crate::native::new_validation_backend_with_device(device).map(MetalBackend::from_platform_parts)
}

/// Uses a real validation backend whose first committed command reports
/// deterministic device loss after native completion.
///
/// # Errors
///
/// Returns a stage-classified initialization error when real Metal objects
/// cannot be created.
#[cfg(alpine_native_validation)]
pub fn new_validation_backend_with_device_loss(
    device: NativeDevice,
) -> Result<MetalBackend, InitializationError> {
    crate::native::new_validation_backend_with_device_loss(device)
        .map(MetalBackend::from_platform_parts)
}

/// Validates and submits one immutable scene to one callback-provided drawable.
pub fn render_callback_drawable(
    backend: &mut MetalBackend,
    scene: &Scene,
    descriptor: OffscreenDescriptor,
    texture: &ProtocolObject<dyn MTLTexture>,
    drawable: &ProtocolObject<dyn MTLDrawable>,
) -> DrawableAttempt {
    DrawableAttempt::from_native(
        backend.render_callback_drawable(scene, descriptor, texture, drawable),
    )
}

/// Validates, encodes, commits, and directly presents one callback drawable,
/// then returns without waiting for GPU completion.
pub fn submit_callback_drawable(
    backend: &mut MetalBackend,
    slot: DrawableSlot,
    scene: &Scene,
    descriptor: OffscreenDescriptor,
    texture: &ProtocolObject<dyn MTLTexture>,
    drawable: &ProtocolObject<dyn MTLDrawable>,
) -> DrawableSubmitAttempt {
    let frame = match backend.admit_frame(scene, descriptor) {
        Ok(frame) => frame,
        Err(error) => {
            return DrawableSubmitAttempt::Rejected(DrawableAttempt {
                committed: false,
                present_called: false,
                result: Err(error),
            });
        }
    };
    let in_flight = u64::from(backend.native.presentation_snapshot().occupied_slots);
    let Some(sequence) = backend
        .accounting
        .submitted_frames()
        .checked_add(in_flight)
        .and_then(|value| value.checked_add(1))
    else {
        let result = match backend.accounting.record_accepted(
            &frame,
            AccountingOutcome::Failed,
            false,
            FrameOperationUsage::default(),
            FrameResourceUsage::default(),
        ) {
            Ok(()) => Err(RenderError::SubmissionSequenceExhausted),
            Err(()) => Err(RenderError::AccountingOverflow),
        };
        return DrawableSubmitAttempt::Rejected(DrawableAttempt {
            committed: false,
            present_called: false,
            result,
        });
    };

    match objc2::rc::autoreleasepool(|_| {
        backend
            .native
            .submit_drawable(slot.get(), &frame, texture, drawable)
    }) {
        crate::native::NativeDrawableSubmitAttempt::Rejected(attempt) => {
            DrawableSubmitAttempt::Rejected(finish_presentation_attempt(
                backend,
                frame.consumed_primitives(),
                frame.omitted_primitives(),
                sequence,
                attempt,
            ))
        }
        crate::native::NativeDrawableSubmitAttempt::Submitted(submission) => {
            DrawableSubmitAttempt::Submitted(DrawableSubmission {
                native: submission.id,
                sequence,
                primitives: frame.consumed_primitives(),
                omitted_primitives: frame.omitted_primitives(),
            })
        }
    }
}

/// Polls one exact submission without waiting or exposing native handles.
pub fn poll_callback_drawable(
    backend: &mut MetalBackend,
    submission: DrawableSubmission,
) -> DrawableCompletionPoll {
    match backend.native.poll_drawable(submission.native) {
        Ok(None) => DrawableCompletionPoll::Pending,
        Ok(Some(attempt)) => DrawableCompletionPoll::Complete(finish_presentation_attempt(
            backend,
            submission.primitives,
            submission.omitted_primitives,
            submission.sequence,
            attempt,
        )),
        Err(error) => DrawableCompletionPoll::Complete(DrawableAttempt {
            committed: true,
            present_called: true,
            result: Err(error),
        }),
    }
}

fn finish_presentation_attempt(
    backend: &mut MetalBackend,
    primitives: usize,
    omitted_primitives: usize,
    sequence: u64,
    attempt: NativeDrawableAttempt,
) -> DrawableAttempt {
    let committed = attempt.committed;
    let present_called = attempt.present_called;
    let result = record_presentation_attempt(backend, primitives, omitted_primitives, &attempt)
        .and_then(|()| {
            attempt.result?;
            Ok(FrameReport {
                submission: sequence,
                primitives,
                omitted_primitives,
                draw_calls: attempt.operations.draw_calls,
                uploaded_bytes: attempt
                    .operations
                    .uploaded_bytes()
                    .ok_or(RenderError::AccountingOverflow)?,
                instance_upload_bytes: attempt.operations.instance_upload_bytes,
                atlas_upload_bytes: attempt.operations.atlas_upload_bytes,
                allocated_bytes: attempt.resources.allocated_bytes,
                retained_bytes: attempt.resources.peak_retained_bytes,
                readback_bytes: 0,
            })
        });
    DrawableAttempt {
        committed,
        present_called,
        result,
    }
}

fn record_presentation_attempt(
    backend: &mut MetalBackend,
    primitives: usize,
    omitted_primitives: usize,
    attempt: &NativeDrawableAttempt,
) -> Result<(), RenderError> {
    let outcome = verify_presentation_lifecycle(attempt)?;
    let result = backend
        .accounting
        .record_values(
            primitives,
            omitted_primitives,
            outcome,
            attempt.committed,
            attempt.operations,
            attempt.resources,
        )
        .map_err(|()| RenderError::AccountingOverflow);
    if attempt.device_lost {
        backend.accounting.invalidate_device();
    } else if result.is_err() && attempt.committed {
        backend.accounting.stop();
    }
    result
}

fn verify_presentation_lifecycle(
    attempt: &NativeDrawableAttempt,
) -> Result<AccountingOutcome, RenderError> {
    let mut lifecycle = FrameLifecycle::new();
    lifecycle
        .apply(LifecycleAction::BeginFrame)
        .and_then(|()| lifecycle.apply(LifecycleAction::Encode))
        .map_err(|_| RenderError::SubmissionInvariantViolated)?;
    if attempt.committed {
        lifecycle
            .apply(LifecycleAction::Submit)
            .map_err(|_| RenderError::SubmissionInvariantViolated)?;
    }
    if attempt.present_called != attempt.committed {
        return Err(RenderError::SubmissionInvariantViolated);
    }
    let outcome = match (&attempt.result, attempt.committed) {
        (Ok(()), true) => {
            lifecycle
                .apply(LifecycleAction::Complete)
                .map_err(|_| RenderError::SubmissionInvariantViolated)?;
            AccountingOutcome::Completed
        }
        (Err(_), true) => {
            lifecycle
                .apply(LifecycleAction::Fail)
                .map_err(|_| RenderError::SubmissionInvariantViolated)?;
            AccountingOutcome::Failed
        }
        (Err(_), false) => {
            lifecycle
                .apply(LifecycleAction::FailBeforeSubmit)
                .map_err(|_| RenderError::SubmissionInvariantViolated)?;
            AccountingOutcome::Failed
        }
        (Ok(()), false) => return Err(RenderError::SubmissionInvariantViolated),
    };
    if lifecycle.invariants_hold() {
        Ok(outcome)
    } else {
        Err(RenderError::SubmissionInvariantViolated)
    }
}

/// Returns exact bounded presentation-resource ownership evidence.
#[must_use]
pub fn presentation_snapshot(backend: &MetalBackend) -> PresentationSnapshot {
    PresentationSnapshot(backend.native.presentation_snapshot())
}

/// Releases free reusable uploads immediately and marks occupied slots to shed
/// their upload after terminal completion.
pub fn release_presentation_uploads_on_pressure(backend: &mut MetalBackend) {
    backend.native.release_presentation_uploads_on_pressure();
}

#[cfg(all(test, not(miri)))]
mod tests {
    use std::error::Error;

    use alpine_renderer::FrameReport;

    use crate::{
        BackendState, RenderError, RenderStage,
        accounting::{FrameOperationUsage, FrameResourceUsage},
        native::tests::callback_fixture,
        submission::{DrawableRenderAttempt, NativeDrawableAttempt},
    };

    use super::{
        DrawableAttempt, DrawableCompletionPoll, DrawableSlot, DrawableSubmitAttempt,
        finish_presentation_attempt, poll_callback_drawable, presentation_snapshot,
        release_presentation_uploads_on_pressure, submit_callback_drawable,
        verify_presentation_lifecycle,
    };

    #[test]
    fn drawable_slots_admit_exactly_three_stable_indices() {
        assert_eq!(DrawableSlot::new(0).map(DrawableSlot::get), Some(0));
        assert_eq!(DrawableSlot::new(1).map(DrawableSlot::get), Some(1));
        assert_eq!(DrawableSlot::new(2).map(DrawableSlot::get), Some(2));
        assert_eq!(DrawableSlot::new(3), None);
        assert_eq!(DrawableSlot::new(u8::MAX), None);
    }

    #[test]
    fn split_phase_spi_submits_polls_accounts_reuses_and_sheds() -> Result<(), Box<dyn Error>> {
        let mut fixture = callback_fixture()?;
        let slot = DrawableSlot::new(0).ok_or("slot zero")?;
        let submitted = submit_callback_drawable(
            &mut fixture.backend,
            slot,
            &fixture.scene,
            fixture.descriptor,
            &fixture.texture,
            objc2::runtime::ProtocolObject::from_ref(&*fixture.drawable),
        );
        let DrawableSubmitAttempt::Submitted(submission) = submitted else {
            return Err("valid split-phase submission was rejected".into());
        };
        let in_flight = presentation_snapshot(&fixture.backend);
        assert_eq!(in_flight.capacity(), 3);
        assert_eq!(in_flight.occupied_slots(), 1);
        assert!(in_flight.current_upload_bytes() > 0);
        assert!(in_flight.peak_upload_bytes() >= in_flight.current_upload_bytes());
        assert_eq!(in_flight.upload_allocations(), 1);
        assert_eq!(in_flight.upload_trims(), 0);
        assert_eq!(
            in_flight.slot_upload_bytes(),
            [in_flight.current_upload_bytes(), 0, 0]
        );
        assert_eq!(
            in_flight.slot_peak_upload_bytes(),
            [in_flight.peak_upload_bytes(), 0, 0]
        );
        assert_eq!(fixture.drawable.present_calls(), 1);

        release_presentation_uploads_on_pressure(&mut fixture.backend);
        let pressure_pending = presentation_snapshot(&fixture.backend);
        assert!(pressure_pending.current_upload_bytes() > 0);
        assert_eq!(pressure_pending.upload_trims(), 0);
        assert!(fixture.backend.native.wait_drawable(submission.native));
        let DrawableCompletionPoll::Complete(completed) =
            poll_callback_drawable(&mut fixture.backend, submission)
        else {
            return Err("ready completion remained pending".into());
        };
        assert!(completed.committed());
        assert!(completed.present_called());
        let report = completed.into_result()?;
        assert_eq!(report.submission, 1);
        assert_eq!(report.primitives, 4);
        assert_eq!(report.omitted_primitives, 1);
        assert_eq!(report.draw_calls, 1);
        assert!(report.uploaded_bytes > 0);
        assert!(report.allocated_bytes > 0);
        assert!(report.retained_bytes >= report.allocated_bytes);
        assert_eq!(report.readback_bytes, 0);
        assert_eq!(fixture.backend.submission_count(), 1);
        assert_eq!(fixture.backend.accounting.state(), BackendState::Ready);
        assert_eq!(presentation_snapshot(&fixture.backend).occupied_slots(), 0);

        let released = presentation_snapshot(&fixture.backend);
        assert_eq!(released.current_upload_bytes(), 0);
        assert_eq!(released.upload_trims(), 1);

        let resubmitted = submit_callback_drawable(
            &mut fixture.backend,
            slot,
            &fixture.scene,
            fixture.descriptor,
            &fixture.texture,
            objc2::runtime::ProtocolObject::from_ref(&*fixture.drawable),
        );
        let DrawableSubmitAttempt::Submitted(second_submission) = resubmitted else {
            return Err("released presentation slot was not reusable".into());
        };
        assert_eq!(
            presentation_snapshot(&fixture.backend).upload_allocations(),
            2
        );
        assert!(
            fixture
                .backend
                .native
                .wait_drawable(second_submission.native)
        );
        let DrawableCompletionPoll::Complete(second_completion) =
            poll_callback_drawable(&mut fixture.backend, second_submission)
        else {
            return Err("second ready completion remained pending".into());
        };
        assert_eq!(second_completion.into_result()?.submission, 2);
        Ok(())
    }

    #[test]
    fn split_phase_spi_rejects_invalid_lifecycle_and_stops_on_accounting_failure()
    -> Result<(), Box<dyn Error>> {
        let attempt = |committed, present_called, result| NativeDrawableAttempt {
            committed,
            present_called,
            device_lost: false,
            operations: FrameOperationUsage::default(),
            resources: FrameResourceUsage::default(),
            result,
        };
        assert!(verify_presentation_lifecycle(&attempt(true, true, Ok(()))).is_ok());
        assert!(
            verify_presentation_lifecycle(&attempt(
                false,
                false,
                Err(RenderError::SubmissionInvariantViolated,)
            ))
            .is_ok()
        );
        for invalid in [
            attempt(true, false, Ok(())),
            attempt(false, true, Err(RenderError::SubmissionInvariantViolated)),
            attempt(false, false, Ok(())),
        ] {
            assert_eq!(
                verify_presentation_lifecycle(&invalid),
                Err(RenderError::SubmissionInvariantViolated)
            );
        }

        let mut fixture = callback_fixture()?;
        fixture.backend.accounting.exhaust_render_sequence();
        let failed =
            finish_presentation_attempt(&mut fixture.backend, 1, 0, 1, attempt(true, true, Ok(())));
        assert_eq!(failed.into_result(), Err(RenderError::AccountingOverflow));
        assert_eq!(fixture.backend.accounting.state(), BackendState::Stopped);

        let rejected = submit_callback_drawable(
            &mut fixture.backend,
            DrawableSlot::new(0).ok_or("slot zero")?,
            &fixture.scene,
            fixture.descriptor,
            &fixture.texture,
            objc2::runtime::ProtocolObject::from_ref(&*fixture.drawable),
        );
        let DrawableSubmitAttempt::Rejected(rejected) = rejected else {
            return Err("stopped backend admitted split-phase work".into());
        };
        assert_eq!(
            rejected.into_result().err().map(|error| error.stage()),
            Some(RenderStage::SubmissionSequence)
        );
        Ok(())
    }

    #[test]
    fn drawable_attempt_preserves_each_native_fact_and_terminal_result() {
        let report = FrameReport {
            submission: 7,
            primitives: 11,
            ..FrameReport::default()
        };
        let completed = DrawableAttempt::from_native(DrawableRenderAttempt {
            committed: true,
            present_called: true,
            result: Ok(report),
        });
        assert!(completed.committed());
        assert!(completed.present_called());
        assert_eq!(completed.into_result(), Ok(report));

        let failed = DrawableAttempt::from_native(DrawableRenderAttempt {
            committed: false,
            present_called: false,
            result: Err(RenderError::SubmissionInvariantViolated),
        });
        assert!(!failed.committed());
        assert!(!failed.present_called());
        assert_eq!(
            failed.into_result(),
            Err(RenderError::SubmissionInvariantViolated)
        );
    }
}
