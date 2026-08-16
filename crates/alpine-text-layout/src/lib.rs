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
        let visible_count = ceil_to_usize(viewport_height.get() / line_height.get())?
            .checked_add(1)
            .ok_or(LayoutError::ArithmeticOverflow)?;
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
            .checked_add(
                u64::try_from(self.previous.len()).map_err(|_| LayoutError::ArithmeticOverflow)?,
            )
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
    pub fn layout_line<S: TextShaper>(
        &mut self,
        snapshot: &BufferSnapshot,
        line: usize,
        font: FontKey,
        wrap_width: PositiveFinite,
        shaper: &mut S,
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
        self.current
            .try_reserve(1)
            .map_err(|_| LayoutError::AllocationFailed)?;
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
        let expected = usize::try_from(width.get())
            .ok()
            .and_then(|value| value.checked_mul(usize::try_from(height.get()).ok()?))
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
    rect: AtlasRect,
    last_used: u64,
}

/// Exact current and peak evidence for the A8 atlas.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GlyphAtlasSnapshot {
    dimension: u32,
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
    pixels: Vec<u8>,
    entries: Vec<AtlasEntry>,
    free: Vec<AtlasRect>,
    budget_bytes: NonZeroUsize,
    tick: u64,
    peak_bytes: usize,
    hits: u64,
    misses: u64,
    evictions: u64,
    pressure_events: u64,
}

impl GlyphAtlas {
    /// Creates an empty atlas that allocates no pixel storage until first use.
    #[must_use]
    pub fn new(budget_bytes: NonZeroUsize) -> Self {
        Self {
            dimension: 0,
            pixels: Vec::new(),
            entries: Vec::new(),
            free: Vec::new(),
            budget_bytes,
            tick: 0,
            peak_bytes: 0,
            hits: 0,
            misses: 0,
            evictions: 0,
            pressure_events: 0,
        }
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
        self.tick = self
            .tick
            .checked_add(1)
            .ok_or(LayoutError::SequenceExhausted)?;
        if let Some(entry) = self.entries.iter_mut().find(|entry| entry.key == key) {
            entry.last_used = self.tick;
            self.hits = self
                .hits
                .checked_add(1)
                .ok_or(LayoutError::SequenceExhausted)?;
            return Ok(entry.rect);
        }
        self.misses = self
            .misses
            .checked_add(1)
            .ok_or(LayoutError::SequenceExhausted)?;
        let required = bitmap.pixels.len();
        if required > self.budget_bytes.get() {
            return Err(LayoutError::GlyphExceedsAtlasBudget {
                bytes: required,
                budget: self.budget_bytes.get(),
            });
        }

        loop {
            self.entries
                .try_reserve(1)
                .map_err(|_| LayoutError::AllocationFailed)?;
            self.free
                .try_reserve(2)
                .map_err(|_| LayoutError::AllocationFailed)?;
            if self.current_bytes() > self.budget_bytes.get() {
                return Err(LayoutError::AtlasSaturated);
            }
            if let Some(rect) = self.allocate_rect(bitmap.width, bitmap.height) {
                self.copy_bitmap(rect, bitmap)?;
                self.entries.push(AtlasEntry {
                    key,
                    rect,
                    last_used: self.tick,
                });
                self.update_peak()?;
                return Ok(rect);
            }
            if self.grow(bitmap.width.get().max(bitmap.height.get()))? {
                continue;
            }
            if !self.evict_oldest()? {
                return Err(LayoutError::AtlasSaturated);
            }
        }
    }

