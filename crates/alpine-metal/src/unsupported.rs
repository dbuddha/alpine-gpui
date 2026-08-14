use crate::{InitializationError, MetalCapabilities};

pub(crate) struct NativeBackend;

pub(crate) fn new_backend() -> Result<(NativeBackend, MetalCapabilities), InitializationError> {
    Err(InitializationError::UnsupportedPlatform {
        architecture: std::env::consts::ARCH,
        operating_system: std::env::consts::OS,
    })
}
