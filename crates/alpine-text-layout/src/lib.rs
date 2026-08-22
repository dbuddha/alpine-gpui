//! Bounded visible text layout and monochrome glyph-atlas ownership.

use std::{
    error::Error,
    fmt,
    mem::size_of,
    num::{NonZeroU32, NonZeroUsize},
    ops::Range,
    sync::Arc,
};

use alpine_text::{BufferSnapshot, TextError, TextFingerprint};

#[cfg(kani)]
mod proofs;

#[cfg(all(target_arch = "aarch64", target_os = "macos"))]
mod native;

#[cfg(all(target_arch = "aarch64", target_os = "macos"))]
pub use native::CoreTextSystem;

/// Default combined retention ceiling for current and previous line layouts.
pub const DEFAULT_LAYOUT_BUDGET_BYTES: usize = 32 * 1024 * 1024;

/// Default hard ceiling for A8 atlas pixels and owned allocator metadata.
pub const DEFAULT_ATLAS_BUDGET_BYTES: usize = 16 * 1024 * 1024;

/// Default visible-line overscan on each side of the viewport.
pub const DEFAULT_OVERSCAN_LINES: usize = 3;

/// Maximum UTF-8 bytes admitted for one shaped line in the first editor slice.
pub const DEFAULT_MAX_LINE_BYTES: usize = 1024 * 1024;

/// Maximum shaped glyphs admitted for one line in the first editor slice.
pub const DEFAULT_MAX_GLYPHS_PER_LINE: usize = 1024 * 1024;

/// A finite positive scalar used by text layout.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PositiveFinite(f32);

impl PositiveFinite {
    /// Creates a positive finite scalar.
    #[must_use]
    pub fn new(value: f32) -> Option<Self> {
        (value.is_finite() && value > 0.0).then_some(Self(value))
    }

    /// Returns the validated scalar.
    #[must_use]
    pub const fn get(self) -> f32 {
        self.0
    }
}

/// Immutable font and shaping identity for one line.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct FontKey {
    family: u64,
    size_bits: u32,
    scale_bits: u32,
    tab_columns: NonZeroU32,
}

impl FontKey {
    /// Creates a stable font key from an application-owned family identity.
    #[must_use]
    pub fn new(
        family: u64,
        size: PositiveFinite,
        scale: PositiveFinite,
        tab_columns: NonZeroU32,
    ) -> Self {
        Self {
            family,
            size_bits: size.get().to_bits(),
            scale_bits: scale.get().to_bits(),
            tab_columns,
        }
    }

    /// Returns the application-owned family identity.
    #[must_use]
    pub const fn family(self) -> u64 {
        self.family
    }

    /// Returns the point size.
    #[must_use]
    pub fn size(self) -> f32 {
        f32::from_bits(self.size_bits)
    }

    /// Returns the backing scale.
    #[must_use]
    pub fn scale(self) -> f32 {
        f32::from_bits(self.scale_bits)
    }

    /// Returns the tab width in columns.
    #[must_use]
    pub const fn tab_columns(self) -> NonZeroU32 {
        self.tab_columns
    }
}

/// A checked visible line range and its bounded overscan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VisibleLines {
    visible: Range<usize>,
    laid_out: Range<usize>,
}

impl VisibleLines {
    /// Maps a vertical viewport to document lines using fixed line height.
    ///
    /// # Errors
    ///
    /// Returns a structured error for non-finite scroll, arithmetic overflow,
    /// or a zero line count.
    pub fn new(
        line_count: usize,
        scroll_y: f32,
        viewport_height: PositiveFinite,
        line_height: PositiveFinite,
        overscan_lines: usize,
    ) -> Result<Self, LayoutError> {
        if line_count == 0 {
            return Err(LayoutError::EmptyDocument);
        }
        if !scroll_y.is_finite() || scroll_y < 0.0 {
            return Err(LayoutError::InvalidScroll);
        }
        let first = floor_to_usize(scroll_y / line_height.get())?.min(line_count - 1);
        let visible_count = ceil_to_usize(viewport_height.get() / line_height.get())? + 1;
        let visible_end = first.saturating_add(visible_count).min(line_count);
        let laid_out_start = first.saturating_sub(overscan_lines);
        let laid_out_end = visible_end.saturating_add(overscan_lines).min(line_count);
        Ok(Self {
            visible: first..visible_end,
            laid_out: laid_out_start..laid_out_end,
        })
    }

    /// Returns lines intersecting the viewport.
    #[must_use]
    pub fn visible(&self) -> Range<usize> {
        self.visible.clone()
    }

    /// Returns the only lines admitted for layout and paint work.
    #[must_use]
    pub fn laid_out(&self) -> Range<usize> {
        self.laid_out.clone()
    }
}

fn floor_to_usize(value: f32) -> Result<usize, LayoutError> {
    const MAX_EXACT_F32_INTEGER: f32 = 16_777_216.0;
    if !value.is_finite() || !(0.0..=MAX_EXACT_F32_INTEGER).contains(&value) {
        return Err(LayoutError::ArithmeticOverflow);
    }
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the finite non-negative value is bounded to f32's exact integer domain"
    )]
    Ok(value.floor() as usize)
}

fn ceil_to_usize(value: f32) -> Result<usize, LayoutError> {
    const MAX_EXACT_F32_INTEGER: f32 = 16_777_216.0;
    if !value.is_finite() || !(0.0..=MAX_EXACT_F32_INTEGER).contains(&value) {
        return Err(LayoutError::ArithmeticOverflow);
    }
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the finite non-negative value is bounded to f32's exact integer domain"
    )]
    Ok(value.ceil() as usize)
}

fn reserve_cache_entries(
    values: &mut Vec<CacheEntry>,
    additional: usize,
) -> Result<(), LayoutError> {
    values
        .try_reserve(additional)
        .map_err(|_| LayoutError::AllocationFailed)
}

fn reserve_atlas_entries(
    values: &mut Vec<AtlasEntry>,
    additional: usize,
) -> Result<(), LayoutError> {
    values
        .try_reserve(additional)
        .map_err(|_| LayoutError::AllocationFailed)
}

fn reserve_atlas_index_slots(
    values: &mut Vec<Option<AtlasIndexSlot>>,
    additional: usize,
) -> Result<(), LayoutError> {
    values
        .try_reserve_exact(additional)
        .map_err(|_| LayoutError::AllocationFailed)
}

fn reserve_atlas_rects(values: &mut Vec<AtlasRect>, additional: usize) -> Result<(), LayoutError> {
    values
        .try_reserve(additional)
        .map_err(|_| LayoutError::AllocationFailed)
}

fn reserve_bytes_exact(values: &mut Vec<u8>, additional: usize) -> Result<(), LayoutError> {
    values
        .try_reserve_exact(additional)
        .map_err(|_| LayoutError::AllocationFailed)
}

const fn exceeds_budget(current: usize, budget: usize) -> bool {
    current > budget
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "u32 fits every supported 32-bit or 64-bit Alpine target"
)]
const fn usize_from_u32(value: u32) -> usize {
    value as usize
}

/// One backend-independent shaped glyph.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShapedGlyph {
    glyph_id: u32,
    x: f32,
    y: f32,
    advance: f32,
    source_utf16: u32,
    resolved_family: u64,
}

impl ShapedGlyph {
    /// Creates one validated shaped glyph.
    ///
    /// # Errors
    ///
    /// Returns [`LayoutError::InvalidShaperOutput`] for non-finite positions
    /// or a negative or non-finite advance.
    pub fn new(
        glyph_id: u32,
        x: f32,
        y: f32,
        advance: f32,
        source_utf16: u32,
    ) -> Result<Self, LayoutError> {
        if !x.is_finite() || !y.is_finite() || !advance.is_finite() || advance < 0.0 {
            return Err(LayoutError::InvalidShaperOutput);
        }
        Ok(Self {
            glyph_id,
            x,
            y,
            advance,
            source_utf16,
            resolved_family: 0,
        })
    }

    /// Creates one validated glyph with the native font family selected by
    /// the shaper, including a fallback family when required.
    ///
    /// # Errors
    ///
    /// Returns [`LayoutError::InvalidShaperOutput`] for invalid metrics.
    pub fn new_resolved(
        glyph_id: u32,
        x: f32,
        y: f32,
        advance: f32,
        source_utf16: u32,
        resolved_family: u64,
    ) -> Result<Self, LayoutError> {
        let mut glyph = Self::new(glyph_id, x, y, advance, source_utf16)?;
        glyph.resolved_family = resolved_family;
        Ok(glyph)
    }

    /// Returns the native font glyph identity.
    #[must_use]
    pub const fn glyph_id(self) -> u32 {
        self.glyph_id
    }

    /// Returns the logical x position.
    #[must_use]
    pub const fn x(self) -> f32 {
        self.x
    }

    /// Returns the logical y position relative to the baseline.
    #[must_use]
    pub const fn y(self) -> f32 {
        self.y
    }

    /// Returns the logical advance.
    #[must_use]
    pub const fn advance(self) -> f32 {
        self.advance
    }

