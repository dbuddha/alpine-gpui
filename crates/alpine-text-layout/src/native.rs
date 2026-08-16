//! Audited CoreText and CoreGraphics boundary.

use std::{
    collections::HashMap,
    ffi::c_void,
    num::NonZeroU32,
    ptr::{self, NonNull},
};

use objc2_core_foundation::{
    CFAttributedString, CFDictionary, CFIndex, CFRange, CFRetained, CFString, CGPoint, CGRect,
    CGSize, kCFTypeDictionaryKeyCallBacks, kCFTypeDictionaryValueCallBacks,
};
use objc2_core_graphics::{CGBitmapContextCreate, CGColorSpace, CGContext, CGImageAlphaInfo};
use objc2_core_text::{CTFont, CTFontOrientation, CTLine, CTRun, kCTFontAttributeName};

use crate::{
    DEFAULT_MAX_GLYPHS_PER_LINE, FontKey, GlyphBitmap, GlyphRasterizer, LayoutError, LineLayout,
    RasterizedGlyph, ShapedGlyph, TextShaper,
};

const FIRST_FALLBACK_FAMILY: u64 = 1_u64 << 63;
const QUARTER_PHASES: u8 = 4;

/// Safe Alpine-owned CoreText shaping and A8 rasterization service.
///
/// All native objects remain private. Public results contain copied scalar,
/// vector, string, and bitmap data only.
pub struct CoreTextSystem {
    registered_names: HashMap<u64, String>,
    resolved_names: HashMap<String, u64>,
    fonts: HashMap<(u64, u32), CFRetained<CTFont>>,
    next_fallback_family: u64,
}

