use std::{ffi::c_void, mem::size_of, ptr::NonNull, slice};

use dispatch2::DispatchData;
use objc2::{rc::Retained, runtime::ProtocolObject};
use objc2_foundation::{NSError, NSString};
use objc2_metal::{
    MTLBlendFactor, MTLBlitCommandEncoder, MTLBuffer, MTLClearColor, MTLCommandBuffer,
    MTLCommandBufferStatus, MTLCommandEncoder, MTLCommandQueue, MTLCreateSystemDefaultDevice,
    MTLDevice, MTLFunction, MTLGPUFamily, MTLLibrary, MTLLoadAction, MTLOrigin, MTLPixelFormat,
    MTLPrimitiveType, MTLRenderCommandEncoder, MTLRenderPassDescriptor,
    MTLRenderPipelineDescriptor, MTLRenderPipelineState, MTLResourceOptions, MTLSize,
    MTLStorageMode, MTLStoreAction, MTLTexture, MTLTextureDescriptor, MTLTextureUsage,
};

use crate::initialization::{
    FRAGMENT_ENTRY_POINT, InitializationDriver, InitializationError, Initialized,
    MetalCapabilities, NativeFailure, VERTEX_ENTRY_POINT, initialize,
};
use crate::submission::{
    CommandStatus, NativeRenderAttempt, RenderError, RenderStage, compact_readback,
};
use crate::{Bgra8Image, MAX_METAL3_TEXTURE_DIMENSION_2D, ValidatedFrame};

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
}

impl NativeBackend {
    #[allow(clippy::cast_precision_loss)]
    pub(crate) fn render(&mut self, frame: &ValidatedFrame) -> NativeRenderAttempt {
        let resources = match FrameResources::new(&self.initialized.device, frame) {
            Ok(resources) => resources,
            Err(error) => {
                return NativeRenderAttempt {
                    committed: false,
                    result: Err(error),
                };
            }
        };
        let Some(command) = self.initialized.queue.commandBuffer() else {
            return NativeRenderAttempt {
                committed: false,
                result: Err(RenderError::ResourceUnavailable {
                    stage: RenderStage::CommandBuffer,
                    requested_bytes: None,
                }),
            };
        };

        if let Err(error) =
            encode_render_pass(&command, &self.initialized.pipeline, &resources, frame)
        {
            return NativeRenderAttempt {
                committed: false,
                result: Err(error),
            };
        }
        if let Err(error) = encode_readback(&command, &resources, frame) {
            return NativeRenderAttempt {
                committed: false,
                result: Err(error),
            };
        }

        command.commit();
        command.waitUntilCompleted();
        let status = command_status(command.status());
        let result = match status {
            CommandStatus::Completed => read_compact_image(&resources.readback, frame),
            CommandStatus::Error => Err(RenderError::CommandFailed {
                status,
                failure: command.error().map(|error| copy_error(&error)),
            }),
            status => Err(RenderError::UnexpectedCommandStatus { status }),
        };

        NativeRenderAttempt {
            committed: true,
            result,
        }
    }
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
}

impl FrameResources {
    fn new(device: &Device, frame: &ValidatedFrame) -> Result<Self, RenderError> {
        let descriptor = frame.descriptor();
        if descriptor.pixel_width() > MAX_METAL3_TEXTURE_DIMENSION_2D
            || descriptor.pixel_height() > MAX_METAL3_TEXTURE_DIMENSION_2D
        {
            return Err(RenderError::TextureExtentUnsupported {
                width: descriptor.pixel_width(),
                height: descriptor.pixel_height(),
                limit: MAX_METAL3_TEXTURE_DIMENSION_2D,
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
        let texture = device.newTextureWithDescriptor(&texture_descriptor).ok_or(
            RenderError::ResourceUnavailable {
                stage: RenderStage::RenderTexture,
                requested_bytes: None,
            },
        )?;

        let layout = frame.readback_layout();
        let readback = device
            .newBufferWithLength_options(layout.buffer_len(), MTLResourceOptions::StorageModeShared)
            .ok_or(RenderError::ResourceUnavailable {
                stage: RenderStage::ReadbackBuffer,
                requested_bytes: Some(layout.buffer_len()),
            })?;

        let upload = if frame.quads().is_empty() {
            None
        } else {
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
            .ok_or(RenderError::ResourceUnavailable {
                stage: RenderStage::UploadBuffer,
                requested_bytes: Some(frame.upload_bytes()),
            })?
            .into()
        };

        Ok(Self {
            texture,
            readback,
            upload,
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

#[cfg(test)]
mod tests {
    use std::error::Error;

    use alpine_core::{LinearRgba, Point, Rect, Size};
    use alpine_renderer::Renderer;
    use alpine_scene::{Primitive, Scene, SceneBuilder, SceneRevision};

    use crate::initialization::{
        InitializationError, InitializationStage, MetalBackend, initialize_for_native_validation,
    };
    use crate::{Bgra8Image, OffscreenDescriptor, OffscreenTarget, RenderStage, ValidatedFrame};

    use super::{
        BlendConfiguration, FRAGMENT_ENTRY_POINT, NativeBackend, NativeDriver, OFFLINE_LIBRARY,
        VERTEX_ENTRY_POINT, command_status, new_backend,
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
        let driver = NativeDriver {
            library: OFFLINE_LIBRARY,
            vertex_name: VERTEX_ENTRY_POINT,
            fragment_name: FRAGMENT_ENTRY_POINT,
            blend,
        };
        let initialized = initialize_for_native_validation(&driver)?;
        let capabilities = initialized.capabilities.clone();
        Ok(MetalBackend::from_platform_parts((
            NativeBackend { initialized },
            capabilities,
        )))
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
}