    /// Returns the source UTF-16 code-unit index.
    #[must_use]
    pub const fn source_utf16(self) -> u32 {
        self.source_utf16
    }

    /// Returns the shaper-selected family identity.
    ///
    /// Zero means the backend-independent constructor did not resolve a
    /// native family. Native shapers always return a nonzero identity.
    #[must_use]
    pub const fn resolved_family(self) -> u64 {
        self.resolved_family
    }
}

/// Immutable copied output of one shaping operation.
#[derive(Clone, Debug, PartialEq)]
pub struct LineLayout {
    glyphs: Arc<[ShapedGlyph]>,
    width: f32,
    ascent: f32,
    descent: f32,
}

impl LineLayout {
    /// Creates a checked immutable line layout.
    ///
    /// # Errors
    ///
    /// Returns a structured error when the glyph ceiling is exceeded or any
    /// typographic metric is negative or non-finite.
    pub fn new(
        glyphs: Vec<ShapedGlyph>,
        width: f32,
        ascent: f32,
        descent: f32,
        max_glyphs: usize,
    ) -> Result<Self, LayoutError> {
        if glyphs.len() > max_glyphs {
            return Err(LayoutError::GlyphLimitExceeded {
                glyphs: glyphs.len(),
                limit: max_glyphs,
            });
        }
        if !width.is_finite()
            || !ascent.is_finite()
            || !descent.is_finite()
            || width < 0.0
            || ascent < 0.0
            || descent < 0.0
        {
            return Err(LayoutError::InvalidShaperOutput);
        }
        Ok(Self {
            glyphs: glyphs.into(),
            width,
            ascent,
            descent,
        })
    }

    /// Returns shaped glyphs in visual order.
    #[must_use]
    pub fn glyphs(&self) -> &[ShapedGlyph] {
        &self.glyphs
    }

    /// Returns the typographic width.
    #[must_use]
    pub const fn width(&self) -> f32 {
        self.width
    }

    /// Returns ascent above the baseline.
    #[must_use]
    pub const fn ascent(&self) -> f32 {
        self.ascent
    }

    /// Returns descent below the baseline.
    #[must_use]
    pub const fn descent(&self) -> f32 {
        self.descent
    }

    fn retained_bytes(&self) -> usize {
        self.glyphs.len().saturating_mul(size_of::<ShapedGlyph>())
    }
}

/// Alpine-owned shaping interface implemented by CoreText on macOS.
pub trait TextShaper {
    /// Shapes one line after a cache miss.
    ///
    /// # Errors
    ///
    /// Returns a structured unsupported, native, allocation, or output error.
    fn shape(&mut self, text: &str, font: FontKey) -> Result<LineLayout, LayoutError>;
}

/// Copied monochrome raster output and its logical baseline bearings.
#[derive(Clone, Debug, PartialEq)]
pub struct RasterizedGlyph {
    bitmap: Option<GlyphBitmap>,
    left: f32,
    top: f32,
}

impl RasterizedGlyph {
    /// Creates copied raster output with finite logical bearings.
    ///
    /// # Errors
    ///
    /// Returns [`LayoutError::InvalidShaperOutput`] for invalid bearings.
    pub fn new(bitmap: Option<GlyphBitmap>, left: f32, top: f32) -> Result<Self, LayoutError> {
        if !left.is_finite() || !top.is_finite() {
            return Err(LayoutError::InvalidShaperOutput);
        }
        Ok(Self { bitmap, left, top })
    }

    /// Returns the tightly packed A8 bitmap, or `None` for an empty glyph.
    #[must_use]
    pub const fn bitmap(&self) -> Option<&GlyphBitmap> {
        self.bitmap.as_ref()
    }

    /// Returns the logical left bearing from the glyph origin.
    #[must_use]
    pub const fn left(&self) -> f32 {
        self.left
    }

    /// Returns the logical top bearing above the baseline.
    #[must_use]
    pub const fn top(&self) -> f32 {
        self.top
    }
}

/// Alpine-owned monochrome glyph-rasterization interface.
pub trait GlyphRasterizer {
    /// Rasterizes one native glyph at a quarter-device-pixel x phase.
    ///
    /// # Errors
    ///
    /// Returns a structured unsupported, native, allocation, or output error.
    fn rasterize(
        &mut self,
        font: FontKey,
        glyph_id: u32,
        subpixel_x: u8,
    ) -> Result<RasterizedGlyph, LayoutError>;
}

#[derive(Clone)]
struct CacheEntry {
    fingerprint: TextFingerprint,
    snapshot: BufferSnapshot,
    range: Range<usize>,
    font: FontKey,
    wrap_width_bits: u32,
    layout: Arc<LineLayout>,
    retained_bytes: usize,
}

impl CacheEntry {
    fn matches(
        &self,
        fingerprint: TextFingerprint,
        snapshot: &BufferSnapshot,
        range: Range<usize>,
        font: FontKey,
        wrap_width_bits: u32,
    ) -> Result<bool, TextError> {
        if self.fingerprint != fingerprint
            || self.font != font
            || self.wrap_width_bits != wrap_width_bits
        {
            return Ok(false);
        }
        self.snapshot.range_eq(self.range.clone(), snapshot, range)
    }
}

/// Exact current and peak evidence for two-frame line-layout retention.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LayoutCacheSnapshot {
    current_bytes: usize,
    peak_bytes: usize,
    budget_bytes: usize,
    current_entries: usize,
    previous_entries: usize,
    hits: u64,
    misses: u64,
    evictions: u64,
    shaped_lines: u64,
}

impl LayoutCacheSnapshot {
    /// Returns retained layout bytes in both generations.
    #[must_use]
    pub const fn current_bytes(self) -> usize {
        self.current_bytes
    }

    /// Returns the observed retained-byte peak.
    #[must_use]
    pub const fn peak_bytes(self) -> usize {
        self.peak_bytes
    }

    /// Returns the hard retention ceiling.
    #[must_use]
    pub const fn budget_bytes(self) -> usize {
        self.budget_bytes
    }

    /// Returns entries used by the active frame.
    #[must_use]
    pub const fn current_entries(self) -> usize {
        self.current_entries
    }

    /// Returns reusable entries from the preceding frame.
    #[must_use]
    pub const fn previous_entries(self) -> usize {
        self.previous_entries
    }

    /// Returns collision-confirmed cache hits.
    #[must_use]
    pub const fn hits(self) -> u64 {
        self.hits
    }

    /// Returns cache misses.
    #[must_use]
    pub const fn misses(self) -> u64 {
        self.misses
    }

    /// Returns deterministic budget evictions.
    #[must_use]
    pub const fn evictions(self) -> u64 {
        self.evictions
    }

    /// Returns lines passed to the shaping boundary.
    #[must_use]
    pub const fn shaped_lines(self) -> u64 {
        self.shaped_lines
    }
}

/// Current-frame and previous-frame line-layout cache with a hard byte ceiling.
pub struct LineLayoutCache {
    current: Vec<CacheEntry>,
    previous: Vec<CacheEntry>,
    budget_bytes: NonZeroUsize,
    peak_bytes: usize,
    hits: u64,
    misses: u64,
    evictions: u64,
    shaped_lines: u64,
    max_line_bytes: usize,
    max_glyphs_per_line: usize,
}

impl LineLayoutCache {
    /// Creates an empty bounded cache.
    #[must_use]
    pub fn new(budget_bytes: NonZeroUsize) -> Self {
        Self {
            current: Vec::new(),
            previous: Vec::new(),
            budget_bytes,
            peak_bytes: 0,
            hits: 0,
            misses: 0,
            evictions: 0,
            shaped_lines: 0,
            max_line_bytes: DEFAULT_MAX_LINE_BYTES,
            max_glyphs_per_line: DEFAULT_MAX_GLYPHS_PER_LINE,
        }
    }

    /// Starts one frame and retains only layouts used by the preceding frame.
    ///
    /// # Errors
    ///
    /// Returns [`LayoutError::SequenceExhausted`] if eviction accounting can
    /// no longer advance.
    pub fn begin_frame(&mut self) -> Result<(), LayoutError> {
        self.evictions = self
            .evictions
            .checked_add(self.previous.len() as u64)
            .ok_or(LayoutError::SequenceExhausted)?;
        self.previous = std::mem::take(&mut self.current);
        Ok(())
    }

