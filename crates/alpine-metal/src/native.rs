#[cfg(test)]
use std::{cell::Cell, rc::Rc};
use std::{ffi::c_void, mem::size_of, ptr::NonNull, slice};
#[cfg(feature = "platform-spi")]
use std::{
    sync::{Arc, Condvar, Mutex, MutexGuard},
    time::Duration,
};

#[cfg(feature = "platform-spi")]
use block2::RcBlock;
use dispatch2::DispatchData;
use objc2::{rc::Retained, runtime::ProtocolObject};
use objc2_foundation::{NSError, NSString};
#[cfg(feature = "platform-spi")]
use objc2_metal::MTLDrawable;
use objc2_metal::{
    MTLBlendFactor, MTLBlitCommandEncoder, MTLBuffer, MTLClearColor, MTLCommandBuffer,
    MTLCommandBufferError, MTLCommandBufferStatus, MTLCommandEncoder, MTLCommandQueue,
    MTLCreateSystemDefaultDevice, MTLDevice, MTLFunction, MTLGPUFamily, MTLLibrary, MTLLoadAction,
    MTLOrigin, MTLPixelFormat, MTLPrimitiveType, MTLRenderCommandEncoder, MTLRenderPassDescriptor,
    MTLRenderPipelineDescriptor, MTLRenderPipelineState, MTLResource, MTLResourceOptions, MTLSize,
    MTLStorageMode, MTLStoreAction, MTLTexture, MTLTextureDescriptor, MTLTextureUsage,
};

#[cfg(all(feature = "platform-spi", any(test, alpine_native_validation)))]
use crate::initialization::initialize_for_native_validation;
use crate::initialization::{
    FRAGMENT_ENTRY_POINT, InitializationDriver, InitializationError, Initialized,
    MetalCapabilities, NativeFailure, VERTEX_ENTRY_POINT, initialize,
};
#[cfg(feature = "platform-spi")]
use crate::submission::NativeDrawableAttempt;
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
type PipelineState = Retained<ProtocolObject<dyn MTLRenderPipelineState>>;
type Buffer = Retained<ProtocolObject<dyn MTLBuffer>>;
type CommandBuffer = Retained<ProtocolObject<dyn MTLCommandBuffer>>;
type Texture = Retained<ProtocolObject<dyn MTLTexture>>;

struct Pipelines {
    linear_offscreen: PipelineState,
    srgb_presentation: PipelineState,
}

#[derive(Clone, Copy)]
#[cfg_attr(
    not(any(test, feature = "platform-spi")),
    allow(
        dead_code,
        reason = "the presentation contract is retained for platform SPI builds"
    )
)]
enum TargetContract {
    LinearOffscreen,
    SrgbPresentation,
}

impl TargetContract {
    const fn pixel_format(self) -> MTLPixelFormat {
        match self {
            Self::LinearOffscreen => MTLPixelFormat::BGRA8Unorm,
            Self::SrgbPresentation => MTLPixelFormat::BGRA8Unorm_sRGB,
        }
    }

    const fn pipeline(self, pipelines: &Pipelines) -> &PipelineState {
        match self {
            Self::LinearOffscreen => &pipelines.linear_offscreen,
            Self::SrgbPresentation => &pipelines.srgb_presentation,
        }
    }
}

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
    atlas_cache: GlyphAtlasCache,
    #[cfg(feature = "platform-spi")]
    presentation: PresentationSlots,
    #[cfg(feature = "platform-spi")]
    atlas_pressure_pending: bool,
    #[cfg(any(test, alpine_native_validation))]
    fault: NativeFault,
    #[cfg(test)]
    probe: ResourceProbe,
}

#[cfg(feature = "platform-spi")]
const PRESENTATION_SLOT_COUNT: usize = 3;
#[cfg(feature = "platform-spi")]
const PRESENTATION_UPLOAD_LIMIT: usize = 8 * 1024 * 1024;
const ATLAS_STAGING_LIMIT: usize = 16 * 1024 * 1024;
#[cfg(feature = "platform-spi")]
const ATLAS_STAGING_TRIM_TERMINALS: u16 = 120;
#[cfg(feature = "platform-spi")]
const PRESENTATION_TRIM_TERMINALS: u16 = 120;
#[cfg(feature = "platform-spi")]
const COMPLETION_WAIT_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(feature = "platform-spi")]
const TEST_COMPLETION_WAIT_TIMEOUT: Duration = Duration::from_secs(1);
#[cfg(feature = "platform-spi")]
type CompletionHandler = dyn Fn(NonNull<ProtocolObject<dyn MTLCommandBuffer>>);

#[cfg(feature = "platform-spi")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct NativePresentationId {
    slot: u8,
    sequence: u64,
}

#[cfg(feature = "platform-spi")]
#[derive(Clone, Copy)]
pub(crate) struct NativeDrawableSubmission {
    pub(crate) id: NativePresentationId,
}

#[cfg(feature = "platform-spi")]
pub(crate) enum NativeDrawableSubmitAttempt {
    Rejected(NativeDrawableAttempt),
    Submitted(NativeDrawableSubmission),
}

#[cfg(feature = "platform-spi")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct NativePresentationSnapshot {
    pub(crate) occupied_slots: u8,
    pub(crate) current_upload_bytes: usize,
    pub(crate) peak_upload_bytes: usize,
    pub(crate) slot_upload_bytes: [usize; PRESENTATION_SLOT_COUNT],
    pub(crate) slot_peak_upload_bytes: [usize; PRESENTATION_SLOT_COUNT],
    pub(crate) upload_allocations: u64,
    pub(crate) upload_trims: u64,
    pub(crate) current_atlas_bytes: usize,
    pub(crate) peak_atlas_bytes: usize,
    pub(crate) atlas_allocations: u64,
    pub(crate) atlas_uploads: u64,
    pub(crate) atlas_reuses: u64,
    pub(crate) atlas_pressure_releases: u64,
    pub(crate) current_atlas_staging_bytes: usize,
    pub(crate) peak_atlas_staging_bytes: usize,
    pub(crate) slot_atlas_staging_bytes: [usize; PRESENTATION_SLOT_COUNT],
    pub(crate) slot_peak_atlas_staging_bytes: [usize; PRESENTATION_SLOT_COUNT],
    pub(crate) atlas_staging_allocations: u64,
    pub(crate) atlas_staging_trims: u64,
}

struct GlyphAtlasCache {
    image: Option<alpine_scene::GlyphAtlasImage>,
    buffer: Option<Buffer>,
    current_bytes: usize,
    peak_bytes: usize,
    allocations: u64,
    uploads: u64,
    reuses: u64,
    #[cfg_attr(not(feature = "platform-spi"), allow(dead_code))]
    pressure_releases: u64,
}

impl GlyphAtlasCache {
    const fn new() -> Self {
        Self {
            image: None,
            buffer: None,
            current_bytes: 0,
            peak_bytes: 0,
            allocations: 0,
            uploads: 0,
            reuses: 0,
            pressure_releases: 0,
        }
    }

    #[allow(
        clippy::too_many_lines,
        reason = "cache reuse, delta catch-up, full resynchronization, and deferred publication remain one audited residency transaction"
    )]
    fn prepare(
        &mut self,
        device: &Device,
        image: Option<&alpine_scene::GlyphAtlasImage>,
        mut staging: Option<&mut AtlasStaging>,
    ) -> Result<AtlasPreparation, RenderError> {
        let Some(image) = image else {
            if let Some(buffer) = &self.buffer {
                let retained_bytes = self
                    .current_bytes
                    .checked_add(staging.as_deref().map_or(0, AtlasStaging::current_bytes))
                    .ok_or(RenderError::AccountingOverflow)?;
                return Ok(AtlasPreparation {
                    buffer: Some(buffer.clone()),
                    retained_bytes,
                    ..AtlasPreparation::default()
                });
            }
            let next_allocations = self
                .allocations
                .checked_add(1)
                .ok_or(RenderError::AccountingOverflow)?;
            let buffer = create_solid_binding_atlas(device)?;
            let allocated_bytes = buffer.allocatedSize();
            self.image = None;
            self.buffer = Some(buffer.clone());
            self.current_bytes = allocated_bytes;
            self.peak_bytes = self.peak_bytes.max(allocated_bytes);
            self.allocations = next_allocations;
            let retained_bytes = allocated_bytes
                .checked_add(staging.as_deref().map_or(0, AtlasStaging::current_bytes))
                .ok_or(RenderError::AccountingOverflow)?;
            return Ok(AtlasPreparation {
                buffer: Some(buffer),
                upload: None,
                allocated_bytes,
                retained_bytes,
                uploaded_bytes: 0,
                commit: AtlasCacheCommit::None,
            });
        };
        let matches = self.image.as_ref().is_some_and(|cached| {
            cached.revision() == image.revision()
                && cached.width() == image.width()
                && cached.height() == image.height()
                && cached.shares_storage_with(image)
        });
        if matches {
            let next_reuses = self
                .reuses
                .checked_add(1)
                .ok_or(RenderError::AccountingOverflow)?;
            let retained_bytes = self
                .current_bytes
                .checked_add(staging.as_deref().map_or(0, AtlasStaging::current_bytes))
                .ok_or(RenderError::AccountingOverflow)?;
            return Ok(AtlasPreparation {
                buffer: self.buffer.clone(),
                retained_bytes,
                commit: AtlasCacheCommit::Reuse {
                    reuses: next_reuses,
                },
                ..AtlasPreparation::default()
            });
        }

        let patchable = self.image.as_ref().is_some_and(|cached| {
            AtlasPatchCompatibility {
                base_revision: AtlasMatch::from_equality(
                    cached.base_revision() == image.base_revision(),
                ),
                revision_ordering: cached.revision().cmp(&image.revision()),
                extent: AtlasMatch::from_equality(
                    (cached.width(), cached.height()) == (image.width(), image.height()),
                ),
                storage: AtlasMatch::from_equality(cached.shares_storage_with(image)),
                row_patches: AtlasPatchPresence::from_presence(!image.row_patches().is_empty()),
            }
            .is_compatible()
        });
        if patchable {
            let next_uploads = self
                .uploads
                .checked_add(1)
                .ok_or(RenderError::AccountingOverflow)?;
            let kind = if self
                .image
                .as_ref()
                .is_some_and(|cached| cached.revision() == image.delta_source_revision())
            {
                AtlasUploadKind::DeltaRows
            } else {
                AtlasUploadKind::RecoveryRows
            };
            let (upload, allocated_bytes, staging_bytes) =
                prepare_atlas_upload(device, image, kind, staging.as_deref_mut())?;
            let retained_bytes = self
                .current_bytes
                .checked_add(staging_bytes)
                .ok_or(RenderError::AccountingOverflow)?;
            let uploaded_bytes = upload.uploaded_bytes()?;
            return Ok(AtlasPreparation {
                buffer: self.buffer.clone(),
                upload: Some(upload),
                allocated_bytes,
                retained_bytes,
                uploaded_bytes,
                commit: AtlasCacheCommit::Advance {
                    image: image.clone(),
                    uploads: next_uploads,
                },
            });
        }

        let next_allocations = self
            .allocations
            .checked_add(1)
            .ok_or(RenderError::AccountingOverflow)?;
        let next_uploads = self
            .uploads
            .checked_add(1)
            .ok_or(RenderError::AccountingOverflow)?;
        let buffer = create_private_glyph_atlas(device, image.pixels().len())?;
        let (upload, staging_allocated_bytes, staging_bytes) =
            prepare_atlas_upload(device, image, AtlasUploadKind::Full, staging)?;
        let buffer_bytes = buffer.allocatedSize();
        let allocated_bytes = buffer_bytes
            .checked_add(staging_allocated_bytes)
            .ok_or(RenderError::AccountingOverflow)?;
        let retained_bytes = buffer_bytes
            .checked_add(staging_bytes)
            .ok_or(RenderError::AccountingOverflow)?;
        Ok(AtlasPreparation {
            buffer: Some(buffer.clone()),
            upload: Some(upload),
            allocated_bytes,
            retained_bytes,
            uploaded_bytes: image.pixels().len(),
            commit: AtlasCacheCommit::Replace {
                image: image.clone(),
                buffer,
                current_bytes: buffer_bytes,
                peak_bytes: self.peak_bytes.max(buffer_bytes),
                allocations: next_allocations,
                uploads: next_uploads,
            },
        })
    }

    fn commit(&mut self, commit: AtlasCacheCommit) {
        match commit {
            AtlasCacheCommit::None => {}
            AtlasCacheCommit::Reuse { reuses } => self.reuses = reuses,
            AtlasCacheCommit::Advance { image, uploads } => {
                self.image = Some(image);
                self.uploads = uploads;
            }
            AtlasCacheCommit::Replace {
                image,
                buffer,
                current_bytes,
                peak_bytes,
                allocations,
                uploads,
            } => {
                self.image = Some(image);
                self.buffer = Some(buffer);
                self.current_bytes = current_bytes;
                self.peak_bytes = peak_bytes;
                self.allocations = allocations;
                self.uploads = uploads;
            }
        }
    }

    #[cfg_attr(not(any(test, feature = "platform-spi")), allow(dead_code))]
    fn pressure(&mut self) {
        if self.buffer.take().is_some() {
            self.pressure_releases = self.pressure_releases.saturating_add(1);
        }
        self.image = None;
        self.current_bytes = 0;
    }
}

#[derive(Default)]
struct AtlasPreparation {
    buffer: Option<Buffer>,
    upload: Option<AtlasUpload>,
    allocated_bytes: usize,
    retained_bytes: usize,
    uploaded_bytes: usize,
    commit: AtlasCacheCommit,
}

#[derive(Default)]
enum AtlasCacheCommit {
    #[default]
    None,
    Reuse {
        reuses: u64,
    },
    Advance {
        image: alpine_scene::GlyphAtlasImage,
        uploads: u64,
    },
    Replace {
        image: alpine_scene::GlyphAtlasImage,
        buffer: Buffer,
        current_bytes: usize,
        peak_bytes: usize,
        allocations: u64,
        uploads: u64,
    },
}

struct AtlasUpload {
    buffer: Buffer,
    copies: Box<[AtlasCopy]>,
}

