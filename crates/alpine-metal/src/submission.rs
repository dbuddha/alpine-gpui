use std::{error::Error, fmt};

use alpine_renderer::{FrameReport, Renderer, RendererCapabilities};
use alpine_scene::Scene;

use crate::{
    BackendGeneration, BackendState, Bgra8Image, FrameLifecycle, InitializationError,
    LifecycleAction, NativeFailure, OffscreenDescriptor, OffscreenError, ValidatedFrame,
    accounting::{AccountingOutcome, FrameResourceUsage},
    initialization::MetalBackend,
};

/// Caller action appropriate for a classified render failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoveryClassification {
    /// Correct the request before trying again.
    FixRequest,
    /// A later frame may be retried without rebuilding the backend.
    RetryFrame,
    /// Consume and recreate the invalid backend generation.
    RecreateBackend,
    /// The current target cannot support Direct Metal.
    Unsupported,
    /// The owner already stopped this backend.
    Stopped,
    /// Internal accounting or ownership invariants failed closed.
    Fatal,
}

/// Native stage at which an offscreen render stopped.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RenderStage {
    /// Pure descriptor and scene validation.
    Validation,
    /// Monotonic submission-sequence reservation.
    SubmissionSequence,
    /// Private render-texture allocation.
    RenderTexture,
    /// Shared readback-buffer allocation.
    ReadbackBuffer,
    /// Shader-instance upload allocation.
    UploadBuffer,
    /// Retained command-buffer creation.
    CommandBuffer,
    /// Render-command encoding.
    RenderEncoder,
    /// Texture-to-buffer blit encoding.
    BlitEncoder,
    /// Terminal command-buffer completion.
    Completion,
    /// Padding removal and compact image construction.
    Readback,
}

/// Stable terminal status copied from a native Metal command buffer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandStatus {
    /// The command buffer was never enqueued.
    NotEnqueued,
    /// The command buffer was enqueued but not committed.
    Enqueued,
    /// The command buffer was committed but not scheduled.
    Committed,
    /// The command buffer was scheduled but not terminal.
    Scheduled,
    /// The command buffer completed successfully.
    Completed,
    /// The command buffer terminated with an error.
    Error,
    /// Metal returned a status unknown to this Alpine version.
    Unknown(usize),
}

/// A fail-closed offscreen validation, submission, or readback error.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RenderError {
    /// Pure validation rejected the scene before native work began.
    Validation(OffscreenError),
    /// The process is not running on the supported Metal platform.
    UnsupportedPlatform {
        /// Compile-time processor architecture.
        architecture: &'static str,
        /// Compile-time operating system.
        operating_system: &'static str,
    },
    /// The backend generation no longer admits work.
    BackendUnavailable {
        /// Generation that rejected the call.
        generation: BackendGeneration,
        /// Terminal backend state.
        state: BackendState,
    },
    /// The next submission sequence could not be represented.
    SubmissionSequenceExhausted,
    /// Cumulative evidence could not represent another operation.
    AccountingOverflow,
    /// Metal could not allocate a required resource.
    ResourceUnavailable {
        /// Resource-creation stage.
        stage: RenderStage,
        /// Exact requested byte count when the resource is byte-addressed.
        requested_bytes: Option<usize>,
    },
    /// The target exceeds the guaranteed Metal 3 two-dimensional limit.
    TextureExtentUnsupported {
        /// Requested physical width.
        width: u32,
        /// Requested physical height.
        height: u32,
        /// Guaranteed maximum for either dimension.
        limit: u32,
    },
    /// Metal returned no encoder for a valid command buffer.
    EncoderUnavailable {
        /// Encoder-creation stage.
        stage: RenderStage,
    },
    /// A committed command buffer terminated with an error.
    CommandFailed {
        /// Stable terminal status.
        status: CommandStatus,
        /// Copied native details when Metal supplied them.
        failure: Option<NativeFailure>,
        /// Stable recovery policy derived at the native boundary.
        recovery: RecoveryClassification,
    },
    /// Waiting returned without a successful or failed terminal status.
    UnexpectedCommandStatus {
        /// Observed nonterminal or unknown status.
        status: CommandStatus,
    },
    /// Metal exposed a readback buffer with an unexpected length.
    ReadbackLengthMismatch {
        /// Checked aligned byte count required by the frame.
        expected: usize,
        /// Native buffer byte count or supplied test-fixture length.
        actual: usize,
    },
    /// Compact image storage could not be reserved.
    ReadbackAllocationFailed {
        /// Exact compact byte count requested.
        bytes: usize,
    },
    /// Native code reported success without committing exactly one buffer.
    SubmissionInvariantViolated,
}

impl RenderError {
    /// Returns the stage at which rendering stopped.
    #[must_use]
    pub const fn stage(&self) -> RenderStage {
        match self {
            Self::Validation(_)
            | Self::UnsupportedPlatform { .. }
            | Self::BackendUnavailable { .. } => RenderStage::Validation,
            Self::SubmissionSequenceExhausted | Self::AccountingOverflow => {
                RenderStage::SubmissionSequence
            }
            Self::TextureExtentUnsupported { .. } => RenderStage::RenderTexture,
            Self::ResourceUnavailable { stage, .. } | Self::EncoderUnavailable { stage } => *stage,
            Self::CommandFailed { .. } | Self::UnexpectedCommandStatus { .. } => {
                RenderStage::Completion
            }
            Self::ReadbackLengthMismatch { .. } | Self::ReadbackAllocationFailed { .. } => {
                RenderStage::Readback
            }
            Self::SubmissionInvariantViolated => RenderStage::CommandBuffer,
        }
    }

