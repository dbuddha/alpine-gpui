#[cfg(test)]
use std::{cell::Cell, rc::Rc};
use std::{ffi::c_void, mem::size_of, ptr::NonNull, slice};

use dispatch2::DispatchData;
use objc2::{rc::Retained, runtime::ProtocolObject};
use objc2_foundation::{NSError, NSString};
use objc2_metal::{
    MTLBlendFactor, MTLBlitCommandEncoder, MTLBuffer, MTLClearColor, MTLCommandBuffer,
    MTLCommandBufferError, MTLCommandBufferStatus, MTLCommandEncoder, MTLCommandQueue,
    MTLCreateSystemDefaultDevice, MTLDevice, MTLFunction, MTLGPUFamily, MTLLibrary, MTLLoadAction,
    MTLOrigin, MTLPixelFormat, MTLPrimitiveType, MTLRenderCommandEncoder, MTLRenderPassDescriptor,
    MTLRenderPipelineDescriptor, MTLRenderPipelineState, MTLResource, MTLResourceOptions, MTLSize,
    MTLStorageMode, MTLStoreAction, MTLTexture, MTLTextureDescriptor, MTLTextureUsage,
};

use crate::initialization::{
    FRAGMENT_ENTRY_POINT, InitializationDriver, InitializationError, Initialized,
    MetalCapabilities, NativeFailure, VERTEX_ENTRY_POINT, initialize,
};
use crate::submission::{
    CommandStatus, NativeRenderAttempt, RecoveryClassification, RenderError, RenderStage,
    compact_readback,
};
use crate::{
    Bgra8Image, MAX_METAL3_TEXTURE_DIMENSION_2D, ValidatedFrame,
    accounting::{FrameOperationUsage, FrameResourceUsage},
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
type Buffer = Retained<ProtocolObject<dyn MTLBuffer>>;
type CommandBuffer = Retained<ProtocolObject<dyn MTLCommandBuffer>>;
type Texture = Retained<ProtocolObject<dyn MTLTexture>>;

#[derive(Clone, Copy)]
enum BlendConfiguration {
    PremultipliedSourceOver,
    #[cfg(test)]
    DisabledFaultControl,
}

pub(crate) struct NativeBackend {
    #[allow(
        dead_code,
        reason = "the initialized objects must remain retained for the backend lifetime"
    )]
    initialized: Initialized<NativeDriver>,
    #[cfg(test)]
    fault: NativeFault,
    #[cfg(test)]
    probe: ResourceProbe,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NativeFault {
    None,
    TextureAllocation,
    ReadbackAllocation,
    UploadAllocation,
    CommandBuffer,
    RenderEncoder,
    BlitEncoder,
    TerminalError(i64),
    UnexpectedStatus,
    ReadbackLength,
}

#[cfg(test)]
#[derive(Clone, Default)]
struct ResourceProbe(Rc<ResourceProbeState>);

#[cfg(test)]
#[derive(Default)]
struct ResourceProbeState {
    acquired: Cell<u64>,
    released: Cell<u64>,
    active: Cell<u64>,
}

#[cfg(test)]
impl ResourceProbe {
    fn acquire(&self) -> ResourceLease {
        self.0.acquired.set(self.0.acquired.get() + 1);
        self.0.active.set(self.0.active.get() + 1);
        ResourceLease(self.clone())
    }

    fn counts(&self) -> (u64, u64, u64) {
        (
            self.0.acquired.get(),
            self.0.released.get(),
            self.0.active.get(),
        )
    }
}

#[cfg(test)]
struct ResourceLease(ResourceProbe);

#[cfg(test)]
impl Drop for ResourceLease {
    fn drop(&mut self) {
        self.0.0.released.set(self.0.0.released.get() + 1);
        self.0.0.active.set(self.0.0.active.get() - 1);
    }
}

impl NativeBackend {
    #[allow(
        clippy::cast_precision_loss,
        clippy::too_many_lines,
        reason = "the linear command ownership protocol remains visible in one audited boundary"
    )]
    pub(crate) fn render(&mut self, frame: &ValidatedFrame) -> NativeRenderAttempt {
        let resources = match FrameResources::new(
            &self.initialized.device,
            frame,
            #[cfg(test)]
            self.fault,
            #[cfg(test)]
            &self.probe,
        ) {
            Ok(resources) => resources,
            Err(failure) => {
                return NativeRenderAttempt {
                    committed: false,
                    device_lost: false,
                    operations: FrameOperationUsage::default(),
                    resources: failure.usage,
                    result: Err(failure.error),
                };
            }
        };
        let resource_usage = resources.usage;
        let mut operations = FrameOperationUsage {
            draw_calls: 0,
            uploaded_bytes: resources
                .upload
                .as_ref()
                .map_or(0, |_| frame.upload_bytes()),
        };
        #[cfg(test)]
        if self.fault == NativeFault::CommandBuffer {
            return NativeRenderAttempt {
                committed: false,
                device_lost: false,
                operations,
                resources: resource_usage,
                result: Err(RenderError::ResourceUnavailable {
                    stage: RenderStage::CommandBuffer,
                    requested_bytes: None,
                }),
            };
        }
        let Some(command) = self.initialized.queue.commandBuffer() else {
            return NativeRenderAttempt {
                committed: false,
                device_lost: false,
                operations,
                resources: resource_usage,
                result: Err(RenderError::ResourceUnavailable {
                    stage: RenderStage::CommandBuffer,
                    requested_bytes: None,
                }),
            };
        };

        #[cfg(test)]
        if self.fault == NativeFault::RenderEncoder {
            return NativeRenderAttempt {
                committed: false,
                device_lost: false,
                operations,
                resources: resource_usage,
                result: Err(RenderError::EncoderUnavailable {
                    stage: RenderStage::RenderEncoder,
                }),
            };
        }

        if let Err(error) =
            encode_render_pass(&command, &self.initialized.pipeline, &resources, frame)
        {
            return NativeRenderAttempt {
                committed: false,
                device_lost: false,
                operations,
                resources: resource_usage,
                result: Err(error),
            };
        }
        operations.draw_calls = usize::from(!frame.quads().is_empty());
        #[cfg(test)]
        if self.fault == NativeFault::BlitEncoder {
            return NativeRenderAttempt {
                committed: false,
                device_lost: false,
                operations,
                resources: resource_usage,
                result: Err(RenderError::EncoderUnavailable {
                    stage: RenderStage::BlitEncoder,
                }),
            };
        }
        if let Err(error) = encode_readback(&command, &resources, frame) {
            return NativeRenderAttempt {
                committed: false,
                device_lost: false,
                operations,
                resources: resource_usage,
                result: Err(error),
            };
        }

        command.commit();
        command.waitUntilCompleted();
        let status = command_status(command.status());
        let terminal = match status {
            CommandStatus::Completed => (read_compact_image(&resources.readback, frame), false),
            CommandStatus::Error => {
                let failure = command.error().map(|error| copy_error(&error));
                let (recovery, device_lost) = classify_command_failure(failure.as_ref());
                (
                    Err(RenderError::CommandFailed {
                        status,
                        failure,
                        recovery,
                    }),
                    device_lost,
                )
            }
            status => (Err(RenderError::UnexpectedCommandStatus { status }), false),
        };
        #[cfg(test)]
        let terminal = injected_terminal_result(self.fault, terminal.0, frame);
        let (result, device_lost) = terminal;

        NativeRenderAttempt {
            committed: true,
            device_lost,
            operations,
            resources: resource_usage,
            result,
        }
    }
}