impl AtlasUpload {
    fn uploaded_bytes(&self) -> Result<usize, RenderError> {
        self.copies.iter().try_fold(0_usize, |total, copy| {
            total
                .checked_add(copy.size)
                .ok_or(RenderError::AccountingOverflow)
        })
    }
}

#[derive(Clone, Copy)]
struct AtlasCopy {
    source_offset: usize,
    destination_offset: usize,
    size: usize,
}

#[derive(Clone, Copy)]
enum AtlasUploadKind {
    Full,
    DeltaRows,
    RecoveryRows,
}

#[derive(Clone, Copy)]
enum AtlasMatch {
    Same,
    Different,
}

impl AtlasMatch {
    const fn from_equality(matches: bool) -> Self {
        if matches { Self::Same } else { Self::Different }
    }
}

#[derive(Clone, Copy)]
enum AtlasPatchPresence {
    Present,
    Empty,
}

impl AtlasPatchPresence {
    const fn from_presence(present: bool) -> Self {
        if present { Self::Present } else { Self::Empty }
    }
}

#[derive(Clone, Copy)]
struct AtlasPatchCompatibility {
    base_revision: AtlasMatch,
    revision_ordering: std::cmp::Ordering,
    extent: AtlasMatch,
    storage: AtlasMatch,
    row_patches: AtlasPatchPresence,
}

impl AtlasPatchCompatibility {
    const fn is_compatible(self) -> bool {
        matches!(self.base_revision, AtlasMatch::Same)
            && matches!(self.revision_ordering, std::cmp::Ordering::Less)
            && matches!(self.extent, AtlasMatch::Same)
            && matches!(self.storage, AtlasMatch::Same)
            && matches!(self.row_patches, AtlasPatchPresence::Present)
    }
}

#[derive(Default)]
struct AtlasStaging {
    buffer: Option<Buffer>,
    peak_bytes: usize,
    allocations: u64,
    #[cfg(feature = "platform-spi")]
    trims: u64,
    last_demand: usize,
    #[cfg(feature = "platform-spi")]
    underused_terminals: u16,
}

impl AtlasStaging {
    fn current_bytes(&self) -> usize {
        self.buffer.as_deref().map_or(0, MTLResource::allocatedSize)
    }

    fn prepare(
        &mut self,
        device: &Device,
        atlas: &alpine_scene::GlyphAtlasImage,
        kind: AtlasUploadKind,
    ) -> Result<(AtlasUpload, usize), RenderError> {
        let required = atlas_upload_bytes(atlas, kind)?;
        let desired = atlas_staging_capacity(required).ok_or(RenderError::ResourceUnavailable {
            stage: RenderStage::UploadBuffer,
            requested_bytes: Some(required),
        })?;
        let needs_growth = self
            .buffer
            .as_deref()
            .is_none_or(|buffer| buffer.length() < desired);
        let allocated_bytes = if needs_growth {
            let buffer = device
                .newBufferWithLength_options(desired, MTLResourceOptions::StorageModeShared)
                .ok_or(RenderError::ResourceUnavailable {
                    stage: RenderStage::UploadBuffer,
                    requested_bytes: Some(desired),
                })?;
            let allocated = buffer.allocatedSize();
            self.buffer = Some(buffer);
            self.allocations = self
                .allocations
                .checked_add(1)
                .ok_or(RenderError::AccountingOverflow)?;
            self.peak_bytes = self.peak_bytes.max(allocated);
            allocated
        } else {
            0
        };
        let buffer = self
            .buffer
            .as_ref()
            .ok_or(RenderError::SubmissionInvariantViolated)?;
        let upload = write_atlas_upload(buffer.clone(), atlas, kind)?;
        self.last_demand = required;
        Ok((upload, allocated_bytes))
    }

    #[cfg(feature = "platform-spi")]
    fn observe_terminal(&mut self) {
        let desired = atlas_staging_capacity(self.last_demand).unwrap_or(0);
        if self.current_bytes() <= desired {
            self.underused_terminals = 0;
            return;
        }
        self.underused_terminals = self.underused_terminals.saturating_add(1);
        if self.underused_terminals >= ATLAS_STAGING_TRIM_TERMINALS {
            self.pressure();
        }
    }

    #[cfg(feature = "platform-spi")]
    fn pressure(&mut self) {
        if self.buffer.take().is_some() {
            self.trims = self.trims.saturating_add(1);
        }
        self.last_demand = 0;
        self.underused_terminals = 0;
    }
}

#[cfg(feature = "platform-spi")]
struct PresentationSlots {
    slots: [PresentationSlot; PRESENTATION_SLOT_COUNT],
    peak_upload_bytes: usize,
}

#[cfg(feature = "platform-spi")]
impl PresentationSlots {
    fn new() -> Self {
        Self {
            slots: std::array::from_fn(|_| PresentationSlot::new()),
            peak_upload_bytes: 0,
        }
    }

    fn snapshot(&self) -> NativePresentationSnapshot {
        let slot_upload_bytes = std::array::from_fn(|index| self.slots[index].upload_bytes());
        let slot_peak_upload_bytes =
            std::array::from_fn(|index| self.slots[index].peak_upload_bytes);
        let slot_atlas_staging_bytes =
            std::array::from_fn(|index| self.slots[index].atlas_staging.current_bytes());
        let slot_peak_atlas_staging_bytes =
            std::array::from_fn(|index| self.slots[index].atlas_staging.peak_bytes);
        NativePresentationSnapshot {
            occupied_slots: u8::try_from(
                self.slots
                    .iter()
                    .filter(|slot| slot.pending.is_some())
                    .count(),
            )
            .unwrap_or(3),
            current_upload_bytes: slot_upload_bytes.iter().sum(),
            peak_upload_bytes: self.peak_upload_bytes,
            slot_upload_bytes,
            slot_peak_upload_bytes,
            upload_allocations: self.slots.iter().map(|slot| slot.upload_allocations).sum(),
            upload_trims: self.slots.iter().map(|slot| slot.upload_trims).sum(),
            current_atlas_bytes: 0,
            peak_atlas_bytes: 0,
            atlas_allocations: 0,
            atlas_uploads: 0,
            atlas_reuses: 0,
            atlas_pressure_releases: 0,
            current_atlas_staging_bytes: slot_atlas_staging_bytes.iter().sum(),
            peak_atlas_staging_bytes: slot_peak_atlas_staging_bytes.iter().sum(),
            slot_atlas_staging_bytes,
            slot_peak_atlas_staging_bytes,
            atlas_staging_allocations: self
                .slots
                .iter()
                .map(|slot| slot.atlas_staging.allocations)
                .sum(),
            atlas_staging_trims: self.slots.iter().map(|slot| slot.atlas_staging.trims).sum(),
        }
    }

    fn record_upload_peak(&mut self, transient_upload_bytes: usize) {
        self.peak_upload_bytes = self.peak_upload_bytes.max(transient_upload_bytes);
    }

    fn has_pending(&self) -> bool {
        self.slots.iter().any(|slot| slot.pending.is_some())
    }

    fn pressure(&mut self) {
        for slot in &mut self.slots {
            slot.pressure();
        }
    }
}

#[cfg(feature = "platform-spi")]
struct PresentationSlot {
    upload: Option<Buffer>,
    atlas_staging: AtlasStaging,
    peak_upload_bytes: usize,
    upload_allocations: u64,
    upload_trims: u64,
    next_sequence: u64,
    last_upload_demand: usize,
    underused_terminals: u16,
    pressure_pending: bool,
    completion: Arc<CompletionSignal>,
    pending: Option<PendingDrawable>,
}

#[cfg(feature = "platform-spi")]
impl PresentationSlot {
    fn new() -> Self {
        Self {
            upload: None,
            atlas_staging: AtlasStaging::default(),
            peak_upload_bytes: 0,
            upload_allocations: 0,
            upload_trims: 0,
            next_sequence: 0,
            last_upload_demand: 0,
            underused_terminals: 0,
            pressure_pending: false,
            completion: Arc::new(CompletionSignal::new()),
            pending: None,
        }
    }

    fn upload_bytes(&self) -> usize {
        self.upload.as_deref().map_or(0, MTLResource::allocatedSize)
    }

    fn prepare_upload(
        &mut self,
        device: &Device,
        frame: &ValidatedFrame,
        #[cfg(any(test, alpine_native_validation))] fault: NativeFault,
    ) -> Result<UploadPreparation, RenderError> {
        let required = frame.upload_bytes();
        let desired =
            presentation_upload_capacity(required).ok_or(RenderError::ResourceUnavailable {
                stage: RenderStage::UploadBuffer,
                requested_bytes: Some(required),
            })?;
        let needs_growth = self
            .upload
            .as_deref()
            .is_none_or(|buffer| buffer.length() < desired);
        let allocated_bytes = if desired == 0 || !needs_growth {
            0
        } else {
            #[cfg(test)]
            if fault == NativeFault::UploadAllocation {
                return Err(RenderError::ResourceUnavailable {
                    stage: RenderStage::UploadBuffer,
                    requested_bytes: Some(desired),
                });
            }
            let buffer = device
                .newBufferWithLength_options(desired, MTLResourceOptions::StorageModeShared)
                .ok_or(RenderError::ResourceUnavailable {
                    stage: RenderStage::UploadBuffer,
                    requested_bytes: Some(desired),
                })?;
            let allocated = buffer.allocatedSize();
            self.upload = Some(buffer);
            self.upload_allocations = self
                .upload_allocations
                .checked_add(1)
                .ok_or(RenderError::AccountingOverflow)?;
            self.peak_upload_bytes = self.peak_upload_bytes.max(allocated);
            allocated
        };

        if required != 0 {
            let Some(upload) = self.upload.as_deref() else {
                return Err(RenderError::SubmissionInvariantViolated);
            };
            let source = frame.paints().as_ptr().cast::<u8>();
            let destination = upload.contents().cast::<u8>().as_ptr();
            // SAFETY: `ValidatedFrame::upload_bytes` is the exact byte length
            // of the contiguous repr(C) quad slice. The shared Metal buffer is
            // at least `desired >= required` bytes and this slot cannot be
            // submitted again until its prior command reaches terminal state.
            unsafe { std::ptr::copy_nonoverlapping(source, destination, required) };
        }
        self.last_upload_demand = required;
        Ok(UploadPreparation {
            allocated_bytes,
            current_upload_bytes: self.upload_bytes(),
        })
    }

    fn next_id(&mut self, slot: u8) -> Result<NativePresentationId, RenderError> {
        let sequence = self
            .next_sequence
            .checked_add(1)
            .ok_or(RenderError::SubmissionSequenceExhausted)?;
        self.next_sequence = sequence;
        Ok(NativePresentationId { slot, sequence })
    }

    fn pressure(&mut self) {
        if self.pending.is_some() {
            self.pressure_pending = true;
        } else {
            self.trim_upload();
        }
    }

    fn observe_terminal(&mut self) {
        self.atlas_staging.observe_terminal();
        let desired = presentation_upload_capacity(self.last_upload_demand).unwrap_or(0);
        let (underused_terminals, should_trim) = presentation_trim_decision(
            self.upload_bytes(),
            desired,
            self.underused_terminals,
            self.pressure_pending,
        );
        self.underused_terminals = underused_terminals;
        if should_trim {
            self.trim_upload();
        }
    }

    fn trim_upload(&mut self) {
        if self.upload.take().is_some() {
            self.upload_trims = self.upload_trims.saturating_add(1);
        }
        self.underused_terminals = 0;
        self.pressure_pending = false;
        self.atlas_staging.pressure();
    }
}

#[cfg(feature = "platform-spi")]
struct UploadPreparation {
    allocated_bytes: usize,
    current_upload_bytes: usize,
}

#[cfg(feature = "platform-spi")]
struct PendingDrawable {
    id: NativePresentationId,
    _command: CommandBuffer,
    operations: FrameOperationUsage,
    resources: FrameResourceUsage,
    _atlas: Option<Buffer>,
    _atlas_upload: Option<AtlasUpload>,
}

#[cfg(feature = "platform-spi")]
struct NativeTerminal {
    device_lost: bool,
    result: Result<(), RenderError>,
}

#[cfg(feature = "platform-spi")]
struct CompletionState {
    sequence: u64,
    terminal: Option<NativeTerminal>,
}

#[cfg(feature = "platform-spi")]
struct CompletionSignal {
    state: Mutex<CompletionState>,
    ready: Condvar,
}