impl CoreTextSystem {
    /// Creates an empty font registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            registered_names: HashMap::new(),
            resolved_names: HashMap::new(),
            fonts: HashMap::new(),
            next_fallback_family: FIRST_FALLBACK_FAMILY,
        }
    }

    /// Registers one application-owned family identity and PostScript name.
    ///
    /// Repeating an identical registration is idempotent. Reusing either the
    /// family identity or PostScript name for a different font is rejected.
    ///
    /// # Errors
    ///
    /// Returns a structured native failure for zero identities, empty names,
    /// or conflicting registrations.
    pub fn register_font(
        &mut self,
        family: u64,
        post_script_name: &str,
    ) -> Result<(), LayoutError> {
        if family == 0 || post_script_name.is_empty() {
            return Err(LayoutError::NativeFailure("font registration"));
        }
        if let Some(existing) = self.registered_names.get(&family) {
            return if existing == post_script_name {
                Ok(())
            } else {
                Err(LayoutError::NativeFailure("font family conflict"))
            };
        }
        if self.resolved_names.contains_key(post_script_name) {
            return Err(LayoutError::NativeFailure("font name conflict"));
        }
        self.registered_names
            .try_reserve(1)
            .map_err(|_| LayoutError::AllocationFailed)?;
        self.resolved_names
            .try_reserve(1)
            .map_err(|_| LayoutError::AllocationFailed)?;
        self.registered_names
            .insert(family, post_script_name.to_owned());
        self.resolved_names
            .insert(post_script_name.to_owned(), family);
        Ok(())
    }

    fn font(&mut self, key: FontKey) -> Result<CFRetained<CTFont>, LayoutError> {
        let cache_key = (key.family(), key.size().to_bits());
        if let Some(font) = self.fonts.get(&cache_key) {
            return Ok(font.clone());
        }
        let name = self
            .registered_names
            .get(&key.family())
            .ok_or(LayoutError::NativeFailure("unregistered font"))?;
        let name = CFString::from_str(name);
        // SAFETY: The CFString is live and a null matrix selects CoreText's
        // documented identity transform.
        let font = unsafe { CTFont::with_name(&name, f64::from(key.size()), ptr::null()) };
        self.fonts
            .try_reserve(1)
            .map_err(|_| LayoutError::AllocationFailed)?;
        self.fonts.insert(cache_key, font.clone());
        Ok(font)
    }

    fn intern_run_font(&mut self, requested: FontKey, font: &CTFont) -> Result<u64, LayoutError> {
        // SAFETY: `font` is a live CoreText font supplied by a live run.
        let name = unsafe { font.post_script_name() }.to_string();
        if let Some(family) = self.resolved_names.get(&name).copied() {
            self.retain_run_font(family, requested.size(), font)?;
            return Ok(family);
        }
        let family = self.allocate_fallback_family()?;
        self.registered_names
            .try_reserve(1)
            .map_err(|_| LayoutError::AllocationFailed)?;
        self.resolved_names
            .try_reserve(1)
            .map_err(|_| LayoutError::AllocationFailed)?;
        self.registered_names.insert(family, name.clone());
        self.resolved_names.insert(name, family);
        self.retain_run_font(family, requested.size(), font)?;
        Ok(family)
    }

    fn allocate_fallback_family(&mut self) -> Result<u64, LayoutError> {
        let start = self.next_fallback_family;
        loop {
            let candidate = self.next_fallback_family;
            self.next_fallback_family = self
                .next_fallback_family
                .checked_add(1)
                .ok_or(LayoutError::SequenceExhausted)?;
            if candidate != 0 && !self.registered_names.contains_key(&candidate) {
                return Ok(candidate);
            }
            if self.next_fallback_family == start {
                return Err(LayoutError::SequenceExhausted);
            }
        }
    }

    fn retain_run_font(
        &mut self,
        family: u64,
        size: f32,
        font: &CTFont,
    ) -> Result<(), LayoutError> {
        let key = (family, size.to_bits());
        if self.fonts.contains_key(&key) {
            return Ok(());
        }
        self.fonts
            .try_reserve(1)
            .map_err(|_| LayoutError::AllocationFailed)?;
        // SAFETY: The run owns this live CTFont for the duration of the call;
        // retaining it gives the cache independent ownership.
        self.fonts
            .insert(key, unsafe { CFRetained::retain(font.into()) });
        Ok(())
    }

    fn expand_tabs(text: &str, tab_columns: NonZeroU32) -> Result<(String, Vec<u32>), LayoutError> {
        let mut expanded = String::new();
        let mut source_indices = Vec::new();
        expanded
            .try_reserve(text.len())
            .map_err(|_| LayoutError::AllocationFailed)?;
        source_indices
            .try_reserve(text.encode_utf16().count())
            .map_err(|_| LayoutError::AllocationFailed)?;
        let mut source_utf16 = 0_u32;
        let mut column = 0_u32;
        for character in text.chars() {
            if character == '\t' {
                let remainder = column % tab_columns.get();
                let spaces = tab_columns.get() - remainder;
                for _ in 0..spaces {
                    expanded.push(' ');
                    source_indices.push(source_utf16);
                }
                column = column
                    .checked_add(spaces)
                    .ok_or(LayoutError::ArithmeticOverflow)?;
            } else {
                expanded.push(character);
                let units = u32::try_from(character.len_utf16())
                    .map_err(|_| LayoutError::ArithmeticOverflow)?;
                for _ in 0..units {
                    source_indices.push(source_utf16);
                }
                source_utf16 = source_utf16
                    .checked_add(units)
                    .ok_or(LayoutError::ArithmeticOverflow)?;
                column = column
                    .checked_add(1)
                    .ok_or(LayoutError::ArithmeticOverflow)?;
                continue;
            }
            source_utf16 = source_utf16
                .checked_add(1)
                .ok_or(LayoutError::ArithmeticOverflow)?;
        }
        Ok((expanded, source_indices))
    }

    fn attributed_line(text: &str, font: &CTFont) -> Result<CFRetained<CTLine>, LayoutError> {
        let string = CFString::from_str(text);
        // SAFETY: Both pointers reference live CFType objects, the count is
        // exact, and Core Foundation's standard callbacks retain both values.
        let attributes = unsafe {
            let key = kCTFontAttributeName;
            let mut keys = [(key as *const CFString).cast::<c_void>()];
            let mut values = [(font as *const CTFont).cast::<c_void>()];
            CFDictionary::new(
                None,
                keys.as_mut_ptr(),
                values.as_mut_ptr(),
                1,
                ptr::addr_of!(kCFTypeDictionaryKeyCallBacks),
                ptr::addr_of!(kCFTypeDictionaryValueCallBacks),
            )
            .ok_or(LayoutError::AllocationFailed)?
        };
        // SAFETY: The string and correctly typed attribute dictionary are live.
        let attributed = unsafe { CFAttributedString::new(None, Some(&string), Some(&attributes)) }
            .ok_or(LayoutError::AllocationFailed)?;
        // SAFETY: The attributed string is live and immutable for this call.
        Ok(unsafe { CTLine::with_attributed_string(&attributed) })
    }

    fn run_font(run: &CTRun) -> Result<&CTFont, LayoutError> {
        // SAFETY: CoreText documents the run attributes as a live dictionary
        // containing a CTFont under kCTFontAttributeName.
        unsafe {
            let attributes = run.attributes();
            let key = kCTFontAttributeName;
            let value = attributes.value((key as *const CFString).cast::<c_void>());
            value
                .cast::<CTFont>()
                .as_ref()
                .ok_or(LayoutError::NativeFailure("run font"))
        }
    }

    fn mapped_source_index(indices: &[u32], expanded_index: CFIndex) -> Result<u32, LayoutError> {
        let index =
            usize::try_from(expanded_index).map_err(|_| LayoutError::InvalidShaperOutput)?;
        indices
            .get(index)
            .copied()
            .ok_or(LayoutError::InvalidShaperOutput)
    }

    fn raster_bounds(
        font: &CTFont,
        glyph: u16,
        scale: f32,
    ) -> Result<Option<(f32, f32, usize, usize)>, LayoutError> {
        let mut native_glyph = glyph;
        let glyph_ptr = NonNull::from(&mut native_glyph);
        let mut rect = CGRect {
            origin: CGPoint { x: 0.0, y: 0.0 },
            size: CGSize {
                width: 0.0,
                height: 0.0,
            },
        };
        // SAFETY: Both output pointers are live for exactly one glyph.
        unsafe {
            font.bounding_rects_for_glyphs(
                CTFontOrientation::Default,
                glyph_ptr,
                ptr::from_mut(&mut rect),
                1,
            );
        }
        let scale = f64::from(scale);
        let left = (rect.origin.x * scale).floor() / scale;
        let bottom = (rect.origin.y * scale).floor() / scale;
        let right = ((rect.origin.x + rect.size.width) * scale).ceil() / scale;
        let top = ((rect.origin.y + rect.size.height) * scale).ceil() / scale;
        if right <= left || top <= bottom {
            return Ok(None);
        }
        let width = usize::try_from(((right - left) * scale).round() as i128)
            .map_err(|_| LayoutError::ArithmeticOverflow)?;
        let height = usize::try_from(((top - bottom) * scale).round() as i128)
            .map_err(|_| LayoutError::ArithmeticOverflow)?;
        if width == 0 || height == 0 {
            return Ok(None);
        }
        Ok(Some((left as f32, top as f32, width, height)))
    }
}