    /// Returns the caller recovery class without exposing native objects.
    #[must_use]
    pub const fn recovery(&self) -> RecoveryClassification {
        match self {
            Self::Validation(_) | Self::TextureExtentUnsupported { .. } => {
                RecoveryClassification::FixRequest
            }
            Self::UnsupportedPlatform { .. } => RecoveryClassification::Unsupported,
            Self::BackendUnavailable {
                state: BackendState::Stopped,
                ..
            } => RecoveryClassification::Stopped,
            Self::BackendUnavailable {
                state: BackendState::DeviceLost,
                ..
            } => RecoveryClassification::RecreateBackend,
            Self::BackendUnavailable {
                state: BackendState::Ready,
                ..
            }
            | Self::SubmissionSequenceExhausted
            | Self::AccountingOverflow
            | Self::UnexpectedCommandStatus { .. }
            | Self::ReadbackLengthMismatch { .. }
            | Self::SubmissionInvariantViolated => RecoveryClassification::Fatal,
            Self::ResourceUnavailable { .. }
            | Self::EncoderUnavailable { .. }
            | Self::ReadbackAllocationFailed { .. } => RecoveryClassification::RetryFrame,
            Self::CommandFailed { recovery, .. } => *recovery,
        }
    }
}

impl fmt::Display for RenderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Validation(error) => write!(formatter, "offscreen validation failed: {error}"),
            Self::UnsupportedPlatform {
                architecture,
                operating_system,
            } => write!(
                formatter,
                "Direct Metal requires Apple Silicon macOS, found {architecture}-{operating_system}"
            ),
            Self::BackendUnavailable { generation, state } => write!(
                formatter,
                "Metal backend generation {} is {state}",
                generation.get()
            ),
            Self::SubmissionSequenceExhausted => {
                formatter.write_str("Metal submission sequence exhausted")
            }
            Self::AccountingOverflow => formatter.write_str("Metal accounting exhausted"),
            Self::ResourceUnavailable {
                stage,
                requested_bytes,
            } => match requested_bytes {
                Some(bytes) => write!(
                    formatter,
                    "Metal resource allocation failed at {stage:?} for {bytes} bytes"
                ),
                None => write!(formatter, "Metal resource allocation failed at {stage:?}"),
            },
            Self::TextureExtentUnsupported {
                width,
                height,
                limit,
            } => write!(
                formatter,
                "offscreen target {width}x{height} exceeds Metal 3 limit {limit}"
            ),
            Self::EncoderUnavailable { stage } => {
                write!(formatter, "Metal returned no encoder at {stage:?}")
            }
            Self::CommandFailed {
                status, failure, ..
            } => match failure {
                Some(failure) => write!(formatter, "Metal command ended as {status:?}: {failure}"),
                None => write!(
                    formatter,
                    "Metal command ended as {status:?} without details"
                ),
            },
            Self::UnexpectedCommandStatus { status } => {
                write!(
                    formatter,
                    "Metal wait returned nonterminal status {status:?}"
                )
            }
            Self::ReadbackLengthMismatch { expected, actual } => write!(
                formatter,
                "Metal readback length mismatch: expected {expected} bytes, found {actual}"
            ),
            Self::ReadbackAllocationFailed { bytes } => {
                write!(formatter, "cannot reserve {bytes} compact readback bytes")
            }
            Self::SubmissionInvariantViolated => {
                formatter.write_str("native render succeeded without exactly one submission")
            }
        }
    }
}

impl Error for RenderError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Validation(error) => Some(error),
            Self::CommandFailed {
                failure: Some(failure),
                ..
            } => Some(failure),
            Self::UnsupportedPlatform { .. }
            | Self::BackendUnavailable { .. }
            | Self::SubmissionSequenceExhausted
            | Self::AccountingOverflow
            | Self::ResourceUnavailable { .. }
            | Self::TextureExtentUnsupported { .. }
            | Self::EncoderUnavailable { .. }
            | Self::CommandFailed { failure: None, .. }
            | Self::UnexpectedCommandStatus { .. }
            | Self::ReadbackLengthMismatch { .. }
            | Self::ReadbackAllocationFailed { .. }
            | Self::SubmissionInvariantViolated => None,
        }
    }
}

/// Error returned while replacing a device-lost backend generation.
#[derive(Debug)]
pub enum RecoveryError {
    /// Recovery was requested from a backend that was not device-lost.
    BackendNotDeviceLost {
        /// Generation that rejected recovery.
        generation: BackendGeneration,
        /// Current backend state.
        state: BackendState,
    },
    /// The next generation identifier cannot be represented.
    GenerationExhausted,
    /// Creating the replacement native owner failed.
    Initialization(InitializationError),
}

impl fmt::Display for RecoveryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BackendNotDeviceLost { generation, state } => write!(
                formatter,
                "backend generation {} is {state}, not device-lost",
                generation.get()
            ),
            Self::GenerationExhausted => formatter.write_str("Metal backend generation exhausted"),
            Self::Initialization(error) => write!(formatter, "Metal recovery failed: {error}"),
        }
    }
}