#[cfg(feature = "platform-spi")]
impl CompletionSignal {
    const fn new() -> Self {
        Self {
            state: Mutex::new(CompletionState {
                sequence: 0,
                terminal: None,
            }),
            ready: Condvar::new(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, CompletionState> {
        match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn reset(&self, sequence: u64) -> Result<(), RenderError> {
        let mut state = self.lock();
        if state.terminal.is_some() {
            return Err(RenderError::SubmissionInvariantViolated);
        }
        state.sequence = sequence;
        Ok(())
    }

    fn publish(&self, sequence: u64, terminal: NativeTerminal) {
        let mut state = self.lock();
        if state.sequence == sequence && state.terminal.is_none() {
            state.terminal = Some(terminal);
            self.ready.notify_one();
        }
    }

    fn take(&self, sequence: u64) -> Option<NativeTerminal> {
        let mut state = self.lock();
        if state.sequence == sequence {
            state.terminal.take()
        } else {
            None
        }
    }

    fn wait_ready(&self, sequence: u64) -> bool {
        let timeout = if cfg!(test) {
            TEST_COMPLETION_WAIT_TIMEOUT
        } else {
            COMPLETION_WAIT_TIMEOUT
        };
        let state = self.lock();
        if state.sequence != sequence {
            return false;
        }
        let (state, _) = match self
            .ready
            .wait_timeout_while(state, timeout, |state| state.terminal.is_none())
        {
            Ok(result) => result,
            Err(poisoned) => poisoned.into_inner(),
        };
        state.terminal.is_some()
    }
}

#[cfg(feature = "platform-spi")]
const fn presentation_upload_capacity(required: usize) -> Option<usize> {
    if required == 0 {
        Some(0)
    } else {
        match required.checked_next_power_of_two() {
            Some(capacity) if capacity <= PRESENTATION_UPLOAD_LIMIT => Some(capacity),
            _ => None,
        }
    }
}

const fn atlas_staging_capacity(required: usize) -> Option<usize> {
    if required == 0 {
        None
    } else {
        match required.checked_next_power_of_two() {
            Some(capacity) if capacity <= ATLAS_STAGING_LIMIT => Some(capacity),
            _ => None,
        }
    }
}

#[cfg(feature = "platform-spi")]
const fn presentation_trim_decision(
    current_capacity: usize,
    desired_capacity: usize,
    underused_terminals: u16,
    pressure: bool,
) -> (u16, bool) {
    if !pressure && current_capacity <= desired_capacity {
        (0, false)
    } else {
        let next = underused_terminals.saturating_add(1);
        (next, pressure || next >= PRESENTATION_TRIM_TERMINALS)
    }
}

#[cfg(any(test, alpine_native_validation))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NativeFault {
    None,
    #[cfg(test)]
    TextureAllocation,
    #[cfg(test)]
    ReadbackAllocation,
    #[cfg(test)]
    UploadAllocation,
    #[cfg(test)]
    CommandBuffer,
    #[cfg(test)]
    RenderEncoder,
    #[cfg(test)]
    BlitEncoder,
    TerminalError(i64),
    #[cfg(test)]
    UnexpectedStatus,
    #[cfg(test)]
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
    pub(crate) fn render(&mut self, frame: &ValidatedFrame) -> NativeRenderAttempt {
        self.render_to_readback(frame, TargetContract::LinearOffscreen)
    }

    #[allow(
        clippy::cast_precision_loss,
        clippy::too_many_lines,
        reason = "the linear command ownership protocol remains visible in one audited boundary"
    )]
    fn render_to_readback(
        &mut self,
        frame: &ValidatedFrame,
        target: TargetContract,
    ) -> NativeRenderAttempt {
        let device = self.initialized.device.clone();
        let resources = match FrameResources::new(
            &device,
            frame,
            target.pixel_format(),
            &mut self.atlas_cache,
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
            instance_upload_bytes: resources
                .upload
                .as_ref()
                .map_or(0, |_| frame.upload_bytes()),
            atlas_upload_bytes: resources.atlas_uploaded_bytes,
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

        if let (Some(upload), Some(atlas)) =
            (resources.atlas_upload.as_ref(), resources.atlas.as_deref())
            && let Err(error) = encode_atlas_upload(&command, upload, atlas)
        {
            return NativeRenderAttempt {
                committed: false,
                device_lost: false,
                operations,
                resources: resource_usage,
                result: Err(error),
            };
        }

        if let Err(error) = encode_render_pass(
            &command,
            target.pipeline(&self.initialized.pipeline),
            &resources.texture,
            resources.upload.as_deref(),
            resources.atlas.as_deref(),
            frame,
        ) {
            return NativeRenderAttempt {
                committed: false,
                device_lost: false,
                operations,
                resources: resource_usage,
                result: Err(error),
            };
        }
        operations.draw_calls = usize::from(!frame.paints().is_empty());
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

        if result.is_ok() {
            self.atlas_cache.commit(resources.atlas_commit);
        } else {
            self.atlas_cache.pressure();
        }

        NativeRenderAttempt {
            committed: true,
            device_lost,
            operations,
            resources: resource_usage,
            result,
        }
    }

    #[cfg(feature = "platform-spi")]
    pub(crate) fn render_drawable(
        &mut self,
        frame: &ValidatedFrame,
        texture: &ProtocolObject<dyn MTLTexture>,
        drawable: &ProtocolObject<dyn MTLDrawable>,
    ) -> NativeDrawableAttempt {
        match self.submit_drawable(0, frame, texture, drawable) {
            NativeDrawableSubmitAttempt::Rejected(attempt) => attempt,
            NativeDrawableSubmitAttempt::Submitted(submission) => {
                if !self.wait_drawable(submission.id) {
                    return invalid_native_committed_drawable();
                }
                match self.poll_drawable(submission.id) {
                    Ok(Some(attempt)) => attempt,
                    Ok(None) | Err(_) => invalid_native_committed_drawable(),
                }
            }
        }
    }

    #[cfg(feature = "platform-spi")]
    #[allow(
        clippy::too_many_lines,
        reason = "slot admission, upload ownership, encoding, handler registration, commit, and direct present remain one audited native transaction"
    )]
    pub(crate) fn submit_drawable(
        &mut self,
        slot: u8,
        frame: &ValidatedFrame,
        texture: &ProtocolObject<dyn MTLTexture>,
        drawable: &ProtocolObject<dyn MTLDrawable>,
    ) -> NativeDrawableSubmitAttempt {
        let index = usize::from(slot);
        if index >= PRESENTATION_SLOT_COUNT {
            return rejected_drawable(RenderError::SubmissionInvariantViolated);
        }
        if self.presentation.slots[index].pending.is_some() {
            return rejected_drawable(RenderError::ResourceUnavailable {
                stage: RenderStage::CommandBuffer,
                requested_bytes: None,
            });
        }
        let target = TargetContract::SrgbPresentation;
        let device = self.initialized.device.clone();
        let resources = match DrawableResources::new(
            &device,
            texture,
            frame,
            target.pixel_format(),
            &mut self.atlas_cache,
            &mut self.presentation.slots[index].atlas_staging,
        ) {
            Ok(resources) => resources,
            Err(failure) => {
                return NativeDrawableSubmitAttempt::Rejected(NativeDrawableAttempt {
                    committed: false,
                    present_called: false,
                    device_lost: false,
                    operations: FrameOperationUsage::default(),
                    resources: failure.usage,
                    result: Err(failure.error),
                });
            }
        };
        let upload_before = self.presentation.snapshot().current_upload_bytes;
        let upload = match self.presentation.slots[index].prepare_upload(
            &self.initialized.device,
            frame,
            #[cfg(any(test, alpine_native_validation))]
            self.fault,
        ) {
            Ok(upload) => upload,
            Err(error) => return rejected_drawable(error),
        };
        let Some(transient_upload) = upload_before.checked_add(upload.allocated_bytes) else {
            return rejected_drawable(RenderError::AccountingOverflow);
        };
        self.presentation.record_upload_peak(transient_upload);
        let Some(retained_bytes) = resources
            .retained_texture_bytes
            .checked_add(upload.current_upload_bytes)
        else {
            return rejected_drawable(RenderError::AccountingOverflow);
        };
        let Some(allocated_bytes) = upload
            .allocated_bytes
            .checked_add(resources.allocated_atlas_bytes)
        else {
            return rejected_drawable(RenderError::AccountingOverflow);
        };
        let resource_usage = FrameResourceUsage {
            allocated_bytes,
            peak_retained_bytes: retained_bytes,
            current_retained_bytes: 0,
            readback_bytes: 0,
        };
        let mut operations = FrameOperationUsage {
            draw_calls: 0,
            instance_upload_bytes: frame.upload_bytes(),
            atlas_upload_bytes: resources.atlas_uploaded_bytes,
        };
        let Some(command) = self.initialized.queue.commandBuffer() else {
            return NativeDrawableSubmitAttempt::Rejected(NativeDrawableAttempt {
                committed: false,
                present_called: false,
                device_lost: false,
                operations,
                resources: resource_usage,
                result: Err(RenderError::ResourceUnavailable {
                    stage: RenderStage::CommandBuffer,
                    requested_bytes: None,
                }),
            });
        };
        #[cfg(test)]
        if self.fault == NativeFault::RenderEncoder {
            return NativeDrawableSubmitAttempt::Rejected(NativeDrawableAttempt {
                committed: false,
                present_called: false,
                device_lost: false,
                operations,
                resources: resource_usage,
                result: Err(RenderError::EncoderUnavailable {
                    stage: RenderStage::RenderEncoder,
                }),
            });
        }
        if let (Some(upload), Some(atlas)) =
            (resources.atlas_upload.as_ref(), resources.atlas.as_deref())
            && let Err(error) = encode_atlas_upload(&command, upload, atlas)
        {
            return NativeDrawableSubmitAttempt::Rejected(NativeDrawableAttempt {
                committed: false,
                present_called: false,
                device_lost: false,
                operations,
                resources: resource_usage,
                result: Err(error),
            });
        }
        let upload_buffer = self.presentation.slots[index].upload.clone();
        if let Err(error) = encode_render_pass(
            &command,
            target.pipeline(&self.initialized.pipeline),
            texture,
            upload_buffer.as_deref(),
            resources.atlas.as_deref(),
            frame,
        ) {
            return NativeDrawableSubmitAttempt::Rejected(NativeDrawableAttempt {
                committed: false,
                present_called: false,
                device_lost: false,
                operations,
                resources: resource_usage,
                result: Err(error),
            });
        }
        operations.draw_calls = usize::from(!frame.paints().is_empty());
        let id = match self.presentation.slots[index].next_id(slot) {
            Ok(id) => id,
            Err(error) => return rejected_drawable(error),
        };
        let completion = Arc::clone(&self.presentation.slots[index].completion);
        if let Err(error) = completion.reset(id.sequence) {
            return rejected_drawable(error);
        }
        #[cfg(any(test, alpine_native_validation))]
        let fault = self.fault;
        let handler: RcBlock<CompletionHandler> = RcBlock::new(
            move |command: NonNull<ProtocolObject<dyn MTLCommandBuffer>>| {
                // SAFETY: Metal invokes this handler with the valid command
                // buffer registered below for the duration of this call.
                let command = unsafe { command.as_ref() };
                let terminal = drawable_terminal(
                    command,
                    #[cfg(any(test, alpine_native_validation))]
                    fault,
                );
                completion.publish(id.sequence, terminal);
            },
        );
        // SAFETY: `handler` has the exact generated Metal block signature and
        // remains valid for this call. Metal copies completion handlers and
        // releases its copy after invocation.
        unsafe { command.addCompletedHandler(RcBlock::as_ptr(&handler)) };
        self.presentation.slots[index].pending = Some(PendingDrawable {
            id,
            _command: command.clone(),
            operations,
            resources: resource_usage,
            _atlas: resources.atlas,
            _atlas_upload: resources.atlas_upload,
        });
        let atlas_commit = resources.atlas_commit;
        command.commit();
        self.atlas_cache.commit(atlas_commit);
        drawable.present();
        NativeDrawableSubmitAttempt::Submitted(NativeDrawableSubmission { id })
    }

    #[cfg(feature = "platform-spi")]
    pub(crate) fn poll_drawable(
        &mut self,
        id: NativePresentationId,
    ) -> Result<Option<NativeDrawableAttempt>, RenderError> {
        let Some(slot) = self.presentation.slots.get_mut(usize::from(id.slot)) else {
            return Err(RenderError::SubmissionInvariantViolated);
        };
        if slot.pending.as_ref().map(|pending| pending.id) != Some(id) {
            return Err(RenderError::SubmissionInvariantViolated);
        }
        let Some(terminal) = slot.completion.take(id.sequence) else {
            return Ok(None);
        };
        let Some(pending) = slot.pending.take() else {
            return Err(RenderError::SubmissionInvariantViolated);
        };
        slot.observe_terminal();
        let terminal_failed = terminal.result.is_err();
        let attempt = NativeDrawableAttempt {
            committed: true,
            present_called: true,
            device_lost: terminal.device_lost,
            operations: pending.operations,
            resources: pending.resources,
            result: terminal.result,
        };
        if terminal_failed {
            self.atlas_cache.pressure();
        }
        if self.atlas_pressure_pending && !self.presentation.has_pending() {
            self.atlas_cache.pressure();
            self.atlas_pressure_pending = false;
        }
        Ok(Some(attempt))
    }

    #[cfg(feature = "platform-spi")]
    pub(crate) fn wait_drawable(&self, id: NativePresentationId) -> bool {
        self.presentation
            .slots
            .get(usize::from(id.slot))
            .is_some_and(|slot| {
                slot.pending.as_ref().map(|pending| pending.id) == Some(id)
                    && slot.completion.wait_ready(id.sequence)
            })
    }

    #[cfg(feature = "platform-spi")]
    pub(crate) fn presentation_snapshot(&self) -> NativePresentationSnapshot {
        let mut snapshot = self.presentation.snapshot();
        snapshot.current_atlas_bytes = self.atlas_cache.current_bytes;
        snapshot.peak_atlas_bytes = self.atlas_cache.peak_bytes;
        snapshot.atlas_allocations = self.atlas_cache.allocations;
        snapshot.atlas_uploads = self.atlas_cache.uploads;
        snapshot.atlas_reuses = self.atlas_cache.reuses;
        snapshot.atlas_pressure_releases = self.atlas_cache.pressure_releases;
        snapshot
    }

    #[cfg(feature = "platform-spi")]
    pub(crate) fn release_presentation_uploads_on_pressure(&mut self) {
        self.presentation.pressure();
        if self.presentation.has_pending() {
            self.atlas_pressure_pending = true;
        } else {
            self.atlas_cache.pressure();
        }
    }
}

#[cfg(feature = "platform-spi")]
fn rejected_drawable(error: RenderError) -> NativeDrawableSubmitAttempt {
    NativeDrawableSubmitAttempt::Rejected(NativeDrawableAttempt {
        committed: false,
        present_called: false,
        device_lost: false,
        operations: FrameOperationUsage::default(),
        resources: FrameResourceUsage::default(),
        result: Err(error),
    })
}

#[cfg(feature = "platform-spi")]
fn invalid_native_committed_drawable() -> NativeDrawableAttempt {
    NativeDrawableAttempt {
        committed: true,
        present_called: true,
        device_lost: false,
        operations: FrameOperationUsage::default(),
        resources: FrameResourceUsage::default(),
        result: Err(RenderError::SubmissionInvariantViolated),
    }
}