pub(crate) fn new_backend() -> Result<(NativeBackend, MetalCapabilities), InitializationError> {
    initialize(&NativeDriver::production()).map(|initialized| {
        let capabilities = initialized.capabilities.clone();
        (
            NativeBackend {
                initialized,
                #[cfg(test)]
                fault: NativeFault::None,
                #[cfg(test)]
                probe: ResourceProbe::default(),
            },
            capabilities,
        )
    })
}

struct NativeDriver {
    library: &'static [u8],
    vertex_name: &'static str,
    fragment_name: &'static str,
    blend: BlendConfiguration,
}

impl NativeDriver {
    const fn production() -> Self {
        Self {
            library: OFFLINE_LIBRARY,
            vertex_name: VERTEX_ENTRY_POINT,
            fragment_name: FRAGMENT_ENTRY_POINT,
            blend: BlendConfiguration::PremultipliedSourceOver,
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
        match self.blend {
            BlendConfiguration::PremultipliedSourceOver => {
                color.setBlendingEnabled(true);
                color.setSourceRGBBlendFactor(MTLBlendFactor::One);
                color.setDestinationRGBBlendFactor(MTLBlendFactor::OneMinusSourceAlpha);
                color.setSourceAlphaBlendFactor(MTLBlendFactor::One);
                color.setDestinationAlphaBlendFactor(MTLBlendFactor::OneMinusSourceAlpha);
            }
            #[cfg(test)]
            BlendConfiguration::DisabledFaultControl => color.setBlendingEnabled(false),
        }

        device
            .newRenderPipelineStateWithDescriptor_error(&descriptor)
            .map_err(|error| copy_error(&error))
    }
}

struct FrameResources {
    texture: Texture,
    readback: Buffer,
    upload: Option<Buffer>,
    usage: FrameResourceUsage,
    #[cfg(test)]
    _lease: ResourceLease,
}

struct ResourceBuildFailure {
    error: RenderError,
    usage: FrameResourceUsage,
}

impl FrameResources {
    #[allow(
        clippy::too_many_lines,
        reason = "partial native allocation and its exact accounting remain one linear transaction"
    )]
    fn new(
        device: &Device,
        frame: &ValidatedFrame,
        #[cfg(test)] fault: NativeFault,
        #[cfg(test)] probe: &ResourceProbe,
    ) -> Result<Self, ResourceBuildFailure> {
        #[cfg(test)]
        let lease = probe.acquire();
        let descriptor = frame.descriptor();
        if descriptor.pixel_width() > MAX_METAL3_TEXTURE_DIMENSION_2D
            || descriptor.pixel_height() > MAX_METAL3_TEXTURE_DIMENSION_2D
        {
            return Err(ResourceBuildFailure {
                error: RenderError::TextureExtentUnsupported {
                    width: descriptor.pixel_width(),
                    height: descriptor.pixel_height(),
                    limit: MAX_METAL3_TEXTURE_DIMENSION_2D,
                },
                usage: FrameResourceUsage::default(),
            });
        }
        #[cfg(test)]
        if fault == NativeFault::TextureAllocation {
            return Err(ResourceBuildFailure {
                error: RenderError::ResourceUnavailable {
                    stage: RenderStage::RenderTexture,
                    requested_bytes: None,
                },
                usage: FrameResourceUsage::default(),
            });
        }
        // SAFETY: OffscreenDescriptor rejects zero dimensions and dimensions
        // above the Metal 3 family guarantee before this native call.
        let texture_descriptor = unsafe {
            MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                MTLPixelFormat::BGRA8Unorm,
                descriptor.pixel_width() as usize,
                descriptor.pixel_height() as usize,
                false,
            )
        };
        texture_descriptor.setStorageMode(MTLStorageMode::Private);
        texture_descriptor.setUsage(MTLTextureUsage::RenderTarget);
        let texture = device
            .newTextureWithDescriptor(&texture_descriptor)
            .ok_or_else(|| ResourceBuildFailure {
                error: RenderError::ResourceUnavailable {
                    stage: RenderStage::RenderTexture,
                    requested_bytes: None,
                },
                usage: FrameResourceUsage::default(),
            })?;
        let texture_bytes = texture.allocatedSize();

        let layout = frame.readback_layout();
        #[cfg(test)]
        if fault == NativeFault::ReadbackAllocation {
            return Err(ResourceBuildFailure {
                error: RenderError::ResourceUnavailable {
                    stage: RenderStage::ReadbackBuffer,
                    requested_bytes: Some(layout.buffer_len()),
                },
                usage: FrameResourceUsage {
                    allocated_bytes: texture_bytes,
                    peak_retained_bytes: texture_bytes,
                    current_retained_bytes: 0,
                    readback_bytes: 0,
                },
            });
        }
        let readback = device
            .newBufferWithLength_options(layout.buffer_len(), MTLResourceOptions::StorageModeShared)
            .ok_or_else(|| ResourceBuildFailure {
                error: RenderError::ResourceUnavailable {
                    stage: RenderStage::ReadbackBuffer,
                    requested_bytes: Some(layout.buffer_len()),
                },
                usage: FrameResourceUsage {
                    allocated_bytes: texture_bytes,
                    peak_retained_bytes: texture_bytes,
                    current_retained_bytes: 0,
                    readback_bytes: 0,
                },
            })?;
        let readback_bytes = readback.allocatedSize();
        let base_allocated =
            texture_bytes
                .checked_add(readback_bytes)
                .ok_or_else(|| ResourceBuildFailure {
                    error: RenderError::AccountingOverflow,
                    usage: FrameResourceUsage::default(),
                })?;

        let upload = if frame.quads().is_empty() {
            None
        } else {
            #[cfg(test)]
            if fault == NativeFault::UploadAllocation {
                return Err(ResourceBuildFailure {
                    error: RenderError::ResourceUnavailable {
                        stage: RenderStage::UploadBuffer,
                        requested_bytes: Some(frame.upload_bytes()),
                    },
                    usage: FrameResourceUsage {
                        allocated_bytes: base_allocated,
                        peak_retained_bytes: base_allocated,
                        current_retained_bytes: 0,
                        readback_bytes: readback.length(),
                    },
                });
            }
            let first = NonNull::from(&frame.quads()[0]).cast::<c_void>();
            // SAFETY: `first` points to `frame.upload_bytes()` initialized,
            // contiguous bytes because LoweredQuad is Copy and repr(C). Metal
            // copies those bytes before this call returns.
            unsafe {
                device.newBufferWithBytes_length_options(
                    first,
                    frame.upload_bytes(),
                    MTLResourceOptions::StorageModeShared,
                )
            }
            .ok_or_else(|| ResourceBuildFailure {
                error: RenderError::ResourceUnavailable {
                    stage: RenderStage::UploadBuffer,
                    requested_bytes: Some(frame.upload_bytes()),
                },
                usage: FrameResourceUsage {
                    allocated_bytes: base_allocated,
                    peak_retained_bytes: base_allocated,
                    current_retained_bytes: 0,
                    readback_bytes: readback.length(),
                },
            })?
            .into()
        };

        let allocated_bytes = upload.as_deref().map_or(Some(base_allocated), |upload| {
            base_allocated.checked_add(upload.allocatedSize())
        });
        let allocated_bytes = allocated_bytes.ok_or_else(|| ResourceBuildFailure {
            error: RenderError::AccountingOverflow,
            usage: FrameResourceUsage::default(),
        })?;
        let usage = FrameResourceUsage {
            allocated_bytes,
            peak_retained_bytes: allocated_bytes,
            current_retained_bytes: 0,
            readback_bytes: readback.length(),
        };

        Ok(Self {
            texture,
            readback,
            upload,
            usage,
            #[cfg(test)]
            _lease: lease,
        })
    }
}