impl Default for CoreTextSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl TextShaper for CoreTextSystem {
    fn shape(&mut self, text: &str, font_key: FontKey) -> Result<LineLayout, LayoutError> {
        let font = self.font(font_key)?;
        let (expanded, source_indices) = Self::expand_tabs(text, font_key.tab_columns())?;
        let line = Self::attributed_line(&expanded, &font)?;
        // SAFETY: The line stays live while its retained run array is read.
        let runs = unsafe { line.glyph_runs() };
        let run_count =
            usize::try_from(runs.count()).map_err(|_| LayoutError::ArithmeticOverflow)?;
        let glyph_count = usize::try_from(unsafe { line.glyph_count() })
            .map_err(|_| LayoutError::ArithmeticOverflow)?;
        if glyph_count > DEFAULT_MAX_GLYPHS_PER_LINE {
            return Err(LayoutError::GlyphLimitExceeded {
                glyphs: glyph_count,
                limit: DEFAULT_MAX_GLYPHS_PER_LINE,
            });
        }
        let mut glyphs = Vec::new();
        glyphs
            .try_reserve_exact(glyph_count)
            .map_err(|_| LayoutError::AllocationFailed)?;
        for run_index in 0..run_count {
            let run_index =
                CFIndex::try_from(run_index).map_err(|_| LayoutError::ArithmeticOverflow)?;
            // SAFETY: The index is below the retained array count and CoreText
            // documents every member as CTRun.
            let run = unsafe {
                runs.value_at_index(run_index)
                    .cast::<CTRun>()
                    .as_ref()
                    .ok_or(LayoutError::NativeFailure("glyph run"))?
            };
            let count = usize::try_from(unsafe { run.glyph_count() })
                .map_err(|_| LayoutError::ArithmeticOverflow)?;
            let mut native_glyphs = vec![0_u16; count];
            let mut positions = vec![CGPoint { x: 0.0, y: 0.0 }; count];
            let mut advances = vec![
                CGSize {
                    width: 0.0,
                    height: 0.0
                };
                count
            ];
            let mut string_indices = vec![0_isize; count];
            if count > 0 {
                // SAFETY: Every vector has exactly `count` initialized slots;
                // the zero CFRange requests the complete run.
                unsafe {
                    run.glyphs(
                        CFRange::new(0, 0),
                        NonNull::new_unchecked(native_glyphs.as_mut_ptr()),
                    );
                    run.positions(
                        CFRange::new(0, 0),
                        NonNull::new_unchecked(positions.as_mut_ptr()),
                    );
                    run.advances(
                        CFRange::new(0, 0),
                        NonNull::new_unchecked(advances.as_mut_ptr()),
                    );
                    run.string_indices(
                        CFRange::new(0, 0),
                        NonNull::new_unchecked(string_indices.as_mut_ptr()),
                    );
                }
            }
            let run_font = Self::run_font(run)?;
            let resolved_family = self.intern_run_font(font_key, run_font)?;
            for index in 0..count {
                let source_utf16 =
                    Self::mapped_source_index(&source_indices, string_indices[index])?;
                glyphs.push(ShapedGlyph::new_resolved(
                    u32::from(native_glyphs[index]),
                    positions[index].x as f32,
                    positions[index].y as f32,
                    advances[index].width as f32,
                    source_utf16,
                    resolved_family,
                )?);
            }
        }
        let mut ascent = 0.0;
        let mut descent = 0.0;
        let mut leading = 0.0;
        // SAFETY: All metric outputs are valid stack pointers and the line is live.
        let width = unsafe { line.typographic_bounds(&mut ascent, &mut descent, &mut leading) };
        LineLayout::new(
            glyphs,
            width as f32,
            ascent as f32,
            descent as f32,
            DEFAULT_MAX_GLYPHS_PER_LINE,
        )
    }
}