    /// Returns or shapes one line without materializing text on a true hit.
    ///
    /// # Errors
    ///
    /// Returns structured text, limit, shaping, accounting, or allocation
    /// failure. Rejected work does not enter either generation.
    pub fn layout_line(
        &mut self,
        snapshot: &BufferSnapshot,
        line: usize,
        font: FontKey,
        wrap_width: PositiveFinite,
        shaper: &mut dyn TextShaper,
    ) -> Result<Arc<LineLayout>, LayoutError> {
        let range = snapshot.line_byte_range(line)?;
        let bytes = range.end - range.start;
        if bytes > self.max_line_bytes {
            return Err(LayoutError::LineByteLimitExceeded {
                line,
                bytes,
                limit: self.max_line_bytes,
            });
        }
        let fingerprint = snapshot.fingerprint(range.clone())?;
        let wrap_width_bits = wrap_width.get().to_bits();

        if let Some(index) = find_match(
            &self.current,
            fingerprint,
            snapshot,
            range.clone(),
            font,
            wrap_width_bits,
        )? {
            self.hits = self
                .hits
                .checked_add(1)
                .ok_or(LayoutError::SequenceExhausted)?;
            return Ok(Arc::clone(&self.current[index].layout));
        }
        if let Some(index) = find_match(
            &self.previous,
            fingerprint,
            snapshot,
            range.clone(),
            font,
            wrap_width_bits,
        )? {
            let entry = self.previous.remove(index);
            let layout = Arc::clone(&entry.layout);
            self.current.push(entry);
            self.hits = self
                .hits
                .checked_add(1)
                .ok_or(LayoutError::SequenceExhausted)?;
            return Ok(layout);
        }

        self.misses = self
            .misses
            .checked_add(1)
            .ok_or(LayoutError::SequenceExhausted)?;
        let mut text = snapshot.slice(range.clone())?;
        if text.ends_with('\n') {
            text.pop();
            if text.ends_with('\r') {
                text.pop();
            }
        } else if text.ends_with('\r') {
            text.pop();
        }
        let layout = Arc::new(shaper.shape(&text, font)?);
        if layout.glyphs().len() > self.max_glyphs_per_line {
            return Err(LayoutError::GlyphLimitExceeded {
                glyphs: layout.glyphs().len(),
                limit: self.max_glyphs_per_line,
            });
        }
        let retained_bytes = layout.retained_bytes();
        self.shaped_lines = self
            .shaped_lines
            .checked_add(1)
            .ok_or(LayoutError::SequenceExhausted)?;
        reserve_cache_entries(&mut self.current, 1)?;
        self.current.push(CacheEntry {
            fingerprint,
            snapshot: snapshot.clone(),
            range,
            font,
            wrap_width_bits,
            layout: Arc::clone(&layout),
            retained_bytes,
        });
        self.enforce_budget()?;
        Ok(layout)
    }

    /// Returns handle-free cache accounting.
    #[must_use]
    pub fn snapshot(&self) -> LayoutCacheSnapshot {
        LayoutCacheSnapshot {
            current_bytes: self.retained_bytes(),
            peak_bytes: self.peak_bytes,
            budget_bytes: self.budget_bytes.get(),
            current_entries: self.current.len(),
            previous_entries: self.previous.len(),
            hits: self.hits,
            misses: self.misses,
            evictions: self.evictions,
            shaped_lines: self.shaped_lines,
        }
    }

    fn retained_bytes(&self) -> usize {
        let layouts = self
            .current
            .iter()
            .chain(&self.previous)
            .map(|entry| entry.retained_bytes)
            .sum::<usize>();
        layouts
            .saturating_add(
                self.current
                    .capacity()
                    .saturating_mul(size_of::<CacheEntry>()),
            )
            .saturating_add(
                self.previous
                    .capacity()
                    .saturating_mul(size_of::<CacheEntry>()),
            )
    }

    fn enforce_budget(&mut self) -> Result<(), LayoutError> {
        while self.retained_bytes() > self.budget_bytes.get() {
            if !self.previous.is_empty() {
                self.previous.remove(0);
            } else if self.current.len() > 1 {
                self.current.remove(0);
            } else {
                let bytes = self.retained_bytes();
                self.current.pop();
                self.current.shrink_to_fit();
                self.previous.shrink_to_fit();
                return Err(LayoutError::LayoutExceedsBudget {
                    bytes,
                    budget: self.budget_bytes.get(),
                });
            }
            self.evictions = self
                .evictions
                .checked_add(1)
                .ok_or(LayoutError::SequenceExhausted)?;
            self.current.shrink_to_fit();
            self.previous.shrink_to_fit();
        }
        self.peak_bytes = self.peak_bytes.max(self.retained_bytes());
        Ok(())
    }
}

fn find_match(
    entries: &[CacheEntry],
    fingerprint: TextFingerprint,
    snapshot: &BufferSnapshot,
    range: Range<usize>,
    font: FontKey,
    wrap_width_bits: u32,
) -> Result<Option<usize>, TextError> {
    for (index, entry) in entries.iter().enumerate() {
        if entry.matches(fingerprint, snapshot, range.clone(), font, wrap_width_bits)? {
            return Ok(Some(index));
        }
    }
    Ok(None)
}

/// Stable raster-cache identity for one monochrome glyph.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct GlyphKey {
    font: FontKey,
    glyph_id: u32,
    subpixel_x: u8,
}

impl GlyphKey {
    /// Creates a key. Subpixel identity is quantized by the caller.
    #[must_use]
    pub const fn new(font: FontKey, glyph_id: u32, subpixel_x: u8) -> Self {
        Self {
            font,
            glyph_id,
            subpixel_x,
        }
    }
}

/// A non-empty integer rectangle in an A8 atlas.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AtlasRect {
    x: u32,
    y: u32,
    width: NonZeroU32,
    height: NonZeroU32,
}

/// Cached atlas placement and native raster bearings for one glyph key.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AtlasGlyph {
    rect: Option<AtlasRect>,
    left: f32,
    top: f32,
}

impl AtlasGlyph {
    const fn new(rect: Option<AtlasRect>, left: f32, top: f32) -> Self {
        Self { rect, left, top }
    }

    /// Returns the atlas rectangle, or `None` for a cached empty glyph.
    #[must_use]
    pub const fn rect(self) -> Option<AtlasRect> {
        self.rect
    }

    /// Returns the logical left bearing copied from the native rasterizer.
    #[must_use]
    pub const fn left(self) -> f32 {
        self.left
    }

    /// Returns the logical top bearing copied from the native rasterizer.
    #[must_use]
    pub const fn top(self) -> f32 {
        self.top
    }
}

impl AtlasRect {
    fn new(x: u32, y: u32, width: NonZeroU32, height: NonZeroU32) -> Self {
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

    fn right(self) -> u32 {
        self.x.saturating_add(self.width.get())
    }

    fn bottom(self) -> u32 {
        self.y.saturating_add(self.height.get())
    }
}

/// Copied A8 glyph pixels and metrics returned by the rasterizer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GlyphBitmap {
    width: NonZeroU32,
    height: NonZeroU32,
    pixels: Box<[u8]>,
}

impl GlyphBitmap {
    /// Creates a tightly packed A8 bitmap.
    ///
    /// # Errors
    ///
    /// Returns a structured arithmetic error or reports a pixel length that
    /// does not exactly equal width times height.
    pub fn new(
        width: NonZeroU32,
        height: NonZeroU32,
        pixels: Vec<u8>,
    ) -> Result<Self, LayoutError> {
        let expected = usize_from_u32(width.get())
            .checked_mul(usize_from_u32(height.get()))
            .ok_or(LayoutError::ArithmeticOverflow)?;
        if pixels.len() != expected {
            return Err(LayoutError::InvalidGlyphBitmap {
                expected,
                actual: pixels.len(),
            });
        }
        Ok(Self {
            width,
            height,
            pixels: pixels.into_boxed_slice(),
        })
    }
}

#[derive(Clone, Copy)]
struct AtlasEntry {
    key: GlyphKey,
    glyph: AtlasGlyph,
    last_used: u64,
}

#[derive(Clone, Copy)]
struct AtlasIndexSlot {
    key: GlyphKey,
    entry_index: usize,
}

