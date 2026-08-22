//! Immutable renderer input for Alpine GPUI.

use std::{error::Error, fmt, num::NonZeroU32, sync::Arc};

use alpine_core::{LinearRgba, Rect, Size};

/// Monotonically increasing identity for a scene snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SceneRevision(u64);

impl SceneRevision {
    /// Creates a revision from its persisted integer value.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }
    /// Returns the underlying integer value.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Stable index into one scene's clip array.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClipId(usize);

impl ClipId {
    /// Returns the array index.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0
    }
}

/// Stable index into one scene's quad array.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QuadId(usize);

impl QuadId {
    /// Returns the array index.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0
    }
}

/// Stable index into one scene's glyph array.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GlyphId(usize);

impl GlyphId {
    /// Returns the array index.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0
    }
}

/// One axis-aligned scene clip.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Clip {
    bounds: Rect,
}

impl Clip {
    /// Creates an axis-aligned clip.
    #[must_use]
    pub const fn new(bounds: Rect) -> Self {
        Self { bounds }
    }
    /// Returns logical clip bounds.
    #[must_use]
    pub const fn bounds(self) -> Rect {
        self.bounds
    }
}

/// One solid axis-aligned quad.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quad {
    bounds: Rect,
    color: LinearRgba,
    clip: Option<ClipId>,
}

impl Quad {
    /// Creates an unclipped quad.
    #[must_use]
    pub const fn new(bounds: Rect, color: LinearRgba) -> Self {
        Self {
            bounds,
            color,
            clip: None,
        }
    }
    /// Applies one scene clip.
    #[must_use]
    pub const fn clipped(mut self, clip: ClipId) -> Self {
        self.clip = Some(clip);
        self
    }
    /// Returns logical bounds.
    #[must_use]
    pub const fn bounds(self) -> Rect {
        self.bounds
    }
    /// Returns linear unpremultiplied color.
    #[must_use]
    pub const fn color(self) -> LinearRgba {
        self.color
    }
    /// Returns the optional scene clip.
    #[must_use]
    pub const fn clip(self) -> Option<ClipId> {
        self.clip
    }
}

/// Integer bounds inside the scene's A8 glyph atlas.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AtlasBounds {
    x: u32,
    y: u32,
    width: NonZeroU32,
    height: NonZeroU32,
}

impl AtlasBounds {
    /// Creates non-empty atlas bounds.
    #[must_use]
    pub const fn new(x: u32, y: u32, width: NonZeroU32, height: NonZeroU32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
    /// Returns the left pixel coordinate.
    #[must_use]
    pub const fn x(self) -> u32 {
        self.x
    }
    /// Returns the top pixel coordinate.
    #[must_use]
    pub const fn y(self) -> u32 {
        self.y
    }
    /// Returns the pixel width.
    #[must_use]
    pub const fn width(self) -> NonZeroU32 {
        self.width
    }
    /// Returns the pixel height.
    #[must_use]
    pub const fn height(self) -> NonZeroU32 {
        self.height
    }
}

/// One monochrome glyph atlas instance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glyph {
    bounds: Rect,
    atlas_bounds: AtlasBounds,
    color: LinearRgba,
    clip: Option<ClipId>,
}

impl Glyph {
    /// Creates an unclipped glyph instance.
    #[must_use]
    pub const fn new(bounds: Rect, atlas_bounds: AtlasBounds, color: LinearRgba) -> Self {
        Self {
            bounds,
            atlas_bounds,
            color,
            clip: None,
        }
    }
    /// Applies one scene clip.
    #[must_use]
    pub const fn clipped(mut self, clip: ClipId) -> Self {
        self.clip = Some(clip);
        self
    }
    /// Returns logical destination bounds.
    #[must_use]
    pub const fn bounds(self) -> Rect {
        self.bounds
    }
    /// Returns source atlas bounds.
    #[must_use]
    pub const fn atlas_bounds(self) -> AtlasBounds {
        self.atlas_bounds
    }
    /// Returns linear unpremultiplied color.
    #[must_use]
    pub const fn color(self) -> LinearRgba {
        self.color
    }
    /// Returns the optional scene clip.
    #[must_use]
    pub const fn clip(self) -> Option<ClipId> {
        self.clip
    }
}