#[allow(clippy::cast_precision_loss)]
fn encode_render_pass(
    command: &CommandBuffer,
    pipeline: &Pipeline,
    resources: &FrameResources,
    frame: &ValidatedFrame,
) -> Result<(), RenderError> {
    let pass = MTLRenderPassDescriptor::renderPassDescriptor();
    let attachments = pass.colorAttachments();
    // SAFETY: Metal render-pass descriptors always expose eight color
    // attachment slots, so fixed slot zero is within the documented range.
    let color = unsafe { attachments.objectAtIndexedSubscript(0) };
    color.setTexture(Some(&resources.texture));
    color.setLoadAction(MTLLoadAction::Clear);
    color.setStoreAction(MTLStoreAction::Store);
    let clear = frame.descriptor().clear();
    let alpha = f64::from(clear.alpha());
    color.setClearColor(MTLClearColor {
        red: f64::from(clear.red()) * alpha,
        green: f64::from(clear.green()) * alpha,
        blue: f64::from(clear.blue()) * alpha,
        alpha,
    });

    let encoder = command.renderCommandEncoderWithDescriptor(&pass).ok_or(
        RenderError::EncoderUnavailable {
            stage: RenderStage::RenderEncoder,
        },
    )?;
    encoder.setRenderPipelineState(pipeline);
    let viewport = [
        frame.descriptor().pixel_width() as f32,
        frame.descriptor().pixel_height() as f32,
    ];
    // SAFETY: `viewport` contains exactly two initialized f32 values and Metal
    // copies all eight bytes into fixed shader buffer index zero immediately.
    unsafe {
        encoder.setVertexBytes_length_atIndex(
            NonNull::from(&viewport).cast::<c_void>(),
            size_of::<[f32; 2]>(),
            0,
        );
    }
    if let Some(upload) = resources.upload.as_deref() {
        // SAFETY: The retained upload buffer contains exactly the validated
        // LoweredQuad slice, offset zero is aligned, shader index one is fixed,
        // and both the local owner and retained command buffer keep it alive.
        unsafe {
            encoder.setVertexBuffer_offset_atIndex(Some(upload), 0, 1);
            encoder.drawPrimitives_vertexStart_vertexCount_instanceCount(
                MTLPrimitiveType::Triangle,
                0,
                6,
                frame.quads().len(),
            );
        }
    }
    encoder.endEncoding();
    Ok(())
}