impl Error for RecoveryError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Initialization(error) => Some(error),
            Self::BackendNotDeviceLost { .. } | Self::GenerationExhausted => None,
        }
    }
}

/// Evidence returned when a validated frame is cancelled before submission.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CancellationReport {
    generation: BackendGeneration,
    primitives: usize,
    omitted_primitives: usize,
    uploaded_bytes_avoided: usize,
}

impl CancellationReport {
    /// Returns the generation that accepted the cancellation.
    #[must_use]
    pub const fn generation(self) -> BackendGeneration {
        self.generation
    }

    /// Returns consumed source primitives.
    #[must_use]
    pub const fn primitives(self) -> usize {
        self.primitives
    }

    /// Returns primitives omitted during pure lowering.
    #[must_use]
    pub const fn omitted_primitives(self) -> usize {
        self.omitted_primitives
    }

    /// Returns native upload bytes avoided by cancellation.
    #[must_use]
    pub const fn uploaded_bytes_avoided(self) -> usize {
        self.uploaded_bytes_avoided
    }
}

impl From<OffscreenError> for RenderError {
    fn from(error: OffscreenError) -> Self {
        Self::Validation(error)
    }
}

/// Owned pixels and observable work for one completed offscreen frame.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OffscreenFrame {
    image: Bgra8Image,
    report: FrameReport,
}

/// Portable owner of the latest completed Metal offscreen image.
#[derive(Clone, Debug, PartialEq)]
pub struct OffscreenTarget {
    descriptor: OffscreenDescriptor,
    image: Option<Bgra8Image>,
}

impl OffscreenTarget {
    /// Creates an empty target from a validated descriptor.
    #[must_use]
    pub const fn new(descriptor: OffscreenDescriptor) -> Self {
        Self {
            descriptor,
            image: None,
        }
    }

    /// Returns the immutable target description.
    #[must_use]
    pub const fn descriptor(&self) -> OffscreenDescriptor {
        self.descriptor
    }

    /// Returns the latest successfully completed image, if any.
    #[must_use]
    pub const fn image(&self) -> Option<&Bgra8Image> {
        self.image.as_ref()
    }

    /// Removes and returns the latest successfully completed image.
    pub fn take_image(&mut self) -> Option<Bgra8Image> {
        self.image.take()
    }
}

impl OffscreenFrame {
    /// Returns compact top-to-bottom BGRA8 pixels.
    #[must_use]
    pub const fn image(&self) -> &Bgra8Image {
        &self.image
    }

    /// Returns work observed for the completed submission.
    #[must_use]
    pub const fn report(&self) -> FrameReport {
        self.report
    }
}

pub(crate) struct NativeRenderAttempt {
    pub(crate) committed: bool,
    pub(crate) device_lost: bool,
    pub(crate) usage: FrameResourceUsage,
    pub(crate) result: Result<Bgra8Image, RenderError>,
}

impl MetalBackend {
    /// Validates and renders one immutable scene into a compact BGRA8 image.
    ///
    /// Validation completes before any native resource or command buffer is
    /// created. A successful call commits exactly one retained command buffer
    /// and waits for successful terminal completion before reading CPU bytes.
    ///
    /// # Errors
    ///
    /// Returns a stage-classified error without pixels or a success report.
    pub fn render_offscreen(
        &mut self,
        scene: &Scene,
        descriptor: OffscreenDescriptor,
    ) -> Result<OffscreenFrame, RenderError> {
        let frame = self.admit_frame(scene, descriptor)?;
        self.submit_validated(&frame)
    }

    /// Validates and cancels one frame before native allocation or submission.
    ///
    /// # Errors
    ///
    /// Returns a validation, admission, or accounting error. A successful
    /// cancellation performs no native allocation, upload, or submission.
    pub fn cancel_offscreen(
        &mut self,
        scene: &Scene,
        descriptor: OffscreenDescriptor,
    ) -> Result<CancellationReport, RenderError> {
        let frame = self.admit_frame(scene, descriptor)?;
        verify_cancellation_lifecycle()?;
        self.accounting
            .record_accepted(
                &frame,
                AccountingOutcome::Cancelled,
                false,
                FrameResourceUsage::default(),
            )
            .map_err(|()| RenderError::AccountingOverflow)?;
        Ok(CancellationReport {
            generation: self.accounting.generation(),
            primitives: frame.consumed_primitives(),
            omitted_primitives: frame.omitted_primitives(),
            uploaded_bytes_avoided: frame.upload_bytes(),
        })
    }

    /// Stops admitting work after the synchronous backend is fully drained.
    pub fn shutdown(&mut self) {
        self.accounting.stop();
    }

    /// Consumes a device-lost backend and creates the next generation.
    ///
    /// # Errors
    ///
    /// Returns an error unless this generation is device-lost, its sequence can
    /// advance, and the replacement native backend initializes successfully.
    pub fn recover(self) -> Result<Self, RecoveryError> {
        let state = self.accounting.state();
        let generation = self.accounting.generation();
        if state != BackendState::DeviceLost {
            return Err(RecoveryError::BackendNotDeviceLost { generation, state });
        }
        let next = generation
            .next()
            .ok_or(RecoveryError::GenerationExhausted)?;
        crate::initialization::new_backend_generation(next).map_err(RecoveryError::Initialization)
    }

    /// Returns the number of command buffers committed by this backend.
    #[must_use]
    pub const fn submission_count(&self) -> u64 {
        self.accounting.submitted_frames()
    }

