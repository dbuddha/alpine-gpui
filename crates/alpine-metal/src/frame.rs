use std::{error::Error, fmt, mem::size_of};

use alpine_core::LinearRgba;
use alpine_scene::{GlyphAtlasImage, PaintOperation, Scene, SceneRevision};

/// Bytes in one compact BGRA8 pixel.
pub const BGRA_BYTES_PER_PIXEL: usize = 4;
/// Maximum A8 atlas payload admitted by one validated frame.
pub const MAX_GLYPH_ATLAS_BYTES: usize = 16 * 1024 * 1024;

/// Required byte alignment for one Metal texture-to-buffer row.
pub const READBACK_ROW_ALIGNMENT: usize = 256;

/// Guaranteed two-dimensional texture limit for the required Metal 3 family.
pub const MAX_METAL3_TEXTURE_DIMENSION_2D: u32 = 16_384;

/// A validated deterministic offscreen target.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OffscreenDescriptor {
    pixel_width: u32,
    pixel_height: u32,
    scale: f32,
    clear: LinearRgba,
}

impl OffscreenDescriptor {
    /// Validates a non-empty physical target and a positive finite scale.
    ///
    /// # Errors
    ///
    /// Returns [`OffscreenError::ZeroPixelExtent`] for an empty target and
    /// [`OffscreenError::InvalidScale`] for a zero, negative, or non-finite scale.
    pub fn new(
        pixel_width: u32,
        pixel_height: u32,
        scale: f32,
        clear: LinearRgba,
    ) -> Result<Self, OffscreenError> {
        if pixel_width == 0 || pixel_height == 0 {
            return Err(OffscreenError::ZeroPixelExtent);
        }
        if !scale.is_finite() || scale <= 0.0 {
            return Err(OffscreenError::InvalidScale);
        }
        Ok(Self {
            pixel_width,
            pixel_height,
            scale,
            clear,
        })
    }

    /// Returns the target width in physical pixels.
    #[must_use]
    pub const fn pixel_width(self) -> u32 {
        self.pixel_width
    }

    /// Returns the target height in physical pixels.
    #[must_use]
    pub const fn pixel_height(self) -> u32 {
        self.pixel_height
    }

    /// Returns the logical-to-physical scale.
    #[must_use]
    pub const fn scale(self) -> f32 {
        self.scale
    }

    /// Returns the unpremultiplied linear clear color.
    #[must_use]
    pub const fn clear(self) -> LinearRgba {
        self.clear
    }
}

/// Checked compact and Metal-aligned byte layout for offscreen readback.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReadbackLayout {
    pub(crate) compact_bytes_per_row: usize,
    pub(crate) aligned_bytes_per_row: usize,
    pub(crate) compact_len: usize,
    pub(crate) buffer_len: usize,
}

impl ReadbackLayout {
    pub(crate) fn new(width: u32, height: u32) -> Result<Self, OffscreenError> {
        let compact_row_u64 = u64::from(width)
            .checked_mul(BGRA_BYTES_PER_PIXEL as u64)
            .ok_or(OffscreenError::CompactRowSizeOverflow)?;
        let compact_bytes_per_row =
            usize::try_from(compact_row_u64).map_err(|_| OffscreenError::CompactRowSizeOverflow)?;
        let aligned_bytes_per_row = compact_bytes_per_row
            .checked_add(READBACK_ROW_ALIGNMENT - 1)
            .map(|value| value & !(READBACK_ROW_ALIGNMENT - 1))
            .ok_or(OffscreenError::AlignedRowSizeOverflow)?;
        let height = usize::try_from(height).map_err(|_| OffscreenError::ReadbackSizeOverflow)?;
        let compact_len = compact_bytes_per_row
            .checked_mul(height)
            .ok_or(OffscreenError::CompactImageSizeOverflow)?;
        let buffer_len = aligned_bytes_per_row
            .checked_mul(height)
            .ok_or(OffscreenError::ReadbackSizeOverflow)?;
        Ok(Self {
            compact_bytes_per_row,
            aligned_bytes_per_row,
            compact_len,
            buffer_len,
        })
    }

    /// Returns bytes in one compact image row.
    #[must_use]
    pub const fn compact_bytes_per_row(self) -> usize {
        self.compact_bytes_per_row
    }

    /// Returns bytes in one Metal-aligned readback row.
    #[must_use]
    pub const fn aligned_bytes_per_row(self) -> usize {
        self.aligned_bytes_per_row
    }

    /// Returns the compact image byte length.
    #[must_use]
    pub const fn compact_len(self) -> usize {
        self.compact_len
    }