impl GlyphRasterizer for CoreTextSystem {
    fn rasterize(
        &mut self,
        font_key: FontKey,
        glyph_id: u32,
        subpixel_x: u8,
    ) -> Result<RasterizedGlyph, LayoutError> {
        if subpixel_x >= QUARTER_PHASES {
            return Err(LayoutError::NativeFailure("glyph subpixel phase"));
        }
        let glyph = u16::try_from(glyph_id).map_err(|_| LayoutError::NativeFailure("glyph id"))?;
        let font = self.font(font_key)?;
        let Some((left, top, width, height)) = Self::raster_bounds(&font, glyph, font_key.scale())?
        else {
            return RasterizedGlyph::new(None, 0.0, 0.0);
        };
        let bytes = width
            .checked_mul(height)
            .ok_or(LayoutError::ArithmeticOverflow)?;
        let mut pixels = Vec::new();
        pixels
            .try_reserve_exact(bytes)
            .map_err(|_| LayoutError::AllocationFailed)?;
        pixels.resize(bytes, 0);
        let color_space = CGColorSpace::new_device_gray()
            .ok_or(LayoutError::NativeFailure("gray color space"))?;
        // SAFETY: The pixel allocation is live, writable, tightly packed, and
        // retained until the bitmap context is dropped before this function returns.
        let context = unsafe {
            CGBitmapContextCreate(
                pixels.as_mut_ptr().cast::<c_void>(),
                width,
                height,
                8,
                width,
                Some(&color_space),
                CGImageAlphaInfo::None.0,
            )
        }
        .ok_or(LayoutError::NativeFailure("bitmap context"))?;
        CGContext::set_should_antialias(Some(&context), true);
        CGContext::set_allows_font_smoothing(Some(&context), false);
        CGContext::set_gray_fill_color(Some(&context), 1.0, 1.0);
        CGContext::scale_ctm(
            Some(&context),
            f64::from(font_key.scale()),
            f64::from(font_key.scale()),
        );
        let phase = f64::from(subpixel_x) / f64::from(QUARTER_PHASES) / f64::from(font_key.scale());
        let bottom = top - height as f32 / font_key.scale();
        CGContext::translate_ctm(Some(&context), -f64::from(left) + phase, -f64::from(bottom));
        let mut native_glyph = glyph;
        let mut position = CGPoint { x: 0.0, y: 0.0 };
        // SAFETY: Both arrays contain one initialized element and the context
        // remains live through the draw.
        unsafe {
            font.draw_glyphs(
                NonNull::from(&mut native_glyph),
                NonNull::from(&mut position),
                1,
                &context,
            );
        }
        drop(context);
        let mut top_down = Vec::new();
        top_down
            .try_reserve_exact(bytes)
            .map_err(|_| LayoutError::AllocationFailed)?;
        for row in pixels.chunks_exact(width).rev() {
            top_down.extend_from_slice(row);
        }
        let width =
            NonZeroU32::new(u32::try_from(width).map_err(|_| LayoutError::ArithmeticOverflow)?)
                .ok_or(LayoutError::InvalidShaperOutput)?;
        let height =
            NonZeroU32::new(u32::try_from(height).map_err(|_| LayoutError::ArithmeticOverflow)?)
                .ok_or(LayoutError::InvalidShaperOutput)?;
        let bitmap = GlyphBitmap::new(width, height, top_down)?;
        RasterizedGlyph::new(Some(bitmap), left, top)
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU32;

    use super::*;
    use crate::PositiveFinite;

    fn font_key() -> Result<FontKey, LayoutError> {
        Ok(FontKey::new(
            1,
            PositiveFinite::new(14.0).ok_or(LayoutError::InvalidShaperOutput)?,
            PositiveFinite::new(2.0).ok_or(LayoutError::InvalidShaperOutput)?,
            NonZeroU32::new(4).ok_or(LayoutError::InvalidShaperOutput)?,
        ))
    }

    #[test]
    fn shapes_tabs_unicode_and_rasterizes_copied_a8() -> Result<(), LayoutError> {
        let mut system = CoreTextSystem::new();
        system.register_font(1, "Menlo-Regular")?;
        let key = font_key()?;
        let layout = system.shape("a\té漢", key)?;
        assert!(layout.width() > 0.0);
        assert!(layout.ascent() > 0.0);
        assert!(
            layout
                .glyphs()
                .iter()
                .all(|glyph| glyph.resolved_family() != 0)
        );
        assert!(
            layout
                .glyphs()
                .windows(2)
                .all(|pair| pair[0].source_utf16() <= pair[1].source_utf16())
        );

        let visible = layout
            .glyphs()
            .iter()
            .copied()
            .find(|glyph| glyph.advance() > 0.0)
            .ok_or(LayoutError::NativeFailure("test glyph"))?;
        let resolved = FontKey::new(
            visible.resolved_family(),
            PositiveFinite::new(key.size()).ok_or(LayoutError::InvalidShaperOutput)?,
            PositiveFinite::new(key.scale()).ok_or(LayoutError::InvalidShaperOutput)?,
            key.tab_columns(),
        );
        let raster = system.rasterize(resolved, visible.glyph_id(), 0)?;
        if let Some(bitmap) = raster.bitmap() {
            assert!(bitmap.pixels.iter().any(|alpha| *alpha != 0));
        }
        Ok(())
    }

    #[test]
    fn registration_and_subpixel_policy_fail_structurally() -> Result<(), LayoutError> {
        let mut system = CoreTextSystem::new();
        system.register_font(1, "Menlo-Regular")?;
        assert_eq!(
            system.register_font(1, "SFMono-Regular"),
            Err(LayoutError::NativeFailure("font family conflict"))
        );
        assert_eq!(
            system.rasterize(font_key()?, 1, QUARTER_PHASES),
            Err(LayoutError::NativeFailure("glyph subpixel phase"))
        );
        Ok(())
    }
}