const MIN_ATLAS_INDEX_SLOTS: usize = 8;
/// Maximum disjoint dirty-row ranges retained between atlas publications.
pub const MAX_ATLAS_DIRTY_ROW_RANGES: usize = 64;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct DirtyRowRange {
    start: u32,
    end: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DirtyAtlasRows<const CAPACITY: usize> {
    source_revision: u64,
    ranges: [DirtyRowRange; CAPACITY],
    len: usize,
    full: bool,
}

impl<const CAPACITY: usize> DirtyAtlasRows<CAPACITY> {
    const fn new() -> Self {
        Self {
            source_revision: 0,
            ranges: [DirtyRowRange { start: 0, end: 0 }; CAPACITY],
            len: 0,
            full: false,
        }
    }

    const fn is_empty(self) -> bool {
        self.len == 0 && !self.full
    }

    fn clear(&mut self, revision: u64) {
        self.source_revision = revision;
        self.len = 0;
        self.full = false;
    }

    fn mark_full(&mut self, source_revision: u64) {
        if self.is_empty() {
            self.source_revision = source_revision;
        }
        self.len = 0;
        self.full = true;
    }

    fn insert(&mut self, source_revision: u64, start: u32, end: u32) {
        if start >= end || self.full {
            return;
        }
        if self.is_empty() {
            self.source_revision = source_revision;
        }
        if CAPACITY == 0 {
            self.mark_full(source_revision);
            return;
        }

        let mut merged = DirtyRowRange { start, end };
        let mut index = self.coalesce_overlaps(&mut merged);
        if self.len == CAPACITY {
            if self.len == 1 {
                merged.start = merged.start.min(self.ranges[0].start);
                merged.end = merged.end.max(self.ranges[0].end);
                self.ranges[0] = DirtyRowRange::default();
                self.len = 0;
            } else {
                let merge_at = self.ranges[..self.len]
                    .windows(2)
                    .enumerate()
                    .min_by_key(|(_, pair)| pair[1].start.saturating_sub(pair[0].end))
                    .map_or(0, |(pair, _)| pair);
                self.ranges[merge_at].end = self.ranges[merge_at + 1].end;
                self.remove(merge_at + 1);
            }
            index = self.coalesce_overlaps(&mut merged);
        }

        self.ranges.copy_within(index..self.len, index + 1);
        self.ranges[index] = merged;
        self.len += 1;
    }

    fn coalesce_overlaps(&mut self, merged: &mut DirtyRowRange) -> usize {
        let original_len = self.len;
        let mut insertion = original_len;
        let mut remove_start = original_len;
        let mut remove_end = original_len;

        for (index, current) in self.ranges[..original_len].iter().copied().enumerate() {
            if current.end < merged.start {
                insertion = index + 1;
                continue;
            }
            if merged.end < current.start {
                insertion = index;
                break;
            }
            if remove_start == original_len {
                remove_start = index;
                insertion = index;
            }
            remove_end = index + 1;
            merged.start = merged.start.min(current.start);
            merged.end = merged.end.max(current.end);
        }

        self.ranges
            .copy_within(remove_end..original_len, remove_start);
        self.len -= remove_end - remove_start;
        self.ranges[self.len..original_len].fill(DirtyRowRange::default());
        insertion
    }

    fn remove(&mut self, index: usize) {
        self.ranges.copy_within(index + 1..self.len, index);
        self.len -= 1;
        self.ranges[self.len] = DirtyRowRange::default();
    }
}

/// One tightly packed set of complete changed A8 atlas rows.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GlyphAtlasRowUpdate {
    start_row: u32,
    row_count: NonZeroU32,
    pixels: Box<[u8]>,
}

impl GlyphAtlasRowUpdate {
    /// Returns the first changed zero-based row.
    #[must_use]
    pub const fn start_row(&self) -> u32 {
        self.start_row
    }

    /// Returns the number of complete rows in this payload.
    #[must_use]
    pub const fn row_count(&self) -> NonZeroU32 {
        self.row_count
    }

    /// Returns tightly packed complete row bytes.
    #[must_use]
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }
}

/// One revision-bound CPU atlas publication plan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GlyphAtlasPublication {
    /// The consumer already owns the current atlas pixels.
    Unchanged {
        /// Current pixel revision.
        revision: u64,
    },
    /// The consumer must replace all atlas pixels.
    Full {
        /// Current square atlas dimension.
        dimension: NonZeroU32,
        /// Current pixel revision.
        revision: u64,
        /// Tightly packed complete A8 pixels.
        pixels: Box<[u8]>,
    },
    /// The consumer may advance a matching source revision using changed rows.
    Rows {
        /// Current square atlas dimension.
        dimension: NonZeroU32,
        /// Revision the consumer must already own.
        source_revision: u64,
        /// Revision produced after applying every row update.
        revision: u64,
        /// Sorted, disjoint changed-row payloads.
        rows: Box<[GlyphAtlasRowUpdate]>,
        /// Exact sum of row payload bytes.
        byte_count: usize,
    },
}

/// Exact current and peak evidence for the A8 atlas.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GlyphAtlasSnapshot {
    dimension: u32,
    pixel_revision: u64,
    pixel_bytes: usize,
    metadata_bytes: usize,
    peak_bytes: usize,
    budget_bytes: usize,
    entries: usize,
    hits: u64,
    misses: u64,
    evictions: u64,
    pressure_events: u64,
}

impl GlyphAtlasSnapshot {
    /// Returns the square atlas dimension, or zero while empty.
    #[must_use]
    pub const fn dimension(self) -> u32 {
        self.dimension
    }

    /// Returns the monotonic identity of the current A8 pixel contents.
    #[must_use]
    pub const fn pixel_revision(self) -> u64 {
        self.pixel_revision
    }

    /// Returns owned A8 pixel bytes.
    #[must_use]
    pub const fn pixel_bytes(self) -> usize {
        self.pixel_bytes
    }

    /// Returns exact owned allocator metadata capacity bytes.
    #[must_use]
    pub const fn metadata_bytes(self) -> usize {
        self.metadata_bytes
    }

    /// Returns the observed total-byte peak.
    #[must_use]
    pub const fn peak_bytes(self) -> usize {
        self.peak_bytes
    }

    /// Returns the hard total-byte ceiling.
    #[must_use]
    pub const fn budget_bytes(self) -> usize {
        self.budget_bytes
    }

    /// Returns resident glyph entries.
    #[must_use]
    pub const fn entries(self) -> usize {
        self.entries
    }

    /// Returns cache hits.
    #[must_use]
    pub const fn hits(self) -> u64 {
        self.hits
    }

    /// Returns cache misses.
    #[must_use]
    pub const fn misses(self) -> u64 {
        self.misses
    }

    /// Returns removable-entry evictions.
    #[must_use]
    pub const fn evictions(self) -> u64 {
        self.evictions
    }

    /// Returns explicit pressure requests.
    #[must_use]
    pub const fn pressure_events(self) -> u64 {
        self.pressure_events
    }
}

/// Demand-allocated, removable, hard-budgeted A8 glyph atlas.
pub struct GlyphAtlas {
    dimension: u32,
    pixel_revision: u64,
    pixels: Vec<u8>,
    entries: Vec<AtlasEntry>,
    index_slots: Vec<Option<AtlasIndexSlot>>,
    free: Vec<AtlasRect>,
    budget_bytes: NonZeroUsize,
    tick: u64,
    peak_bytes: usize,
    hits: u64,
    misses: u64,
    evictions: u64,
    pressure_events: u64,
    dirty_rows: DirtyAtlasRows<MAX_ATLAS_DIRTY_ROW_RANGES>,
}

impl GlyphAtlas {
    /// Creates an empty atlas that allocates no pixel storage until first use.
    #[must_use]
    pub fn new(budget_bytes: NonZeroUsize) -> Self {
        Self {
            dimension: 0,
            pixel_revision: 0,
            pixels: Vec::new(),
            entries: Vec::new(),
            index_slots: Vec::new(),
            free: Vec::new(),
            budget_bytes,
            tick: 0,
            peak_bytes: 0,
            hits: 0,
            misses: 0,
            evictions: 0,
            pressure_events: 0,
            dirty_rows: DirtyAtlasRows::new(),
        }
    }

    /// Builds a revision-bound atlas publication without discarding dirty state.
    ///
    /// A compatible consumer receives only complete changed rows. Initialization,
    /// growth, an unknown source revision, or an explicit full-dirty marker returns
    /// one full replacement. An unchanged consumer performs no allocation or pixel
    /// traversal.
    ///
    /// # Errors
    ///
    /// Returns a structured conversion, arithmetic, or bounded-allocation error.
    pub fn publication_since(
        &self,
        source_revision: u64,
    ) -> Result<GlyphAtlasPublication, LayoutError> {
        if source_revision == self.pixel_revision {
            return Ok(GlyphAtlasPublication::Unchanged {
                revision: self.pixel_revision,
            });
        }
        let dimension = NonZeroU32::new(self.dimension).ok_or(LayoutError::ArithmeticOverflow)?;
        if self.dirty_rows.full || self.dirty_rows.source_revision != source_revision {
            let mut pixels = Vec::new();
            reserve_bytes_exact(&mut pixels, self.pixels.len())?;
            pixels.extend_from_slice(&self.pixels);
            return Ok(GlyphAtlasPublication::Full {
                dimension,
                revision: self.pixel_revision,
                pixels: pixels.into_boxed_slice(),
            });
        }

        let mut rows = Vec::new();
        rows.try_reserve_exact(self.dirty_rows.len)
            .map_err(|_| LayoutError::AllocationFailed)?;
        let stride = usize_from_u32(self.dimension);
        let mut byte_count = 0usize;
        for range in &self.dirty_rows.ranges[..self.dirty_rows.len] {
            let start = usize_from_u32(range.start)
                .checked_mul(stride)
                .ok_or(LayoutError::ArithmeticOverflow)?;
            let end = usize_from_u32(range.end)
                .checked_mul(stride)
                .ok_or(LayoutError::ArithmeticOverflow)?;
            let source = self
                .pixels
                .get(start..end)
                .ok_or(LayoutError::ArithmeticOverflow)?;
            let mut pixels = Vec::new();
            reserve_bytes_exact(&mut pixels, source.len())?;
            pixels.extend_from_slice(source);
            byte_count = byte_count
                .checked_add(pixels.len())
                .ok_or(LayoutError::ArithmeticOverflow)?;
            rows.push(GlyphAtlasRowUpdate {
                start_row: range.start,
                row_count: NonZeroU32::new(range.end - range.start)
                    .ok_or(LayoutError::ArithmeticOverflow)?,
                pixels: pixels.into_boxed_slice(),
            });
        }
        Ok(GlyphAtlasPublication::Rows {
            dimension,
            source_revision,
            revision: self.pixel_revision,
            rows: rows.into_boxed_slice(),
            byte_count,
        })
    }

