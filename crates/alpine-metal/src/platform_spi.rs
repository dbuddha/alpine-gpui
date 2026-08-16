//! Target-only bridge between Alpine's native macOS owner and Metal backend.
//!
//! This module is enabled only by the workspace platform implementation. It is
//! not an application contract and does not exist on portable targets.

use alpine_renderer::FrameReport;
use alpine_scene::Scene;
use objc2::{rc::Retained, runtime::ProtocolObject};
use objc2_metal::{MTLDevice, MTLDrawable, MTLTexture};

use crate::{
    InitializationError, MetalBackend, OffscreenDescriptor, RenderError,
    submission::{
        DrawableCompletionPoll as NativeCompletionPoll, DrawableRenderAttempt,
        DrawableSubmission as NativeSubmission, DrawableSubmitAttempt as NativeSubmitAttempt,
    },
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
pub struct DrawableSubmission(NativeSubmission);

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
    match backend.submit_callback_drawable(slot.get(), scene, descriptor, texture, drawable) {
        NativeSubmitAttempt::Rejected(attempt) => {
            DrawableSubmitAttempt::Rejected(DrawableAttempt::from_native(attempt))
        }
        NativeSubmitAttempt::Submitted(submission) => {
            DrawableSubmitAttempt::Submitted(DrawableSubmission(submission))
        }
    }
}

/// Polls one exact submission without waiting or exposing native handles.
pub fn poll_callback_drawable(
    backend: &mut MetalBackend,
    submission: DrawableSubmission,
) -> DrawableCompletionPoll {
    match backend.poll_callback_drawable(submission.0) {
        NativeCompletionPoll::Pending => DrawableCompletionPoll::Pending,
        NativeCompletionPoll::Complete(attempt) => {
            DrawableCompletionPoll::Complete(DrawableAttempt::from_native(attempt))
        }
    }
}

/// Returns exact bounded presentation-resource ownership evidence.
#[must_use]
pub fn presentation_snapshot(backend: &MetalBackend) -> PresentationSnapshot {
    PresentationSnapshot(backend.presentation_snapshot())
}

/// Releases free reusable uploads immediately and marks occupied slots to shed
/// their upload after terminal completion.
pub fn release_presentation_uploads_on_pressure(backend: &mut MetalBackend) {
    backend.release_presentation_uploads_on_pressure();
}

#[cfg(test)]
mod tests {
    use alpine_renderer::FrameReport;

    use crate::{RenderError, submission::DrawableRenderAttempt};

    use super::{DrawableAttempt, DrawableSlot};

    #[test]
    fn drawable_slots_admit_exactly_three_stable_indices() {
        assert_eq!(DrawableSlot::new(0).map(DrawableSlot::get), Some(0));
        assert_eq!(DrawableSlot::new(1).map(DrawableSlot::get), Some(1));
        assert_eq!(DrawableSlot::new(2).map(DrawableSlot::get), Some(2));
        assert_eq!(DrawableSlot::new(3), None);
        assert_eq!(DrawableSlot::new(u8::MAX), None);
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
