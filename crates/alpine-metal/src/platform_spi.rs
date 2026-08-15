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
    submission::DrawableRenderAttempt,
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

#[cfg(test)]
mod tests {
    use alpine_renderer::FrameReport;

    use crate::{RenderError, submission::DrawableRenderAttempt};

    use super::DrawableAttempt;

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