    /// Discards dirty-row evidence only when the acknowledged revision is current.
    #[must_use]
    pub fn acknowledge_publication(&mut self, revision: u64) -> bool {
        if revision != self.pixel_revision {
            return false;
        }
        self.dirty_rows.clear(revision);
        true
    }

    /// Looks up one retained glyph before native rasterization.
    ///
    /// A hit refreshes deterministic least-recently-used ownership and hit
    /// evidence. An absent key does not record a miss until a raster result is
    /// admitted, so rejected or failed native work cannot inflate the cache.
    ///
    /// # Errors
    ///
    /// Returns a structured sequence error if use or hit evidence is exhausted.
    pub fn lookup(&mut self, key: GlyphKey) -> Result<Option<AtlasGlyph>, LayoutError> {
        let Some(index) = self.index_lookup(key) else {
            return Ok(None);
        };
        self.record_hit(index).map(Some)
    }

    /// Retains one confirmed native raster result, including empty glyphs.
    ///
    /// Empty outcomes are metadata-only negative cache entries. They prevent
    /// whitespace and other non-painting glyphs from re-entering CoreText on a
    /// warm frame without consuming atlas pixels.
    ///
    /// # Errors
    ///
    /// Returns a structured arithmetic, budget, allocation, bitmap, or
    /// sequence error.
    pub fn insert_rasterized(
        &mut self,
        key: GlyphKey,
        rasterized: &RasterizedGlyph,
    ) -> Result<AtlasGlyph, LayoutError> {
        if let Some(index) = self.index_lookup(key) {
            return self.record_hit(index);
        }
        self.record_miss()?;
        let left = rasterized.left();
        let top = rasterized.top();
        let Some(bitmap) = rasterized.bitmap() else {
            return self.insert_empty_miss(key, left, top);
        };
        let required = bitmap.pixels.len();
        if required > self.budget_bytes.get() {
            return Err(LayoutError::GlyphExceedsAtlasBudget {
                bytes: required,
                budget: self.budget_bytes.get(),
            });
        }
        let attempts = self
            .entries
            .len()
            .checked_add(u32::BITS as usize)
            .ok_or(LayoutError::ArithmeticOverflow)?;
        self.insert_miss(key, bitmap, left, top, attempts)
    }

    /// Finds or inserts one A8 glyph, evicting least-recently-used entries
    /// deterministically when required.
    ///
    /// # Errors
    ///
    /// Returns a structured arithmetic, budget, allocation, or bitmap error.
    pub fn insert(
        &mut self,
        key: GlyphKey,
        bitmap: &GlyphBitmap,
    ) -> Result<AtlasRect, LayoutError> {
        if let Some(index) = self.index_lookup(key) {
            let glyph = self.record_hit(index)?;
            if let Some(rect) = glyph.rect() {
                return Ok(rect);
            }
            self.remove_indexed_entry(index)?;
        }
        self.record_miss()?;
        let required = bitmap.pixels.len();
        if required > self.budget_bytes.get() {
            return Err(LayoutError::GlyphExceedsAtlasBudget {
                bytes: required,
                budget: self.budget_bytes.get(),
            });
        }

        let attempts = self
            .entries
            .len()
            .checked_add(u32::BITS as usize)
            .ok_or(LayoutError::ArithmeticOverflow)?;
        self.insert_miss(key, bitmap, 0.0, 0.0, attempts)?
            .rect()
            .ok_or(LayoutError::InvalidShaperOutput)
    }

    fn record_hit(&mut self, index: usize) -> Result<AtlasGlyph, LayoutError> {
        let tick = self
            .tick
            .checked_add(1)
            .ok_or(LayoutError::SequenceExhausted)?;
        let hits = self
            .hits
            .checked_add(1)
            .ok_or(LayoutError::SequenceExhausted)?;
        let entry = self
            .entries
            .get_mut(index)
            .ok_or(LayoutError::ArithmeticOverflow)?;
        entry.last_used = tick;
        self.tick = tick;
        self.hits = hits;
        Ok(entry.glyph)
    }

    fn record_miss(&mut self) -> Result<(), LayoutError> {
        let tick = self
            .tick
            .checked_add(1)
            .ok_or(LayoutError::SequenceExhausted)?;
        let misses = self
            .misses
            .checked_add(1)
            .ok_or(LayoutError::SequenceExhausted)?;
        self.tick = tick;
        self.misses = misses;
        Ok(())
    }

    fn index_lookup(&self, key: GlyphKey) -> Option<usize> {
        atlas_index_slot(&self.index_slots, key)
            .and_then(|slot| self.index_slots[slot].map(|entry| entry.entry_index))
    }

    fn prepare_index_for_insert(&mut self) -> Result<(), LayoutError> {
        let next_entries = self
            .entries
            .len()
            .checked_add(1)
            .ok_or(LayoutError::ArithmeticOverflow)?;
        let required_slots = atlas_index_slot_count(next_entries)?;
        if self.index_slots.len() >= required_slots {
            return Ok(());
        }

        let mut candidate = Vec::new();
        reserve_atlas_index_slots(&mut candidate, required_slots)?;
        candidate.resize(required_slots, None);
        for (entry_index, entry) in self.entries.iter().enumerate() {
            let slot = atlas_index_insertion_slot(&candidate, entry.key)
                .ok_or(LayoutError::ArithmeticOverflow)?;
            candidate[slot] = Some(AtlasIndexSlot {
                key: entry.key,
                entry_index,
            });
        }
        let retained = self
            .pixels
            .capacity()
            .checked_add(self.metadata_without_index_bytes())
            .and_then(|bytes| {
                candidate
                    .capacity()
                    .checked_mul(size_of::<Option<AtlasIndexSlot>>())
                    .and_then(|index_bytes| bytes.checked_add(index_bytes))
            })
            .ok_or(LayoutError::ArithmeticOverflow)?;
        if exceeds_budget(retained, self.budget_bytes.get()) {
            return Err(LayoutError::AtlasSaturated);
        }
        self.index_slots = candidate;
        self.update_peak()
    }

    fn remove_indexed_entry(&mut self, index: usize) -> Result<AtlasEntry, LayoutError> {
        let last_index = self
            .entries
            .len()
            .checked_sub(1)
            .ok_or(LayoutError::ArithmeticOverflow)?;
        let removed_key = self
            .entries
            .get(index)
            .ok_or(LayoutError::ArithmeticOverflow)?
            .key;
        let removed_slot = atlas_index_slot(&self.index_slots, removed_key)
            .ok_or(LayoutError::ArithmeticOverflow)?;

        if index != last_index {
            let moved_key = self.entries[last_index].key;
            let moved_slot = atlas_index_slot(&self.index_slots, moved_key)
                .ok_or(LayoutError::ArithmeticOverflow)?;
            let moved = self.index_slots[moved_slot]
                .as_mut()
                .ok_or(LayoutError::ArithmeticOverflow)?;
            moved.entry_index = index;
        }
        remove_atlas_index_slot(&mut self.index_slots, removed_slot)?;
        Ok(self.entries.swap_remove(index))
    }

    fn insert_empty_miss(
        &mut self,
        key: GlyphKey,
        left: f32,
        top: f32,
    ) -> Result<AtlasGlyph, LayoutError> {
        reserve_atlas_entries(&mut self.entries, 1)?;
        self.prepare_index_for_insert()?;
        if exceeds_budget(self.current_bytes(), self.budget_bytes.get()) {
            return Err(LayoutError::AtlasSaturated);
        }
        let entry_index = self.entries.len();
        let index_slot = atlas_index_insertion_slot(&self.index_slots, key)
            .ok_or(LayoutError::ArithmeticOverflow)?;
        let glyph = AtlasGlyph::new(None, left, top);
        self.entries.push(AtlasEntry {
            key,
            glyph,
            last_used: self.tick,
        });
        self.index_slots[index_slot] = Some(AtlasIndexSlot { key, entry_index });
        self.update_peak()?;
        Ok(glyph)
    }

    fn insert_miss(
        &mut self,
        key: GlyphKey,
        bitmap: &GlyphBitmap,
        left: f32,
        top: f32,
        attempts: usize,
    ) -> Result<AtlasGlyph, LayoutError> {
        for _ in 0..attempts {
            reserve_atlas_entries(&mut self.entries, 1)?;
            reserve_atlas_rects(&mut self.free, 2)?;
            self.prepare_index_for_insert()?;
            if exceeds_budget(self.current_bytes(), self.budget_bytes.get()) {
                return Err(LayoutError::AtlasSaturated);
            }
            if let Some(rect) = self.allocate_rect(bitmap.width, bitmap.height) {
                let entry_index = self.entries.len();
                let index_slot = atlas_index_insertion_slot(&self.index_slots, key)
                    .ok_or(LayoutError::ArithmeticOverflow)?;
                self.copy_bitmap(rect, bitmap)?;
                let glyph = AtlasGlyph::new(Some(rect), left, top);
                self.entries.push(AtlasEntry {
                    key,
                    glyph,
                    last_used: self.tick,
                });
                self.index_slots[index_slot] = Some(AtlasIndexSlot { key, entry_index });
                self.update_peak()?;
                return Ok(glyph);
            }
            if self.grow(bitmap.width.get().max(bitmap.height.get()))? {
                continue;
            }
            if !self.evict_oldest()? {
                return Err(LayoutError::AtlasSaturated);
            }
        }
        Err(LayoutError::AtlasSaturated)
    }

