use crate::{
    InitializationError, MetalCapabilities, OffscreenStageTimings, ValidatedFrame,
    accounting::{FrameOperationUsage, FrameResourceUsage},
    submission::{NativeRenderAttempt, RenderError},
};

pub(crate) struct NativeBackend;

pub(crate) fn new_backend() -> Result<(NativeBackend, MetalCapabilities), InitializationError> {
    Err(InitializationError::UnsupportedPlatform {
        architecture: std::env::consts::ARCH,
        operating_system: std::env::consts::OS,
    })
}

impl NativeBackend {
    #[allow(
        clippy::unused_self,
        reason = "the portable stub preserves the native backend method boundary"
    )]
    pub(crate) fn render<const PROFILE: bool>(
        &mut self,
        _frame: &ValidatedFrame,
    ) -> NativeRenderAttempt {
        NativeRenderAttempt {
            committed: false,
            device_lost: false,
            operations: FrameOperationUsage::default(),
            resources: FrameResourceUsage::default(),
            timings: PROFILE.then(OffscreenStageTimings::default),
            result: Err(RenderError::UnsupportedPlatform {
                architecture: std::env::consts::ARCH,
                operating_system: std::env::consts::OS,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use alpine_core::{LinearRgba, Size};
    use alpine_scene::{SceneBuilder, SceneRevision};

    use super::{NativeBackend, new_backend};
    use crate::{
        OffscreenDescriptor, OffscreenStageTimings, RenderError, ValidatedFrame,
        accounting::{FrameOperationUsage, FrameResourceUsage},
    };

    fn frame() -> Result<ValidatedFrame, Box<dyn Error>> {
        let viewport = Size::new(1.0, 1.0).ok_or("fixture viewport must be valid")?;
        let scene = SceneBuilder::new(SceneRevision::new(1), viewport).finish();
        let clear =
            LinearRgba::new(0.0, 0.0, 0.0, 0.0).ok_or("fixture clear color must be valid")?;
        let descriptor = OffscreenDescriptor::new(1, 1, 1.0, clear)?;
        Ok(ValidatedFrame::new(&scene, descriptor)?)
    }

    #[test]
    fn portable_backend_preserves_exact_unsupported_results() -> Result<(), Box<dyn Error>> {
        assert!(matches!(
            new_backend(),
            Err(crate::InitializationError::UnsupportedPlatform {
                architecture,
                operating_system,
            }) if architecture == std::env::consts::ARCH
                && operating_system == std::env::consts::OS
        ));

        let mut backend = NativeBackend;
        let frame = frame()?;
        let unprofiled = backend.render::<false>(&frame);
        assert!(!unprofiled.committed);
        assert!(!unprofiled.device_lost);
        assert_eq!(unprofiled.operations, FrameOperationUsage::default());
        assert_eq!(unprofiled.resources, FrameResourceUsage::default());
        assert!(unprofiled.timings.is_none());
        assert!(matches!(
            unprofiled.result,
            Err(RenderError::UnsupportedPlatform {
                architecture,
                operating_system,
            }) if architecture == std::env::consts::ARCH
                && operating_system == std::env::consts::OS
        ));

        let profiled = backend.render::<true>(&frame);
        assert!(!profiled.committed);
        assert!(!profiled.device_lost);
        assert_eq!(profiled.operations, FrameOperationUsage::default());
        assert_eq!(profiled.resources, FrameResourceUsage::default());
        assert_eq!(profiled.timings, Some(OffscreenStageTimings::default()));
        assert!(matches!(
            profiled.result,
            Err(RenderError::UnsupportedPlatform {
                architecture,
                operating_system,
            }) if architecture == std::env::consts::ARCH
                && operating_system == std::env::consts::OS
        ));
        Ok(())
    }
}