#[cfg(feature = "platform-spi")]
fn drawable_terminal(
    command: &ProtocolObject<dyn MTLCommandBuffer>,
    #[cfg(any(test, alpine_native_validation))] fault: NativeFault,
) -> NativeTerminal {
    let status = command_status(command.status());
    let failure = command.error().as_deref().map(copy_error);
    let (result, device_lost) = match (status, failure) {
        (CommandStatus::Completed, None) => (Ok(()), false),
        (CommandStatus::Completed | CommandStatus::Error, failure) => {
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
        (status, _) => (Err(RenderError::UnexpectedCommandStatus { status }), false),
    };
    #[cfg(any(test, alpine_native_validation))]
    let (result, device_lost) = injected_command_failure(fault).unwrap_or((result, device_lost));
    NativeTerminal {
        device_lost,
        result,
    }
}

pub(crate) fn new_backend() -> Result<(NativeBackend, MetalCapabilities), InitializationError> {
    build_backend(initialize(&NativeDriver::production()))
}

#[cfg(feature = "platform-spi")]
pub(crate) fn new_backend_with_device(
    device: Device,
) -> Result<(NativeBackend, MetalCapabilities), InitializationError> {
    build_backend(initialize(&NativeDriver::with_device(device)))
}

#[cfg(all(feature = "platform-spi", any(test, alpine_native_validation)))]
pub(crate) fn new_validation_backend_with_device(
    device: Device,
) -> Result<(NativeBackend, MetalCapabilities), InitializationError> {
    build_backend(initialize_for_native_validation(
        &NativeDriver::with_device(device),
    ))
}

#[cfg(all(feature = "platform-spi", alpine_native_validation))]
pub(crate) fn new_validation_backend_with_device_loss(
    device: Device,
) -> Result<(NativeBackend, MetalCapabilities), InitializationError> {
    let (mut backend, capabilities) = build_backend(initialize_for_native_validation(
        &NativeDriver::with_device(device),
    ))?;
    let device_removed = i64::try_from(MTLCommandBufferError::DeviceRemoved.0).map_err(|_| {
        InitializationError::PipelineCreationFailed(NativeFailure::new(
            "MTLCommandBufferErrorDomain".to_owned(),
            0,
            "device-loss validation code is not representable".to_owned(),
        ))
    })?;
    backend.fault = NativeFault::TerminalError(device_removed);
    Ok((backend, capabilities))
}

fn build_backend(
    initialized: Result<Initialized<NativeDriver>, InitializationError>,
) -> Result<(NativeBackend, MetalCapabilities), InitializationError> {
    initialized.map(|initialized| {
        let capabilities = initialized.capabilities.clone();
        (
            NativeBackend {
                initialized,
                atlas_cache: GlyphAtlasCache::new(),
                #[cfg(feature = "platform-spi")]
                presentation: PresentationSlots::new(),
                #[cfg(feature = "platform-spi")]
                atlas_pressure_pending: false,
                #[cfg(any(test, alpine_native_validation))]
                fault: NativeFault::None,
                #[cfg(test)]
                probe: ResourceProbe::default(),
            },
            capabilities,
        )
    })
}

struct NativeDriver {
    device: Option<Device>,
    library: &'static [u8],
    vertex_name: &'static str,
    fragment_name: &'static str,
    blend: BlendConfiguration,
}

impl NativeDriver {
    const fn production() -> Self {
        Self {
            device: None,
            library: OFFLINE_LIBRARY,
            vertex_name: VERTEX_ENTRY_POINT,
            fragment_name: FRAGMENT_ENTRY_POINT,
            blend: BlendConfiguration::PremultipliedSourceOver,
        }
    }

    #[cfg(feature = "platform-spi")]
    fn with_device(device: Device) -> Self {
        Self {
            device: Some(device),
            ..Self::production()
        }
    }
}

impl InitializationDriver for NativeDriver {
    type Device = Device;
    type Function = Function;
    type Library = Library;
    type Pipeline = Pipelines;
    type Queue = Queue;

    fn create_device(&self) -> Option<Self::Device> {
        self.device
            .clone()
            .or_else(|| MTLCreateSystemDefaultDevice())
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
        Ok(Pipelines {
            linear_offscreen: create_pipeline_state(
                device,
                vertex,
                fragment,
                MTLPixelFormat::BGRA8Unorm,
                self.blend,
            )?,
            srgb_presentation: create_pipeline_state(
                device,
                vertex,
                fragment,
                MTLPixelFormat::BGRA8Unorm_sRGB,
                self.blend,
            )?,
        })
    }
}

fn create_pipeline_state(
    device: &Device,
    vertex: &Function,
    fragment: &Function,
    pixel_format: MTLPixelFormat,
    blend: BlendConfiguration,
) -> Result<PipelineState, NativeFailure> {
    let descriptor = MTLRenderPipelineDescriptor::new();
    descriptor.setVertexFunction(Some(vertex));
    descriptor.setFragmentFunction(Some(fragment));

    let attachments = descriptor.colorAttachments();
    // SAFETY: Metal render-pipeline descriptors always expose eight color
    // attachment slots, so fixed slot zero is within the documented range.
    let color = unsafe { attachments.objectAtIndexedSubscript(0) };
    color.setPixelFormat(pixel_format);
    match blend {
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

struct FrameResources {
    texture: Texture,
    atlas: Option<Buffer>,
    atlas_upload: Option<AtlasUpload>,
    readback: Buffer,
    upload: Option<Buffer>,
    atlas_uploaded_bytes: usize,
    atlas_commit: AtlasCacheCommit,
    usage: FrameResourceUsage,
    #[cfg(test)]
    _lease: ResourceLease,
}

struct ResourceBuildFailure {
    error: RenderError,
    usage: FrameResourceUsage,
}

#[cfg(feature = "platform-spi")]
struct DrawableResources {
    retained_texture_bytes: usize,
    allocated_atlas_bytes: usize,
    atlas_uploaded_bytes: usize,
    atlas: Option<Buffer>,
    atlas_upload: Option<AtlasUpload>,
    atlas_commit: AtlasCacheCommit,
}

#[cfg(feature = "platform-spi")]
impl DrawableResources {
    fn new(
        device: &Device,
        texture: &ProtocolObject<dyn MTLTexture>,
        frame: &ValidatedFrame,
        expected_pixel_format: MTLPixelFormat,
        atlas_cache: &mut GlyphAtlasCache,
        atlas_staging: &mut AtlasStaging,
    ) -> Result<Self, ResourceBuildFailure> {
        let descriptor = frame.descriptor();
        if texture.width() != descriptor.pixel_width() as usize
            || texture.height() != descriptor.pixel_height() as usize
        {
            return Err(ResourceBuildFailure {
                error: RenderError::DrawableExtentMismatch {
                    expected_width: descriptor.pixel_width(),
                    expected_height: descriptor.pixel_height(),
                    actual_width: texture.width(),
                    actual_height: texture.height(),
                },
                usage: FrameResourceUsage::default(),
            });
        }
        if texture.pixelFormat() != expected_pixel_format {
            return Err(ResourceBuildFailure {
                error: RenderError::DrawablePixelFormatMismatch {
                    actual: texture.pixelFormat().0,
                },
                usage: FrameResourceUsage::default(),
            });
        }

        let atlas = if frame.paints().is_empty() {
            AtlasPreparation::default()
        } else {
            atlas_cache
                .prepare(device, frame.glyph_atlas(), Some(atlas_staging))
                .map_err(|error| ResourceBuildFailure {
                    error,
                    usage: FrameResourceUsage::default(),
                })?
        };
        let retained_texture_bytes = texture
            .allocatedSize()
            .checked_add(atlas.retained_bytes)
            .ok_or_else(|| ResourceBuildFailure {
                error: RenderError::AccountingOverflow,
                usage: FrameResourceUsage::default(),
            })?;
        Ok(Self {
            retained_texture_bytes,
            allocated_atlas_bytes: atlas.allocated_bytes,
            atlas_uploaded_bytes: atlas.uploaded_bytes,
            atlas: atlas.buffer,
            atlas_upload: atlas.upload,
            atlas_commit: atlas.commit,
        })
    }
}

impl FrameResources {
    #[allow(
        clippy::too_many_lines,
        reason = "partial native allocation and its exact accounting remain one linear transaction"
    )]
    fn new(
        device: &Device,
        frame: &ValidatedFrame,
        pixel_format: MTLPixelFormat,
        atlas_cache: &mut GlyphAtlasCache,
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
                pixel_format,
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
        let atlas = if frame.paints().is_empty() {
            AtlasPreparation::default()
        } else {
            atlas_cache
                .prepare(device, frame.glyph_atlas(), None)
                .map_err(|error| ResourceBuildFailure {
                    error,
                    usage: FrameResourceUsage {
                        allocated_bytes: texture_bytes,
                        peak_retained_bytes: texture_bytes,
                        current_retained_bytes: 0,
                        readback_bytes: 0,
                    },
                })?
        };
        let atlas_retained_bytes = atlas.retained_bytes;
        let atlas_allocated_bytes = atlas.allocated_bytes;
        let allocated_before_readback = texture_bytes
            .checked_add(atlas_allocated_bytes)
            .ok_or_else(|| ResourceBuildFailure {
                error: RenderError::AccountingOverflow,
                usage: FrameResourceUsage::default(),
            })?;
        let retained_before_readback =
            texture_bytes
                .checked_add(atlas_retained_bytes)
                .ok_or_else(|| ResourceBuildFailure {
                    error: RenderError::AccountingOverflow,
                    usage: FrameResourceUsage::default(),
                })?;

        let layout = frame.readback_layout();
        #[cfg(test)]
        if fault == NativeFault::ReadbackAllocation {
            return Err(ResourceBuildFailure {
                error: RenderError::ResourceUnavailable {
                    stage: RenderStage::ReadbackBuffer,
                    requested_bytes: Some(layout.buffer_len()),
                },
                usage: FrameResourceUsage {
                    allocated_bytes: allocated_before_readback,
                    peak_retained_bytes: retained_before_readback,
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
                    allocated_bytes: allocated_before_readback,
                    peak_retained_bytes: retained_before_readback,
                    current_retained_bytes: 0,
                    readback_bytes: 0,
                },
            })?;
        let readback_bytes = readback.allocatedSize();
        let base_allocated = allocated_before_readback
            .checked_add(readback_bytes)
            .ok_or_else(|| ResourceBuildFailure {
                error: RenderError::AccountingOverflow,
                usage: FrameResourceUsage::default(),
            })?;
        let base_retained = retained_before_readback
            .checked_add(readback_bytes)
            .ok_or_else(|| ResourceBuildFailure {
                error: RenderError::AccountingOverflow,
                usage: FrameResourceUsage::default(),
            })?;

        let upload = if frame.paints().is_empty() {
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
                        peak_retained_bytes: base_retained,
                        current_retained_bytes: 0,
                        readback_bytes: readback.length(),
                    },
                });
            }
            let first = NonNull::from(&frame.paints()[0]).cast::<c_void>();
            // SAFETY: `first` points to `frame.upload_bytes()` initialized,
            // contiguous bytes because LoweredPaint is Copy and repr(C). Metal
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
                    peak_retained_bytes: base_retained,
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
        let retained_bytes = upload.as_deref().map_or(Some(base_retained), |upload| {
            base_retained.checked_add(upload.allocatedSize())
        });
        let retained_bytes = retained_bytes.ok_or_else(|| ResourceBuildFailure {
            error: RenderError::AccountingOverflow,
            usage: FrameResourceUsage::default(),
        })?;
        let usage = FrameResourceUsage {
            allocated_bytes,
            peak_retained_bytes: retained_bytes,
            current_retained_bytes: 0,
            readback_bytes: readback.length(),
        };

        Ok(Self {
            texture,
            atlas: atlas.buffer,
            atlas_upload: atlas.upload,
            readback,
            upload,
            atlas_uploaded_bytes: atlas.uploaded_bytes,
            atlas_commit: atlas.commit,
            usage,
            #[cfg(test)]
            _lease: lease,
        })
    }
}

#[allow(clippy::cast_precision_loss)]
fn encode_render_pass(
    command: &CommandBuffer,
    pipeline: &PipelineState,
    texture: &ProtocolObject<dyn MTLTexture>,
    upload: Option<&ProtocolObject<dyn MTLBuffer>>,
    atlas: Option<&ProtocolObject<dyn MTLBuffer>>,
    frame: &ValidatedFrame,
) -> Result<(), RenderError> {
    if !texture.usage().contains(MTLTextureUsage::RenderTarget) {
        return Err(RenderError::SubmissionInvariantViolated);
    }
    if upload.is_some() && atlas.is_none() {
        return Err(RenderError::SubmissionInvariantViolated);
    }
    let pass = MTLRenderPassDescriptor::renderPassDescriptor();
    let attachments = pass.colorAttachments();
    // SAFETY: Metal render-pass descriptors always expose eight color
    // attachment slots, so fixed slot zero is within the documented range.
    let color = unsafe { attachments.objectAtIndexedSubscript(0) };
    color.setTexture(Some(texture));
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
    if let Some(upload) = upload {
        if let Some(atlas) = atlas {
            let atlas_extent = frame
                .glyph_atlas()
                .map_or([1, 1], |image| [image.width().get(), image.height().get()]);
            // SAFETY: `atlas_extent` contains exactly two initialized u32
            // values and Metal copies all eight bytes into fragment buffer
            // index zero immediately.
            unsafe {
                encoder.setFragmentBytes_length_atIndex(
                    NonNull::from(&atlas_extent).cast::<c_void>(),
                    size_of::<[u32; 2]>(),
                    0,
                );
                encoder.setFragmentBuffer_offset_atIndex(Some(atlas), 0, 1);
            }
        }
        // SAFETY: The retained upload buffer contains exactly the validated
        // LoweredPaint slice, offset zero is aligned, shader index one is fixed,
        // and both the local owner and retained command buffer keep it alive.
        unsafe {
            encoder.setVertexBuffer_offset_atIndex(Some(upload), 0, 1);
            encoder.drawPrimitives_vertexStart_vertexCount_instanceCount(
                MTLPrimitiveType::Triangle,
                0,
                6,
                frame.paints().len(),
            );
        }
    }
    encoder.endEncoding();
    Ok(())
}

