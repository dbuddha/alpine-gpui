use dispatch2::DispatchData;
use objc2::{rc::Retained, runtime::ProtocolObject};
use objc2_foundation::{NSError, NSString};
use objc2_metal::{
    MTLBlendFactor, MTLCommandQueue, MTLCreateSystemDefaultDevice, MTLDevice, MTLFunction,
    MTLGPUFamily, MTLLibrary, MTLPixelFormat, MTLRenderPipelineDescriptor, MTLRenderPipelineState,
};

use crate::initialization::{
    FRAGMENT_ENTRY_POINT, InitializationDriver, InitializationError, Initialized,
    MetalCapabilities, NativeFailure, VERTEX_ENTRY_POINT, initialize,
};

static OFFLINE_LIBRARY: &[u8] = include_bytes!(env!("ALPINE_METALLIB_PATH"));

// SAFETY: This declaration only links the Apple system framework required by
// MTLCreateSystemDefaultDevice. It declares no callable foreign symbols.
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {}

type Device = Retained<ProtocolObject<dyn MTLDevice>>;
type Queue = Retained<ProtocolObject<dyn MTLCommandQueue>>;
type Library = Retained<ProtocolObject<dyn MTLLibrary>>;
type Function = Retained<ProtocolObject<dyn MTLFunction>>;
type Pipeline = Retained<ProtocolObject<dyn MTLRenderPipelineState>>;

pub(crate) struct NativeBackend {
    #[allow(
        dead_code,
        reason = "the initialized objects must remain retained for the backend lifetime"
    )]
    initialized: Initialized<NativeDriver>,
}

pub(crate) fn new_backend() -> Result<(NativeBackend, MetalCapabilities), InitializationError> {
    initialize(&NativeDriver::production()).map(|initialized| {
        let capabilities = initialized.capabilities.clone();
        (NativeBackend { initialized }, capabilities)
    })
}

struct NativeDriver {
    library: &'static [u8],
    vertex_name: &'static str,
    fragment_name: &'static str,
}

impl NativeDriver {
    const fn production() -> Self {
        Self {
            library: OFFLINE_LIBRARY,
            vertex_name: VERTEX_ENTRY_POINT,
            fragment_name: FRAGMENT_ENTRY_POINT,
        }
    }
}

impl InitializationDriver for NativeDriver {
    type Device = Device;
    type Function = Function;
    type Library = Library;
    type Pipeline = Pipeline;
    type Queue = Queue;

    fn create_device(&self) -> Option<Self::Device> {
        MTLCreateSystemDefaultDevice()
    }

    fn capabilities(&self, device: &Self::Device) -> Result<MetalCapabilities, NativeFailure> {
        Ok(
            MetalCapabilities::new(device.name().to_string(), device.registryID())
                .with_metal3(device.supportsFamily(MTLGPUFamily::Metal3))
                .with_unified_memory(device.hasUnifiedMemory())
                .with_low_power(device.isLowPower())
                .with_removable(device.isRemovable()),
        )
    }

    fn create_queue(&self, device: &Self::Device) -> Option<Self::Queue> {
        device.newCommandQueue()
    }

    fn load_library(&self, device: &Self::Device) -> Result<Self::Library, NativeFailure> {
        let data = DispatchData::from_static_bytes(self.library);
        device
            .newLibraryWithData_error(&data)
            .map_err(|error| copy_error(&error))
    }

    fn find_function(&self, library: &Self::Library, name: &'static str) -> Option<Self::Function> {
        let configured_name = if name == VERTEX_ENTRY_POINT {
            self.vertex_name
        } else {
            self.fragment_name
        };
        library.newFunctionWithName(&NSString::from_str(configured_name))
    }

    fn create_pipeline(
        &self,
        device: &Self::Device,
        vertex: &Self::Function,
        fragment: &Self::Function,
    ) -> Result<Self::Pipeline, NativeFailure> {
        let descriptor = MTLRenderPipelineDescriptor::new();
        descriptor.setVertexFunction(Some(vertex));
        descriptor.setFragmentFunction(Some(fragment));

        let attachments = descriptor.colorAttachments();
        // SAFETY: Metal render-pipeline descriptors always expose eight color
        // attachment slots, so fixed slot zero is within the documented range.
        let color = unsafe { attachments.objectAtIndexedSubscript(0) };
        color.setPixelFormat(MTLPixelFormat::BGRA8Unorm);
        color.setBlendingEnabled(true);
        color.setSourceRGBBlendFactor(MTLBlendFactor::One);
        color.setDestinationRGBBlendFactor(MTLBlendFactor::OneMinusSourceAlpha);
        color.setSourceAlphaBlendFactor(MTLBlendFactor::One);
        color.setDestinationAlphaBlendFactor(MTLBlendFactor::OneMinusSourceAlpha);

        device
            .newRenderPipelineStateWithDescriptor_error(&descriptor)
            .map_err(|error| copy_error(&error))
    }
}

fn copy_error(error: &NSError) -> NativeFailure {
    NativeFailure::new(
        error.domain().to_string(),
        i64::try_from(error.code()).unwrap_or(i64::MIN),
        error.localizedDescription().to_string(),
    )
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use crate::initialization::{
        InitializationError, InitializationStage, initialize_for_native_validation,
    };

    use super::{
        FRAGMENT_ENTRY_POINT, NativeDriver, OFFLINE_LIBRARY, VERTEX_ENTRY_POINT, new_backend,
    };

    static CORRUPT_LIBRARY: &[u8] = b"not a Metal library";

    #[test]
    fn production_initialization_enforces_the_device_baseline() -> Result<(), Box<dyn Error>> {
        match new_backend() {
            Ok((_backend, capabilities)) => {
                assert!(!capabilities.name().is_empty());
                assert_ne!(capabilities.registry_id(), 0);
                assert!(capabilities.supports_metal3());
                assert!(capabilities.has_unified_memory());
            }
            Err(InitializationError::UnsupportedDevice {
                device_name,
                reason,
            }) => {
                assert!(!device_name.is_empty());
                assert!(matches!(
                    reason,
                    "Metal 3 family support is required" | "unified memory is required"
                ));
            }
            Err(error) => return Err(error.into()),
        }

        Ok(())
    }

    #[test]
    fn creates_real_queue_library_functions_and_pipeline() -> Result<(), Box<dyn Error>> {
        let initialized = initialize_for_native_validation(&NativeDriver::production())?;

        assert!(!initialized.capabilities.name().is_empty());
        Ok(())
    }

    #[test]
    fn rejects_corrupt_offline_library_with_native_error() -> Result<(), Box<dyn Error>> {
        let driver = NativeDriver {
            library: CORRUPT_LIBRARY,
            vertex_name: VERTEX_ENTRY_POINT,
            fragment_name: FRAGMENT_ENTRY_POINT,
        };
        let Err(error) = initialize_for_native_validation(&driver) else {
            return Err("corrupt library unexpectedly initialized".into());
        };

        assert_eq!(error.stage(), InitializationStage::Library);
        assert!(error.source().is_some());
        Ok(())
    }

    #[test]
    fn rejects_absent_shader_entry_with_stage() -> Result<(), Box<dyn Error>> {
        let driver = NativeDriver {
            library: OFFLINE_LIBRARY,
            vertex_name: "alpine_missing_vertex",
            fragment_name: FRAGMENT_ENTRY_POINT,
        };
        let Err(error) = initialize_for_native_validation(&driver) else {
            return Err("missing entry point unexpectedly initialized".into());
        };

        assert_eq!(error.stage(), InitializationStage::VertexFunction);
        assert!(error.source().is_none());
        Ok(())
    }
}