    /// Removes all glyphs and releases pixel storage under explicit pressure.
    ///
    /// # Errors
    ///
    /// Returns a structured sequence or conversion error before releasing any
    /// storage if pressure accounting cannot advance.
    pub fn pressure(&mut self) -> Result<(), LayoutError> {
        let pressure_events = self
            .pressure_events
            .checked_add(1)
            .ok_or(LayoutError::SequenceExhausted)?;
        let evictions = self
            .evictions
            .checked_add(self.entries.len() as u64)
            .ok_or(LayoutError::SequenceExhausted)?;
        let pixel_revision = if self.pixels.is_empty() {
            self.pixel_revision
        } else {
            self.next_pixel_revision()?
        };
        self.pressure_events = pressure_events;
        self.evictions = evictions;
        self.dimension = 0;
        self.pixel_revision = pixel_revision;
        self.pixels = Vec::new();
        self.entries = Vec::new();
        self.index_slots = Vec::new();
        self.free = Vec::new();
        self.dirty_rows.clear(pixel_revision);
        Ok(())
    }

    /// Returns tightly packed atlas pixels for immutable scene publication.
    #[must_use]
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// Returns handle-free current and peak accounting.
    #[must_use]
    pub fn snapshot(&self) -> GlyphAtlasSnapshot {
        GlyphAtlasSnapshot {
            dimension: self.dimension,
            pixel_revision: self.pixel_revision,
            pixel_bytes: self.pixels.capacity(),
            metadata_bytes: self.metadata_bytes(),
            peak_bytes: self.peak_bytes,
            budget_bytes: self.budget_bytes.get(),
            entries: self.entries.len(),
            hits: self.hits,
            misses: self.misses,
            evictions: self.evictions,
            pressure_events: self.pressure_events,
        }
    }

    fn allocate_rect(&mut self, width: NonZeroU32, height: NonZeroU32) -> Option<AtlasRect> {
        let index = self
            .free
            .iter()
            .enumerate()
            .filter(|(_, rect)| {
                rect.width.get() >= width.get() && rect.height.get() >= height.get()
            })
            .min_by_key(|(_, rect)| u64::from(rect.width.get()) * u64::from(rect.height.get()))
            .map(|(index, _)| index)?;
        let available = self.free.remove(index);
        let allocated = AtlasRect::new(available.x, available.y, width, height);
        let right_width = available.width.get() - width.get();
        if let Some(right_width) = NonZeroU32::new(right_width) {
            self.free.push(AtlasRect::new(
                available.x + width.get(),
                available.y,
                right_width,
                height,
            ));
        }
        let bottom_height = available.height.get() - height.get();
        if let Some(bottom_height) = NonZeroU32::new(bottom_height) {
            self.free.push(AtlasRect::new(
                available.x,
                available.y + height.get(),
                available.width,
                bottom_height,
            ));
        }
        Some(allocated)
    }

    fn grow(&mut self, minimum: u32) -> Result<bool, LayoutError> {
        let source_revision = self.pixel_revision;
        let geometric = NonZeroU32::new(self.dimension).map_or(Ok(256), |dimension| {
            dimension
                .get()
                .checked_mul(2)
                .ok_or(LayoutError::ArithmeticOverflow)
        })?;
        let required = minimum
            .checked_next_power_of_two()
            .ok_or(LayoutError::ArithmeticOverflow)?;
        let next = geometric.max(required);
        let pixel_bytes = usize_from_u32(next)
            .checked_mul(usize_from_u32(next))
            .ok_or(LayoutError::ArithmeticOverflow)?;
        reserve_atlas_rects(&mut self.free, 2)?;
        let mut pixels = Vec::new();
        reserve_bytes_exact(&mut pixels, pixel_bytes)?;
        pixels.resize(pixel_bytes, 0);
        let retained = pixels
            .capacity()
            .checked_add(self.metadata_bytes())
            .ok_or(LayoutError::ArithmeticOverflow)?;
        if exceeds_budget(retained, self.budget_bytes.get()) {
            self.free.shrink_to_fit();
            return Ok(false);
        }
        let pixel_revision = self.next_pixel_revision()?;
        if self.dimension != 0 {
            let old = usize_from_u32(self.dimension);
            let new = usize_from_u32(next);
            for row in 0..old {
                let source = row * old..row * old + old;
                let destination = row * new..row * new + old;
                pixels[destination].copy_from_slice(&self.pixels[source]);
            }
            self.free.push(AtlasRect::new(
                self.dimension,
                0,
                NonZeroU32::new(next - self.dimension).ok_or(LayoutError::ArithmeticOverflow)?,
                NonZeroU32::new(self.dimension).ok_or(LayoutError::ArithmeticOverflow)?,
            ));
            self.free.push(AtlasRect::new(
                0,
                self.dimension,
                NonZeroU32::new(next).ok_or(LayoutError::ArithmeticOverflow)?,
                NonZeroU32::new(next - self.dimension).ok_or(LayoutError::ArithmeticOverflow)?,
            ));
        } else {
            self.free.push(AtlasRect::new(
                0,
                0,
                NonZeroU32::new(next).ok_or(LayoutError::ArithmeticOverflow)?,
                NonZeroU32::new(next).ok_or(LayoutError::ArithmeticOverflow)?,
            ));
        }
        self.dimension = next;
        self.pixel_revision = pixel_revision;
        self.pixels = pixels;
        self.dirty_rows.mark_full(source_revision);
        self.update_peak()?;
        Ok(true)
    }

    fn copy_bitmap(&mut self, rect: AtlasRect, bitmap: &GlyphBitmap) -> Result<(), LayoutError> {
        let stride = usize_from_u32(self.dimension);
        let width = usize_from_u32(rect.width.get());
        let height = usize_from_u32(rect.height.get());
        let x = usize_from_u32(rect.x);
        let y = usize_from_u32(rect.y);
        let final_end = y
            .checked_add(height.saturating_sub(1))
            .and_then(|row| row.checked_mul(stride))
            .and_then(|start| start.checked_add(x))
            .and_then(|start| start.checked_add(width))
            .ok_or(LayoutError::ArithmeticOverflow)?;
        if final_end > self.pixels.len() {
            return Err(LayoutError::ArithmeticOverflow);
        }
        let pixel_revision = self.next_pixel_revision()?;
        self.dirty_rows.insert(
            self.pixel_revision,
            rect.y,
            rect.y
                .checked_add(rect.height.get())
                .ok_or(LayoutError::ArithmeticOverflow)?,
        );
        for row in 0..height {
            let source = row * width..row * width + width;
            let start = (y + row)
                .checked_mul(stride)
                .and_then(|value| value.checked_add(x))
                .ok_or(LayoutError::ArithmeticOverflow)?;
            self.pixels[start..start + width].copy_from_slice(&bitmap.pixels[source]);
        }
        self.pixel_revision = pixel_revision;
        Ok(())
    }

    fn evict_oldest(&mut self) -> Result<bool, LayoutError> {
        let Some((index, _)) = self
            .entries
            .iter()
            .enumerate()
            .min_by_key(|(_, entry)| entry.last_used)
        else {
            return Ok(false);
        };
        let rect = self.entries[index].glyph.rect();
        if rect.is_some() {
            reserve_atlas_rects(&mut self.free, 1)?;
        }
        let removed = self.remove_indexed_entry(index)?;
        if let Some(rect) = removed.glyph.rect() {
            self.clear_rect(rect)?;
            self.free.push(rect);
            self.coalesce_free();
        }
        self.evictions = self
            .evictions
            .checked_add(1)
            .ok_or(LayoutError::SequenceExhausted)?;
        Ok(true)
    }

    fn clear_rect(&mut self, rect: AtlasRect) -> Result<(), LayoutError> {
        let stride = usize_from_u32(self.dimension);
        let width = usize_from_u32(rect.width.get());
        let x = usize_from_u32(rect.x);
        let y = usize_from_u32(rect.y);
        let height = usize_from_u32(rect.height.get());
        let final_end = y
            .checked_add(height.saturating_sub(1))
            .and_then(|row| row.checked_mul(stride))
            .and_then(|start| start.checked_add(x))
            .and_then(|start| start.checked_add(width))
            .ok_or(LayoutError::ArithmeticOverflow)?;
        if final_end > self.pixels.len() {
            return Err(LayoutError::ArithmeticOverflow);
        }
        let pixel_revision = self.next_pixel_revision()?;
        self.dirty_rows.insert(
            self.pixel_revision,
            rect.y,
            rect.y
                .checked_add(rect.height.get())
                .ok_or(LayoutError::ArithmeticOverflow)?,
        );
        for row in 0..height {
            let start = (y + row)
                .checked_mul(stride)
                .and_then(|value| value.checked_add(x))
                .ok_or(LayoutError::ArithmeticOverflow)?;
            let end = start
                .checked_add(width)
                .ok_or(LayoutError::ArithmeticOverflow)?;
            self.pixels
                .get_mut(start..end)
                .ok_or(LayoutError::ArithmeticOverflow)?
                .fill(0);
        }
        self.pixel_revision = pixel_revision;
        Ok(())
    }