    /// Returns the padded Metal readback buffer length.
    #[must_use]
    pub const fn buffer_len(self) -> usize {
        self.buffer_len
    }
}

/// One shader-ready paint instance in physical-pixel coordinates.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LoweredPaint {
    bounds: [f32; 4],
    color: [f32; 4],
    atlas_uv: [f32; 4],
}

impl LoweredPaint {
    /// Returns `[left, top, right, bottom]` in physical pixels.
    #[must_use]
    pub const fn bounds(self) -> [f32; 4] {
        self.bounds
    }

    /// Returns unpremultiplied linear `[red, green, blue, alpha]`.
    #[must_use]
    pub const fn color(self) -> [f32; 4] {
        self.color
    }

    /// Returns normalized atlas UV bounds, or negative values for a solid.
    #[must_use]
    pub const fn atlas_uv(self) -> [f32; 4] {
        self.atlas_uv
    }
}

/// A fully checked, immutable frame ready for native encoding.
#[derive(Clone, Debug, PartialEq)]
pub struct ValidatedFrame {
    revision: SceneRevision,
    descriptor: OffscreenDescriptor,
    readback: ReadbackLayout,
    paints: Vec<LoweredPaint>,
    glyph_atlas: Option<GlyphAtlasImage>,
    consumed_primitives: usize,
    omitted_primitives: usize,
    upload_bytes: usize,
}

impl ValidatedFrame {
    /// Validates and lowers a complete scene without performing native work.
    ///
    /// Positive physical extents use round-to-nearest with half values rounded
    /// away from zero. Empty and fully clipped paints are omitted and counted.
    ///
    /// # Errors
    ///
    /// Returns a structured error for viewport mismatch, unrepresentable
    /// physical coordinates, readback arithmetic overflow, upload overflow, or
    /// frame-plan allocation failure.
    pub fn new(scene: &Scene, descriptor: OffscreenDescriptor) -> Result<Self, OffscreenError> {
        Self::new_with_reservation(scene, descriptor, |paints, count| {
            map_reservation_failure(
                paints.try_reserve_exact(count),
                OffscreenError::FramePlanAllocationFailed { paints: count },
            )
        })
    }

    fn new_with_reservation<F>(
        scene: &Scene,
        descriptor: OffscreenDescriptor,
        reserve: F,
    ) -> Result<Self, OffscreenError>
    where
        F: FnOnce(&mut Vec<LoweredPaint>, usize) -> Result<(), OffscreenError>,
    {
        validate_viewport(scene, descriptor)?;
        if let Some(atlas) = scene.glyph_atlas() {
            validate_glyph_atlas_contract(
                atlas.width().get(),
                atlas.height().get(),
                atlas.pixels().len(),
            )?;
        }
        let readback = ReadbackLayout::new(descriptor.pixel_width(), descriptor.pixel_height())?;
        let consumed_primitives = scene.operation_count();
        let mut paints = Vec::new();
        reserve(&mut paints, consumed_primitives)?;

        for (primitive_index, operation) in scene.operations().iter().enumerate() {
            match *operation {
                PaintOperation::Quad(quad_id) => {
                    let quad = scene
                        .quads()
                        .get(quad_id.index())
                        .ok_or(OffscreenError::InvalidSceneOperation { primitive_index })?;
                    let bounds = quad.bounds();
                    let color = quad.color();
                    let clip = operation_clip(scene, quad.clip(), primitive_index)?;
                    if let Some(quad) = lower_quad(
                        bounds.origin().x(),
                        bounds.origin().y(),
                        bounds.size().width(),
                        bounds.size().height(),
                        color,
                        scene.viewport().width(),
                        scene.viewport().height(),
                        descriptor.scale(),
                        primitive_index,
                        clip,
                    )? {
                        paints.push(quad);
                    }
                }
                PaintOperation::Glyph(glyph_id) => {
                    let glyph = scene
                        .glyphs()
                        .get(glyph_id.index())
                        .ok_or(OffscreenError::InvalidSceneOperation { primitive_index })?;
                    let atlas = scene
                        .glyph_atlas()
                        .ok_or(OffscreenError::MissingGlyphAtlas { primitive_index })?;
                    let clip = operation_clip(scene, glyph.clip(), primitive_index)?;
                    if let Some(glyph) = lower_glyph(
                        *glyph,
                        atlas,
                        scene.viewport().width(),
                        scene.viewport().height(),
                        descriptor.scale(),
                        primitive_index,
                        clip,
                    )? {
                        paints.push(glyph);
                    }
                }
            }
        }

        let upload_bytes = paints
            .len()
            .checked_mul(size_of::<LoweredPaint>())
            .ok_or(OffscreenError::UploadSizeOverflow)?;
        let omitted_primitives = consumed_primitives - paints.len();
        Ok(Self {
            revision: scene.revision(),
            descriptor,
            readback,
            paints,
            glyph_atlas: scene.glyph_atlas().cloned(),
            consumed_primitives,
            omitted_primitives,
            upload_bytes,
        })
    }