    /// Removes all glyphs and releases pixel storage under explicit pressure.
    ///
    /// # Errors
    ///
    /// Returns a structured sequence or conversion error before releasing any
    /// storage if pressure accounting cannot advance.
    pub fn pressure(&mut self) -> Result<(), LayoutError> {
        self.pressure_events = self
            .pressure_events
            .checked_add(1)
            .ok_or(LayoutError::SequenceExhausted)?;
        self.evictions = self
            .evictions
            .checked_add(
                u64::try_from(self.entries.len()).map_err(|_| LayoutError::ArithmeticOverflow)?,
            )
            .ok_or(LayoutError::SequenceExhausted)?;
        self.dimension = 0;
        self.pixels = Vec::new();
        self.entries = Vec::new();
        self.free = Vec::new();
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
        let mut next = if self.dimension == 0 {
            256
        } else {
            self.dimension.saturating_mul(2)
        };
        while next < minimum {
            next = next.checked_mul(2).ok_or(LayoutError::ArithmeticOverflow)?;
        }
        let pixel_bytes = usize::try_from(next)
            .ok()
            .and_then(|value| value.checked_mul(value))
            .ok_or(LayoutError::ArithmeticOverflow)?;
        let free_growth = if self.dimension == 0 { 1 } else { 2 };
        self.free
            .try_reserve(free_growth)
            .map_err(|_| LayoutError::AllocationFailed)?;
        let mut pixels = Vec::new();
        pixels
            .try_reserve_exact(pixel_bytes)
            .map_err(|_| LayoutError::AllocationFailed)?;
        pixels.resize(pixel_bytes, 0);
        if pixels
            .capacity()
            .checked_add(self.metadata_bytes())
            .ok_or(LayoutError::ArithmeticOverflow)?
            > self.budget_bytes.get()
        {
            self.free.shrink_to_fit();
            return Ok(false);
        }
        if self.dimension != 0 {
            let old =
                usize::try_from(self.dimension).map_err(|_| LayoutError::ArithmeticOverflow)?;
            let new = usize::try_from(next).map_err(|_| LayoutError::ArithmeticOverflow)?;
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
        self.pixels = pixels;
        self.update_peak()?;
        Ok(true)
    }

    fn copy_bitmap(&mut self, rect: AtlasRect, bitmap: &GlyphBitmap) -> Result<(), LayoutError> {
        let stride =
            usize::try_from(self.dimension).map_err(|_| LayoutError::ArithmeticOverflow)?;
        let width =
            usize::try_from(rect.width.get()).map_err(|_| LayoutError::ArithmeticOverflow)?;
        let height =
            usize::try_from(rect.height.get()).map_err(|_| LayoutError::ArithmeticOverflow)?;
        let x = usize::try_from(rect.x).map_err(|_| LayoutError::ArithmeticOverflow)?;
        let y = usize::try_from(rect.y).map_err(|_| LayoutError::ArithmeticOverflow)?;
        for row in 0..height {
            let source = row * width..row * width + width;
            let start = (y + row)
                .checked_mul(stride)
                .and_then(|value| value.checked_add(x))
                .ok_or(LayoutError::ArithmeticOverflow)?;
            self.pixels[start..start + width].copy_from_slice(&bitmap.pixels[source]);
        }
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
        self.free
            .try_reserve(1)
            .map_err(|_| LayoutError::AllocationFailed)?;
        let removed = self.entries.remove(index);
        self.clear_rect(removed.rect)?;
        self.free.push(removed.rect);
        self.coalesce_free();
        self.evictions = self
            .evictions
            .checked_add(1)
            .ok_or(LayoutError::SequenceExhausted)?;
        Ok(true)
    }

    fn clear_rect(&mut self, rect: AtlasRect) -> Result<(), LayoutError> {
        let stride =
            usize::try_from(self.dimension).map_err(|_| LayoutError::ArithmeticOverflow)?;
        let width =
            usize::try_from(rect.width.get()).map_err(|_| LayoutError::ArithmeticOverflow)?;
        let x = usize::try_from(rect.x).map_err(|_| LayoutError::ArithmeticOverflow)?;
        let y = usize::try_from(rect.y).map_err(|_| LayoutError::ArithmeticOverflow)?;
        let height =
            usize::try_from(rect.height.get()).map_err(|_| LayoutError::ArithmeticOverflow)?;
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
        Ok(())
    }

    fn coalesce_free(&mut self) {
        loop {
            let mut merged = None;
            'outer: for left in 0..self.free.len() {
                for right in left + 1..self.free.len() {
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
        if current > self.budget_bytes.get() {
            return Err(LayoutError::AtlasSaturated);
        }
        self.peak_bytes = self.peak_bytes.max(current);
        Ok(())
    }
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
mod tests {
    use std::{cell::Cell, num::NonZeroU32};

    use alpine_text::{Buffer, BufferSnapshot};

    use super::*;

    struct FixtureShaper {
        calls: Cell<usize>,
    }

    impl FixtureShaper {
        fn new() -> Self {
            Self {
                calls: Cell::new(0),
            }
        }
    }

    impl TextShaper for FixtureShaper {
        fn shape(&mut self, text: &str, _font: FontKey) -> Result<LineLayout, LayoutError> {
            self.calls.set(self.calls.get() + 1);
            let glyphs = text
                .chars()
                .enumerate()
                .map(|(index, character)| {
                    let index =
                        u16::try_from(index).map_err(|_| LayoutError::ArithmeticOverflow)?;
                    ShapedGlyph::new(
                        character.into(),
                        f32::from(index) * 8.0,
                        0.0,
                        8.0,
                        u32::from(index),
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;
            let character_count =
                u16::try_from(text.chars().count()).map_err(|_| LayoutError::ArithmeticOverflow)?;
            LineLayout::new(glyphs, f32::from(character_count) * 8.0, 10.0, 3.0, 1024)
        }
    }

    fn font() -> Result<FontKey, &'static str> {
        Ok(FontKey::new(
            7,
            PositiveFinite::new(13.0).ok_or("size")?,
            PositiveFinite::new(2.0).ok_or("scale")?,
            NonZeroU32::new(4).ok_or("tabs")?,
        ))
    }

    fn wrap() -> Result<PositiveFinite, &'static str> {
        PositiveFinite::new(800.0).ok_or("wrap")
    }

    fn snapshot(text: &str) -> BufferSnapshot {
        Buffer::new(text).snapshot()
    }

    #[test]
    fn visible_mapping_bounds_layout_to_overscan() -> Result<(), LayoutError> {
        let lines = VisibleLines::new(
            100,
            40.0,
            PositiveFinite::new(60.0).ok_or(LayoutError::InvalidScroll)?,
            PositiveFinite::new(20.0).ok_or(LayoutError::InvalidScroll)?,
            2,
        )?;
        assert_eq!(lines.visible(), 2..6);
        assert_eq!(lines.laid_out(), 0..8);
        assert!(matches!(
            VisibleLines::new(
                0,
                0.0,
                PositiveFinite::new(1.0).ok_or(LayoutError::InvalidScroll)?,
                PositiveFinite::new(1.0).ok_or(LayoutError::InvalidScroll)?,
                0,
            ),
            Err(LayoutError::EmptyDocument)
        ));
        Ok(())
    }

    #[test]
    fn previous_frame_hit_avoids_materialization_and_shaping() -> Result<(), Box<dyn Error>> {
        let snapshot = snapshot("alpha\nbeta\n");
        let mut cache = LineLayoutCache::new(NonZeroUsize::new(4096).ok_or("budget")?);
        let mut shaper = FixtureShaper::new();
        let first = cache.layout_line(&snapshot, 0, font()?, wrap()?, &mut shaper)?;
        cache.begin_frame()?;
        let second = cache.layout_line(&snapshot, 0, font()?, wrap()?, &mut shaper)?;

        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(shaper.calls.get(), 1);
        assert_eq!(cache.snapshot().hits(), 1);
        assert_eq!(cache.snapshot().misses(), 1);
        assert_eq!(cache.snapshot().shaped_lines(), 1);
        Ok(())
    }

    #[test]
    fn changed_line_misses_but_equal_content_across_snapshots_hits() -> Result<(), Box<dyn Error>> {
        let first = snapshot("same\n");
        let equal = snapshot("same\n");
        let changed = snapshot("different\n");
        let mut cache = LineLayoutCache::new(NonZeroUsize::new(4096).ok_or("budget")?);
        let mut shaper = FixtureShaper::new();
        cache.layout_line(&first, 0, font()?, wrap()?, &mut shaper)?;
        cache.begin_frame()?;
        cache.layout_line(&equal, 0, font()?, wrap()?, &mut shaper)?;
        cache.begin_frame()?;
        cache.layout_line(&changed, 0, font()?, wrap()?, &mut shaper)?;

        assert_eq!(shaper.calls.get(), 2);
        assert_eq!(cache.snapshot().hits(), 1);
        assert_eq!(cache.snapshot().misses(), 2);
        Ok(())
    }

    #[test]
    fn fingerprint_candidate_requires_exact_content_confirmation() -> Result<(), Box<dyn Error>> {
        let first = snapshot("alpha\n");
        let different = snapshot("bravo\n");
        let range = first.line_byte_range(0)?;
        let entry = CacheEntry {
            fingerprint: first.fingerprint(range.clone())?,
            snapshot: first,
            range: range.clone(),
            font: font()?,
            wrap_width_bits: wrap()?.get().to_bits(),
            layout: Arc::new(LineLayout::new(Vec::new(), 0.0, 0.0, 0.0, 1)?),
            retained_bytes: 0,
        };

        assert!(!entry.matches(
            entry.fingerprint,
            &different,
            range,
            entry.font,
            entry.wrap_width_bits,
        )?);
        Ok(())
    }

    #[test]
    fn atlas_allocates_on_demand_reuses_evicts_and_drains() -> Result<(), Box<dyn Error>> {
        let budget = NonZeroUsize::new(256 * 256 + 4096).ok_or("budget")?;
        let mut atlas = GlyphAtlas::new(budget);
        assert_eq!(atlas.snapshot().pixel_bytes(), 0);
        let bitmap = GlyphBitmap::new(
            NonZeroU32::new(8).ok_or("width")?,
            NonZeroU32::new(8).ok_or("height")?,
            vec![255; 64],
        )?;
        let key = GlyphKey::new(font()?, 1, 0);
        let first = atlas.insert(key, &bitmap)?;
        let second = atlas.insert(key, &bitmap)?;
        assert_eq!(first, second);
        assert_eq!(atlas.snapshot().hits(), 1);
        assert_eq!(atlas.snapshot().misses(), 1);
        assert!(atlas.snapshot().peak_bytes() <= budget.get());
        assert_eq!(
            &atlas.pixels()[first.y() as usize * 256 + first.x() as usize..][..8],
            &[255; 8]
        );

        let large = GlyphBitmap::new(
            NonZeroU32::new(32).ok_or("width")?,
            NonZeroU32::new(32).ok_or("height")?,
            vec![127; 32 * 32],
        )?;
        for glyph in 2..72 {
            atlas.insert(GlyphKey::new(font()?, glyph, 0), &large)?;
        }
        assert!(atlas.snapshot().evictions() > 0);
        assert!(atlas.snapshot().pixel_bytes() + atlas.snapshot().metadata_bytes() <= budget.get());

        atlas.pressure()?;
        assert_eq!(atlas.snapshot().pixel_bytes(), 0);
        assert_eq!(atlas.snapshot().entries(), 0);
        assert_eq!(atlas.snapshot().pressure_events(), 1);
        Ok(())
    }

    #[test]
    fn invalid_bitmap_and_line_limits_fail_structurally() -> Result<(), Box<dyn Error>> {
        assert!(matches!(
            GlyphBitmap::new(
                NonZeroU32::new(2).ok_or("width")?,
                NonZeroU32::new(2).ok_or("height")?,
                vec![0; 3]
            ),
            Err(LayoutError::InvalidGlyphBitmap {
                expected: 4,
                actual: 3
            })
        ));
        let long = snapshot(&"x".repeat(DEFAULT_MAX_LINE_BYTES + 1));
        let mut cache =
            LineLayoutCache::new(NonZeroUsize::new(DEFAULT_LAYOUT_BUDGET_BYTES).ok_or("budget")?);
        assert!(matches!(
            cache.layout_line(&long, 0, font()?, wrap()?, &mut FixtureShaper::new()),
            Err(LayoutError::LineByteLimitExceeded { .. })
        ));
        Ok(())
    }
}