    fn next_pixel_revision(&self) -> Result<u64, LayoutError> {
        self.pixel_revision
            .checked_add(1)
            .ok_or(LayoutError::SequenceExhausted)
    }

    fn coalesce_free(&mut self) {
        loop {
            let mut merged = None;
            'outer: for left in 0..self.free.len() {
                for right in (left..self.free.len()).skip(1) {
                    if let Some(rect) = merge_rects(self.free[left], self.free[right]) {
                        merged = Some((left, right, rect));
                        break 'outer;
                    }
                }
            }
            let Some((left, right, rect)) = merged else {
                break;
            };
            self.free.remove(right);
            self.free[left] = rect;
        }
    }

    fn metadata_bytes(&self) -> usize {
        self.metadata_without_index_bytes().saturating_add(
            self.index_slots
                .capacity()
                .saturating_mul(size_of::<Option<AtlasIndexSlot>>()),
        )
    }

    fn metadata_without_index_bytes(&self) -> usize {
        self.entries
            .capacity()
            .saturating_mul(size_of::<AtlasEntry>())
            .saturating_add(self.free.capacity().saturating_mul(size_of::<AtlasRect>()))
    }

    fn current_bytes(&self) -> usize {
        self.pixels.capacity().saturating_add(self.metadata_bytes())
    }

    fn update_peak(&mut self) -> Result<(), LayoutError> {
        let current = self.current_bytes();
        if exceeds_budget(current, self.budget_bytes.get()) {
            return Err(LayoutError::AtlasSaturated);
        }
        self.peak_bytes = self.peak_bytes.max(current);
        Ok(())
    }
}

fn atlas_index_slot_count(entries: usize) -> Result<usize, LayoutError> {
    entries
        .checked_mul(2)
        .map(|slots| slots.max(MIN_ATLAS_INDEX_SLOTS))
        .and_then(usize::checked_next_power_of_two)
        .ok_or(LayoutError::ArithmeticOverflow)
}

const fn atlas_hash_mix(state: u64, value: u64) -> u64 {
    let mut mixed = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    state ^ mixed ^ (mixed >> 31)
}

fn glyph_key_hash(key: GlyphKey) -> u64 {
    let mut state = atlas_hash_mix(0x517c_c1b7_2722_0a95, key.font.family);
    state = atlas_hash_mix(state, u64::from(key.font.size_bits));
    state = atlas_hash_mix(state, u64::from(key.font.scale_bits));
    state = atlas_hash_mix(state, u64::from(key.font.tab_columns.get()));
    state = atlas_hash_mix(state, u64::from(key.glyph_id));
    atlas_hash_mix(state, u64::from(key.subpixel_x))
}

const fn atlas_probe_slot(start: usize, probe: usize, slot_count: usize) -> usize {
    start.wrapping_add(probe) & (slot_count - 1)
}

fn atlas_index_start(key: GlyphKey, slot_count: usize) -> Option<usize> {
    if slot_count == 0 || !slot_count.is_power_of_two() {
        return None;
    }
    let mask = u64::try_from(slot_count.checked_sub(1)?).ok()?;
    usize::try_from(glyph_key_hash(key) & mask).ok()
}

fn atlas_index_slot(slots: &[Option<AtlasIndexSlot>], key: GlyphKey) -> Option<usize> {
    let start = atlas_index_start(key, slots.len())?;
    for probe in 0..slots.len() {
        let slot = atlas_probe_slot(start, probe, slots.len());
        match slots[slot] {
            Some(indexed) if indexed.key == key => return Some(slot),
            Some(_) => {}
            None => return None,
        }
    }
    None
}

fn atlas_index_insertion_slot(slots: &[Option<AtlasIndexSlot>], key: GlyphKey) -> Option<usize> {
    let start = atlas_index_start(key, slots.len())?;
    for probe in 0..slots.len() {
        let slot = atlas_probe_slot(start, probe, slots.len());
        match slots[slot] {
            Some(indexed) if indexed.key == key => return Some(slot),
            Some(_) => {}
            None => return Some(slot),
        }
    }
    None
}

fn remove_atlas_index_slot(
    slots: &mut [Option<AtlasIndexSlot>],
    removed: usize,
) -> Result<(), LayoutError> {
    let slot_count = slots.len();
    if slot_count == 0 || !slot_count.is_power_of_two() {
        return Err(LayoutError::ArithmeticOverflow);
    }
    let removed_slot = slots
        .get_mut(removed)
        .ok_or(LayoutError::ArithmeticOverflow)?;
    if removed_slot.take().is_none() {
        return Err(LayoutError::ArithmeticOverflow);
    }

    for probe in 1..slot_count {
        let source = atlas_probe_slot(removed, probe, slot_count);
        let Some(indexed) = slots[source].take() else {
            return Ok(());
        };
        let destination = atlas_index_insertion_slot(slots, indexed.key)
            .ok_or(LayoutError::ArithmeticOverflow)?;
        slots[destination] = Some(indexed);
    }
    Err(LayoutError::ArithmeticOverflow)
}

fn merge_rects(first: AtlasRect, second: AtlasRect) -> Option<AtlasRect> {
    if first.y == second.y && first.height == second.height {
        if first.right() == second.x {
            return NonZeroU32::new(first.width.get() + second.width.get())
                .map(|width| AtlasRect::new(first.x, first.y, width, first.height));
        }
        if second.right() == first.x {
            return NonZeroU32::new(first.width.get() + second.width.get())
                .map(|width| AtlasRect::new(second.x, first.y, width, first.height));
        }
    }
    if first.x == second.x && first.width == second.width {
        if first.bottom() == second.y {
            return NonZeroU32::new(first.height.get() + second.height.get())
                .map(|height| AtlasRect::new(first.x, first.y, first.width, height));
        }
        if second.bottom() == first.y {
            return NonZeroU32::new(first.height.get() + second.height.get())
                .map(|height| AtlasRect::new(first.x, second.y, first.width, height));
        }
    }
    None
}

/// Structured failure from visible layout, shaping, or glyph retention.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LayoutError {
    /// A checked text operation failed.
    Text(TextError),
    /// A viewport cannot map a document with no logical lines.
    EmptyDocument,
    /// Scroll was negative or non-finite.
    InvalidScroll,
    /// Checked integer or float conversion overflowed.
    ArithmeticOverflow,
    /// A line exceeded its explicit UTF-8 byte ceiling.
    LineByteLimitExceeded {
        /// Zero-based logical line.
        line: usize,
        /// Observed bytes.
        bytes: usize,
        /// Accepted ceiling.
        limit: usize,
    },
    /// Shaping produced more glyphs than the explicit ceiling.
    GlyphLimitExceeded {
        /// Observed glyph count.
        glyphs: usize,
        /// Accepted ceiling.
        limit: usize,
    },
    /// One layout alone exceeded the complete cache budget.
    LayoutExceedsBudget {
        /// Required retained bytes.
        bytes: usize,
        /// Configured ceiling.
        budget: usize,
    },
    /// The shaping boundary returned non-finite or negative metrics.
    InvalidShaperOutput,
    /// A glyph bitmap length did not equal width times height.
    InvalidGlyphBitmap {
        /// Required tightly packed length.
        expected: usize,
        /// Supplied length.
        actual: usize,
    },
    /// One glyph cannot fit inside the entire atlas budget.
    GlyphExceedsAtlasBudget {
        /// Required A8 bytes.
        bytes: usize,
        /// Configured ceiling.
        budget: usize,
    },
    /// No free or evictable atlas region can satisfy the glyph.
    AtlasSaturated,
    /// A bounded allocation failed.
    AllocationFailed,
    /// A monotonic local sequence cannot advance.
    SequenceExhausted,
    /// Native shaping is unsupported on this target.
    UnsupportedPlatform,
    /// The native shaping boundary rejected an operation.
    NativeFailure(&'static str),
}

impl fmt::Display for LayoutError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Text(error) => write!(formatter, "text layout input failed: {error}"),
            Self::EmptyDocument => formatter.write_str("text viewport requires one logical line"),
            Self::InvalidScroll => {
                formatter.write_str("text viewport scroll must be finite and non-negative")
            }
            Self::ArithmeticOverflow => formatter.write_str("text layout arithmetic overflowed"),
            Self::LineByteLimitExceeded { line, bytes, limit } => write!(
                formatter,
                "line {line} has {bytes} bytes, exceeding limit {limit}"
            ),
            Self::GlyphLimitExceeded { glyphs, limit } => write!(
                formatter,
                "line produced {glyphs} glyphs, exceeding limit {limit}"
            ),
            Self::LayoutExceedsBudget { bytes, budget } => write!(
                formatter,
                "line layout requires {bytes} bytes, exceeding cache budget {budget}"
            ),
            Self::InvalidShaperOutput => {
                formatter.write_str("text shaper returned invalid metrics")
            }
            Self::InvalidGlyphBitmap { expected, actual } => write!(
                formatter,
                "glyph bitmap requires {expected} bytes, found {actual}"
            ),
            Self::GlyphExceedsAtlasBudget { bytes, budget } => write!(
                formatter,
                "glyph requires {bytes} bytes, exceeding atlas budget {budget}"
            ),
            Self::AtlasSaturated => {
                formatter.write_str("glyph atlas is saturated after bounded eviction")
            }
            Self::AllocationFailed => formatter.write_str("bounded text-layout allocation failed"),
            Self::SequenceExhausted => formatter.write_str("text-layout sequence exhausted"),
            Self::UnsupportedPlatform => {
                formatter.write_str("CoreText shaping requires Apple Silicon macOS")
            }
            Self::NativeFailure(stage) => write!(formatter, "CoreText shaping failed at {stage}"),
        }
    }
}