fn encode_readback(
    command: &CommandBuffer,
    resources: &FrameResources,
    frame: &ValidatedFrame,
) -> Result<(), RenderError> {
    let encoder = command
        .blitCommandEncoder()
        .ok_or(RenderError::EncoderUnavailable {
            stage: RenderStage::BlitEncoder,
        })?;
    let descriptor = frame.descriptor();
    let layout = frame.readback_layout();
    // SAFETY: The validated extent is within the allocated texture, the
    // aligned row pitch and total capacity were checked in pure Rust, offset
    // zero is valid, and retained resources outlive command completion.
    unsafe {
        encoder.copyFromTexture_sourceSlice_sourceLevel_sourceOrigin_sourceSize_toBuffer_destinationOffset_destinationBytesPerRow_destinationBytesPerImage(
            &resources.texture,
            0,
            0,
            MTLOrigin { x: 0, y: 0, z: 0 },
            MTLSize {
                width: descriptor.pixel_width() as usize,
                height: descriptor.pixel_height() as usize,
                depth: 1,
            },
            &resources.readback,
            0,
            layout.aligned_bytes_per_row(),
            layout.buffer_len(),
        );
    }
    encoder.endEncoding();
    Ok(())
}

fn command_status(status: MTLCommandBufferStatus) -> CommandStatus {
    match status {
        MTLCommandBufferStatus::NotEnqueued => CommandStatus::NotEnqueued,
        MTLCommandBufferStatus::Enqueued => CommandStatus::Enqueued,
        MTLCommandBufferStatus::Committed => CommandStatus::Committed,
        MTLCommandBufferStatus::Scheduled => CommandStatus::Scheduled,
        MTLCommandBufferStatus::Completed => CommandStatus::Completed,
        MTLCommandBufferStatus::Error => CommandStatus::Error,
        MTLCommandBufferStatus(value) => CommandStatus::Unknown(value),
    }
}

fn read_compact_image(
    readback: &Buffer,
    frame: &ValidatedFrame,
) -> Result<Bgra8Image, RenderError> {
    let expected = frame.readback_layout().buffer_len();
    let actual = readback.length();
    if actual != expected {
        return Err(RenderError::ReadbackLengthMismatch { expected, actual });
    }
    // SAFETY: The retained shared buffer exposes `actual` initialized bytes.
    // The command buffer reached Completed before this function is called, so
    // GPU writes are terminal and CPU access cannot race them.
    let padded =
        unsafe { slice::from_raw_parts(readback.contents().cast::<u8>().as_ptr(), actual) };
    compact_readback(frame, padded)
}

fn copy_error(error: &NSError) -> NativeFailure {
    NativeFailure::new(
        error.domain().to_string(),
        i64::try_from(error.code()).unwrap_or(i64::MIN),
        error.localizedDescription().to_string(),
    )
}

fn classify_command_failure(failure: Option<&NativeFailure>) -> (RecoveryClassification, bool) {
    let Some(failure) = failure else {
        return (RecoveryClassification::Fatal, false);
    };
    if failure.domain() != "MTLCommandBufferErrorDomain" {
        return (RecoveryClassification::Fatal, false);
    }
    let code = failure.code();
    let device_removed = i64::try_from(MTLCommandBufferError::DeviceRemoved.0).ok();
    let access_revoked = i64::try_from(MTLCommandBufferError::AccessRevoked.0).ok();
    if Some(code) == device_removed || Some(code) == access_revoked {
        return (RecoveryClassification::RecreateBackend, true);
    }
    let out_of_memory = i64::try_from(MTLCommandBufferError::OutOfMemory.0).ok();
    let memoryless = i64::try_from(MTLCommandBufferError::Memoryless.0).ok();
    if Some(code) == out_of_memory || Some(code) == memoryless {
        return (RecoveryClassification::RetryFrame, false);
    }
    let not_permitted = i64::try_from(MTLCommandBufferError::NotPermitted.0).ok();
    if Some(code) == not_permitted {
        return (RecoveryClassification::Unsupported, false);
    }
    (RecoveryClassification::Fatal, false)
}

#[cfg(test)]
fn injected_terminal_result(
    fault: NativeFault,
    result: Result<Bgra8Image, RenderError>,
    frame: &ValidatedFrame,
) -> (Result<Bgra8Image, RenderError>, bool) {
    match fault {
        NativeFault::TerminalError(code) => {
            let failure = NativeFailure::new(
                "MTLCommandBufferErrorDomain".to_owned(),
                code,
                "injected terminal command failure".to_owned(),
            );
            let (recovery, device_lost) = classify_command_failure(Some(&failure));
            (
                Err(RenderError::CommandFailed {
                    status: CommandStatus::Error,
                    failure: Some(failure),
                    recovery,
                }),
                device_lost,
            )
        }
        NativeFault::UnexpectedStatus => (
            Err(RenderError::UnexpectedCommandStatus {
                status: CommandStatus::Scheduled,
            }),
            false,
        ),
        NativeFault::ReadbackLength => (
            Err(RenderError::ReadbackLengthMismatch {
                expected: frame.readback_layout().buffer_len(),
                actual: frame.readback_layout().buffer_len() + 1,
            }),
            false,
        ),
        NativeFault::None
        | NativeFault::TextureAllocation
        | NativeFault::ReadbackAllocation
        | NativeFault::UploadAllocation
        | NativeFault::CommandBuffer
        | NativeFault::RenderEncoder
        | NativeFault::BlitEncoder => (result, false),
    }
}