    /// Returns the source scene revision.
    #[must_use]
    pub const fn revision(&self) -> SceneRevision {
        self.revision
    }

    /// Returns the validated target descriptor.
    #[must_use]
    pub const fn descriptor(&self) -> OffscreenDescriptor {
        self.descriptor
    }

    /// Returns the checked readback layout.
    #[must_use]
    pub const fn readback_layout(&self) -> ReadbackLayout {
        self.readback
    }

    /// Returns visible paints in original painter order.
    #[must_use]
    pub fn paints(&self) -> &[LoweredPaint] {
        &self.paints
    }

    /// Returns the immutable A8 atlas used by glyph paint instances.
    #[must_use]
    pub const fn glyph_atlas(&self) -> Option<&GlyphAtlasImage> {
        self.glyph_atlas.as_ref()
    }

    /// Returns every consumed source primitive, including omitted primitives.
    #[must_use]
    pub const fn consumed_primitives(&self) -> usize {
        self.consumed_primitives
    }

    /// Returns the number of empty or fully clipped primitives.
    #[must_use]
    pub const fn omitted_primitives(&self) -> usize {
        self.omitted_primitives
    }

    /// Returns the exact shader-instance upload byte count.
    #[must_use]
    pub const fn upload_bytes(&self) -> usize {
        self.upload_bytes
    }
}

fn validate_glyph_atlas_contract(
    width: u32,
    height: u32,
    bytes: usize,
) -> Result<(), OffscreenError> {
    if width > MAX_METAL3_TEXTURE_DIMENSION_2D || height > MAX_METAL3_TEXTURE_DIMENSION_2D {
        return Err(OffscreenError::GlyphAtlasExtentUnsupported {
            width,
            height,
            limit: MAX_METAL3_TEXTURE_DIMENSION_2D,
        });
    }
    if bytes > MAX_GLYPH_ATLAS_BYTES {
        return Err(OffscreenError::GlyphAtlasByteLimitExceeded {
            bytes,
            limit: MAX_GLYPH_ATLAS_BYTES,
        });
    }
    Ok(())
}