/// One validated set of complete A8 atlas rows overriding the retained base.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GlyphAtlasRowPatch {
    start_row: u32,
    row_count: NonZeroU32,
    pixels: Arc<[u8]>,
}

impl GlyphAtlasRowPatch {
    /// Creates one tightly packed complete-row patch.
    #[must_use]
    pub const fn new(start_row: u32, row_count: NonZeroU32, pixels: Arc<[u8]>) -> Self {
        Self {
            start_row,
            row_count,
            pixels,
        }
    }

    /// Returns the first changed zero-based row.
    #[must_use]
    pub const fn start_row(&self) -> u32 {
        self.start_row
    }

    /// Returns the number of complete changed rows.
    #[must_use]
    pub const fn row_count(&self) -> NonZeroU32 {
        self.row_count
    }

    /// Returns tightly packed complete-row bytes.
    #[must_use]
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }
}

/// Immutable A8 atlas base plus cumulative bounded row overrides.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GlyphAtlasImage {
    base_revision: u64,
    revision: u64,
    width: NonZeroU32,
    height: NonZeroU32,
    pixels: Arc<[u8]>,
    row_patches: Arc<[GlyphAtlasRowPatch]>,
}

impl GlyphAtlasImage {
    /// Creates a tightly packed A8 atlas snapshot.
    ///
    /// # Errors
    ///
    /// Returns a structured length or arithmetic failure.
    pub fn new(
        revision: u64,
        width: NonZeroU32,
        height: NonZeroU32,
        pixels: Arc<[u8]>,
    ) -> Result<Self, SceneError> {
        let expected = usize::try_from(width.get())
            .ok()
            .and_then(|width| {
                usize::try_from(height.get())
                    .ok()
                    .and_then(|height| width.checked_mul(height))
            })
            .ok_or(SceneError::ArithmeticOverflow)?;
        if pixels.len() != expected {
            return Err(SceneError::InvalidAtlasLength {
                expected,
                actual: pixels.len(),
            });
        }
        Ok(Self {
            base_revision: revision,
            revision,
            width,
            height,
            pixels,
            row_patches: Arc::from([]),
        })
    }

    /// Creates a newer snapshot sharing this image's full base and replacing
    /// its cumulative row overrides.
    ///
    /// # Errors
    ///
    /// Returns a structured revision, range, ordering, length, or arithmetic
    /// failure. Patches must be sorted and disjoint.
    pub fn with_row_patches(
        &self,
        source_revision: u64,
        revision: u64,
        row_patches: Arc<[GlyphAtlasRowPatch]>,
    ) -> Result<Self, SceneError> {
        if source_revision != self.base_revision || revision <= source_revision {
            return Err(SceneError::InvalidAtlasRevision {
                base: self.base_revision,
                source: source_revision,
                revision,
            });
        }
        let width =
            usize::try_from(self.width.get()).map_err(|_| SceneError::ArithmeticOverflow)?;
        let mut previous_end = 0;
        for (index, patch) in row_patches.iter().enumerate() {
            let end = patch
                .start_row
                .checked_add(patch.row_count.get())
                .ok_or(SceneError::ArithmeticOverflow)?;
            if end > self.height.get() || (index != 0 && patch.start_row < previous_end) {
                return Err(SceneError::InvalidAtlasRowRange {
                    start: patch.start_row,
                    rows: patch.row_count.get(),
                    height: self.height.get(),
                });
            }
            let expected = width
                .checked_mul(
                    usize::try_from(patch.row_count.get())
                        .map_err(|_| SceneError::ArithmeticOverflow)?,
                )
                .ok_or(SceneError::ArithmeticOverflow)?;
            if patch.pixels.len() != expected {
                return Err(SceneError::InvalidAtlasLength {
                    expected,
                    actual: patch.pixels.len(),
                });
            }
            previous_end = end;
        }
        Ok(Self {
            base_revision: self.base_revision,
            revision,
            width: self.width,
            height: self.height,
            pixels: Arc::clone(&self.pixels),
            row_patches,
        })
    }