#[cfg(test)]
mod tests {
    use std::{error::Error, process::Command};

    use alpine_core::{LinearRgba, Point, Rect, Size};
    use alpine_renderer::Renderer;
    use alpine_scene::{Primitive, Scene, SceneBuilder, SceneRevision};

    use crate::initialization::{
        InitializationError, InitializationStage, MetalBackend, initialize_for_native_validation,
    };
    use crate::{
        BackendState, Bgra8Image, OffscreenDescriptor, OffscreenTarget, RecoveryClassification,
        RenderError, RenderStage, ValidatedFrame,
    };

    use super::{
        BlendConfiguration, FRAGMENT_ENTRY_POINT, NativeBackend, NativeDriver, NativeFault,
        OFFLINE_LIBRARY, ResourceProbe, VERTEX_ENTRY_POINT, command_status, new_backend,
    };

    static CORRUPT_LIBRARY: &[u8] = b"not a Metal library";

    fn color(red: f32, green: f32, blue: f32, alpha: f32) -> Result<LinearRgba, &'static str> {
        LinearRgba::new(red, green, blue, alpha).ok_or("valid fixture color")
    }

    fn point(x: f32, y: f32) -> Result<Point, &'static str> {
        Point::new(x, y).ok_or("valid fixture point")
    }

    fn size(width: f32, height: f32) -> Result<Size, &'static str> {
        Size::new(width, height).ok_or("valid fixture size")
    }

    fn validation_backend(blend: BlendConfiguration) -> Result<MetalBackend, InitializationError> {
        validation_backend_with_fault(blend, NativeFault::None)
    }

    fn validation_backend_with_fault(
        blend: BlendConfiguration,
        fault: NativeFault,
    ) -> Result<MetalBackend, InitializationError> {
        validation_backend_and_probe(blend, fault).map(|(backend, _probe)| backend)
    }

    fn validation_backend_and_probe(
        blend: BlendConfiguration,
        fault: NativeFault,
    ) -> Result<(MetalBackend, ResourceProbe), InitializationError> {
        let driver = NativeDriver {
            library: OFFLINE_LIBRARY,
            vertex_name: VERTEX_ENTRY_POINT,
            fragment_name: FRAGMENT_ENTRY_POINT,
            blend,
        };
        let initialized = initialize_for_native_validation(&driver)?;
        let capabilities = initialized.capabilities.clone();
        let probe = ResourceProbe::default();
        let backend = MetalBackend::from_platform_parts((
            NativeBackend {
                initialized,
                fault,
                probe: probe.clone(),
            },
            capabilities,
        ));
        Ok((backend, probe))
    }

    fn assert_pixels_within(actual: &Bgra8Image, expected: &Bgra8Image, tolerance: u8) {
        assert_eq!(actual.width(), expected.width());
        assert_eq!(actual.height(), expected.height());
        assert_eq!(actual.bytes().len(), expected.bytes().len());
        for (index, (actual, expected)) in actual.bytes().iter().zip(expected.bytes()).enumerate() {
            assert!(
                actual.abs_diff(*expected) <= tolerance,
                "channel {index} differs: actual {actual}, expected {expected}, tolerance {tolerance}"
            );
        }
    }

    fn resident_bytes() -> Result<u64, Box<dyn Error>> {
        let pid = std::process::id().to_string();
        let output = Command::new("/bin/ps")
            .args(["-o", "rss=", "-p", pid.as_str()])
            .output()?;
        if !output.status.success() {
            return Err(format!("ps exited with {}", output.status).into());
        }
        let kibibytes = String::from_utf8(output.stdout)?.trim().parse::<u64>()?;
        Ok(kibibytes
            .checked_mul(1_024)
            .ok_or("resident byte overflow")?)
    }

    fn host_page_bytes() -> Result<u64, Box<dyn Error>> {
        let output = Command::new("/usr/bin/getconf").arg("PAGESIZE").output()?;
        if !output.status.success() {
            return Err(format!("getconf exited with {}", output.status).into());
        }
        Ok(String::from_utf8(output.stdout)?.trim().parse::<u64>()?)
    }

    fn discriminating_scene() -> Result<(Scene, OffscreenDescriptor), Box<dyn Error>> {
        let mut builder = SceneBuilder::new(SceneRevision::new(71), size(4.0, 3.0)?);
        builder.push(Primitive::Quad {
            bounds: Rect::new(point(0.0, 0.0)?, size(4.0, 3.0)?),
            color: color(1.0, 0.0, 0.0, 0.5)?,
        });
        builder.push(Primitive::Quad {
            bounds: Rect::new(point(1.0, 0.5)?, size(2.0, 2.0)?),
            color: color(0.0, 0.0, 1.0, 0.5)?,
        });
        builder.push(Primitive::Quad {
            bounds: Rect::new(point(-1.0, 2.0)?, size(2.0, 2.0)?),
            color: color(1.0, 1.0, 1.0, 1.0)?,
        });
        builder.push(Primitive::Quad {
            bounds: Rect::new(point(8.0, 8.0)?, size(1.0, 1.0)?),
            color: color(0.0, 1.0, 0.0, 1.0)?,
        });
        let descriptor = OffscreenDescriptor::new(4, 3, 1.0, color(0.0, 1.0, 0.0, 0.25)?)?;
        Ok((builder.finish(), descriptor))
    }

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
            blend: super::BlendConfiguration::PremultipliedSourceOver,
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
            blend: super::BlendConfiguration::PremultipliedSourceOver,
        };
        let Err(error) = initialize_for_native_validation(&driver) else {
            return Err("missing entry point unexpectedly initialized".into());
        };

        assert_eq!(error.stage(), InitializationStage::VertexFunction);
        assert!(error.source().is_none());
        Ok(())
    }

    #[test]
    fn renders_discriminating_scene_once_and_matches_cpu_oracle() -> Result<(), Box<dyn Error>> {
        let (scene, descriptor) = discriminating_scene()?;
        let expected = ValidatedFrame::new(&scene, descriptor)?.reference_image()?;
        let mut backend = validation_backend(BlendConfiguration::PremultipliedSourceOver)?;

        let completed = backend.render_offscreen(&scene, descriptor)?;

        assert_eq!(backend.submission_count(), 1);
        assert_eq!(completed.report().submission, 1);
        assert_eq!(completed.report().primitives, 4);
        assert_eq!(completed.report().draw_calls, 1);
        assert_eq!(completed.report().uploaded_bytes, 3 * 32);
        assert_pixels_within(completed.image(), &expected, 1);

        let mut target = OffscreenTarget::new(descriptor);
        assert!(target.image().is_none());
        let second = backend.render(&scene, &mut target)?;
        assert_eq!(backend.submission_count(), 2);
        assert_eq!(second.submission, 2);
        let renderer_capabilities = Renderer::capabilities(&backend);
        assert_eq!(renderer_capabilities.max_texture_dimension_2d, 16_384);
        assert!(renderer_capabilities.offscreen_readback);
        assert!(!renderer_capabilities.timestamps);
        let image = target.take_image().ok_or("completed target image")?;
        assert_pixels_within(&image, &expected, 1);
        assert!(target.image().is_none());
        Ok(())
    }

    #[test]
    fn renders_empty_scene_as_premultiplied_clear() -> Result<(), Box<dyn Error>> {
        let scene = SceneBuilder::new(SceneRevision::new(72), size(2.0, 2.0)?).finish();
        let descriptor = OffscreenDescriptor::new(2, 2, 1.0, color(1.0, 0.5, 0.25, 0.5)?)?;
        let expected = ValidatedFrame::new(&scene, descriptor)?.reference_image()?;
        let mut backend = validation_backend(BlendConfiguration::PremultipliedSourceOver)?;

        let completed = backend.render_offscreen(&scene, descriptor)?;

        assert_eq!(completed.report().submission, 1);
        assert_eq!(completed.report().primitives, 0);
        assert_eq!(completed.report().draw_calls, 0);
        assert_eq!(completed.report().uploaded_bytes, 0);
        assert_pixels_within(completed.image(), &expected, 1);
        Ok(())
    }

    #[test]
    fn validation_failure_commits_no_command_buffer() -> Result<(), Box<dyn Error>> {
        let scene = SceneBuilder::new(SceneRevision::new(73), size(1.0, 1.0)?).finish();
        let descriptor = OffscreenDescriptor::new(2, 1, 1.0, color(0.0, 0.0, 0.0, 0.0)?)?;
        let mut backend = validation_backend(BlendConfiguration::PremultipliedSourceOver)?;

        let error = backend.render_offscreen(&scene, descriptor).err();

        assert_eq!(
            error.as_ref().map(crate::RenderError::stage),
            Some(RenderStage::Validation)
        );
        assert_eq!(backend.submission_count(), 0);
        Ok(())
    }

    #[test]
    fn oversized_texture_fails_before_command_submission() -> Result<(), Box<dyn Error>> {
        let extent = crate::MAX_METAL3_TEXTURE_DIMENSION_2D + 1;
        let logical_extent = f32::from(u16::try_from(extent)?);
        let scene = SceneBuilder::new(SceneRevision::new(74), size(logical_extent, 1.0)?).finish();
        let descriptor = OffscreenDescriptor::new(extent, 1, 1.0, color(0.0, 0.0, 0.0, 0.0)?)?;
        let mut backend = validation_backend(BlendConfiguration::PremultipliedSourceOver)?;

        let error = backend.render_offscreen(&scene, descriptor).err();

        assert!(matches!(
            error,
            Some(crate::RenderError::TextureExtentUnsupported { .. })
        ));
        assert_eq!(backend.submission_count(), 0);
        Ok(())
    }

    #[test]
    fn faulty_blend_control_is_detected_by_cpu_oracle() -> Result<(), Box<dyn Error>> {
        let (scene, descriptor) = discriminating_scene()?;
        let expected = ValidatedFrame::new(&scene, descriptor)?.reference_image()?;
        let mut backend = validation_backend(BlendConfiguration::DisabledFaultControl)?;

        let faulty = backend.render_offscreen(&scene, descriptor)?;

        assert_eq!(backend.submission_count(), 1);
        assert_ne!(faulty.image(), &expected);
        Ok(())
    }

    #[test]
    fn copies_every_native_command_status_into_stable_data() {
        use objc2_metal::MTLCommandBufferStatus;

        assert_eq!(
            command_status(MTLCommandBufferStatus::NotEnqueued),
            crate::CommandStatus::NotEnqueued
        );
        assert_eq!(
            command_status(MTLCommandBufferStatus::Enqueued),
            crate::CommandStatus::Enqueued
        );
        assert_eq!(
            command_status(MTLCommandBufferStatus::Committed),
            crate::CommandStatus::Committed
        );
        assert_eq!(
            command_status(MTLCommandBufferStatus::Scheduled),
            crate::CommandStatus::Scheduled
        );
        assert_eq!(
            command_status(MTLCommandBufferStatus::Completed),
            crate::CommandStatus::Completed
        );
        assert_eq!(
            command_status(MTLCommandBufferStatus::Error),
            crate::CommandStatus::Error
        );
        assert_eq!(
            command_status(MTLCommandBufferStatus(99)),
            crate::CommandStatus::Unknown(99)
        );
    }

    #[test]
    fn injected_precommit_failures_release_once_and_balance_accounting()
    -> Result<(), Box<dyn Error>> {
        let (scene, descriptor) = discriminating_scene()?;
        let cases = [
            (
                NativeFault::TextureAllocation,
                RenderStage::RenderTexture,
                0,
                0,
            ),
            (
                NativeFault::ReadbackAllocation,
                RenderStage::ReadbackBuffer,
                0,
                0,
            ),
            (
                NativeFault::UploadAllocation,
                RenderStage::UploadBuffer,
                0,
                0,
            ),
            (
                NativeFault::CommandBuffer,
                RenderStage::CommandBuffer,
                96,
                0,
            ),
            (
                NativeFault::RenderEncoder,
                RenderStage::RenderEncoder,
                96,
                0,
            ),
            (NativeFault::BlitEncoder, RenderStage::BlitEncoder, 96, 1),
        ];

        for (fault, expected_stage, uploaded_bytes, draw_calls) in cases {
            let (mut backend, probe) =
                validation_backend_and_probe(BlendConfiguration::PremultipliedSourceOver, fault)?;
            let error = backend
                .render_offscreen(&scene, descriptor)
                .err()
                .ok_or("fault must fail")?;
            let accounting = backend.accounting();

            assert_eq!(error.stage(), expected_stage, "{fault:?}");
            assert_eq!(error.recovery(), RecoveryClassification::RetryFrame);
            assert_eq!(backend.submission_count(), 0);
            assert_eq!(accounting.accepted_frames(), 1);
            assert_eq!(accounting.failed_frames(), 1);
            assert_eq!(accounting.completed_frames(), 0);
            assert_eq!(accounting.uploaded_bytes(), uploaded_bytes, "{fault:?}");
            assert_eq!(accounting.draw_calls(), draw_calls, "{fault:?}");
            assert_eq!(accounting.current_retained_bytes(), 0);
            assert!(accounting.invariants_hold());
            assert_eq!(probe.counts(), (1, 1, 0));
        }
        Ok(())
    }

    #[test]
    fn injected_terminal_failures_never_return_pixels_and_classify_recovery()
    -> Result<(), Box<dyn Error>> {
        use objc2_metal::MTLCommandBufferError;

        let (scene, descriptor) = discriminating_scene()?;
        let cases = [
            (
                i64::try_from(MTLCommandBufferError::OutOfMemory.0)?,
                RecoveryClassification::RetryFrame,
                BackendState::Ready,
            ),
            (
                i64::try_from(MTLCommandBufferError::NotPermitted.0)?,
                RecoveryClassification::Unsupported,
                BackendState::Ready,
            ),
            (
                i64::try_from(MTLCommandBufferError::DeviceRemoved.0)?,
                RecoveryClassification::RecreateBackend,
                BackendState::DeviceLost,
            ),
            (
                i64::try_from(MTLCommandBufferError::AccessRevoked.0)?,
                RecoveryClassification::RecreateBackend,
                BackendState::DeviceLost,
            ),
        ];

        for (code, recovery, state) in cases {
            let (mut backend, probe) = validation_backend_and_probe(
                BlendConfiguration::PremultipliedSourceOver,
                NativeFault::TerminalError(code),
            )?;
            let error = backend
                .render_offscreen(&scene, descriptor)
                .err()
                .ok_or("terminal injection must fail")?;
            let accounting = backend.accounting();

            assert!(matches!(error, RenderError::CommandFailed { .. }));
            assert_eq!(error.recovery(), recovery);
            assert_eq!(backend.submission_count(), 1);
            assert_eq!(accounting.state(), state);
            assert_eq!(accounting.failed_frames(), 1);
            assert_eq!(accounting.uploaded_bytes(), 3 * 32);
            assert_eq!(accounting.draw_calls(), 1);
            assert_eq!(accounting.current_retained_bytes(), 0);
            assert!(accounting.allocated_bytes() > 0);
            assert!(accounting.invariants_hold());
            assert_eq!(probe.counts(), (1, 1, 0));
        }
        Ok(())
    }

    #[test]
    fn unexpected_completion_and_readback_mismatch_fail_closed() -> Result<(), Box<dyn Error>> {
        let (scene, descriptor) = discriminating_scene()?;
        for fault in [NativeFault::UnexpectedStatus, NativeFault::ReadbackLength] {
            let (mut backend, probe) =
                validation_backend_and_probe(BlendConfiguration::PremultipliedSourceOver, fault)?;
            let error = backend
                .render_offscreen(&scene, descriptor)
                .err()
                .ok_or("terminal control must fail")?;

            assert_eq!(error.recovery(), RecoveryClassification::Fatal);
            assert_eq!(backend.submission_count(), 1);
            assert_eq!(backend.accounting().failed_frames(), 1);
            assert!(backend.accounting().invariants_hold());
            assert_eq!(probe.counts(), (1, 1, 0));
        }
        Ok(())
    }

    #[test]
    fn cancellation_shutdown_and_steady_state_have_no_hidden_native_work()
    -> Result<(), Box<dyn Error>> {
        const VALIDATION_WARMUP_FRAMES: u16 = 256;
        const RSS_WARMUP_FRAMES: u16 = 4_096;
        const MEASURED_FRAMES: u16 = 256;

        let (scene, descriptor) = discriminating_scene()?;
        let (mut backend, probe) = validation_backend_and_probe(
            BlendConfiguration::PremultipliedSourceOver,
            NativeFault::None,
        )?;

        let cancellation = backend.cancel_offscreen(&scene, descriptor)?;
        assert_eq!(cancellation.generation().get(), 1);
        assert_eq!(cancellation.primitives(), 4);
        assert_eq!(cancellation.omitted_primitives(), 1);
        assert_eq!(cancellation.uploaded_bytes_avoided(), 3 * 32);
        assert_eq!(probe.counts(), (0, 0, 0));
        assert_eq!(backend.accounting().uploaded_bytes(), 0);
        assert_eq!(backend.accounting().draw_calls(), 0);

        let mut expected_allocated = 0_u128;
        let mut expected_readback = 0_u128;
        let mut retained = None;
        let capture_resident_distribution = std::env::var_os("ALPINE_CAPTURE_RSS").is_some();
        if capture_resident_distribution {
            let _ = resident_bytes()?;
        }
        let warmup_frames = if capture_resident_distribution {
            RSS_WARMUP_FRAMES
        } else {
            VALIDATION_WARMUP_FRAMES
        };
        let total_frames = warmup_frames + MEASURED_FRAMES;
        let mut resident_samples = Vec::with_capacity(17);
        for frame_index in 1_u16..=total_frames {
            let completed = backend.render_offscreen(&scene, descriptor)?;
            let report = completed.report();
            expected_allocated += report.allocated_bytes as u128;
            expected_readback += report.readback_bytes as u128;
            assert_eq!(
                retained.get_or_insert(report.retained_bytes),
                &report.retained_bytes
            );
            assert_eq!(backend.accounting().current_retained_bytes(), 0);
            if capture_resident_distribution
                && frame_index >= warmup_frames
                && (frame_index - warmup_frames).is_multiple_of(16)
            {
                resident_samples.push((frame_index - warmup_frames, resident_bytes()?));
            }
        }
        let accounting = backend.accounting();
        assert_eq!(accounting.accepted_frames(), 1 + u128::from(total_frames));
        assert_eq!(accounting.cancelled_frames(), 1);
        assert_eq!(accounting.completed_frames(), u128::from(total_frames));
        assert_eq!(accounting.submitted_frames(), u64::from(total_frames));
        assert_eq!(accounting.draw_calls(), u128::from(total_frames));
        assert_eq!(
            accounting.uploaded_bytes(),
            u128::from(total_frames) * 3 * 32
        );
        assert_eq!(accounting.allocated_bytes(), expected_allocated);
        assert_eq!(accounting.readback_bytes(), expected_readback);
        assert_eq!(
            accounting.peak_retained_bytes(),
            retained.ok_or("retained sample")?
        );
        assert_eq!(
            probe.counts(),
            (u64::from(total_frames), u64::from(total_frames), 0)
        );
        assert!(accounting.invariants_hold());
        if capture_resident_distribution {
            assert_eq!(resident_samples.len(), 17);
            let baseline = resident_samples[0].1;
            let page_bytes = host_page_bytes()?;
            let maximum = baseline
                .checked_add(page_bytes)
                .ok_or("resident ceiling overflow")?;
            for (frame, bytes) in &resident_samples {
                assert!(*bytes > 0);
                assert!(
                    *bytes <= maximum,
                    "resident bytes grew beyond one host page after warmup: baseline {baseline}, frame {frame}, actual {bytes}, page {page_bytes}"
                );
                println!("alpine-memory-sample frame={frame} resident_bytes={bytes}");
            }
        }

        backend.shutdown();
        assert_eq!(backend.accounting().state(), BackendState::Stopped);
        let error = backend
            .render_offscreen(&scene, descriptor)
            .err()
            .ok_or("stopped backend must reject work")?;
        assert_eq!(error.recovery(), RecoveryClassification::Stopped);
        assert_eq!(backend.submission_count(), u64::from(total_frames));
        assert_eq!(
            probe.counts(),
            (u64::from(total_frames), u64::from(total_frames), 0)
        );
        assert!(backend.accounting().invariants_hold());
        Ok(())
    }

    #[test]
    fn device_loss_invalidates_generation_and_recovery_is_guarded() -> Result<(), Box<dyn Error>> {
        use objc2_metal::MTLCommandBufferError;

        let (scene, descriptor) = discriminating_scene()?;
        let code = i64::try_from(MTLCommandBufferError::DeviceRemoved.0)?;
        let mut backend = validation_backend_with_fault(
            BlendConfiguration::PremultipliedSourceOver,
            NativeFault::TerminalError(code),
        )?;
        let _ = backend.render_offscreen(&scene, descriptor).err();
        assert_eq!(backend.accounting().state(), BackendState::DeviceLost);
        assert_eq!(backend.accounting().generation().get(), 1);

        let rejected = backend
            .render_offscreen(&scene, descriptor)
            .err()
            .ok_or("device-lost generation must reject")?;
        assert!(matches!(
            rejected,
            RenderError::BackendUnavailable {
                state: BackendState::DeviceLost,
                ..
            }
        ));
        assert_eq!(backend.submission_count(), 1);

        match backend.recover() {
            Ok(recovered) => {
                assert_eq!(recovered.accounting().generation().get(), 2);
                assert_eq!(recovered.accounting().state(), BackendState::Ready);
            }
            Err(crate::RecoveryError::Initialization(InitializationError::UnsupportedDevice {
                ..
            })) => {}
            Err(error) => return Err(error.into()),
        }

        let ready = validation_backend(BlendConfiguration::PremultipliedSourceOver)?;
        assert!(matches!(
            ready.recover(),
            Err(crate::RecoveryError::BackendNotDeviceLost {
                state: BackendState::Ready,
                ..
            })
        ));
        Ok(())
    }
}