fn encode_atlas_upload(
    command: &CommandBuffer,
    upload: &AtlasUpload,
    buffer: &ProtocolObject<dyn MTLBuffer>,
) -> Result<(), RenderError> {
    if upload.copies.is_empty() {
        return Err(RenderError::SubmissionInvariantViolated);
    }
    validate_atlas_upload_ranges(upload, buffer.length())?;
    let encoder = command
        .blitCommandEncoder()
        .ok_or(RenderError::EncoderUnavailable {
            stage: RenderStage::BlitEncoder,
        })?;
    for copy in &upload.copies {
        // SAFETY: `validate_atlas_upload_ranges` proved both retained buffer
        // ranges before the encoder was acquired. The buffers remain alive
        // through command completion.
        unsafe {
            encoder.copyFromBuffer_sourceOffset_toBuffer_destinationOffset_size(
                &upload.buffer,
                copy.source_offset,
                buffer,
                copy.destination_offset,
                copy.size,
            );
        }
    }
    encoder.endEncoding();
    Ok(())
}

fn validate_atlas_upload_ranges(
    upload: &AtlasUpload,
    destination_length: usize,
) -> Result<(), RenderError> {
    for copy in &upload.copies {
        let source_end = copy
            .source_offset
            .checked_add(copy.size)
            .ok_or(RenderError::AccountingOverflow)?;
        let destination_end = copy
            .destination_offset
            .checked_add(copy.size)
            .ok_or(RenderError::AccountingOverflow)?;
        if source_end > upload.buffer.length() || destination_end > destination_length {
            return Err(RenderError::SubmissionInvariantViolated);
        }
    }
    Ok(())
}

fn create_private_glyph_atlas(device: &Device, byte_len: usize) -> Result<Buffer, RenderError> {
    let buffer = device
        .newBufferWithLength_options(byte_len, MTLResourceOptions::StorageModePrivate)
        .ok_or(RenderError::ResourceUnavailable {
            stage: RenderStage::UploadBuffer,
            requested_bytes: Some(byte_len),
        })?;
    Ok(buffer)
}

fn prepare_atlas_upload(
    device: &Device,
    atlas: &alpine_scene::GlyphAtlasImage,
    kind: AtlasUploadKind,
    staging: Option<&mut AtlasStaging>,
) -> Result<(AtlasUpload, usize, usize), RenderError> {
    if let Some(staging) = staging {
        let (upload, allocated_bytes) = staging.prepare(device, atlas, kind)?;
        return Ok((upload, allocated_bytes, staging.current_bytes()));
    }
    let required = atlas_upload_bytes(atlas, kind)?;
    let buffer = device
        .newBufferWithLength_options(required, MTLResourceOptions::StorageModeShared)
        .ok_or(RenderError::ResourceUnavailable {
            stage: RenderStage::UploadBuffer,
            requested_bytes: Some(required),
        })?;
    let retained_bytes = buffer.allocatedSize();
    let upload = write_atlas_upload(buffer, atlas, kind)?;
    Ok((upload, retained_bytes, retained_bytes))
}

fn atlas_upload_bytes(
    atlas: &alpine_scene::GlyphAtlasImage,
    kind: AtlasUploadKind,
) -> Result<usize, RenderError> {
    if matches!(kind, AtlasUploadKind::Full) {
        return Ok(atlas.pixels().len());
    }
    let bytes = atlas_upload_patches(atlas, kind)
        .iter()
        .try_fold(0_usize, |total, patch| {
            total
                .checked_add(patch.pixels().len())
                .ok_or(RenderError::AccountingOverflow)
        })?;
    if bytes == 0 {
        return Err(RenderError::SubmissionInvariantViolated);
    }
    Ok(bytes)
}

fn atlas_upload_patches(
    atlas: &alpine_scene::GlyphAtlasImage,
    kind: AtlasUploadKind,
) -> &[alpine_scene::GlyphAtlasRowPatch] {
    match kind {
        AtlasUploadKind::Full => &[],
        AtlasUploadKind::DeltaRows => atlas.delta_row_patches(),
        AtlasUploadKind::RecoveryRows => atlas.row_patches(),
    }
}

fn write_atlas_upload(
    buffer: Buffer,
    atlas: &alpine_scene::GlyphAtlasImage,
    kind: AtlasUploadKind,
) -> Result<AtlasUpload, RenderError> {
    let required = atlas_upload_bytes(atlas, kind)?;
    if buffer.length() < required {
        return Err(RenderError::SubmissionInvariantViolated);
    }
    let destination = buffer.contents().cast::<u8>().as_ptr();
    if matches!(kind, AtlasUploadKind::Full) {
        // SAFETY: Scene admission proves the immutable base has exactly the
        // atlas byte length and the shared staging buffer is large enough.
        unsafe { std::ptr::copy_nonoverlapping(atlas.pixels().as_ptr(), destination, required) };
        let width =
            usize::try_from(atlas.width().get()).map_err(|_| RenderError::AccountingOverflow)?;
        for patch in atlas.row_patches() {
            let offset = usize::try_from(patch.start_row())
                .ok()
                .and_then(|row| row.checked_mul(width))
                .ok_or(RenderError::AccountingOverflow)?;
            // SAFETY: Scene admission validates every complete-row patch inside
            // the full atlas allocation.
            unsafe {
                std::ptr::copy_nonoverlapping(
                    patch.pixels().as_ptr(),
                    destination.add(offset),
                    patch.pixels().len(),
                );
            };
        }
        return Ok(AtlasUpload {
            buffer,
            copies: vec![AtlasCopy {
                source_offset: 0,
                destination_offset: 0,
                size: required,
            }]
            .into_boxed_slice(),
        });
    }

    let patches = atlas_upload_patches(atlas, kind);
    let mut copies = Vec::new();
    copies
        .try_reserve_exact(patches.len())
        .map_err(|_| RenderError::ResourceUnavailable {
            stage: RenderStage::UploadBuffer,
            requested_bytes: Some(patches.len().saturating_mul(size_of::<AtlasCopy>())),
        })?;
    let width =
        usize::try_from(atlas.width().get()).map_err(|_| RenderError::AccountingOverflow)?;
    let mut source_offset = 0_usize;
    for patch in patches {
        let destination_offset = usize::try_from(patch.start_row())
            .ok()
            .and_then(|row| row.checked_mul(width))
            .ok_or(RenderError::AccountingOverflow)?;
        copies.push(AtlasCopy {
            source_offset,
            destination_offset,
            size: patch.pixels().len(),
        });
        // SAFETY: `required` is the checked sum of patch bytes, source offsets
        // are monotonic, and the shared staging buffer is large enough.
        unsafe {
            std::ptr::copy_nonoverlapping(
                patch.pixels().as_ptr(),
                destination.add(source_offset),
                patch.pixels().len(),
            );
        };
        source_offset = source_offset
            .checked_add(patch.pixels().len())
            .ok_or(RenderError::AccountingOverflow)?;
    }
    Ok(AtlasUpload {
        buffer,
        copies: copies.into_boxed_slice(),
    })
}