    /// Returns the retained full-image base revision.
    #[must_use]
    pub const fn base_revision(&self) -> u64 {
        self.base_revision
    }
    /// Returns the atlas content revision.
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }
    /// Returns the atlas width.
    #[must_use]
    pub const fn width(&self) -> NonZeroU32 {
        self.width
    }
    /// Returns the atlas height.
    #[must_use]
    pub const fn height(&self) -> NonZeroU32 {
        self.height
    }
    /// Returns tightly packed top-down A8 base pixels before row overrides.
    #[must_use]
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }
    /// Returns cumulative complete-row overrides after the base revision.
    #[must_use]
    pub fn row_patches(&self) -> &[GlyphAtlasRowPatch] {
        &self.row_patches
    }
    /// Returns the current A8 value after applying cumulative row overrides.
    #[must_use]
    pub fn pixel(&self, x: u32, y: u32) -> Option<u8> {
        if x >= self.width.get() || y >= self.height.get() {
            return None;
        }
        let width = usize::try_from(self.width.get()).ok()?;
        if let Some(patch) = self.row_patches.iter().find(|patch| {
            y >= patch.start_row && y < patch.start_row.saturating_add(patch.row_count.get())
        }) {
            let local_row = usize::try_from(y.checked_sub(patch.start_row)?).ok()?;
            let index = local_row
                .checked_mul(width)?
                .checked_add(usize::try_from(x).ok()?)?;
            return patch.pixels.get(index).copied();
        }
        let index = usize::try_from(y)
            .ok()?
            .checked_mul(width)?
            .checked_add(usize::try_from(x).ok()?)?;
        self.pixels.get(index).copied()
    }
    /// Returns whether both snapshots retain the same immutable pixel storage.
    #[must_use]
    pub fn shares_storage_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.pixels, &other.pixels)
    }
}

/// A primitive accepted by the compatibility builder entry point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Primitive {
    /// A solid axis-aligned rectangle.
    Quad {
        /// Bounds in logical pixels.
        bounds: Rect,
        /// Linear unpremultiplied color.
        color: LinearRgba,
    },
}

/// One typed painter-order reference into a primitive array.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PaintOperation {
    /// Paint one solid quad.
    Quad(QuadId),
    /// Paint one monochrome atlas glyph.
    Glyph(GlyphId),
}

/// An immutable snapshot consumed by a renderer.
#[derive(Clone, Debug, PartialEq)]
pub struct Scene {
    revision: SceneRevision,
    viewport: Size,
    clips: Box<[Clip]>,
    quads: Box<[Quad]>,
    glyphs: Box<[Glyph]>,
    operations: Box<[PaintOperation]>,
    glyph_atlas: Option<GlyphAtlasImage>,
}

impl Scene {
    /// Returns this snapshot's revision.
    #[must_use]
    pub const fn revision(&self) -> SceneRevision {
        self.revision
    }
    /// Returns the logical viewport size.
    #[must_use]
    pub const fn viewport(&self) -> Size {
        self.viewport
    }
    /// Returns clips in storage order.
    #[must_use]
    pub fn clips(&self) -> &[Clip] {
        &self.clips
    }
    /// Returns quads in storage order.
    #[must_use]
    pub fn quads(&self) -> &[Quad] {
        &self.quads
    }
    /// Returns glyphs in storage order.
    #[must_use]
    pub fn glyphs(&self) -> &[Glyph] {
        &self.glyphs
    }
    /// Returns typed operations in painter order.
    #[must_use]
    pub fn operations(&self) -> &[PaintOperation] {
        &self.operations
    }
    /// Returns the number of painter-order operations.
    #[must_use]
    pub const fn operation_count(&self) -> usize {
        self.operations.len()
    }
    /// Returns the optional A8 atlas snapshot.
    #[must_use]
    pub const fn glyph_atlas(&self) -> Option<&GlyphAtlasImage> {
        self.glyph_atlas.as_ref()
    }
}

