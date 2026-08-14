use crate::{
    InitializationError, MetalCapabilities, ValidatedFrame,
    accounting::FrameResourceUsage,
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
    pub(crate) fn render(&mut self, _frame: &ValidatedFrame) -> NativeRenderAttempt {
        NativeRenderAttempt {
            committed: false,
            device_lost: false,
            usage: FrameResourceUsage::default(),
            result: Err(RenderError::UnsupportedPlatform {
                architecture: std::env::consts::ARCH,
                operating_system: std::env::consts::OS,
            }),
        }
    }
}