fn create_solid_binding_atlas(device: &Device) -> Result<Buffer, RenderError> {
    let texel = [u8::MAX];
    let bytes = NonNull::from(&texel).cast::<c_void>();
    // SAFETY: The source is one initialized byte and Metal copies it before
    // returning, providing a valid fallback binding for solid-only draws.
    unsafe {
        device.newBufferWithBytes_length_options(bytes, 1, MTLResourceOptions::StorageModeShared)
    }
    .ok_or(RenderError::ResourceUnavailable {
        stage: RenderStage::UploadBuffer,
        requested_bytes: Some(1),
    })
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

#[cfg(any(test, alpine_native_validation))]
fn injected_command_failure(fault: NativeFault) -> Option<(Result<(), RenderError>, bool)> {
    let NativeFault::TerminalError(code) = fault else {
        return None;
    };
    let failure = NativeFailure::new(
        "MTLCommandBufferErrorDomain".to_owned(),
        code,
        "injected terminal command failure".to_owned(),
    );
    let (recovery, device_lost) = classify_command_failure(Some(&failure));
    Some((
        Err(RenderError::CommandFailed {
            status: CommandStatus::Error,
            failure: Some(failure),
            recovery,
        }),
        device_lost,
    ))
}

#[cfg(test)]
fn injected_terminal_result(
    fault: NativeFault,
    result: Result<Bgra8Image, RenderError>,
    frame: &ValidatedFrame,
) -> (Result<Bgra8Image, RenderError>, bool) {
    if let Some((Err(error), device_lost)) = injected_command_failure(fault) {
        return (Err(error), device_lost);
    }
    match fault {
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
        | NativeFault::BlitEncoder
        | NativeFault::TerminalError(_) => (result, false),
    }
}

#[cfg(all(test, not(miri)))]
pub(crate) mod tests {
    #[cfg(feature = "platform-spi")]
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::{error::Error, num::NonZeroU32, process::Command, sync::Arc};

    use alpine_core::{LinearRgba, Point, Rect, Size};
    use alpine_renderer::Renderer;
    use alpine_scene::{
        AtlasBounds, Glyph, GlyphAtlasImage, GlyphAtlasRowPatch, Primitive, Scene, SceneBuilder,
        SceneRevision,
    };
    #[cfg(feature = "platform-spi")]
    use objc2::{
        AnyThread, DefinedClass, define_class, msg_send, rc::Retained, runtime::ProtocolObject,
    };
    #[cfg(feature = "platform-spi")]
    use objc2_foundation::{NSObject, NSObjectProtocol};
    #[cfg(feature = "platform-spi")]
    use objc2_metal::{
        MTLBuffer, MTLCreateSystemDefaultDevice, MTLDevice, MTLDrawable,
        MTLDrawablePresentedHandler, MTLPixelFormat, MTLStorageMode, MTLTexture,
        MTLTextureDescriptor, MTLTextureUsage,
    };

    use crate::initialization::{
        InitializationError, InitializationStage, MetalBackend, initialize_for_native_validation,
    };
    use crate::{
        BackendState, Bgra8Image, OffscreenDescriptor, OffscreenTarget, RecoveryClassification,
        RenderError, RenderStage, ValidatedFrame,
    };

    use super::{
        BlendConfiguration, FRAGMENT_ENTRY_POINT, NativeBackend, NativeDriver, NativeFault,
        OFFLINE_LIBRARY, ResourceProbe, TargetContract, VERTEX_ENTRY_POINT, command_status,
        new_backend,
    };
    #[cfg(feature = "platform-spi")]
    use super::{DrawableResources, new_backend_with_device, new_validation_backend_with_device};
    #[cfg(feature = "platform-spi")]
    use crate::accounting::FrameResourceUsage;

    static CORRUPT_LIBRARY: &[u8] = b"not a Metal library";

    #[cfg(feature = "platform-spi")]
    pub(crate) struct TestDrawableIvars {
        present_calls: AtomicU64,
    }

    #[cfg(feature = "platform-spi")]
    define_class!(
        // SAFETY: NSObject has no subclassing requirements, the atomic ivar is
        // valid for any callback thread, and this fixture has no custom Drop.
        #[unsafe(super = NSObject)]
        #[ivars = TestDrawableIvars]
        pub(crate) struct TestDrawable;

        // SAFETY: NSObjectProtocol adds no unimplemented requirements.
        unsafe impl NSObjectProtocol for TestDrawable {}

        // SAFETY: Both generated selector signatures are implemented exactly.
        unsafe impl MTLDrawable for TestDrawable {
            #[unsafe(method(present))]
            fn present(&self) {
                self.ivars().present_calls.fetch_add(1, Ordering::Relaxed);
            }

            #[allow(
                non_snake_case,
                reason = "the generated protocol requires this method name"
            )]
            #[unsafe(method(presentAtTime:))]
            fn presentAtTime(&self, _presentation_time: f64) {}

            #[allow(
                non_snake_case,
                reason = "the generated protocol requires this method name"
            )]
            #[unsafe(method(presentAfterMinimumDuration:))]
            fn presentAfterMinimumDuration(&self, _duration: f64) {}

            #[allow(
                non_snake_case,
                reason = "the generated protocol requires this method name"
            )]
            #[unsafe(method(addPresentedHandler:))]
            unsafe fn addPresentedHandler(&self, _block: MTLDrawablePresentedHandler) {}

            #[allow(
                non_snake_case,
                reason = "the generated protocol requires this method name"
            )]
            #[unsafe(method(presentedTime))]
            fn presentedTime(&self) -> f64 {
                1.0
            }

            #[allow(
                non_snake_case,
                reason = "the generated protocol requires this method name"
            )]
            #[unsafe(method(drawableID))]
            fn drawableID(&self) -> usize {
                17
            }
        }
    );

    #[cfg(feature = "platform-spi")]
    impl TestDrawable {
        pub(crate) fn new() -> Retained<Self> {
            let allocated = Self::alloc().set_ivars(TestDrawableIvars {
                present_calls: AtomicU64::new(0),
            });
            // SAFETY: This is NSObject's parameterless initializer and the
            // allocated object already contains initialized Rust ivars.
            unsafe { msg_send![super(allocated), init] }
        }

        pub(crate) fn present_calls(&self) -> u64 {
            self.ivars().present_calls.load(Ordering::Relaxed)
        }
    }

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

    #[cfg(feature = "platform-spi")]
    pub(crate) struct CallbackFixture {
        pub(crate) backend: MetalBackend,
        pub(crate) scene: Scene,
        pub(crate) descriptor: OffscreenDescriptor,
        pub(crate) texture: Retained<ProtocolObject<dyn MTLTexture>>,
        pub(crate) drawable: Retained<TestDrawable>,
    }

    #[cfg(feature = "platform-spi")]
    pub(crate) fn callback_fixture() -> Result<CallbackFixture, Box<dyn Error>> {
        let (scene, descriptor) = discriminating_scene()?;
        let backend = validation_backend(BlendConfiguration::PremultipliedSourceOver)?;
        // SAFETY: The dimensions are finite, nonzero, and admitted by the
        // validated descriptor; this fixture creates no mip levels or mapping.
        let texture_descriptor = unsafe {
            MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                MTLPixelFormat::BGRA8Unorm_sRGB,
                descriptor.pixel_width() as usize,
                descriptor.pixel_height() as usize,
                false,
            )
        };
        texture_descriptor.setStorageMode(MTLStorageMode::Private);
        texture_descriptor.setUsage(MTLTextureUsage::RenderTarget);
        let texture = backend
            .native
            .initialized
            .device
            .newTextureWithDescriptor(&texture_descriptor)
            .ok_or("callback fixture texture")?;
        Ok(CallbackFixture {
            backend,
            scene,
            descriptor,
            texture,
            drawable: TestDrawable::new(),
        })
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
            device: None,
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
                atlas_cache: super::GlyphAtlasCache::new(),
                #[cfg(feature = "platform-spi")]
                presentation: super::PresentationSlots::new(),
                #[cfg(feature = "platform-spi")]
                atlas_pressure_pending: false,
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

    fn srgb_encode(linear: f32) -> f32 {
        if linear <= 0.003_130_8 {
            12.92 * linear
        } else {
            1.055 * linear.powf(1.0 / 2.4) - 0.055
        }
    }

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn quantize_unorm(channel: f32) -> u8 {
        (channel.clamp(0.0, 1.0) * 255.0).round() as u8
    }

    fn bgra8(linear: [f32; 4], encode_rgb: bool) -> [u8; 4] {
        let encode = |channel| {
            quantize_unorm(if encode_rgb {
                srgb_encode(channel)
            } else {
                channel
            })
        };
        [
            encode(linear[2]),
            encode(linear[1]),
            encode(linear[0]),
            quantize_unorm(linear[3]),
        ]
    }

    const RSS_SAMPLE_COUNT: usize = 65;

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

    fn qualify_resident_plateau(
        samples: &[(u16, u64)],
        page_bytes: u64,
    ) -> Result<(), Box<dyn Error>> {
        const PLATEAU_SAMPLES: usize = 9;
        const OBSERVATION_PAGE_BUDGET: u64 = 16;

        if samples.len() != RSS_SAMPLE_COUNT {
            return Err(format!(
                "resident qualification requires {RSS_SAMPLE_COUNT} samples, received {}",
                samples.len()
            )
            .into());
        }
        if page_bytes == 0 {
            return Err("resident qualification requires a nonzero host page".into());
        }

        let initial = samples[0].1;
        let observation_ceiling = initial
            .checked_add(
                page_bytes
                    .checked_mul(OBSERVATION_PAGE_BUDGET)
                    .ok_or("resident observation budget overflow")?,
            )
            .ok_or("resident observation ceiling overflow")?;
        let observation_maximum = samples
            .iter()
            .map(|sample| sample.1)
            .max()
            .ok_or("resident observation samples")?;
        if observation_maximum > observation_ceiling {
            return Err(format!(
                "resident bytes exceeded bounded observation budget: initial {initial}, maximum {observation_maximum}, page {page_bytes}"
            )
            .into());
        }

        let plateau = &samples[RSS_SAMPLE_COUNT - PLATEAU_SAMPLES..];
        let plateau_minimum = plateau
            .iter()
            .map(|sample| sample.1)
            .min()
            .ok_or("resident plateau samples")?;
        let plateau_maximum = plateau
            .iter()
            .map(|sample| sample.1)
            .max()
            .ok_or("resident plateau samples")?;
        let plateau_ceiling = plateau_minimum
            .checked_add(page_bytes)
            .ok_or("resident plateau ceiling overflow")?;
        if plateau_maximum > plateau_ceiling {
            return Err(format!(
                "resident bytes did not plateau within one host page: minimum {plateau_minimum}, maximum {plateau_maximum}, page {page_bytes}"
            )
            .into());
        }
        Ok(())
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

    fn glyph_scene(
        revision: u64,
        pixels: Arc<[u8]>,
    ) -> Result<(Scene, OffscreenDescriptor), Box<dyn Error>> {
        glyph_scene_with_atlas(
            revision,
            GlyphAtlasImage::new(
                revision,
                NonZeroU32::new(3).ok_or("atlas width")?,
                NonZeroU32::new(2).ok_or("atlas height")?,
                pixels,
            )?,
        )
    }

    fn glyph_scene_with_atlas(
        revision: u64,
        atlas: GlyphAtlasImage,
    ) -> Result<(Scene, OffscreenDescriptor), Box<dyn Error>> {
        let mut builder = SceneBuilder::new(SceneRevision::new(revision), size(3.0, 1.0)?);
        builder.push(Primitive::Quad {
            bounds: Rect::new(point(0.0, 0.0)?, size(3.0, 1.0)?),
            color: color(1.0, 0.0, 0.0, 1.0)?,
        });
        builder.set_glyph_atlas(atlas)?;
        for (source_x, destination_x) in [0_u32, 1, 2].into_iter().zip([0.0_f32, 1.0, 2.0]) {
            builder.push_glyph(Glyph::new(
                Rect::new(point(destination_x, 0.0)?, size(1.0, 1.0)?),
                AtlasBounds::new(
                    source_x,
                    0,
                    NonZeroU32::new(1).ok_or("glyph width")?,
                    NonZeroU32::new(1).ok_or("glyph height")?,
                ),
                color(1.0, 1.0, 1.0, 1.0)?,
            ))?;
        }
        let descriptor = OffscreenDescriptor::new(3, 1, 1.0, color(0.0, 0.0, 0.0, 0.0)?)?;
        Ok((builder.finish(), descriptor))
    }

    #[test]
    fn renders_a8_glyphs_and_reuses_only_identical_atlas_storage() -> Result<(), Box<dyn Error>> {
        let pixels: Arc<[u8]> = Arc::from([0_u8, 128, 255, 0, 0, 0]);
        let (scene, descriptor) = glyph_scene(81, Arc::clone(&pixels))?;
        let expected = ValidatedFrame::new(&scene, descriptor)?.reference_image()?;
        assert_eq!(expected.pixel(0, 0), Some([0, 0, 255, 255]));
        assert_eq!(expected.pixel(1, 0), Some([128, 128, 255, 255]));
        assert_eq!(expected.pixel(2, 0), Some([255, 255, 255, 255]));

        let mut backend = validation_backend(BlendConfiguration::PremultipliedSourceOver)?;
        let first = backend.render_offscreen(&scene, descriptor)?;
        assert_pixels_within(first.image(), &expected, 1);
        assert_eq!(
            first.report().instance_upload_bytes,
            4 * size_of::<crate::LoweredPaint>()
        );
        assert_eq!(first.report().atlas_upload_bytes, 6);
        assert_eq!(
            first.report().uploaded_bytes,
            first.report().instance_upload_bytes + 6
        );

        let reused = backend.render_offscreen(&scene, descriptor)?;
        assert_pixels_within(reused.image(), &expected, 1);
        assert_eq!(reused.report().atlas_upload_bytes, 0);
        assert_eq!(
            reused.report().retained_bytes,
            reused
                .report()
                .allocated_bytes
                .checked_add(backend.native.atlas_cache.current_bytes)
                .ok_or("reused atlas accounting overflow")?
        );
        for _ in 0..32 {
            let steady = backend.render_offscreen(&scene, descriptor)?;
            assert_eq!(steady.report().atlas_upload_bytes, 0);
        }
        assert_eq!(backend.native.atlas_cache.allocations, 1);
        assert_eq!(backend.native.atlas_cache.uploads, 1);
        assert_eq!(backend.native.atlas_cache.reuses, 33);

        let row_replacement = scene.glyph_atlas().ok_or("glyph atlas")?.with_row_patches(
            81,
            82,
            Arc::from([GlyphAtlasRowPatch::new(
                0,
                NonZeroU32::new(1).ok_or("row count")?,
                Arc::from([255_u8, 128, 0]),
            )]),
        )?;
        let (row_scene, row_descriptor) = glyph_scene_with_atlas(82, row_replacement.clone())?;
        let row_expected = ValidatedFrame::new(&row_scene, row_descriptor)?.reference_image()?;
        let row_updated = backend.render_offscreen(&row_scene, row_descriptor)?;
        assert_pixels_within(row_updated.image(), &row_expected, 1);
        assert_eq!(row_updated.report().atlas_upload_bytes, 3);
        assert_eq!(backend.native.atlas_cache.allocations, 1);
        assert_eq!(backend.native.atlas_cache.uploads, 2);

        let second_row = row_replacement.advance_with_row_patches(
            82,
            83,
            Arc::from([GlyphAtlasRowPatch::new(
                1,
                NonZeroU32::new(1).ok_or("row count")?,
                Arc::from([0_u8, 255, 0]),
            )]),
        )?;
        let (second_row_scene, second_row_descriptor) = glyph_scene_with_atlas(83, second_row)?;
        let second_row_frame = ValidatedFrame::new(&second_row_scene, second_row_descriptor)?;
        let second_row_expected = second_row_frame.reference_image()?;
        backend.native.fault = super::NativeFault::BlitEncoder;
        let rejected = backend.native.render(&second_row_frame);
        assert!(!rejected.committed);
        assert_eq!(backend.native.atlas_cache.uploads, 2);
        assert_eq!(
            backend
                .native
                .atlas_cache
                .image
                .as_ref()
                .map(alpine_scene::GlyphAtlasImage::revision),
            Some(82)
        );
        backend.native.fault = super::NativeFault::None;
        let retried = backend.render_offscreen(&second_row_scene, second_row_descriptor)?;
        assert_pixels_within(retried.image(), &second_row_expected, 1);
        assert_eq!(retried.report().atlas_upload_bytes, 3);
        assert_eq!(backend.native.atlas_cache.allocations, 1);
        assert_eq!(backend.native.atlas_cache.uploads, 3);

        let (replacement, replacement_descriptor) =
            glyph_scene(81, Arc::from([255_u8, 128, 0, 0, 0, 0]))?;
        let replaced = backend.render_offscreen(&replacement, replacement_descriptor)?;
        assert_eq!(replaced.report().atlas_upload_bytes, 6);
        assert_eq!(backend.native.atlas_cache.allocations, 2);
        assert_eq!(backend.native.atlas_cache.uploads, 4);
        assert!(backend.native.atlas_cache.current_bytes >= 6);
        assert_eq!(
            backend.native.atlas_cache.peak_bytes,
            backend.native.atlas_cache.current_bytes
        );

        backend.native.atlas_cache.pressure();
        assert_eq!(backend.native.atlas_cache.current_bytes, 0);
        assert_eq!(backend.native.atlas_cache.pressure_releases, 1);
        Ok(())
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

    #[cfg(feature = "platform-spi")]
    #[test]
    fn supplied_device_is_preserved_by_both_backend_constructors() -> Result<(), Box<dyn Error>> {
        let device = MTLCreateSystemDefaultDevice().ok_or("system Metal device")?;
        let registry_id = device.registryID();
        let driver = NativeDriver::with_device(device.clone());
        assert_eq!(
            driver.device.as_deref().map(MTLDevice::registryID),
            Some(registry_id)
        );

        let (_backend, validation_capabilities) =
            new_validation_backend_with_device(device.clone())?;
        assert_eq!(validation_capabilities.registry_id(), registry_id);

        match new_backend_with_device(device) {
            Ok((_backend, capabilities)) => {
                assert_eq!(capabilities.registry_id(), registry_id);
            }
            Err(InitializationError::UnsupportedDevice { .. }) => {}
            Err(error) => return Err(error.into()),
        }
        Ok(())
    }

    #[cfg(feature = "platform-spi")]
    #[test]
    fn callback_drawable_encodes_commits_and_presents_once() -> Result<(), Box<dyn Error>> {
        let (scene, descriptor) = discriminating_scene()?;
        let frame = ValidatedFrame::new(&scene, descriptor)?;
        let mut backend = validation_backend(BlendConfiguration::PremultipliedSourceOver)?;
        // SAFETY: The dimensions are finite, nonzero, and inside the validated
        // device baseline; the fixture creates no mip levels or CPU mapping.
        let texture_descriptor = unsafe {
            MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                MTLPixelFormat::BGRA8Unorm_sRGB,
                descriptor.pixel_width() as usize,
                descriptor.pixel_height() as usize,
                false,
            )
        };
        texture_descriptor.setStorageMode(MTLStorageMode::Private);
        texture_descriptor.setUsage(MTLTextureUsage::RenderTarget);
        let texture = backend
            .native
            .initialized
            .device
            .newTextureWithDescriptor(&texture_descriptor)
            .ok_or("callback texture")?;
        let drawable = TestDrawable::new();

        let attempt =
            backend
                .native
                .render_drawable(&frame, &texture, ProtocolObject::from_ref(&*drawable));

        assert!(attempt.committed);
        assert!(attempt.present_called);
        assert_eq!(attempt.result, Ok(()));
        assert_eq!(attempt.operations.draw_calls, 1);
        assert_eq!(
            attempt.operations.uploaded_bytes(),
            Some(frame.upload_bytes())
        );
        assert_eq!(attempt.resources.readback_bytes, 0);
        assert_eq!(drawable.present_calls(), 1);
        let first = backend.native.presentation_snapshot();
        assert_eq!(first.occupied_slots, 0);
        assert_eq!(first.upload_allocations, 1);
        assert!(first.current_upload_bytes >= frame.upload_bytes());
        assert_eq!(first.atlas_allocations, 1);
        assert_eq!(first.atlas_uploads, 0);
        assert!(first.current_atlas_bytes >= 1);

        let reused =
            backend
                .native
                .render_drawable(&frame, &texture, ProtocolObject::from_ref(&*drawable));
        assert_eq!(reused.result, Ok(()));
        assert_eq!(reused.resources.allocated_bytes, 0);
        assert_eq!(backend.native.presentation_snapshot().upload_allocations, 1);
        assert_eq!(backend.native.presentation_snapshot().atlas_allocations, 1);
        assert_eq!(drawable.present_calls(), 2);

        backend.native.release_presentation_uploads_on_pressure();
        let released = backend.native.presentation_snapshot();
        assert_eq!(released.current_upload_bytes, 0);
        assert_eq!(released.upload_trims, 1);
        assert_eq!(released.current_atlas_bytes, 0);
        assert_eq!(released.atlas_pressure_releases, 1);
        Ok(())
    }

    #[cfg(feature = "platform-spi")]
    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "pressure, in-flight ownership, full resynchronization, and delta reuse form one native lifecycle journey"
    )]
    fn atlas_pressure_release_preserves_in_flight_drawable_ownership() -> Result<(), Box<dyn Error>>
    {
        let (scene, descriptor) = glyph_scene(82, Arc::from([0_u8, 128, 255, 0, 0, 0]))?;
        let frame = ValidatedFrame::new(&scene, descriptor)?;
        let mut backend = validation_backend(BlendConfiguration::PremultipliedSourceOver)?;
        // SAFETY: The dimensions come from a validated frame and the fixture
        // creates one render-target texture without mip levels or CPU mapping.
        let texture_descriptor = unsafe {
            MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                MTLPixelFormat::BGRA8Unorm_sRGB,
                descriptor.pixel_width() as usize,
                descriptor.pixel_height() as usize,
                false,
            )
        };
        texture_descriptor.setStorageMode(MTLStorageMode::Private);
        texture_descriptor.setUsage(MTLTextureUsage::RenderTarget);
        let texture = backend
            .native
            .initialized
            .device
            .newTextureWithDescriptor(&texture_descriptor)
            .ok_or("glyph callback texture")?;
        let drawable = TestDrawable::new();

        let super::NativeDrawableSubmitAttempt::Submitted(first) = backend.native.submit_drawable(
            0,
            &frame,
            &texture,
            ProtocolObject::from_ref(&*drawable),
        ) else {
            return Err("glyph drawable submission was rejected".into());
        };
        let occupied = backend.native.presentation_snapshot();
        assert_eq!(occupied.occupied_slots, 1);
        assert_eq!(occupied.atlas_allocations, 1);
        assert_eq!(occupied.atlas_uploads, 1);
        assert!(occupied.current_atlas_bytes >= 6);
        assert!(occupied.current_atlas_staging_bytes >= 6);
        assert_eq!(occupied.atlas_staging_allocations, 1);

        backend.native.release_presentation_uploads_on_pressure();
        let released = backend.native.presentation_snapshot();
        assert!(released.current_atlas_bytes >= 6);
        assert!(released.current_atlas_staging_bytes >= 6);
        assert_eq!(released.atlas_pressure_releases, 0);
        assert_eq!(released.occupied_slots, 1);
        assert!(backend.native.wait_drawable(first.id));
        let first = backend
            .native
            .poll_drawable(first.id)?
            .ok_or("in-flight glyph drawable did not complete after pressure")?;
        assert_eq!(first.result, Ok(()));
        assert_eq!(first.operations.atlas_upload_bytes, 6);
        let drained = backend.native.presentation_snapshot();
        assert_eq!(drained.current_atlas_bytes, 0);
        assert_eq!(drained.current_atlas_staging_bytes, 0);
        assert_eq!(drained.atlas_staging_trims, 1);
        assert_eq!(drained.atlas_pressure_releases, 1);

        let super::NativeDrawableSubmitAttempt::Submitted(second) = backend.native.submit_drawable(
            0,
            &frame,
            &texture,
            ProtocolObject::from_ref(&*drawable),
        ) else {
            return Err("glyph drawable was not admitted after pressure".into());
        };
        assert!(backend.native.wait_drawable(second.id));
        let second = backend
            .native
            .poll_drawable(second.id)?
            .ok_or("replacement glyph drawable did not complete")?;
        assert_eq!(second.result, Ok(()));
        assert_eq!(second.operations.atlas_upload_bytes, 6);
        let replaced = backend.native.presentation_snapshot();
        assert_eq!(replaced.atlas_allocations, 2);
        assert_eq!(replaced.atlas_uploads, 2);
        assert_eq!(replaced.atlas_reuses, 0);
        assert_eq!(replaced.atlas_staging_allocations, 2);

        let row_replacement = scene.glyph_atlas().ok_or("glyph atlas")?.with_row_patches(
            82,
            83,
            Arc::from([GlyphAtlasRowPatch::new(
                0,
                NonZeroU32::new(1).ok_or("row count")?,
                Arc::from([255_u8, 128, 0]),
            )]),
        )?;
        let (row_scene, row_descriptor) = glyph_scene_with_atlas(83, row_replacement)?;
        let row_frame = ValidatedFrame::new(&row_scene, row_descriptor)?;
        let super::NativeDrawableSubmitAttempt::Submitted(row_submission) =
            backend.native.submit_drawable(
                0,
                &row_frame,
                &texture,
                ProtocolObject::from_ref(&*drawable),
            )
        else {
            return Err("row glyph drawable was rejected".into());
        };
        assert!(backend.native.wait_drawable(row_submission.id));
        let row_attempt = backend
            .native
            .poll_drawable(row_submission.id)?
            .ok_or("row glyph drawable did not complete")?;
        assert_eq!(row_attempt.result, Ok(()));
        assert_eq!(row_attempt.operations.atlas_upload_bytes, 3);
        let row_reused = backend.native.presentation_snapshot();
        assert_eq!(row_reused.atlas_allocations, 2);
        assert_eq!(row_reused.atlas_staging_allocations, 2);
        assert!(row_reused.slot_atlas_staging_bytes[0] >= 6);
        assert!(row_reused.slot_peak_atlas_staging_bytes[0] >= 6);
        assert!(row_reused.peak_atlas_staging_bytes >= 6);
        Ok(())
    }

    #[cfg(feature = "platform-spi")]
    #[test]
    fn split_phase_drawables_bound_reorder_and_reuse_three_slots() -> Result<(), Box<dyn Error>> {
        let (scene, descriptor) = discriminating_scene()?;
        let frame = ValidatedFrame::new(&scene, descriptor)?;
        let mut backend = validation_backend(BlendConfiguration::PremultipliedSourceOver)?;
        // SAFETY: The validated finite dimensions are supported by the test
        // device and this fixture creates no mip levels or CPU texture mapping.
        let texture_descriptor = unsafe {
            MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                MTLPixelFormat::BGRA8Unorm_sRGB,
                descriptor.pixel_width() as usize,
                descriptor.pixel_height() as usize,
                false,
            )
        };
        texture_descriptor.setStorageMode(MTLStorageMode::Private);
        texture_descriptor.setUsage(MTLTextureUsage::RenderTarget);
        let texture = backend
            .native
            .initialized
            .device
            .newTextureWithDescriptor(&texture_descriptor)
            .ok_or("split-phase callback texture")?;
        let drawable = TestDrawable::new();
        let mut submissions = [None; 3];

        for slot in 0_u8..3 {
            let super::NativeDrawableSubmitAttempt::Submitted(submission) = backend
                .native
                .submit_drawable(slot, &frame, &texture, ProtocolObject::from_ref(&*drawable))
            else {
                return Err("one of three native slots rejected valid work".into());
            };
            submissions[usize::from(slot)] = Some(submission);
        }

        let saturated = backend.native.submit_drawable(
            0,
            &frame,
            &texture,
            ProtocolObject::from_ref(&*drawable),
        );
        let super::NativeDrawableSubmitAttempt::Rejected(saturated) = saturated else {
            return Err("occupied slot admitted a replacement command".into());
        };
        assert!(!saturated.committed);
        assert_eq!(
            saturated.result.as_ref().err().map(RenderError::stage),
            Some(RenderStage::CommandBuffer)
        );

        let occupied = backend.native.presentation_snapshot();
        assert_eq!(occupied.occupied_slots, 3);
        assert_eq!(occupied.upload_allocations, 3);
        assert!(occupied.current_upload_bytes <= 24 * 1024 * 1024);
        assert!(occupied.peak_upload_bytes <= 24 * 1024 * 1024);
        assert_eq!(drawable.present_calls(), 3);

        for index in [2_usize, 0, 1] {
            let submission = submissions[index].ok_or("missing native submission")?;
            assert!(backend.native.wait_drawable(submission.id));
            let attempt = backend
                .native
                .poll_drawable(submission.id)?
                .ok_or("terminal command was not observable")?;
            assert_eq!(attempt.result, Ok(()));
        }
        assert_eq!(backend.native.presentation_snapshot().occupied_slots, 0);

        let super::NativeDrawableSubmitAttempt::Submitted(reused) = backend.native.submit_drawable(
            0,
            &frame,
            &texture,
            ProtocolObject::from_ref(&*drawable),
        ) else {
            return Err("released slot did not admit replacement work".into());
        };
        assert!(backend.native.wait_drawable(reused.id));
        let reused = backend
            .native
            .poll_drawable(reused.id)?
            .ok_or("reused slot did not complete")?;
        assert_eq!(reused.resources.allocated_bytes, 0);
        assert_eq!(backend.native.presentation_snapshot().upload_allocations, 3);
        Ok(())
    }

    #[cfg(feature = "platform-spi")]
    #[test]
    fn presentation_capacity_signal_and_trim_policy_fail_closed() -> Result<(), RenderError> {
        assert_eq!(super::presentation_upload_capacity(0), Some(0));
        assert_eq!(super::presentation_upload_capacity(1), Some(1));
        assert_eq!(super::presentation_upload_capacity(33), Some(64));
        assert_eq!(
            super::presentation_upload_capacity(8 * 1024 * 1024),
            Some(8 * 1024 * 1024)
        );
        assert_eq!(
            super::presentation_upload_capacity(8 * 1024 * 1024 + 1),
            None
        );
        assert_eq!(super::presentation_upload_capacity(usize::MAX), None);

        assert_eq!(
            super::presentation_trim_decision(64, 64, 119, false),
            (0, false)
        );
        assert_eq!(
            super::presentation_trim_decision(64, 32, 118, false),
            (119, false)
        );
        assert_eq!(
            super::presentation_trim_decision(64, 32, 119, false),
            (120, true)
        );
        assert_eq!(
            super::presentation_trim_decision(64, 64, 0, true),
            (1, true)
        );

        let signal = super::CompletionSignal::new();
        signal.reset(7)?;
        signal.publish(
            6,
            super::NativeTerminal {
                device_lost: false,
                result: Ok(()),
            },
        );
        assert!(signal.take(7).is_none());
        signal.publish(
            7,
            super::NativeTerminal {
                device_lost: false,
                result: Ok(()),
            },
        );
        assert!(signal.wait_ready(7));
        assert!(!signal.wait_ready(6));
        assert_eq!(signal.take(7).map(|terminal| terminal.result), Some(Ok(())));
        assert!(signal.take(6).is_none());
        Ok(())
    }

    #[cfg(feature = "platform-spi")]
    #[test]
    fn presentation_upload_copies_reuses_and_grows_exact_paint_bytes() -> Result<(), Box<dyn Error>>
    {
        let (scene, descriptor) = discriminating_scene()?;
        let frame = ValidatedFrame::new(&scene, descriptor)?;
        let mut backend = validation_backend(BlendConfiguration::PremultipliedSourceOver)?;
        let device = backend.native.initialized.device.clone();
        let slot = &mut backend.native.presentation.slots[0];

        let first = slot.prepare_upload(&device, &frame, NativeFault::None)?;
        assert!(first.allocated_bytes > 0);
        assert!(first.current_upload_bytes >= frame.upload_bytes());
        let upload = slot.upload.as_deref().ok_or("first presentation upload")?;
        let first_capacity = upload.length();
        // SAFETY: The slot owns a shared buffer of at least upload_bytes and
        // the validated repr(C) quad slice remains alive for both byte views.
        let actual = unsafe {
            std::slice::from_raw_parts(
                upload.contents().cast::<u8>().as_ptr(),
                frame.upload_bytes(),
            )
        };
        // SAFETY: `upload_bytes` is defined as the exact contiguous byte size
        // of this validated quad slice.
        let expected = unsafe {
            std::slice::from_raw_parts(frame.paints().as_ptr().cast::<u8>(), frame.upload_bytes())
        };
        assert_eq!(actual, expected);

        let reused = slot.prepare_upload(&device, &frame, NativeFault::None)?;
        assert_eq!(reused.allocated_bytes, 0);
        assert_eq!(reused.current_upload_bytes, first.current_upload_bytes);

        let quad_bytes = frame.upload_bytes() / frame.paints().len();
        let required_quads = first_capacity / quad_bytes + 1;
        let mut builder = SceneBuilder::new(SceneRevision::new(72), size(4.0, 3.0)?);
        for _ in 0..required_quads {
            builder.push(Primitive::Quad {
                bounds: Rect::new(point(1.0, 1.0)?, size(1.0, 1.0)?),
                color: color(0.25, 0.5, 0.75, 1.0)?,
            });
        }
        let larger = ValidatedFrame::new(&builder.finish(), descriptor)?;
        assert!(larger.upload_bytes() > first_capacity);
        let grown = slot.prepare_upload(&device, &larger, NativeFault::None)?;
        assert!(grown.allocated_bytes > 0);
        let upload = slot.upload.as_deref().ok_or("grown presentation upload")?;
        assert!(upload.length() > first_capacity);
        // SAFETY: The grown shared buffer is at least the validated upload byte
        // count and the larger frame remains alive for this comparison.
        let actual = unsafe {
            std::slice::from_raw_parts(
                upload.contents().cast::<u8>().as_ptr(),
                larger.upload_bytes(),
            )
        };
        // SAFETY: `upload_bytes` exactly covers the contiguous larger quad slice.
        let expected = unsafe {
            std::slice::from_raw_parts(larger.paints().as_ptr().cast::<u8>(), larger.upload_bytes())
        };
        assert_eq!(actual, expected);
        Ok(())
    }

    #[test]
    fn rejects_corrupt_offline_library_with_native_error() -> Result<(), Box<dyn Error>> {
        let driver = NativeDriver {
            device: None,
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
            device: None,
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
        assert_eq!(
            completed.report().uploaded_bytes,
            3 * std::mem::size_of::<crate::LoweredPaint>()
        );
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
    fn srgb_presentation_encodes_after_linear_blending_and_rejects_linear_control()
    -> Result<(), Box<dyn Error>> {
        let mut builder = SceneBuilder::new(SceneRevision::new(711), size(1.0, 1.0)?);
        let bounds = Rect::new(point(0.0, 0.0)?, size(1.0, 1.0)?);
        builder.push(Primitive::Quad {
            bounds,
            color: color(0.18, 0.50, 0.75, 1.0)?,
        });
        builder.push(Primitive::Quad {
            bounds,
            color: color(0.80, 0.20, 0.04, 0.25)?,
        });
        let scene = builder.finish();
        let descriptor = OffscreenDescriptor::new(1, 1, 1.0, color(0.0, 0.0, 0.0, 1.0)?)?;
        let frame = ValidatedFrame::new(&scene, descriptor)?;
        let mut backend = validation_backend(BlendConfiguration::PremultipliedSourceOver)?;

        let linear_attempt = backend
            .native
            .render_to_readback(&frame, TargetContract::LinearOffscreen);
        let srgb_attempt = backend
            .native
            .render_to_readback(&frame, TargetContract::SrgbPresentation);
        assert!(linear_attempt.committed);
        assert!(srgb_attempt.committed);
        let linear_image = linear_attempt.result?;
        let srgb_image = srgb_attempt.result?;

        let blended_linear = [
            0.80 * 0.25 + 0.18 * 0.75,
            0.20 * 0.25 + 0.50 * 0.75,
            0.04 * 0.25 + 0.75 * 0.75,
            1.0,
        ];
        let expected_linear = bgra8(blended_linear, false);
        let expected_srgb = bgra8(blended_linear, true);
        for (actual, expected) in linear_image.bytes().iter().zip(expected_linear) {
            assert!(actual.abs_diff(expected) <= 1);
        }
        for (actual, expected) in srgb_image.bytes().iter().zip(expected_srgb) {
            assert!(actual.abs_diff(expected) <= 1);
        }
        assert!(
            srgb_image
                .bytes()
                .iter()
                .zip(expected_linear)
                .any(|(actual, wrong)| actual.abs_diff(wrong) > 12),
            "a direct linear-unorm transfer must not qualify as sRGB presentation"
        );
        assert_ne!(expected_linear, expected_srgb);
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

    #[cfg(feature = "platform-spi")]
    #[test]
    fn callback_drawable_rejects_extent_and_format_before_allocation() -> Result<(), Box<dyn Error>>
    {
        let initialized = initialize_for_native_validation(&NativeDriver::production())?;
        let scene = SceneBuilder::new(SceneRevision::new(731), size(2.0, 2.0)?).finish();
        let descriptor = OffscreenDescriptor::new(2, 2, 1.0, color(0.0, 0.0, 0.0, 1.0)?)?;
        let frame = ValidatedFrame::new(&scene, descriptor)?;

        // SAFETY: Both controls use finite nonzero dimensions supported by the
        // validation device and create no mipmapped or CPU-visible resource.
        let wrong_extent_descriptor = unsafe {
            MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                MTLPixelFormat::BGRA8Unorm_sRGB,
                1,
                2,
                false,
            )
        };
        wrong_extent_descriptor.setStorageMode(MTLStorageMode::Private);
        wrong_extent_descriptor.setUsage(MTLTextureUsage::RenderTarget);
        let wrong_extent = initialized
            .device
            .newTextureWithDescriptor(&wrong_extent_descriptor)
            .ok_or("wrong-extent texture")?;
        let mut extent_staging = super::AtlasStaging::default();
        let extent_failure = DrawableResources::new(
            &initialized.device,
            &wrong_extent,
            &frame,
            MTLPixelFormat::BGRA8Unorm_sRGB,
            &mut super::GlyphAtlasCache::new(),
            &mut extent_staging,
        )
        .err()
        .ok_or("wrong extent must fail")?;
        assert_eq!(
            extent_failure.error,
            RenderError::DrawableExtentMismatch {
                expected_width: 2,
                expected_height: 2,
                actual_width: 1,
                actual_height: 2,
            }
        );
        assert_eq!(extent_failure.usage, FrameResourceUsage::default());

        // SAFETY: This control differs only in pixel format and otherwise uses
        // the same finite supported target dimensions.
        let wrong_format_descriptor = unsafe {
            MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                MTLPixelFormat::BGRA8Unorm,
                2,
                2,
                false,
            )
        };
        wrong_format_descriptor.setStorageMode(MTLStorageMode::Private);
        wrong_format_descriptor.setUsage(MTLTextureUsage::RenderTarget);
        let wrong_format = initialized
            .device
            .newTextureWithDescriptor(&wrong_format_descriptor)
            .ok_or("wrong-format texture")?;
        let mut format_staging = super::AtlasStaging::default();
        let format_failure = DrawableResources::new(
            &initialized.device,
            &wrong_format,
            &frame,
            MTLPixelFormat::BGRA8Unorm_sRGB,
            &mut super::GlyphAtlasCache::new(),
            &mut format_staging,
        )
        .err()
        .ok_or("wrong format must fail")?;
        assert_eq!(
            format_failure.error,
            RenderError::DrawablePixelFormatMismatch {
                actual: MTLPixelFormat::BGRA8Unorm.0,
            }
        );
        assert_eq!(format_failure.usage, FrameResourceUsage::default());
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
                144,
                0,
            ),
            (
                NativeFault::RenderEncoder,
                RenderStage::RenderEncoder,
                144,
                0,
            ),
            (NativeFault::BlitEncoder, RenderStage::BlitEncoder, 144, 1),
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
            assert_eq!(
                accounting.uploaded_bytes(),
                u128::try_from(3 * std::mem::size_of::<crate::LoweredPaint>())?
            );
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
    #[allow(
        clippy::too_many_lines,
        reason = "the cancellation, shutdown, and steady-state accounting journey remains one fixture"
    )]
    fn cancellation_shutdown_and_steady_state_have_no_hidden_native_work()
    -> Result<(), Box<dyn Error>> {
        const VALIDATION_WARMUP_FRAMES: u16 = 256;
        const RSS_WARMUP_FRAMES: u16 = 4_096;
        const VALIDATION_MEASURED_FRAMES: u16 = 256;
        const RSS_MEASURED_FRAMES: u16 = 1_024;

        let (scene, descriptor) = discriminating_scene()?;
        let (mut backend, probe) = validation_backend_and_probe(
            BlendConfiguration::PremultipliedSourceOver,
            NativeFault::None,
        )?;

        let cancellation = backend.cancel_offscreen(&scene, descriptor)?;
        assert_eq!(cancellation.generation().get(), 1);
        assert_eq!(cancellation.primitives(), 4);
        assert_eq!(cancellation.omitted_primitives(), 1);
        assert_eq!(
            cancellation.uploaded_bytes_avoided(),
            3 * std::mem::size_of::<crate::LoweredPaint>()
        );
        assert_eq!(probe.counts(), (0, 0, 0));
        assert_eq!(backend.accounting().uploaded_bytes(), 0);
        assert_eq!(backend.accounting().draw_calls(), 0);

        let mut expected_allocated = 0_u128;
        let mut expected_readback = 0_u128;
        let mut retained = None;
        let capture_resident_distribution = std::env::var_os("ALPINE_CAPTURE_RSS").is_some();
        if capture_resident_distribution {
            for _ in 0..RSS_SAMPLE_COUNT {
                let _ = resident_bytes()?;
            }
        }
        let warmup_frames = if capture_resident_distribution {
            RSS_WARMUP_FRAMES
        } else {
            VALIDATION_WARMUP_FRAMES
        };
        let measured_frames = if capture_resident_distribution {
            RSS_MEASURED_FRAMES
        } else {
            VALIDATION_MEASURED_FRAMES
        };
        let total_frames = warmup_frames + measured_frames;
        let mut resident_samples = Vec::with_capacity(RSS_SAMPLE_COUNT);
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
        let frame_upload_bytes = u128::try_from(3 * std::mem::size_of::<crate::LoweredPaint>())?;
        assert_eq!(
            accounting.uploaded_bytes(),
            u128::from(total_frames) * frame_upload_bytes
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
            assert_eq!(resident_samples.len(), RSS_SAMPLE_COUNT);
            for (frame, bytes) in &resident_samples {
                assert!(*bytes > 0);
                println!("alpine-memory-sample frame={frame} resident_bytes={bytes}");
            }
            let page_bytes = host_page_bytes()?;
            qualify_resident_plateau(&resident_samples, page_bytes)?;
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
    fn resident_plateau_rejects_unbounded_or_late_growth() {
        let bounded = (0_u16..65)
            .map(|index| (index, 10_000 + u64::from(index.min(4)) * 4_096))
            .collect::<Vec<_>>();
        assert!(qualify_resident_plateau(&bounded, 16_384).is_ok());

        let delayed_but_bounded_settling = (0_u16..65)
            .map(|index| (index, 10_000 + u64::from(index.min(12)) * 4_096))
            .collect::<Vec<_>>();
        assert!(qualify_resident_plateau(&delayed_but_bounded_settling, 16_384).is_ok());

        let excessive_settling = (0_u16..65)
            .map(|index| (index, 10_000 + u64::from(index) * 32_768))
            .collect::<Vec<_>>();
        assert!(qualify_resident_plateau(&excessive_settling, 16_384).is_err());

        let late_growth = (0_u16..65)
            .map(|index| {
                let bytes = if index < 56 {
                    10_000
                } else {
                    10_000 + u64::from(index - 56) * 16_384
                };
                (index, bytes)
            })
            .collect::<Vec<_>>();
        assert!(qualify_resident_plateau(&late_growth, 16_384).is_err());
        assert!(qualify_resident_plateau(&bounded[..64], 16_384).is_err());
        assert!(qualify_resident_plateau(&bounded, 0).is_err());
    }

    #[test]
    fn atlas_patch_compatibility_and_staging_limit_boundaries_are_exact() {
        use std::cmp::Ordering;

        use super::{
            AtlasMatch::{Different, Same},
            AtlasPatchCompatibility,
            AtlasPatchPresence::{Empty, Present},
        };

        let compatible = AtlasPatchCompatibility {
            base_revision: Same,
            revision_ordering: Ordering::Less,
            extent: Same,
            storage: Same,
            row_patches: Present,
        };
        assert!(compatible.is_compatible());
        for rejected in [
            AtlasPatchCompatibility {
                base_revision: Different,
                ..compatible
            },
            AtlasPatchCompatibility {
                revision_ordering: Ordering::Equal,
                ..compatible
            },
            AtlasPatchCompatibility {
                revision_ordering: Ordering::Greater,
                ..compatible
            },
            AtlasPatchCompatibility {
                extent: Different,
                ..compatible
            },
            AtlasPatchCompatibility {
                storage: Different,
                ..compatible
            },
            AtlasPatchCompatibility {
                row_patches: Empty,
                ..compatible
            },
        ] {
            assert!(!rejected.is_compatible());
        }

        assert_eq!(super::ATLAS_STAGING_LIMIT, 16_777_216);
        assert_eq!(super::atlas_staging_capacity(1), Some(1));
        assert_eq!(
            super::atlas_staging_capacity(super::ATLAS_STAGING_LIMIT),
            Some(super::ATLAS_STAGING_LIMIT)
        );
        assert_eq!(
            super::atlas_staging_capacity(super::ATLAS_STAGING_LIMIT + 1),
            None
        );
    }

    #[cfg(feature = "platform-spi")]
    #[test]
    fn atlas_staging_reuses_exact_capacity_and_trims_after_sustained_underuse()
    -> Result<(), Box<dyn Error>> {
        let (scene, _) = glyph_scene(141, Arc::from([0_u8, 128, 255, 0, 0, 0]))?;
        let atlas = scene.glyph_atlas().ok_or("glyph atlas")?;
        let row_atlas = atlas.with_row_patches(
            141,
            142,
            Arc::from([GlyphAtlasRowPatch::new(
                0,
                NonZeroU32::new(1).ok_or("row count")?,
                Arc::from([255_u8, 128, 0]),
            )]),
        )?;
        let backend = validation_backend(BlendConfiguration::PremultipliedSourceOver)?;
        let device = &backend.native.initialized.device;
        let mut staging = super::AtlasStaging::default();

        let (_, first_allocation) = staging.prepare(device, atlas, super::AtlasUploadKind::Full)?;
        let full_capacity = staging.current_bytes();
        assert!(first_allocation >= atlas.pixels().len());
        assert_eq!(full_capacity, 8);
        assert_eq!(staging.allocations, 1);

        let (_, repeated_allocation) =
            staging.prepare(device, atlas, super::AtlasUploadKind::Full)?;
        assert_eq!(repeated_allocation, 0);
        assert_eq!(staging.current_bytes(), full_capacity);
        assert_eq!(staging.allocations, 1);

        let (_, delta_allocation) =
            staging.prepare(device, &row_atlas, super::AtlasUploadKind::DeltaRows)?;
        assert_eq!(delta_allocation, 0);
        assert_eq!(staging.last_demand, 3);
        for _ in 1..super::ATLAS_STAGING_TRIM_TERMINALS {
            staging.observe_terminal();
        }
        assert_eq!(staging.current_bytes(), full_capacity);
        assert_eq!(staging.trims, 0);

        staging.observe_terminal();
        assert_eq!(staging.current_bytes(), 0);
        assert_eq!(staging.trims, 1);
        assert_eq!(staging.underused_terminals, 0);
        Ok(())
    }

    #[test]
    fn atlas_blit_rejects_invalid_ranges_before_encoder_acquisition() -> Result<(), Box<dyn Error>>
    {
        use objc2_metal::{MTLCommandQueue as _, MTLDevice as _, MTLResourceOptions};

        let backend = validation_backend(BlendConfiguration::PremultipliedSourceOver)?;
        let device = &backend.native.initialized.device;
        let queue = device.newCommandQueue().ok_or("command queue")?;
        let command = queue.commandBuffer().ok_or("command buffer")?;
        let source = device
            .newBufferWithLength_options(1, MTLResourceOptions::StorageModeShared)
            .ok_or("atlas source")?;
        let destination = device
            .newBufferWithLength_options(1, MTLResourceOptions::StorageModePrivate)
            .ok_or("atlas destination")?;
        let upload = super::AtlasUpload {
            buffer: source,
            copies: vec![super::AtlasCopy {
                source_offset: 1,
                destination_offset: 0,
                size: 1,
            }]
            .into_boxed_slice(),
        };

        assert_eq!(
            super::encode_atlas_upload(&command, &upload, &destination),
            Err(RenderError::SubmissionInvariantViolated)
        );

        let valid_source = device
            .newBufferWithLength_options(1, MTLResourceOptions::StorageModeShared)
            .ok_or("valid atlas source")?;
        let invalid_destination_upload = super::AtlasUpload {
            buffer: valid_source,
            copies: vec![super::AtlasCopy {
                source_offset: 0,
                destination_offset: 1,
                size: 1,
            }]
            .into_boxed_slice(),
        };
        assert_eq!(
            super::encode_atlas_upload(&command, &invalid_destination_upload, &destination),
            Err(RenderError::SubmissionInvariantViolated)
        );
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