/// Single-use builder for an immutable scene.
#[derive(Debug)]
pub struct SceneBuilder {
    revision: SceneRevision,
    viewport: Size,
    clips: Vec<Clip>,
    quads: Vec<Quad>,
    glyphs: Vec<Glyph>,
    operations: Vec<PaintOperation>,
    glyph_atlas: Option<GlyphAtlasImage>,
}

impl SceneBuilder {
    /// Starts a scene for the given revision and viewport.
    #[must_use]
    pub const fn new(revision: SceneRevision, viewport: Size) -> Self {
        Self {
            revision,
            viewport,
            clips: Vec::new(),
            quads: Vec::new(),
            glyphs: Vec::new(),
            operations: Vec::new(),
            glyph_atlas: None,
        }
    }
    /// Appends a compatibility primitive in painter order.
    pub fn push(&mut self, primitive: Primitive) {
        match primitive {
            Primitive::Quad { bounds, color } => {
                let id = QuadId(self.quads.len());
                self.quads.push(Quad::new(bounds, color));
                self.operations.push(PaintOperation::Quad(id));
            }
        }
    }
    /// Stores one clip and returns its stable scene-local identity.
    pub fn push_clip(&mut self, clip: Clip) -> ClipId {
        let id = ClipId(self.clips.len());
        self.clips.push(clip);
        id
    }
    /// Appends one quad in painter order.
    ///
    /// # Errors
    ///
    /// Returns a structured error when the quad references an invalid clip.
    pub fn push_quad(&mut self, quad: Quad) -> Result<QuadId, SceneError> {
        self.validate_clip(quad.clip())?;
        let id = QuadId(self.quads.len());
        self.quads.push(quad);
        self.operations.push(PaintOperation::Quad(id));
        Ok(id)
    }
    /// Installs the immutable atlas used by subsequent glyphs.
    ///
    /// # Errors
    ///
    /// Returns a structured error if existing glyphs already reference the
    /// installed image.
    pub fn set_glyph_atlas(&mut self, atlas: GlyphAtlasImage) -> Result<(), SceneError> {
        if !self.glyphs.is_empty() {
            return Err(SceneError::AtlasAlreadyReferenced);
        }
        self.glyph_atlas = Some(atlas);
        Ok(())
    }
    /// Validates and appends one glyph in painter order.
    ///
    /// # Errors
    ///
    /// Returns a structured error for a missing atlas, invalid clip, or source bounds.
    pub fn push_glyph(&mut self, glyph: Glyph) -> Result<GlyphId, SceneError> {
        self.validate_clip(glyph.clip())?;
        let atlas = self
            .glyph_atlas
            .as_ref()
            .ok_or(SceneError::MissingGlyphAtlas)?;
        validate_atlas_bounds(glyph.atlas_bounds(), atlas)?;
        let id = GlyphId(self.glyphs.len());
        self.glyphs.push(glyph);
        self.operations.push(PaintOperation::Glyph(id));
        Ok(id)
    }
    /// Freezes the scene for renderer consumption.
    #[must_use]
    pub fn finish(self) -> Scene {
        Scene {
            revision: self.revision,
            viewport: self.viewport,
            clips: self.clips.into_boxed_slice(),
            quads: self.quads.into_boxed_slice(),
            glyphs: self.glyphs.into_boxed_slice(),
            operations: self.operations.into_boxed_slice(),
            glyph_atlas: self.glyph_atlas,
        }
    }
    fn validate_clip(&self, clip: Option<ClipId>) -> Result<(), SceneError> {
        if let Some(clip) = clip
            && clip.index() >= self.clips.len()
        {
            return Err(SceneError::InvalidClip {
                index: clip.index(),
            });
        }
        Ok(())
    }
}