    fn submit_validated(&mut self, frame: &ValidatedFrame) -> Result<OffscreenFrame, RenderError> {
        let Some(next_submission) = self.accounting.submitted_frames().checked_add(1) else {
            self.accounting
                .record_accepted(
                    frame,
                    AccountingOutcome::Failed,
                    false,
                    FrameResourceUsage::default(),
                )
                .map_err(|()| RenderError::AccountingOverflow)?;
            return Err(RenderError::SubmissionSequenceExhausted);
        };
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        let attempt = objc2::rc::autoreleasepool(|_| self.native.render(frame));
        #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
        let attempt = self.native.render(frame);
        record_attempt(&mut self.accounting, frame, &attempt)?;
        complete_attempt(next_submission, frame, attempt)
    }

    fn admit_frame(
        &mut self,
        scene: &Scene,
        descriptor: OffscreenDescriptor,
    ) -> Result<ValidatedFrame, RenderError> {
        if self.accounting.state() != BackendState::Ready {
            self.accounting
                .record_admission_rejection()
                .map_err(|()| RenderError::AccountingOverflow)?;
            return Err(RenderError::BackendUnavailable {
                generation: self.accounting.generation(),
                state: self.accounting.state(),
            });
        }
        match ValidatedFrame::new(scene, descriptor) {
            Ok(frame) => Ok(frame),
            Err(error) => {
                self.accounting
                    .record_validation_rejection()
                    .map_err(|()| RenderError::AccountingOverflow)?;
                Err(RenderError::Validation(error))
            }
        }
    }
}

fn record_attempt(
    accounting: &mut crate::BackendAccounting,
    frame: &ValidatedFrame,
    attempt: &NativeRenderAttempt,
) -> Result<(), RenderError> {
    let outcome = verify_attempt_lifecycle(attempt)?;
    let result = accounting
        .record_accepted(frame, outcome, attempt.committed, attempt.usage)
        .map_err(|()| RenderError::AccountingOverflow);
    if attempt.device_lost {
        accounting.invalidate_device();
    } else if result.is_err() && attempt.committed {
        accounting.stop();
    }
    result
}

fn verify_cancellation_lifecycle() -> Result<(), RenderError> {
    let mut lifecycle = FrameLifecycle::new();
    lifecycle
        .apply(LifecycleAction::BeginFrame)
        .and_then(|()| lifecycle.apply(LifecycleAction::CancelBeforeSubmit))
        .map_err(|_| RenderError::SubmissionInvariantViolated)?;
    if lifecycle.invariants_hold() && lifecycle.release_count() == 1 {
        Ok(())
    } else {
        Err(RenderError::SubmissionInvariantViolated)
    }
}