impl Error for LayoutError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Text(error) => Some(error),
            _ => None,
        }
    }
}

impl From<TextError> for LayoutError {
    fn from(error: TextError) -> Self {
        Self::Text(error)
    }
}

#[cfg(test)]
mod atlas_publication_tests {
    use std::num::{NonZeroU32, NonZeroUsize};

    use super::{DirtyAtlasRows, GlyphAtlas, GlyphAtlasPublication, MAX_ATLAS_DIRTY_ROW_RANGES};

    fn populated_atlas() -> GlyphAtlas {
        let mut atlas = GlyphAtlas::new(NonZeroUsize::new(4_096).expect("nonzero test budget"));
        atlas.dimension = 4;
        atlas.pixel_revision = 3;
        atlas.pixels = (0_u8..16).collect();
        atlas
    }

    #[test]
    fn publication_is_revision_bound_and_acknowledgement_is_stale_safe() {
        let mut atlas = populated_atlas();
        atlas.dirty_rows.source_revision = 2;
        atlas.dirty_rows.insert(2, 1, 2);

        assert_eq!(
            atlas.publication_since(2),
            Ok(GlyphAtlasPublication::Rows {
                dimension: NonZeroU32::new(4).expect("dimension"),
                source_revision: 2,
                revision: 3,
                rows: vec![super::GlyphAtlasRowUpdate {
                    start_row: 1,
                    row_count: NonZeroU32::new(1).expect("one row"),
                    pixels: vec![4, 5, 6, 7].into_boxed_slice(),
                }]
                .into_boxed_slice(),
                byte_count: 4,
            })
        );

        assert!(!atlas.acknowledge_publication(2));
        assert!(matches!(
            atlas.publication_since(2),
            Ok(GlyphAtlasPublication::Rows { .. })
        ));
        assert!(atlas.acknowledge_publication(3));
        assert_eq!(
            atlas.publication_since(3),
            Ok(GlyphAtlasPublication::Unchanged { revision: 3 })
        );
    }

    #[test]
    fn mismatch_and_full_dirty_state_publish_one_complete_image() {
        let mut atlas = populated_atlas();
        atlas.dirty_rows.source_revision = 2;
        atlas.dirty_rows.insert(2, 1, 2);
        assert_eq!(
            atlas.publication_since(1),
            Ok(GlyphAtlasPublication::Full {
                dimension: NonZeroU32::new(4).expect("dimension"),
                revision: 3,
                pixels: (0_u8..16).collect::<Vec<_>>().into_boxed_slice(),
            })
        );

        atlas.dirty_rows.mark_full(3);
        atlas.pixel_revision = 4;
        assert!(matches!(
            atlas.publication_since(3),
            Ok(GlyphAtlasPublication::Full { revision: 4, .. })
        ));
    }

    #[test]
    fn dirty_ranges_coalesce_and_never_exceed_the_fixed_bound() {
        let mut dirty = DirtyAtlasRows::<MAX_ATLAS_DIRTY_ROW_RANGES>::new();
        dirty.insert(7, 3, 5);
        dirty.insert(7, 5, 8);
        dirty.insert(7, 1, 3);
        assert_eq!(dirty.len, 1);
        assert_eq!(dirty.ranges[0].start, 1);
        assert_eq!(dirty.ranges[0].end, 8);

        dirty.clear(7);
        dirty.insert(7, 1, 2);
        dirty.insert(7, 5, 6);
        dirty.insert(7, 3, 4);
        assert_eq!(
            &dirty.ranges[..dirty.len],
            &[
                super::DirtyRowRange { start: 1, end: 2 },
                super::DirtyRowRange { start: 3, end: 4 },
                super::DirtyRowRange { start: 5, end: 6 },
            ]
        );

        dirty.clear(7);
        for row in 0..MAX_ATLAS_DIRTY_ROW_RANGES {
            let start = u32::try_from(row * 2).expect("bounded test row");
            dirty.insert(7, start, start + 1);
        }
        dirty.insert(7, 129, 130);
        assert_eq!(dirty.len, MAX_ATLAS_DIRTY_ROW_RANGES);
        for pair in dirty.ranges[..dirty.len].windows(2) {
            assert!(pair[0].end < pair[1].start);
        }
        assert_eq!(dirty.ranges[dirty.len - 1].end, 130);
    }

    #[test]
    fn dirty_state_rejects_empty_ranges_and_full_state_rejects_insertions() {
        let mut dirty = DirtyAtlasRows::<4>::new();
        assert!(dirty.is_empty());

        dirty.insert(7, 5, 5);
        dirty.insert(7, 9, 3);
        assert_eq!(dirty, DirtyAtlasRows::new());

        dirty.insert(7, 3, 4);
        assert!(!dirty.is_empty());
        dirty.mark_full(99);
        assert_eq!(dirty.source_revision, 7);
        assert_eq!(dirty.len, 0);
        assert!(dirty.full);
        assert!(!dirty.is_empty());

        let full = dirty;
        dirty.insert(100, 20, 21);
        assert_eq!(dirty, full);
    }

    #[test]
    fn dirty_ranges_merge_adjacent_boundaries() {
        let mut dirty = DirtyAtlasRows::<4>::new();
        dirty.insert(7, 1, 2);
        dirty.insert(7, 2, 3);
        assert_eq!(
            &dirty.ranges[..dirty.len],
            &[super::DirtyRowRange { start: 1, end: 3 }]
        );
    }

    #[test]
    fn dirty_ranges_compact_multiple_overlaps_from_a_nonzero_index() {
        let mut dirty = DirtyAtlasRows::<4>::new();
        for (start, end) in [(0, 2), (4, 6), (8, 10), (12, 14)] {
            dirty.insert(7, start, end);
        }

        dirty.insert(7, 5, 13);

        assert_eq!(
            &dirty.ranges[..dirty.len],
            &[
                super::DirtyRowRange { start: 0, end: 2 },
                super::DirtyRowRange { start: 4, end: 14 },
            ]
        );
        assert_eq!(
            &dirty.ranges[dirty.len..],
            &[super::DirtyRowRange::default(); 2]
        );
    }

    #[test]
    fn full_dirty_range_storage_merges_the_smallest_gap() {
        let mut dirty = DirtyAtlasRows::<4>::new();
        for (start, end) in [(1, 2), (10, 11), (13, 14), (30, 31)] {
            dirty.insert(7, start, end);
        }
        dirty.insert(7, 40, 41);
        assert_eq!(
            &dirty.ranges[..dirty.len],
            &[
                super::DirtyRowRange { start: 1, end: 2 },
                super::DirtyRowRange { start: 10, end: 14 },
                super::DirtyRowRange { start: 30, end: 31 },
                super::DirtyRowRange { start: 40, end: 41 },
            ]
        );
    }

    #[test]
    fn equal_dirty_range_gaps_merge_the_first_pair() {
        let mut dirty = DirtyAtlasRows::<4>::new();
        for (start, end) in [(1, 2), (4, 5), (7, 8), (30, 31)] {
            dirty.insert(7, start, end);
        }
        dirty.insert(7, 40, 41);
        assert_eq!(
            &dirty.ranges[..dirty.len],
            &[
                super::DirtyRowRange { start: 1, end: 5 },
                super::DirtyRowRange { start: 7, end: 8 },
                super::DirtyRowRange { start: 30, end: 31 },
                super::DirtyRowRange { start: 40, end: 41 },
            ]
        );
    }

    #[test]
    fn row_update_reports_a_nonzero_start_row() {
        let update = super::GlyphAtlasRowUpdate {
            start_row: 7,
            row_count: NonZeroU32::new(2).expect("row count"),
            pixels: vec![1, 2].into_boxed_slice(),
        };
        assert_eq!(update.start_row(), 7);
    }

    #[test]
    fn pressure_discards_pending_publication_state() {
        let mut atlas = populated_atlas();
        atlas.dirty_rows.source_revision = 2;
        atlas.dirty_rows.insert(2, 1, 2);
        atlas.pressure().expect("pressure");
        assert!(atlas.dirty_rows.is_empty());
        assert_eq!(atlas.dirty_rows.source_revision, atlas.pixel_revision);
    }
}

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

#[cfg(test)]
#[path = "embedded_tests.rs"]
mod tests;