fn validate_atlas_bounds(bounds: AtlasBounds, atlas: &GlyphAtlasImage) -> Result<(), SceneError> {
    let right = bounds
        .x()
        .checked_add(bounds.width().get())
        .ok_or(SceneError::ArithmeticOverflow)?;
    let bottom = bounds
        .y()
        .checked_add(bounds.height().get())
        .ok_or(SceneError::ArithmeticOverflow)?;
    if right > atlas.width().get() || bottom > atlas.height().get() {
        return Err(SceneError::AtlasBoundsOutsideImage);
    }
    Ok(())
}

/// Structured scene-construction failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SceneError {
    /// Checked atlas arithmetic overflowed.
    ArithmeticOverflow,
    /// Atlas byte length did not match width times height.
    InvalidAtlasLength {
        /// Required tightly packed byte length.
        expected: usize,
        /// Supplied byte length.
        actual: usize,
    },
    /// A glyph was added before its atlas snapshot.
    MissingGlyphAtlas,
    /// An atlas replacement would invalidate existing glyph references.
    AtlasAlreadyReferenced,
    /// A scene-local clip identity was invalid.
    InvalidClip {
        /// Invalid clip-array index.
        index: usize,
    },
    /// Glyph source bounds exceeded the atlas image.
    AtlasBoundsOutsideImage,
    /// Atlas row patches do not descend from the retained full base.
    InvalidAtlasRevision {
        /// Retained full-image revision.
        base: u64,
        /// Declared row-patch source revision.
        source: u64,
        /// Declared current content revision.
        revision: u64,
    },
    /// One atlas row patch exceeded the image or overlapped a prior patch.
    InvalidAtlasRowRange {
        /// First changed row.
        start: u32,
        /// Number of changed rows.
        rows: u32,
        /// Atlas height.
        height: u32,
    },
}

impl fmt::Display for SceneError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ArithmeticOverflow => formatter.write_str("scene atlas arithmetic overflowed"),
            Self::InvalidAtlasLength { expected, actual } => write!(
                formatter,
                "scene atlas requires {expected} bytes, found {actual}"
            ),
            Self::MissingGlyphAtlas => formatter.write_str("scene glyph requires an A8 atlas"),
            Self::AtlasAlreadyReferenced => {
                formatter.write_str("scene atlas is already referenced by glyphs")
            }
            Self::InvalidClip { index } => write!(formatter, "scene clip index {index} is invalid"),
            Self::AtlasBoundsOutsideImage => {
                formatter.write_str("scene glyph bounds exceed the A8 atlas")
            }
            Self::InvalidAtlasRevision {
                base,
                source,
                revision,
            } => write!(
                formatter,
                "scene atlas row revision {source}->{revision} does not descend from base {base}"
            ),
            Self::InvalidAtlasRowRange {
                start,
                rows,
                height,
            } => write!(
                formatter,
                "scene atlas row patch {start}+{rows} exceeds or overlaps height {height}"
            ),
        }
    }
}

impl Error for SceneError {}

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

#[cfg(test)]
mod tests {
    use std::{error::Error, num::NonZeroU32, sync::Arc};

    use alpine_core::{LinearRgba, Point, Rect, Size};

    use super::{
        AtlasBounds, Clip, Glyph, GlyphAtlasImage, PaintOperation, Primitive, Quad, SceneBuilder,
        SceneError, SceneRevision,
    };

