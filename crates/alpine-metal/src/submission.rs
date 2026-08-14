use std::{error::Error, fmt};

use alpine_renderer::{FrameReport, Renderer, RendererCapabilities};
use alpine_scene::Scene;

use crate::{
    Bgra8Image, NativeFailure, OffscreenDescriptor, OffscreenError, ValidatedFrame,
    initialization::MetalBackend,
};

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
    /// The next submission sequence could not be represented.
    SubmissionSequenceExhausted,
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
            Self::Validation(_) | Self::UnsupportedPlatform { .. } => RenderStage::Validation,
            Self::SubmissionSequenceExhausted => RenderStage::SubmissionSequence,
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
            Self::SubmissionSequenceExhausted => {
                formatter.write_str("Metal submission sequence exhausted")
            }
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
            Self::CommandFailed { status, failure } => match failure {
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
            | Self::SubmissionSequenceExhausted
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
        let frame = ValidatedFrame::new(scene, descriptor)?;
        self.submit_validated(&frame)
    }

    /// Returns the number of command buffers committed by this backend.
    #[must_use]
    pub const fn submission_count(&self) -> u64 {
        self.submissions
    }

    fn submit_validated(&mut self, frame: &ValidatedFrame) -> Result<OffscreenFrame, RenderError> {
        let next_submission = self
            .submissions
            .checked_add(1)
            .ok_or(RenderError::SubmissionSequenceExhausted)?;
        let attempt = self.native.render(frame);
        complete_attempt(&mut self.submissions, next_submission, frame, attempt)
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
    submissions: &mut u64,
    next_submission: u64,
    frame: &ValidatedFrame,
    attempt: NativeRenderAttempt,
) -> Result<OffscreenFrame, RenderError> {
    if attempt.committed {
        *submissions = next_submission;
    }
    let image = attempt.result?;
    if !attempt.committed {
        return Err(RenderError::SubmissionInvariantViolated);
    }

    Ok(OffscreenFrame {
        image,
        report: FrameReport {
            submission: next_submission,
            primitives: frame.consumed_primitives(),
            draw_calls: usize::from(!frame.quads().is_empty()),
            uploaded_bytes: frame.upload_bytes(),
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
        CommandStatus, NativeRenderAttempt, OffscreenFrame, OffscreenTarget, RenderError,
        RenderStage, compact_readback, compact_readback_with_control, complete_attempt,
        map_readback_reservation_failure, store_render_result,
    };
    use crate::{Bgra8Image, OffscreenDescriptor, ValidatedFrame};

    fn image(byte: u8) -> Bgra8Image {
        Bgra8Image::from_compact_parts(1, 1, vec![byte; 4])
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
        let mut submissions = 4;

        let result = complete_attempt(
            &mut submissions,
            5,
            &frame,
            NativeRenderAttempt {
                committed: true,
                result: Ok(image(7)),
            },
        );

        assert_eq!(submissions, 5);
        assert_eq!(
            result,
            Ok(OffscreenFrame {
                image: image(7),
                report: FrameReport {
                    submission: 5,
                    primitives: 0,
                    draw_calls: 0,
                    uploaded_bytes: 0,
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
                draw_calls: 0,
                uploaded_bytes: 0,
            })
        );
        Ok(())
    }

    #[test]
    fn committed_failure_advances_sequence_without_returning_pixels() -> Result<(), RenderError> {
        let frame = empty_frame(1, 1)?;
        let mut submissions = 8;

        let result = complete_attempt(
            &mut submissions,
            9,
            &frame,
            NativeRenderAttempt {
                committed: true,
                result: Err(RenderError::CommandFailed {
                    status: CommandStatus::Error,
                    failure: None,
                }),
            },
        );

        assert!(matches!(result, Err(RenderError::CommandFailed { .. })));
        assert_eq!(submissions, 9);
        Ok(())
    }

    #[test]
    fn uncommitted_success_is_rejected_without_advancing_sequence() -> Result<(), RenderError> {
        let frame = empty_frame(1, 1)?;
        let mut submissions = 12;

        let result = complete_attempt(
            &mut submissions,
            13,
            &frame,
            NativeRenderAttempt {
                committed: false,
                result: Ok(image(3)),
            },
        );

        assert_eq!(result.err(), Some(RenderError::SubmissionInvariantViolated));
        assert_eq!(submissions, 12);
        Ok(())
    }

    #[test]
    fn target_installs_only_completed_images_and_clears_stale_pixels() -> Result<(), RenderError> {
        let descriptor = empty_frame(1, 1)?.descriptor();
        let mut target = OffscreenTarget::new(descriptor);
        let report = FrameReport {
            submission: 2,
            primitives: 1,
            draw_calls: 1,
            uploaded_bytes: 32,
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
        backend.submissions = u64::MAX;

        let error = backend.render_offscreen(&scene, frame.descriptor()).err();

        assert_eq!(error, Some(RenderError::SubmissionSequenceExhausted));
        assert_eq!(backend.submission_count(), u64::MAX);
        Ok(())
    }
}