fn validate_viewport(scene: &Scene, descriptor: OffscreenDescriptor) -> Result<(), OffscreenError> {
    let expected_width = rounded_physical_extent(scene.viewport().width(), descriptor.scale())?;
    let expected_height = rounded_physical_extent(scene.viewport().height(), descriptor.scale())?;
    if expected_width != descriptor.pixel_width() || expected_height != descriptor.pixel_height() {
        return Err(OffscreenError::ViewportPixelMismatch {
            expected_width,
            expected_height,
            actual_width: descriptor.pixel_width(),
            actual_height: descriptor.pixel_height(),
        });
    }
    Ok(())
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn rounded_physical_extent(logical: f32, scale: f32) -> Result<u32, OffscreenError> {
    let scaled = f64::from(logical) * f64::from(scale);
    let rounded = scaled.round();
    if rounded > f64::from(u32::MAX) {
        return Err(OffscreenError::ViewportScaleOverflow);
    }
    Ok(rounded as u32)
}

#[allow(clippy::too_many_arguments)]
fn lower_quad(
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    color: LinearRgba,
    viewport_width: f32,
    viewport_height: f32,
    scale: f32,
    primitive_index: usize,
    clip: [f32; 4],
) -> Result<Option<LoweredPaint>, OffscreenError> {
    let left = f64::from(x).max(0.0).max(f64::from(clip[0]));
    let top = f64::from(y).max(0.0).max(f64::from(clip[1]));
    let right = (f64::from(x) + f64::from(width))
        .min(f64::from(viewport_width))
        .min(f64::from(clip[2]));
    let bottom = (f64::from(y) + f64::from(height))
        .min(f64::from(viewport_height))
        .min(f64::from(clip[3]));
    if right <= left || bottom <= top {
        return Ok(None);
    }

    let physical = [left, top, right, bottom].map(|value| value * f64::from(scale));
    let bounds = physical.map(to_shader_coordinate);
    if bounds[2] <= bounds[0] || bounds[3] <= bounds[1] {
        return Err(OffscreenError::UnrepresentableQuad { primitive_index });
    }

    Ok(Some(LoweredPaint {
        bounds,
        color: [color.red(), color.green(), color.blue(), color.alpha()],
        atlas_uv: [-1.0; 4],
    }))
}

fn operation_clip(
    scene: &Scene,
    clip: Option<alpine_scene::ClipId>,
    primitive_index: usize,
) -> Result<[f32; 4], OffscreenError> {
    let Some(clip) = clip else {
        return Ok([
            0.0,
            0.0,
            scene.viewport().width(),
            scene.viewport().height(),
        ]);
    };
    let bounds = scene
        .clips()
        .get(clip.index())
        .ok_or(OffscreenError::InvalidSceneOperation { primitive_index })?
        .bounds();
    Ok([
        bounds.origin().x(),
        bounds.origin().y(),
        bounds.origin().x() + bounds.size().width(),
        bounds.origin().y() + bounds.size().height(),
    ])
}

#[allow(clippy::too_many_arguments)]
fn lower_glyph(
    glyph: alpine_scene::Glyph,
    atlas: &GlyphAtlasImage,
    viewport_width: f32,
    viewport_height: f32,
    scale: f32,
    primitive_index: usize,
    clip: [f32; 4],
) -> Result<Option<LoweredPaint>, OffscreenError> {
    let source = glyph.atlas_bounds();
    let bounds = glyph.bounds();
    let x = f64::from(bounds.origin().x());
    let y = f64::from(bounds.origin().y());
    let width = f64::from(bounds.size().width());
    let height = f64::from(bounds.size().height());
    let left = x.max(0.0).max(f64::from(clip[0]));
    let top = y.max(0.0).max(f64::from(clip[1]));
    let right = (x + width)
        .min(f64::from(viewport_width))
        .min(f64::from(clip[2]));
    let bottom = (y + height)
        .min(f64::from(viewport_height))
        .min(f64::from(clip[3]));
    if right <= left || bottom <= top {
        return Ok(None);
    }
    let physical = [left, top, right, bottom].map(|value| value * f64::from(scale));
    let lowered_bounds = physical.map(to_shader_coordinate);
    if lowered_bounds[2] <= lowered_bounds[0] || lowered_bounds[3] <= lowered_bounds[1] {
        return Err(OffscreenError::UnrepresentableQuad { primitive_index });
    }
    let atlas_width = f64::from(atlas.width().get());
    let atlas_height = f64::from(atlas.height().get());
    let source_x = f64::from(source.x());
    let source_y = f64::from(source.y());
    let source_width = f64::from(source.width().get());
    let source_height = f64::from(source.height().get());
    let atlas_uv = [
        (source_x + (left - x) / width * source_width) / atlas_width,
        (source_y + (top - y) / height * source_height) / atlas_height,
        (source_x + (right - x) / width * source_width) / atlas_width,
        (source_y + (bottom - y) / height * source_height) / atlas_height,
    ]
    .map(to_shader_coordinate);
    let color = glyph.color();
    Ok(Some(LoweredPaint {
        bounds: lowered_bounds,
        color: [color.red(), color.green(), color.blue(), color.alpha()],
        atlas_uv,
    }))
}

#[allow(clippy::cast_possible_truncation)]
fn to_shader_coordinate(value: f64) -> f32 {
    debug_assert!(value.is_finite() && value >= 0.0 && value <= f64::from(u32::MAX) + 0.5);
    value as f32
}

pub(crate) fn map_reservation_failure<E>(
    result: Result<(), E>,
    failure: OffscreenError,
) -> Result<(), OffscreenError> {
    result.map_err(|_| failure)
}

/// A fail-closed frame validation or reference-image error.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OffscreenError {
    /// A physical target dimension was zero.
    ZeroPixelExtent,
    /// The logical-to-physical scale was not positive and finite.
    InvalidScale,
    /// Rounded logical viewport dimensions did not equal the target dimensions.
    ViewportPixelMismatch {
        /// Expected target width.
        expected_width: u32,
        /// Expected target height.
        expected_height: u32,
        /// Supplied target width.
        actual_width: u32,
        /// Supplied target height.
        actual_height: u32,
    },
    /// Scaling the viewport exceeded the physical extent representation.
    ViewportScaleOverflow,
    /// A clipped quad could not be represented by the shader ABI.
    UnrepresentableQuad {
        /// Painter-order index of the rejected primitive.
        primitive_index: usize,
    },
    /// A paint operation referenced an absent structure-of-arrays entry.
    InvalidSceneOperation {
        /// Painter-order index of the rejected operation.
        primitive_index: usize,
    },
    /// The current offscreen pipeline has not yet lowered a glyph operation.
    MissingGlyphAtlas {
        /// Painter-order index of the rejected operation.
        primitive_index: usize,
    },
    /// Compact row-byte arithmetic overflowed.
    CompactRowSizeOverflow,
    /// Aligned row-byte arithmetic overflowed.
    AlignedRowSizeOverflow,
    /// Compact image-byte arithmetic overflowed.
    CompactImageSizeOverflow,
    /// Padded readback-buffer arithmetic overflowed.
    ReadbackSizeOverflow,
    /// Shader-instance upload arithmetic overflowed.
    UploadSizeOverflow,
    /// Reserving frame-plan storage failed.
    FramePlanAllocationFailed {
        /// Number of requested quad slots.
        paints: usize,
    },
    /// The glyph atlas exceeds the guaranteed Metal texture extent.
    GlyphAtlasExtentUnsupported {
        /// Requested atlas width.
        width: u32,
        /// Requested atlas height.
        height: u32,
        /// Maximum admitted width or height.
        limit: u32,
    },
    /// The glyph atlas exceeds Alpine's per-backend payload ceiling.
    GlyphAtlasByteLimitExceeded {
        /// Requested tightly packed A8 bytes.
        bytes: usize,
        /// Maximum admitted bytes.
        limit: usize,
    },
    /// The CPU oracle workload exceeds its deterministic safety limit.
    OraclePixelLimitExceeded {
        /// Requested pixel count.
        pixels: usize,
        /// Maximum accepted pixel count.
        limit: usize,
    },
    /// Reserving CPU oracle image storage failed.
    OracleAllocationFailed {
        /// Requested image byte count.
        bytes: usize,
    },
}