    fn point(x: f32, y: f32) -> Result<Point, &'static str> {
        Point::new(x, y).ok_or("valid point")
    }
    fn size(width: f32, height: f32) -> Result<Size, &'static str> {
        Size::new(width, height).ok_or("valid size")
    }
    fn color(red: f32, green: f32, blue: f32, alpha: f32) -> Result<LinearRgba, &'static str> {
        LinearRgba::new(red, green, blue, alpha).ok_or("valid color")
    }
    fn atlas() -> Result<GlyphAtlasImage, SceneError> {
        GlyphAtlasImage::new(
            4,
            NonZeroU32::new(4).ok_or(SceneError::ArithmeticOverflow)?,
            NonZeroU32::new(4).ok_or(SceneError::ArithmeticOverflow)?,
            Arc::from([255_u8; 16]),
        )
    }

    #[test]
    fn atlas_storage_identity_is_distinct_from_content_revision() -> Result<(), SceneError> {
        let two = NonZeroU32::new(2).ok_or(SceneError::ArithmeticOverflow)?;
        let pixels: Arc<[u8]> = Arc::from([0_u8, 64, 128, 255]);
        let first = GlyphAtlasImage::new(1, two, two, Arc::clone(&pixels))?;
        let revised = GlyphAtlasImage::new(2, two, two, pixels)?;
        let copied = GlyphAtlasImage::new(1, two, two, Arc::from([0_u8, 64, 128, 255]))?;

        assert!(first.shares_storage_with(&revised));
        assert!(!first.shares_storage_with(&copied));
        Ok(())
    }

    #[test]
    fn atlas_row_patches_share_the_base_and_override_exact_pixels() -> Result<(), SceneError> {
        let three = NonZeroU32::new(3).ok_or(SceneError::ArithmeticOverflow)?;
        let base =
            GlyphAtlasImage::new(10, three, three, Arc::from([0_u8, 1, 2, 3, 4, 5, 6, 7, 8]))?;
        let patch = super::GlyphAtlasRowPatch::new(
            1,
            NonZeroU32::new(1).ok_or(SceneError::ArithmeticOverflow)?,
            Arc::from([30_u8, 40, 50]),
        );
        let revised = base.with_row_patches(10, 11, Arc::from([patch.clone()]))?;

        assert!(base.shares_storage_with(&revised));
        assert_eq!(revised.base_revision(), 10);
        assert_eq!(revised.revision(), 11);
        assert_eq!(revised.row_patches(), &[patch]);
        assert_eq!(revised.row_patches()[0].start_row(), 1);
        assert_eq!(revised.row_patches()[0].row_count().get(), 1);
        assert_eq!(revised.row_patches()[0].pixels(), &[30, 40, 50]);
        assert_eq!(revised.pixel(0, 0), Some(0));
        assert_eq!(revised.pixel(1, 1), Some(40));
        assert_eq!(revised.pixel(2, 2), Some(8));
        assert_eq!(revised.pixel(3, 0), None);
        assert_eq!(base.pixel(1, 1), Some(4));
        Ok(())
    }

    #[test]
    fn atlas_row_patches_reject_invalid_ancestry_range_and_length() -> Result<(), SceneError> {
        let two = NonZeroU32::new(2).ok_or(SceneError::ArithmeticOverflow)?;
        let one = NonZeroU32::new(1).ok_or(SceneError::ArithmeticOverflow)?;
        let base = GlyphAtlasImage::new(4, two, two, Arc::from([0_u8; 4]))?;
        let valid = super::GlyphAtlasRowPatch::new(1, one, Arc::from([1_u8, 2]));

        let revision_error = base
            .with_row_patches(3, 5, Arc::from([valid.clone()]))
            .expect_err("invalid ancestry");
        assert_eq!(
            revision_error.to_string(),
            "scene atlas row revision 3->5 does not descend from base 4"
        );
        let range_error = base
            .with_row_patches(
                4,
                5,
                Arc::from([super::GlyphAtlasRowPatch::new(2, one, Arc::from([1_u8, 2]))]),
            )
            .expect_err("invalid row range");
        assert_eq!(
            range_error.to_string(),
            "scene atlas row patch 2+1 exceeds or overlaps height 2"
        );
        assert_eq!(
            base.with_row_patches(
                4,
                5,
                Arc::from([super::GlyphAtlasRowPatch::new(1, one, Arc::from([1_u8]))])
            ),
            Err(SceneError::InvalidAtlasLength {
                expected: 2,
                actual: 1,
            })
        );

        let first = super::GlyphAtlasRowPatch::new(0, one, Arc::from([1_u8, 2]));
        let adjacent = super::GlyphAtlasRowPatch::new(1, one, Arc::from([3_u8, 4]));
        assert_eq!(first.start_row(), 0);
        assert!(
            base.with_row_patches(4, 5, Arc::from([first.clone(), adjacent]))
                .is_ok()
        );
        assert!(matches!(
            base.with_row_patches(4, 5, Arc::from([first.clone(), first])),
            Err(SceneError::InvalidAtlasRowRange { .. })
        ));
        Ok(())
    }

    #[test]
    fn freezes_structure_of_arrays_in_painter_order() -> Result<(), Box<dyn Error>> {
        let viewport = size(100.0, 100.0)?;
        let mut builder = SceneBuilder::new(SceneRevision::new(7), viewport);
        let clip = builder.push_clip(Clip::new(Rect::new(point(0.0, 0.0)?, size(80.0, 80.0)?)));
        let first = Quad::new(
            Rect::new(point(0.0, 0.0)?, size(10.0, 10.0)?),
            color(1.0, 0.0, 0.0, 1.0)?,
        );
        builder.push(Primitive::Quad {
            bounds: first.bounds(),
            color: first.color(),
        });
        builder.set_glyph_atlas(atlas()?)?;
        let glyph = Glyph::new(
            Rect::new(point(5.0, 5.0)?, size(2.0, 3.0)?),
            AtlasBounds::new(
                1,
                0,
                NonZeroU32::new(2).ok_or(SceneError::ArithmeticOverflow)?,
                NonZeroU32::new(3).ok_or(SceneError::ArithmeticOverflow)?,
            ),
            color(0.0, 0.0, 1.0, 0.5)?,
        )
        .clipped(clip);
        builder.push_glyph(glyph)?;
        let scene = builder.finish();
        assert_eq!(scene.quads(), &[first]);
        assert_eq!(scene.glyphs(), &[glyph]);
        assert!(matches!(
            scene.operations(),
            [PaintOperation::Quad(_), PaintOperation::Glyph(_)]
        ));
        assert_eq!(scene.glyph_atlas().map(GlyphAtlasImage::revision), Some(4));
        Ok(())
    }

    #[test]
    fn rejects_malformed_atlas_and_glyph_references() -> Result<(), Box<dyn Error>> {
        let two = NonZeroU32::new(2).ok_or(SceneError::ArithmeticOverflow)?;
        assert_eq!(
            GlyphAtlasImage::new(0, two, two, Arc::from([0_u8; 3])),
            Err(SceneError::InvalidAtlasLength {
                expected: 4,
                actual: 3
            })
        );
        let glyph = Glyph::new(
            Rect::new(point(0.0, 0.0)?, size(1.0, 1.0)?),
            AtlasBounds::new(3, 3, two, two),
            color(1.0, 1.0, 1.0, 1.0)?,
        );
        let mut builder = SceneBuilder::new(SceneRevision::new(0), size(4.0, 4.0)?);
        assert_eq!(
            builder.push_glyph(glyph),
            Err(SceneError::MissingGlyphAtlas)
        );
        builder.set_glyph_atlas(atlas()?)?;
        assert_eq!(
            builder.push_glyph(glyph),
            Err(SceneError::AtlasBoundsOutsideImage)
        );
        Ok(())
    }

    #[test]
    fn freezes_an_empty_scene_without_allocated_operations() -> Result<(), &'static str> {
        let scene = SceneBuilder::new(SceneRevision::new(0), size(0.0, 0.0)?).finish();
        assert!(scene.operations().is_empty());
        assert!(scene.quads().is_empty());
        assert!(scene.glyphs().is_empty());
        Ok(())
    }
}