fn verify_attempt_lifecycle(
    attempt: &NativeRenderAttempt,
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
    let outcome = match (&attempt.result, attempt.committed) {
        (Ok(_), true) => {
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
        (Ok(_), false) => return Err(RenderError::SubmissionInvariantViolated),
    };
    if lifecycle.invariants_hold() && lifecycle.release_count() == 1 {
        Ok(outcome)
    } else {
        Err(RenderError::SubmissionInvariantViolated)
    }
}

impl Renderer for MetalBackend {
    type Error = RenderError;
    type Target = OffscreenTarget;

    fn capabilities(&self) -> RendererCapabilities {
        RendererCapabilities {
            max_texture_dimension_2d: crate::MAX_METAL3_TEXTURE_DIMENSION_2D,
            timestamps: false,
            offscreen_readback: true,
        }
    }

    fn render(
        &mut self,
        scene: &Scene,
        target: &mut Self::Target,
    ) -> Result<FrameReport, Self::Error> {
        let result = self.render_offscreen(scene, target.descriptor);
        store_render_result(target, result)
    }
}

fn complete_attempt(
    next_submission: u64,
    frame: &ValidatedFrame,
    attempt: NativeRenderAttempt,
) -> Result<OffscreenFrame, RenderError> {
    let image = attempt.result?;
    if !attempt.committed {
        return Err(RenderError::SubmissionInvariantViolated);
    }

    Ok(OffscreenFrame {
        image,
        report: FrameReport {
            submission: next_submission,
            primitives: frame.consumed_primitives(),
            omitted_primitives: frame.omitted_primitives(),
            draw_calls: usize::from(!frame.quads().is_empty()),
            uploaded_bytes: frame.upload_bytes(),
            allocated_bytes: attempt.usage.allocated_bytes,
            retained_bytes: attempt.usage.peak_retained_bytes,
            readback_bytes: attempt.usage.readback_bytes,
        },
    })
}

fn store_render_result(
    target: &mut OffscreenTarget,
    result: Result<OffscreenFrame, RenderError>,
) -> Result<FrameReport, RenderError> {
    target.image = None;
    let completed = result?;
    let report = completed.report;
    target.image = Some(completed.image);
    Ok(report)
}

#[cfg(any(test, all(target_os = "macos", target_arch = "aarch64")))]
pub(crate) fn compact_readback(
    frame: &ValidatedFrame,
    padded: &[u8],
) -> Result<Bgra8Image, RenderError> {
    compact_readback_with_control(frame, padded, false)
}

#[cfg(any(test, all(target_os = "macos", target_arch = "aarch64")))]
fn map_readback_reservation_failure(
    result: &Result<(), std::collections::TryReserveError>,
    bytes: usize,
) -> Result<(), RenderError> {
    match result {
        Ok(()) => Ok(()),
        Err(_) => Err(RenderError::ReadbackAllocationFailed { bytes }),
    }
}

#[cfg(any(test, all(target_os = "macos", target_arch = "aarch64")))]
fn compact_readback_with_control(
    frame: &ValidatedFrame,
    padded: &[u8],
    fail_reservation: bool,
) -> Result<Bgra8Image, RenderError> {
    let descriptor = frame.descriptor();
    let layout = frame.readback_layout();
    if padded.len() != layout.buffer_len() {
        return Err(RenderError::ReadbackLengthMismatch {
            expected: layout.buffer_len(),
            actual: padded.len(),
        });
    }

    let mut compact = Vec::new();
    let reservation = if fail_reservation {
        compact.try_reserve_exact(usize::MAX)
    } else {
        compact.try_reserve_exact(layout.compact_len())
    };
    map_readback_reservation_failure(&reservation, layout.compact_len())?;
    for row in padded.chunks_exact(layout.aligned_bytes_per_row()) {
        compact.extend_from_slice(&row[..layout.compact_bytes_per_row()]);
    }

    Ok(Bgra8Image::from_compact_parts(
        descriptor.pixel_width(),
        descriptor.pixel_height(),
        compact,
    ))
}

#[cfg(test)]
mod tests {
    use std::error::Error as _;

    use alpine_core::{LinearRgba, Size};
    use alpine_renderer::FrameReport;
    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    use alpine_renderer::Renderer;
    use alpine_scene::{SceneBuilder, SceneRevision};

    use super::{
        CommandStatus, NativeRenderAttempt, OffscreenFrame, OffscreenTarget,
        RecoveryClassification, RenderError, RenderStage, compact_readback,
        compact_readback_with_control, complete_attempt, map_readback_reservation_failure,
        record_attempt, store_render_result,
    };
    use crate::{Bgra8Image, OffscreenDescriptor, ValidatedFrame, accounting::FrameResourceUsage};

    fn image(byte: u8) -> Bgra8Image {
        Bgra8Image::from_compact_parts(1, 1, vec![byte; 4])
    }

    fn usage() -> FrameResourceUsage {
        FrameResourceUsage {
            allocated_bytes: 512,
            peak_retained_bytes: 512,
            current_retained_bytes: 0,
            readback_bytes: 256,
        }
    }

    fn empty_frame(width: u16, height: u16) -> Result<ValidatedFrame, RenderError> {
        let viewport = Size::new(f32::from(width), f32::from(height))
            .ok_or(RenderError::SubmissionInvariantViolated)?;
        let scene = SceneBuilder::new(SceneRevision::new(1), viewport).finish();
        let clear =
            LinearRgba::new(0.0, 0.0, 0.0, 0.0).ok_or(RenderError::SubmissionInvariantViolated)?;
        let descriptor = OffscreenDescriptor::new(u32::from(width), u32::from(height), 1.0, clear)?;
        Ok(ValidatedFrame::new(&scene, descriptor)?)
    }

    #[test]
    fn compact_readback_strips_every_row_padding_byte() -> Result<(), RenderError> {
        let frame = empty_frame(2, 2)?;
        let layout = frame.readback_layout();
        let mut padded = vec![0xEE; layout.buffer_len()];
        padded[..8].copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
        let second = layout.aligned_bytes_per_row();
        padded[second..second + 8].copy_from_slice(&[9, 10, 11, 12, 13, 14, 15, 16]);

        let image = compact_readback(&frame, &padded)?;
        assert_eq!(image.width(), 2);
        assert_eq!(image.height(), 2);
        assert_eq!(
            image.bytes(),
            &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]
        );
        Ok(())
    }

    #[test]
    fn compact_readback_rejects_short_and_long_native_buffers() -> Result<(), RenderError> {
        let frame = empty_frame(1, 1)?;
        let expected = frame.readback_layout().buffer_len();
        for actual in [expected - 1, expected + 1] {
            let error = compact_readback(&frame, &vec![0; actual]).err();
            assert_eq!(
                error,
                Some(RenderError::ReadbackLengthMismatch { expected, actual })
            );
        }
        Ok(())
    }

    #[test]
    fn compact_readback_classifies_allocation_failure() -> Result<(), RenderError> {
        let frame = empty_frame(1, 1)?;
        let padded = vec![0; frame.readback_layout().buffer_len()];
        let mut impossible = Vec::<u8>::new();
        let reservation = impossible.try_reserve_exact(usize::MAX);

        assert_eq!(
            map_readback_reservation_failure(&reservation, 4),
            Err(RenderError::ReadbackAllocationFailed { bytes: 4 })
        );

        let result = compact_readback_with_control(&frame, &padded, true);

        assert_eq!(
            result,
            Err(RenderError::ReadbackAllocationFailed { bytes: 4 })
        );
        Ok(())
    }

    #[test]
    fn completed_attempt_reports_work_and_advances_sequence() -> Result<(), RenderError> {
        let frame = empty_frame(1, 1)?;
        let result = complete_attempt(
            5,
            &frame,
            NativeRenderAttempt {
                committed: true,
                device_lost: false,
                usage: usage(),
                result: Ok(image(7)),
            },
        );

        assert_eq!(
            result,
            Ok(OffscreenFrame {
                image: image(7),
                report: FrameReport {
                    submission: 5,
                    primitives: 0,
                    omitted_primitives: 0,
                    draw_calls: 0,
                    uploaded_bytes: 0,
                    allocated_bytes: 512,
                    retained_bytes: 512,
                    readback_bytes: 256,
                },
            })
        );
        assert_eq!(
            result.as_ref().map(|frame| frame.image().bytes()),
            Ok(&[7; 4][..])
        );
        assert_eq!(
            result.as_ref().map(OffscreenFrame::report),
            Ok(FrameReport {
                submission: 5,
                primitives: 0,
                omitted_primitives: 0,
                draw_calls: 0,
                uploaded_bytes: 0,
                allocated_bytes: 512,
                retained_bytes: 512,
                readback_bytes: 256,
            })
        );
        Ok(())
    }

    #[test]
    fn committed_failure_advances_sequence_without_returning_pixels() -> Result<(), RenderError> {
        let frame = empty_frame(1, 1)?;
        let result = complete_attempt(
            9,
            &frame,
            NativeRenderAttempt {
                committed: true,
                device_lost: false,
                usage: usage(),
                result: Err(RenderError::CommandFailed {
                    status: CommandStatus::Error,
                    failure: None,
                    recovery: RecoveryClassification::RetryFrame,
                }),
            },
        );

        assert!(matches!(result, Err(RenderError::CommandFailed { .. })));
        Ok(())
    }

    #[test]
    fn uncommitted_success_is_rejected_without_advancing_sequence() -> Result<(), RenderError> {
        let frame = empty_frame(1, 1)?;
        let result = complete_attempt(
            13,
            &frame,
            NativeRenderAttempt {
                committed: false,
                device_lost: false,
                usage: usage(),
                result: Ok(image(3)),
            },
        );

        assert_eq!(result.err(), Some(RenderError::SubmissionInvariantViolated));
        Ok(())
    }

    #[test]
    fn target_installs_only_completed_images_and_clears_stale_pixels() -> Result<(), RenderError> {
        let descriptor = empty_frame(1, 1)?.descriptor();
        let mut target = OffscreenTarget::new(descriptor);
        let report = FrameReport {
            submission: 2,
            primitives: 1,
            omitted_primitives: 0,
            draw_calls: 1,
            uploaded_bytes: 32,
            allocated_bytes: 512,
            retained_bytes: 512,
            readback_bytes: 256,
        };

        let returned = store_render_result(
            &mut target,
            Ok(OffscreenFrame {
                image: image(11),
                report,
            }),
        );
        assert_eq!(returned, Ok(report));
        assert_eq!(target.descriptor(), descriptor);
        assert_eq!(target.image().map(Bgra8Image::bytes), Some(&[11; 4][..]));
        assert_eq!(target.take_image(), Some(image(11)));
        assert!(target.image().is_none());

        assert_eq!(
            store_render_result(
                &mut target,
                Ok(OffscreenFrame {
                    image: image(12),
                    report,
                }),
            ),
            Ok(report)
        );

        let error = store_render_result(
            &mut target,
            Err(RenderError::UnsupportedPlatform {
                architecture: "fixture-arch",
                operating_system: "fixture-os",
            }),
        );
        assert!(matches!(
            error,
            Err(RenderError::UnsupportedPlatform { .. })
        ));
        assert!(target.image().is_none());
        assert!(target.take_image().is_none());
        Ok(())
    }

    #[test]
    fn small_readback_grid_strips_padding_without_row_aliasing() -> Result<(), RenderError> {
        for width in 1_u16..=16 {
            for height in 1_u16..=8 {
                let frame = empty_frame(width, height)?;
                let layout = frame.readback_layout();
                let mut padded = vec![0xFF; layout.buffer_len()];
                for row in 0..usize::from(height) {
                    let start = row * layout.aligned_bytes_per_row();
                    let end = start + layout.compact_bytes_per_row();
                    let value = u8::try_from(row + 1)
                        .map_err(|_| RenderError::SubmissionInvariantViolated)?;
                    padded[start..end].fill(value);
                }

                let image = compact_readback(&frame, &padded)?;

                for (row, compact_row) in image
                    .bytes()
                    .chunks_exact(layout.compact_bytes_per_row())
                    .enumerate()
                {
                    let expected = u8::try_from(row + 1)
                        .map_err(|_| RenderError::SubmissionInvariantViolated)?;
                    assert!(compact_row.iter().all(|byte| *byte == expected));
                }
            }
        }
        Ok(())
    }

    #[test]
    fn error_contract_preserves_stage_source_and_native_details() {
        let native =
            crate::NativeFailure::new("FixtureDomain".to_owned(), 9, "fixture failure".to_owned());
        let cases = [
            (
                RenderError::UnsupportedPlatform {
                    architecture: "fixture-arch",
                    operating_system: "fixture-os",
                },
                RenderStage::Validation,
                false,
            ),
            (
                RenderError::SubmissionSequenceExhausted,
                RenderStage::SubmissionSequence,
                false,
            ),
            (
                RenderError::ResourceUnavailable {
                    stage: RenderStage::UploadBuffer,
                    requested_bytes: Some(32),
                },
                RenderStage::UploadBuffer,
                false,
            ),
            (
                RenderError::TextureExtentUnsupported {
                    width: 16_385,
                    height: 1,
                    limit: 16_384,
                },
                RenderStage::RenderTexture,
                false,
            ),
            (
                RenderError::EncoderUnavailable {
                    stage: RenderStage::BlitEncoder,
                },
                RenderStage::BlitEncoder,
                false,
            ),
            (
                RenderError::CommandFailed {
                    status: CommandStatus::Error,
                    failure: Some(native),
                    recovery: RecoveryClassification::RecreateBackend,
                },
                RenderStage::Completion,
                true,
            ),
            (
                RenderError::UnexpectedCommandStatus {
                    status: CommandStatus::Scheduled,
                },
                RenderStage::Completion,
                false,
            ),
            (
                RenderError::ReadbackLengthMismatch {
                    expected: 256,
                    actual: 4,
                },
                RenderStage::Readback,
                false,
            ),
            (
                RenderError::ReadbackAllocationFailed { bytes: 4 },
                RenderStage::Readback,
                false,
            ),
            (
                RenderError::SubmissionInvariantViolated,
                RenderStage::CommandBuffer,
                false,
            ),
        ];

        for (error, stage, has_source) in cases {
            assert_eq!(error.stage(), stage);
            assert_eq!(error.source().is_some(), has_source);
            assert!(!error.to_string().is_empty());
        }
        assert_eq!(CommandStatus::NotEnqueued, CommandStatus::NotEnqueued);
        assert_ne!(CommandStatus::Enqueued, CommandStatus::Committed);
        assert_ne!(CommandStatus::Completed, CommandStatus::Error);
        assert_eq!(CommandStatus::Unknown(27), CommandStatus::Unknown(27));
    }

    #[test]
    fn error_contract_formats_optional_failure_details() {
        let cases = [
            RenderError::Validation(crate::OffscreenError::ZeroPixelExtent),
            RenderError::ResourceUnavailable {
                stage: RenderStage::RenderTexture,
                requested_bytes: None,
            },
            RenderError::CommandFailed {
                status: CommandStatus::Error,
                failure: None,
                recovery: RecoveryClassification::RetryFrame,
            },
        ];

        for error in &cases {
            assert!(!error.to_string().is_empty());
        }
        assert!(cases[0].source().is_some());
        assert!(cases[1].source().is_none());
        assert!(cases[2].source().is_none());
    }

    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    fn portable_backend() -> crate::MetalBackend {
        let capabilities = crate::MetalCapabilities::new("Portable fixture".to_owned(), 1)
            .with_metal3(true)
            .with_unified_memory(true)
            .with_low_power(false)
            .with_removable(false);
        crate::MetalBackend::from_platform_parts((crate::unsupported::NativeBackend, capabilities))
    }

    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    #[test]
    fn portable_render_preserves_validation_order_and_submission_count() -> Result<(), RenderError>
    {
        let frame = empty_frame(1, 1)?;
        let descriptor = frame.descriptor();
        let scene = SceneBuilder::new(
            SceneRevision::new(1),
            Size::new(1.0, 1.0).ok_or(RenderError::SubmissionInvariantViolated)?,
        )
        .finish();
        let mut backend = portable_backend();

        assert_eq!(
            Renderer::capabilities(&backend),
            alpine_renderer::RendererCapabilities {
                max_texture_dimension_2d: 16_384,
                timestamps: false,
                offscreen_readback: true,
            }
        );

        let unsupported = backend.render_offscreen(&scene, descriptor).err();
        assert!(matches!(
            unsupported,
            Some(RenderError::UnsupportedPlatform { .. })
        ));
        assert_eq!(backend.submission_count(), 0);

        let mut target = OffscreenTarget::new(descriptor);
        assert_eq!(
            store_render_result(
                &mut target,
                Ok(OffscreenFrame {
                    image: image(21),
                    report: FrameReport::default(),
                }),
            ),
            Ok(FrameReport::default())
        );
        let trait_error = Renderer::render(&mut backend, &scene, &mut target);
        assert!(matches!(
            trait_error,
            Err(RenderError::UnsupportedPlatform { .. })
        ));
        assert!(target.image().is_none());
        assert_eq!(backend.submission_count(), 0);

        let mismatched = OffscreenDescriptor::new(2, 1, 1.0, descriptor.clear())?;
        let validation = backend.render_offscreen(&scene, mismatched).err();
        assert!(matches!(validation, Some(RenderError::Validation(_))));
        assert_eq!(backend.submission_count(), 0);
        Ok(())
    }

    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    #[test]
    fn exhausted_sequence_rejects_before_platform_submission() -> Result<(), RenderError> {
        let frame = empty_frame(1, 1)?;
        let scene = SceneBuilder::new(
            SceneRevision::new(1),
            Size::new(1.0, 1.0).ok_or(RenderError::SubmissionInvariantViolated)?,
        )
        .finish();
        let mut backend = portable_backend();
        backend.accounting.exhaust_submission_sequence();

        let error = backend.render_offscreen(&scene, frame.descriptor()).err();

        assert_eq!(error, Some(RenderError::SubmissionSequenceExhausted));
        assert_eq!(backend.submission_count(), u64::MAX);
        assert!(backend.accounting().invariants_hold());
        Ok(())
    }

    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    #[test]
    fn portable_cancellation_shutdown_and_recovery_contracts_are_balanced()
    -> Result<(), Box<dyn std::error::Error>> {
        let frame = empty_frame(1, 1)?;
        let scene = SceneBuilder::new(
            SceneRevision::new(1),
            Size::new(1.0, 1.0).ok_or(RenderError::SubmissionInvariantViolated)?,
        )
        .finish();
        let mut backend = portable_backend();

        let cancellation = backend.cancel_offscreen(&scene, frame.descriptor())?;
        assert_eq!(cancellation.generation().get(), 1);
        assert_eq!(cancellation.primitives(), 0);
        assert_eq!(cancellation.omitted_primitives(), 0);
        assert_eq!(cancellation.uploaded_bytes_avoided(), 0);
        assert_eq!(backend.accounting().cancelled_frames(), 1);
        assert_eq!(backend.submission_count(), 0);
        assert!(backend.accounting().invariants_hold());

        backend.shutdown();
        let stopped = backend
            .render_offscreen(&scene, frame.descriptor())
            .err()
            .ok_or("stopped backend must reject")?;
        assert!(matches!(
            stopped,
            RenderError::BackendUnavailable {
                state: crate::BackendState::Stopped,
                ..
            }
        ));
        assert_eq!(stopped.recovery(), RecoveryClassification::Stopped);
        assert!(backend.accounting().invariants_hold());
        assert!(matches!(
            backend.recover(),
            Err(super::RecoveryError::BackendNotDeviceLost {
                state: crate::BackendState::Stopped,
                ..
            })
        ));

        let mut lost = portable_backend();
        lost.accounting.invalidate_device();
        assert!(matches!(
            lost.recover(),
            Err(super::RecoveryError::Initialization(
                crate::InitializationError::UnsupportedPlatform { .. }
            ))
        ));
        Ok(())
    }

    #[test]
    fn attempt_recording_invalidates_device_loss_and_rejects_false_success()
    -> Result<(), RenderError> {
        let frame = empty_frame(1, 1)?;
        let mut accounting = crate::BackendAccounting::new(crate::BackendGeneration::INITIAL);
        let device_loss = NativeRenderAttempt {
            committed: true,
            device_lost: true,
            usage: usage(),
            result: Err(RenderError::CommandFailed {
                status: CommandStatus::Error,
                failure: None,
                recovery: RecoveryClassification::RecreateBackend,
            }),
        };
        record_attempt(&mut accounting, &frame, &device_loss)?;
        assert_eq!(accounting.state(), crate::BackendState::DeviceLost);
        assert!(accounting.invariants_hold());

        let mut false_success = crate::BackendAccounting::new(crate::BackendGeneration::INITIAL);
        let attempt = NativeRenderAttempt {
            committed: false,
            device_lost: false,
            usage: usage(),
            result: Ok(image(1)),
        };
        assert_eq!(
            record_attempt(&mut false_success, &frame, &attempt),
            Err(RenderError::SubmissionInvariantViolated)
        );
        assert_eq!(false_success.accepted_frames(), 0);

        let mut exhausted = crate::BackendAccounting::new(crate::BackendGeneration::INITIAL);
        exhausted.exhaust_render_sequence();
        let committed_success = NativeRenderAttempt {
            committed: true,
            device_lost: false,
            usage: usage(),
            result: Ok(image(2)),
        };
        assert_eq!(
            record_attempt(&mut exhausted, &frame, &committed_success),
            Err(RenderError::AccountingOverflow)
        );
        assert_eq!(exhausted.state(), crate::BackendState::Stopped);
        assert!(exhausted.invariants_hold());

        let mut exhausted_loss = crate::BackendAccounting::new(crate::BackendGeneration::INITIAL);
        exhausted_loss.exhaust_render_sequence();
        let committed_loss = NativeRenderAttempt {
            committed: true,
            device_lost: true,
            usage: usage(),
            result: Err(RenderError::CommandFailed {
                status: CommandStatus::Error,
                failure: None,
                recovery: RecoveryClassification::RecreateBackend,
            }),
        };
        assert_eq!(
            record_attempt(&mut exhausted_loss, &frame, &committed_loss),
            Err(RenderError::AccountingOverflow)
        );
        assert_eq!(exhausted_loss.state(), crate::BackendState::DeviceLost);
        assert!(exhausted_loss.invariants_hold());
        Ok(())
    }

    #[test]
    fn recovery_and_backend_errors_expose_stable_sources_and_messages() {
        let generation = crate::BackendGeneration::INITIAL;
        let errors = [
            super::RecoveryError::BackendNotDeviceLost {
                generation,
                state: crate::BackendState::Ready,
            },
            super::RecoveryError::GenerationExhausted,
            super::RecoveryError::Initialization(crate::InitializationError::DeviceUnavailable),
        ];
        assert!(errors[0].source().is_none());
        assert!(errors[1].source().is_none());
        assert!(errors[2].source().is_some());
        for error in errors {
            assert!(!error.to_string().is_empty());
        }

        let cases = [
            RenderError::BackendUnavailable {
                generation,
                state: crate::BackendState::Ready,
            },
            RenderError::BackendUnavailable {
                generation,
                state: crate::BackendState::DeviceLost,
            },
            RenderError::AccountingOverflow,
        ];
        assert_eq!(cases[0].recovery(), RecoveryClassification::Fatal);
        assert_eq!(cases[1].recovery(), RecoveryClassification::RecreateBackend);
        assert_eq!(cases[2].stage(), RenderStage::SubmissionSequence);
        for error in cases {
            assert!(!error.to_string().is_empty());
        }
    }
}