impl fmt::Display for OffscreenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroPixelExtent => formatter.write_str("offscreen target must be non-empty"),
            Self::InvalidScale => {
                formatter.write_str("offscreen scale must be positive and finite")
            }
            Self::ViewportPixelMismatch {
                expected_width,
                expected_height,
                actual_width,
                actual_height,
            } => write!(
                formatter,
                "viewport rounds to {expected_width}x{expected_height}, target is {actual_width}x{actual_height}"
            ),
            Self::ViewportScaleOverflow => {
                formatter.write_str("scaled viewport does not fit physical pixel extents")
            }
            Self::UnrepresentableQuad { primitive_index } => write!(
                formatter,
                "primitive {primitive_index} does not fit the shader coordinate representation"
            ),
            Self::InvalidSceneOperation { primitive_index } => write!(
                formatter,
                "scene operation at painter index {primitive_index} is invalid"
            ),
            Self::MissingGlyphAtlas { primitive_index } => write!(
                formatter,
                "glyph at painter index {primitive_index} has no A8 atlas"
            ),
            Self::CompactRowSizeOverflow => {
                formatter.write_str("compact readback row size overflowed")
            }
            Self::AlignedRowSizeOverflow => {
                formatter.write_str("aligned readback row size overflowed")
            }
            Self::CompactImageSizeOverflow => {
                formatter.write_str("compact readback image size overflowed")
            }
            Self::ReadbackSizeOverflow => {
                formatter.write_str("padded readback buffer size overflowed")
            }
            Self::UploadSizeOverflow => formatter.write_str("quad upload size overflowed"),
            Self::FramePlanAllocationFailed { paints } => {
                write!(
                    formatter,
                    "cannot reserve storage for {paints} lowered paints"
                )
            }
            Self::GlyphAtlasExtentUnsupported {
                width,
                height,
                limit,
            } => write!(
                formatter,
                "glyph atlas extent {width}x{height} exceeds limit {limit}"
            ),
            Self::GlyphAtlasByteLimitExceeded { bytes, limit } => write!(
                formatter,
                "glyph atlas payload {bytes} bytes exceeds limit {limit}"
            ),
            Self::OraclePixelLimitExceeded { pixels, limit } => write!(
                formatter,
                "CPU oracle pixel count {pixels} exceeds limit {limit}"
            ),
            Self::OracleAllocationFailed { bytes } => {
                write!(formatter, "cannot reserve {bytes} CPU oracle image bytes")
            }
        }
    }
}

impl Error for OffscreenError {}

#[cfg(test)]
#[path = "frame_coverage_tests.rs"]
mod frame_coverage_tests;

#[cfg(test)]
mod tests {
    use std::{
        mem::{offset_of, size_of},
        num::NonZeroU32,
        sync::Arc,
    };

    use alpine_core::{LinearRgba, Point, Rect, Size};
    use alpine_scene::{
        AtlasBounds, Clip, Glyph, GlyphAtlasImage, Primitive, SceneBuilder, SceneRevision,
    };

    use super::{
        LoweredPaint, MAX_METAL3_TEXTURE_DIMENSION_2D, OffscreenDescriptor, OffscreenError,
        READBACK_ROW_ALIGNMENT, ReadbackLayout, ValidatedFrame,
    };

    #[test]
    fn lowered_paint_shader_abi_is_stable() {
        assert_eq!(size_of::<LoweredPaint>(), 48);
        assert_eq!(offset_of!(LoweredPaint, bounds), 0);
        assert_eq!(offset_of!(LoweredPaint, color), 16);
        assert_eq!(offset_of!(LoweredPaint, atlas_uv), 32);
    }

    fn color(red: f32, green: f32, blue: f32, alpha: f32) -> Result<LinearRgba, &'static str> {
        LinearRgba::new(red, green, blue, alpha).ok_or("valid test color")
    }

    fn size(width: f32, height: f32) -> Result<Size, &'static str> {
        Size::new(width, height).ok_or("valid test size")
    }

    fn point(x: f32, y: f32) -> Result<Point, &'static str> {
        Point::new(x, y).ok_or("valid test point")
    }

    fn descriptor(
        width: u32,
        height: u32,
        scale: f32,
    ) -> Result<OffscreenDescriptor, Box<dyn std::error::Error>> {
        let clear = color(0.0, 0.0, 0.0, 0.0)?;
        OffscreenDescriptor::new(width, height, scale, clear).map_err(Into::into)
    }

    #[test]
    fn validates_descriptor_and_viewport_rounding() -> Result<(), Box<dyn std::error::Error>> {
        assert_eq!(
            OffscreenDescriptor::new(0, 1, 1.0, color(0.0, 0.0, 0.0, 0.0)?),
            Err(OffscreenError::ZeroPixelExtent)
        );
        for invalid in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert_eq!(
                OffscreenDescriptor::new(1, 1, invalid, color(0.0, 0.0, 0.0, 0.0)?),
                Err(OffscreenError::InvalidScale)
            );
        }

        let scene = SceneBuilder::new(SceneRevision::new(1), size(1.25, 1.25)?).finish();
        let target = descriptor(3, 3, 2.0)?;
        let plan = ValidatedFrame::new(&scene, target)?;
        assert_eq!(plan.descriptor(), target);
        assert_eq!(plan.revision(), SceneRevision::new(1));

        let mismatch = ValidatedFrame::new(&scene, descriptor(2, 3, 2.0)?);
        assert_eq!(
            mismatch,
            Err(OffscreenError::ViewportPixelMismatch {
                expected_width: 3,
                expected_height: 3,
                actual_width: 2,
                actual_height: 3,
            })
        );

        let exact_max = SceneBuilder::new(SceneRevision::new(2), size(65_535.0, 1.0)?).finish();
        let exact_max_target = descriptor(u32::MAX, 65_537, 65_537.0)?;
        assert!(ValidatedFrame::new(&exact_max, exact_max_target).is_ok());

        let overflow = SceneBuilder::new(SceneRevision::new(3), size(f32::MAX, 1.0)?).finish();
        assert_eq!(
            ValidatedFrame::new(&overflow, descriptor(1, 2, 2.0)?),
            Err(OffscreenError::ViewportScaleOverflow)
        );
        Ok(())
    }

    #[test]
    fn glyph_clipping_crops_destination_and_atlas_uv_together()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut builder = SceneBuilder::new(SceneRevision::new(2), size(4.0, 2.0)?);
        let clip = builder.push_clip(Clip::new(Rect::new(point(1.0, 0.5)?, size(2.0, 1.0)?)));
        let four = NonZeroU32::new(4).ok_or("atlas width")?;
        let two = NonZeroU32::new(2).ok_or("atlas height")?;
        let atlas = GlyphAtlasImage::new(7, four, two, Arc::from([255_u8; 8]))?;
        builder.set_glyph_atlas(atlas)?;
        let bounds = Rect::new(point(0.0, 0.0)?, size(4.0, 2.0)?);
        let glyph = Glyph::new(
            bounds,
            AtlasBounds::new(0, 0, four, two),
            color(1.0, 1.0, 1.0, 1.0)?,
        )
        .clipped(clip);
        builder.push_glyph(glyph)?;

        let frame = ValidatedFrame::new(&builder.finish(), descriptor(4, 2, 1.0)?)?;
        assert_eq!(frame.paints().len(), 1);
        for (actual, expected) in frame.paints()[0]
            .bounds()
            .into_iter()
            .zip([1.0, 0.5, 3.0, 1.5])
        {
            assert!((actual - expected).abs() <= f32::EPSILON);
        }
        for (actual, expected) in frame.paints()[0]
            .atlas_uv()
            .into_iter()
            .zip([0.25, 0.25, 0.75, 0.75])
        {
            assert!((actual - expected).abs() <= f32::EPSILON);
        }
        Ok(())
    }

    #[test]
    fn glyph_atlas_contract_bounds_native_admission() {
        assert_eq!(
            super::validate_glyph_atlas_contract(4_096, 4_096, super::MAX_GLYPH_ATLAS_BYTES),
            Ok(())
        );
        assert_eq!(
            super::validate_glyph_atlas_contract(16_385, 1, 16_385),
            Err(OffscreenError::GlyphAtlasExtentUnsupported {
                width: 16_385,
                height: 1,
                limit: MAX_METAL3_TEXTURE_DIMENSION_2D,
            })
        );
        assert_eq!(
            super::validate_glyph_atlas_contract(4_097, 4_096, super::MAX_GLYPH_ATLAS_BYTES + 1),
            Err(OffscreenError::GlyphAtlasByteLimitExceeded {
                bytes: super::MAX_GLYPH_ATLAS_BYTES + 1,
                limit: super::MAX_GLYPH_ATLAS_BYTES,
            })
        );
    }

    #[test]
    fn readback_layout_companion() -> Result<(), OffscreenError> {
        let layout = ReadbackLayout::new(65, 3)?;
        assert_eq!(layout.compact_bytes_per_row(), 260);
        assert_eq!(layout.aligned_bytes_per_row(), 512);
        assert_eq!(layout.aligned_bytes_per_row() % READBACK_ROW_ALIGNMENT, 0);
        assert_eq!(layout.compact_len(), 780);
        assert_eq!(layout.buffer_len(), 1_536);
        let bounded_max = ReadbackLayout::new(u32::from(u16::MAX), u32::from(u16::MAX))?;
        assert_eq!(
            bounded_max.compact_len(),
            bounded_max.compact_bytes_per_row() * usize::from(u16::MAX)
        );
        assert_eq!(
            bounded_max.buffer_len(),
            bounded_max.aligned_bytes_per_row() * usize::from(u16::MAX)
        );
        assert_eq!(
            ReadbackLayout::new(u32::MAX, u32::MAX),
            Err(OffscreenError::CompactImageSizeOverflow)
        );
        assert_eq!(
            super::map_reservation_failure::<()>(
                Err(()),
                OffscreenError::FramePlanAllocationFailed { paints: 9 }
            ),
            Err(OffscreenError::FramePlanAllocationFailed { paints: 9 })
        );
        Ok(())
    }

    #[test]
    fn frame_plan_allocation_failure_precedes_lowering() -> Result<(), Box<dyn std::error::Error>> {
        let scene = SceneBuilder::new(SceneRevision::new(6), size(1.0, 1.0)?).finish();
        let target = descriptor(1, 1, 1.0)?;
        let result = ValidatedFrame::new_with_reservation(&scene, target, |_, paints| {
            Err(OffscreenError::FramePlanAllocationFailed { paints })
        });
        assert_eq!(
            result,
            Err(OffscreenError::FramePlanAllocationFailed { paints: 0 })
        );
        Ok(())
    }

    #[test]
    fn rejects_quads_that_collapse_in_shader_precision() -> Result<(), Box<dyn std::error::Error>> {
        let paint = color(1.0, 1.0, 1.0, 1.0)?;

        let mut horizontal = SceneBuilder::new(SceneRevision::new(8), size(16_777_218.0, 1.0)?);
        horizontal.push(Primitive::Quad {
            bounds: Rect::new(point(16_777_216.0, 0.0)?, size(1.0, 1.0)?),
            color: paint,
        });
        assert_eq!(
            ValidatedFrame::new(&horizontal.finish(), descriptor(16_777_218, 1, 1.0)?),
            Err(OffscreenError::UnrepresentableQuad { primitive_index: 0 })
        );

        let mut vertical = SceneBuilder::new(SceneRevision::new(9), size(1.0, 16_777_218.0)?);
        vertical.push(Primitive::Quad {
            bounds: Rect::new(point(0.0, 16_777_216.0)?, size(1.0, 1.0)?),
            color: paint,
        });
        assert_eq!(
            ValidatedFrame::new(&vertical.finish(), descriptor(1, 16_777_218, 1.0)?),
            Err(OffscreenError::UnrepresentableQuad { primitive_index: 0 })
        );
        Ok(())
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn lowers_clips_omits_and_preserves_painter_order() -> Result<(), Box<dyn std::error::Error>> {
        let mut builder = SceneBuilder::new(SceneRevision::new(7), size(4.0, 4.0)?);
        let red = color(1.0, 0.0, 0.0, 1.0)?;
        let blue = color(0.0, 0.0, 1.0, 0.5)?;
        builder.push(Primitive::Quad {
            bounds: Rect::new(point(-1.0, -1.0)?, size(3.0, 3.0)?),
            color: red,
        });
        builder.push(Primitive::Quad {
            bounds: Rect::new(point(1.0, 1.0)?, size(3.0, 3.0)?),
            color: blue,
        });
        builder.push(Primitive::Quad {
            bounds: Rect::new(point(8.0, 8.0)?, size(1.0, 1.0)?),
            color: red,
        });
        builder.push(Primitive::Quad {
            bounds: Rect::new(point(0.0, 0.0)?, size(0.0, 1.0)?),
            color: red,
        });

        let plan = ValidatedFrame::new(&builder.finish(), descriptor(8, 8, 2.0)?)?;
        assert_eq!(plan.consumed_primitives(), 4);
        assert_eq!(plan.omitted_primitives(), 2);
        assert_eq!(plan.paints().len(), 2);
        assert_eq!(plan.paints()[0].bounds(), [0.0, 0.0, 4.0, 4.0]);
        assert_eq!(plan.paints()[0].color(), [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(plan.paints()[1].bounds(), [2.0, 2.0, 8.0, 8.0]);
        assert_eq!(plan.paints()[1].color(), [0.0, 0.0, 1.0, 0.5]);
        assert_eq!(plan.upload_bytes(), 2 * std::mem::size_of::<LoweredPaint>());
        Ok(())
    }

    #[test]
    fn small_scene_grid_preserves_bounds_and_order() -> Result<(), Box<dyn std::error::Error>> {
        let paint = color(0.25, 0.5, 0.75, 0.5)?;
        for viewport in 1_u16..=4 {
            for origin in -1_i16..=4 {
                for extent in 0_u16..=3 {
                    let mut builder = SceneBuilder::new(
                        SceneRevision::new(11),
                        size(f32::from(viewport), f32::from(viewport))?,
                    );
                    builder.push(Primitive::Quad {
                        bounds: Rect::new(
                            point(f32::from(origin), f32::from(origin))?,
                            size(f32::from(extent), f32::from(extent))?,
                        ),
                        color: paint,
                    });
                    let plan = ValidatedFrame::new(
                        &builder.finish(),
                        descriptor(u32::from(viewport), u32::from(viewport), 1.0)?,
                    );
                    assert_eq!(
                        plan.as_ref().map(ValidatedFrame::consumed_primitives),
                        Ok(1)
                    );
                    assert_eq!(
                        plan.as_ref()
                            .map(|frame| frame.paints().len() + frame.omitted_primitives()),
                        Ok(1)
                    );
                    for quad in plan.iter().flat_map(ValidatedFrame::paints) {
                        let [left, top, right, bottom] = quad.bounds();
                        assert!(left >= 0.0 && top >= 0.0);
                        assert!(right <= f32::from(viewport));
                        assert!(bottom <= f32::from(viewport));
                        assert!(right > left && bottom > top);
                    }
                }
            }
        }
        Ok(())
    }

    #[test]
    fn errors_have_stable_safe_messages() {
        let errors = [
            OffscreenError::ZeroPixelExtent,
            OffscreenError::InvalidScale,
            OffscreenError::ViewportPixelMismatch {
                expected_width: 1,
                expected_height: 2,
                actual_width: 3,
                actual_height: 4,
            },
            OffscreenError::ViewportScaleOverflow,
            OffscreenError::UnrepresentableQuad { primitive_index: 5 },
            OffscreenError::InvalidSceneOperation { primitive_index: 6 },
            OffscreenError::MissingGlyphAtlas { primitive_index: 7 },
            OffscreenError::CompactRowSizeOverflow,
            OffscreenError::AlignedRowSizeOverflow,
            OffscreenError::CompactImageSizeOverflow,
            OffscreenError::ReadbackSizeOverflow,
            OffscreenError::UploadSizeOverflow,
            OffscreenError::FramePlanAllocationFailed { paints: 6 },
            OffscreenError::GlyphAtlasExtentUnsupported {
                width: 17_000,
                height: 1,
                limit: MAX_METAL3_TEXTURE_DIMENSION_2D,
            },
            OffscreenError::GlyphAtlasByteLimitExceeded {
                bytes: super::MAX_GLYPH_ATLAS_BYTES + 1,
                limit: super::MAX_GLYPH_ATLAS_BYTES,
            },
            OffscreenError::OraclePixelLimitExceeded {
                pixels: 7,
                limit: 6,
            },
            OffscreenError::OracleAllocationFailed { bytes: 8 },
        ];
        for error in errors {
            assert!(!error.to_string().is_empty());
        }
    }
}
