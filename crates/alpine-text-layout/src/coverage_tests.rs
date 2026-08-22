//! Coverage-driving tests kept outside production source reporting.

use std::{
    cell::Cell,
    error::Error,
    num::{NonZeroU32, NonZeroUsize},
    sync::Arc,
};

use alpine_text::{Buffer, BufferSnapshot};

use super::*;

struct FixtureShaper {
    calls: Cell<usize>,
}

struct FailingShaper;

impl TextShaper for FailingShaper {
    fn shape(&mut self, _text: &str, _font: FontKey) -> Result<LineLayout, LayoutError> {
        Err(LayoutError::NativeFailure("injected shaper"))
    }
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
                let index = u16::try_from(index).map_err(|_| LayoutError::ArithmeticOverflow)?;
                ShapedGlyph::new(
                    character.into(),
                    f32::from(index) * 8.0,
                    0.0,
                    8.0,
                    u32::from(index),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let count =
            u16::try_from(text.chars().count()).map_err(|_| LayoutError::ArithmeticOverflow)?;
        LineLayout::new(glyphs, f32::from(count) * 8.0, 10.0, 3.0, 1024)
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

fn retained_entry(bytes: usize) -> Result<CacheEntry, Box<dyn Error>> {
    let text = snapshot("x");
    let range = text.line_byte_range(0)?;
    Ok(CacheEntry {
        fingerprint: text.fingerprint(range.clone())?,
        snapshot: text,
        range,
        font: font()?,
        wrap_width_bits: wrap()?.get().to_bits(),
        layout: Arc::new(LineLayout::new(Vec::new(), 0.0, 0.0, 0.0, 1)?),
        retained_bytes: bytes,
    })
}

#[test]
#[allow(clippy::float_cmp)]
fn value_observers_validation_and_cache_branches_are_complete() -> Result<(), Box<dyn Error>> {
    assert_eq!(DEFAULT_LAYOUT_BUDGET_BYTES, 33_554_432);
    assert_eq!(DEFAULT_ATLAS_BUDGET_BYTES, 16_777_216);
    assert_eq!(DEFAULT_OVERSCAN_LINES, 3);
    assert_eq!(DEFAULT_MAX_LINE_BYTES, 1_048_576);
    assert_eq!(DEFAULT_MAX_GLYPHS_PER_LINE, 1_048_576);
    assert_eq!(floor_to_usize(3.75), Ok(3));
    assert_eq!(ceil_to_usize(3.25), Ok(4));
    assert_eq!(floor_to_usize(-1.0), Err(LayoutError::ArithmeticOverflow));
    assert_eq!(ceil_to_usize(-1.0), Err(LayoutError::ArithmeticOverflow));
    assert_eq!(
        floor_to_usize(f32::NAN),
        Err(LayoutError::ArithmeticOverflow)
    );
    assert_eq!(
        ceil_to_usize(f32::NAN),
        Err(LayoutError::ArithmeticOverflow)
    );
    assert!(!exceeds_budget(7, 7));
    assert!(exceeds_budget(8, 7));
    assert!(PositiveFinite::new(0.0).is_none());
    assert!(PositiveFinite::new(f32::NAN).is_none());
    let key = font()?;
    assert_eq!(key.family(), 7);
    assert_eq!(key.size(), 13.0);
    assert_eq!(key.scale(), 2.0);
    assert_eq!(key.tab_columns().get(), 4);

    let glyph = ShapedGlyph::new_resolved(9, 1.0, 2.0, 3.0, 4, 5)?;
    assert_eq!(glyph.glyph_id(), 9);
    assert_eq!(glyph.x(), 1.0);
    assert_eq!(glyph.y(), 2.0);
    assert_eq!(glyph.advance(), 3.0);
    assert_eq!(glyph.source_utf16(), 4);
    assert_eq!(glyph.resolved_family(), 5);
    assert_eq!(
        ShapedGlyph::new_resolved(0, f32::NAN, 0.0, 0.0, 0, 1),
        Err(LayoutError::InvalidShaperOutput)
    );
    let layout = LineLayout::new(vec![glyph, glyph], 3.0, 4.0, 2.0, 2)?;
    assert_eq!(layout.width(), 3.0);
    assert_eq!(layout.ascent(), 4.0);
    assert_eq!(layout.descent(), 2.0);
    assert_eq!(layout.retained_bytes(), 2 * size_of::<ShapedGlyph>());
    assert!(matches!(
        LineLayout::new(vec![glyph], 0.0, 0.0, 0.0, 0),
        Err(LayoutError::GlyphLimitExceeded { .. })
    ));
    assert_eq!(
        LineLayout::new(Vec::new(), f32::NAN, 0.0, 0.0, 0),
        Err(LayoutError::InvalidShaperOutput)
    );

    let one = NonZeroU32::new(1).ok_or("one")?;
    let bitmap = GlyphBitmap::new(one, one, vec![255])?;
    let raster = RasterizedGlyph::new(Some(bitmap.clone()), -1.0, 2.0)?;
    assert_eq!(raster.bitmap(), Some(&bitmap));
    assert_eq!(raster.left(), -1.0);
    assert_eq!(raster.top(), 2.0);
    assert_eq!(
        RasterizedGlyph::new(None, f32::NAN, 0.0),
        Err(LayoutError::InvalidShaperOutput)
    );

    let unit = PositiveFinite::new(1.0).ok_or("unit")?;
    assert_eq!(
        VisibleLines::new(1, -1.0, unit, unit, 0),
        Err(LayoutError::InvalidScroll)
    );
    let huge = PositiveFinite::new(f32::MAX).ok_or("huge")?;
    let tiny = PositiveFinite::new(f32::MIN_POSITIVE).ok_or("tiny")?;
    assert_eq!(
        VisibleLines::new(1, f32::MAX, unit, tiny, 0),
        Err(LayoutError::ArithmeticOverflow)
    );
    assert_eq!(
        VisibleLines::new(1, 0.0, huge, tiny, 0),
        Err(LayoutError::ArithmeticOverflow)
    );

    let budget = NonZeroUsize::new(4096).ok_or("budget")?;
    let mut cache = LineLayoutCache::new(budget);
    let text = snapshot(
        "alpha\r
beta\r",
    );
    let mut shaper = FixtureShaper::new();
    let first = cache.layout_line(&text, 0, key, wrap()?, &mut shaper)?;
    assert_eq!(first.width(), 40.0);
    let current_hit = cache.layout_line(&text, 0, key, wrap()?, &mut shaper)?;
    assert!(Arc::ptr_eq(&first, &current_hit));
    let carriage_return = cache.layout_line(&text, 1, key, wrap()?, &mut shaper)?;
    assert_eq!(carriage_return.width(), 32.0);
    cache.begin_frame()?;
    let cache_snapshot = cache.snapshot();
    assert!(cache_snapshot.current_bytes() <= cache_snapshot.peak_bytes());
    assert_eq!(cache_snapshot.budget_bytes(), budget.get());
    assert_eq!(cache_snapshot.current_entries(), 0);
    assert_eq!(cache_snapshot.previous_entries(), 2);
    assert_eq!(cache_snapshot.hits(), 1);
    assert_eq!(cache_snapshot.misses(), 2);
    assert_eq!(cache_snapshot.evictions(), 0);
    assert_eq!(cache_snapshot.shaped_lines(), 2);

    let invalid_range = usize::MAX..usize::MAX;
    let valid_range = text.line_byte_range(0)?;
    let fingerprint = text.fingerprint(valid_range.clone())?;
    let entry = CacheEntry {
        fingerprint,
        snapshot: text.clone(),
        range: invalid_range,
        font: key,
        wrap_width_bits: wrap()?.get().to_bits(),
        layout: Arc::new(LineLayout::new(Vec::new(), 0.0, 0.0, 0.0, 1)?),
        retained_bytes: 0,
    };
    let mut invalid_current = LineLayoutCache::new(budget);
    invalid_current.current.push(entry.clone());
    assert!(matches!(
        invalid_current.layout_line(&text, 0, key, wrap()?, &mut shaper),
        Err(LayoutError::Text(_))
    ));
    let mut invalid_previous = LineLayoutCache::new(budget);
    invalid_previous.previous.push(entry);
    assert!(matches!(
        invalid_previous.layout_line(&text, 0, key, wrap()?, &mut shaper),
        Err(LayoutError::Text(_))
    ));

    let mut glyph_limited = LineLayoutCache::new(budget);
    glyph_limited.max_glyphs_per_line = 0;
    assert!(matches!(
        glyph_limited.layout_line(&snapshot("x"), 0, key, wrap()?, &mut shaper),
        Err(LayoutError::GlyphLimitExceeded { .. })
    ));
    let mut exact_bytes = LineLayoutCache::new(budget);
    exact_bytes.max_line_bytes = 2;
    assert_eq!(
        exact_bytes
            .layout_line(&snapshot("aa\nbc"), 1, key, wrap()?, &mut shaper)?
            .width(),
        16.0
    );
    let mut exact_glyphs = LineLayoutCache::new(budget);
    exact_glyphs.max_glyphs_per_line = 1;
    assert_eq!(
        exact_glyphs
            .layout_line(&snapshot("x"), 0, key, wrap()?, &mut shaper)?
            .glyphs()
            .len(),
        1
    );

    let entry = retained_entry(17)?;
    let different_font = FontKey::new(
        8,
        PositiveFinite::new(13.0).ok_or("size")?,
        PositiveFinite::new(2.0).ok_or("scale")?,
        NonZeroU32::new(4).ok_or("tabs")?,
    );
    assert!(!entry.matches(
        entry.fingerprint,
        &entry.snapshot,
        entry.range.clone(),
        different_font,
        entry.wrap_width_bits
    )?);
    assert!(!entry.matches(
        entry.fingerprint,
        &entry.snapshot,
        entry.range.clone(),
        entry.font,
        entry.wrap_width_bits + 1
    )?);

    let mut counters = LineLayoutCache::new(budget);
    let multi = snapshot("a\nb\nc");
    for line in 0..3 {
        counters.layout_line(&multi, line, key, wrap()?, &mut shaper)?;
    }
    for line in 0..2 {
        counters.layout_line(&multi, line, key, wrap()?, &mut shaper)?;
    }
    let counters_snapshot = counters.snapshot();
    assert!(counters_snapshot.current_bytes() > 1);
    assert_eq!(counters_snapshot.current_entries(), 3);
    assert_eq!(counters_snapshot.hits(), 2);
    counters.begin_frame()?;
    counters.begin_frame()?;
    assert_eq!(counters.snapshot().evictions(), 3);
    Ok(())
}

#[test]
fn cache_budget_atlas_geometry_and_error_evidence_are_complete() -> Result<(), Box<dyn Error>> {
    let entry_bytes = size_of::<CacheEntry>();
    let one_entry_budget = NonZeroUsize::new(entry_bytes).ok_or("entry budget")?;
    let mut previous_first = LineLayoutCache::new(one_entry_budget);
    previous_first.current.push(retained_entry(0)?);
    previous_first.previous.push(retained_entry(0)?);
    previous_first.enforce_budget()?;
    assert!(previous_first.previous.is_empty());

    let mut current_first = LineLayoutCache::new(one_entry_budget);
    current_first.current.push(retained_entry(0)?);
    current_first.current.push(retained_entry(0)?);
    current_first.enforce_budget()?;
    assert_eq!(current_first.current.len(), 1);

    let tiny_budget = NonZeroUsize::new(1).ok_or("tiny budget")?;
    let mut oversized = LineLayoutCache::new(tiny_budget);
    oversized.current.push(retained_entry(2)?);
    assert!(matches!(
        oversized.enforce_budget(),
        Err(LayoutError::LayoutExceedsBudget { .. })
    ));

    let one = NonZeroU32::new(1).ok_or("one")?;
    let two = NonZeroU32::new(2).ok_or("two")?;
    let three = NonZeroU32::new(3).ok_or("three")?;
    let four = NonZeroU32::new(4).ok_or("four")?;
    let horizontal = merge_rects(
        AtlasRect::new(0, 0, one, one),
        AtlasRect::new(1, 0, two, one),
    )
    .ok_or("horizontal")?;
    assert_eq!(horizontal.width(), three);
    assert_eq!(
        merge_rects(
            AtlasRect::new(1, 0, two, one),
            AtlasRect::new(0, 0, one, one)
        ),
        Some(horizontal)
    );
    assert_eq!(
        merge_rects(
            AtlasRect::new(0, 0, one, one),
            AtlasRect::new(3, 0, one, one)
        ),
        None
    );
    let vertical = merge_rects(
        AtlasRect::new(0, 0, one, one),
        AtlasRect::new(0, 1, one, two),
    )
    .ok_or("vertical")?;
    assert_eq!(vertical.height(), three);
    assert_eq!(
        merge_rects(
            AtlasRect::new(0, 1, one, two),
            AtlasRect::new(0, 0, one, one)
        ),
        Some(vertical)
    );
    assert_eq!(horizontal.x(), 0);
    assert_eq!(horizontal.y(), 0);
    assert_eq!(horizontal.right(), 3);
    assert_eq!(vertical.bottom(), 3);
    assert_eq!(
        merge_rects(
            AtlasRect::new(0, 0, one, one),
            AtlasRect::new(1, 0, one, two),
        ),
        None
    );
    assert_eq!(
        merge_rects(
            AtlasRect::new(0, 0, one, one),
            AtlasRect::new(0, 1, two, one),
        ),
        None
    );

    let offset_rect = AtlasRect::new(3, 5, two, three);
    assert_eq!(offset_rect.x(), 3);
    assert_eq!(offset_rect.y(), 5);
    assert_eq!(offset_rect.right(), 5);
    assert_eq!(offset_rect.bottom(), 8);

    let growth_budget = NonZeroUsize::new(512 * 512 + 16_384).ok_or("growth budget")?;
    let mut atlas = GlyphAtlas::new(growth_budget);
    let pixel = GlyphBitmap::new(one, one, vec![1])?;
    atlas.insert(GlyphKey::new(font()?, 1, 0), &pixel)?;
    let wide = NonZeroU32::new(257).ok_or("wide")?;
    let wide_bitmap = GlyphBitmap::new(wide, one, vec![2; 257])?;
    let wide_rect = atlas.insert(GlyphKey::new(font()?, 2, 0), &wide_bitmap)?;
    assert_eq!(wide_rect.width(), wide);
    let atlas_snapshot = atlas.snapshot();
    assert_eq!(atlas_snapshot.dimension(), 512);
    assert_eq!(atlas_snapshot.budget_bytes(), growth_budget.get());
    assert!(atlas_snapshot.pixel_bytes() > 1);
    assert!(atlas_snapshot.metadata_bytes() > 1);
    assert!(atlas_snapshot.peak_bytes() > 1);
    assert_eq!(atlas_snapshot.entries(), 2);
    atlas.insert(GlyphKey::new(font()?, 1, 0), &pixel)?;
    atlas.insert(GlyphKey::new(font()?, 1, 0), &pixel)?;
    let reused = atlas.snapshot();
    assert_eq!(reused.hits(), 2);
    assert_eq!(reused.misses(), 2);

    let rasterized = RasterizedGlyph::new(Some(pixel.clone()), -1.0, 2.0)?;
    let raster_key = GlyphKey::new(font()?, 50, 0);
    let mut raster_reuse = GlyphAtlas::new(growth_budget);
    let admitted = raster_reuse.insert_rasterized(raster_key, &rasterized)?;
    assert_eq!(
        raster_reuse.insert_rasterized(raster_key, &rasterized)?,
        admitted
    );

    let empty = RasterizedGlyph::new(None, 0.0, 0.0)?;
    let empty_key = GlyphKey::new(font()?, 51, 0);
    raster_reuse.insert_rasterized(empty_key, &empty)?;
    assert!(raster_reuse.lookup(empty_key)?.is_some());
    assert!(raster_reuse.insert(empty_key, &pixel).is_ok());

    let mut pressure_counts = atlas;
    pressure_counts.pressure()?;
    pressure_counts.pressure()?;
    let pressure_snapshot = pressure_counts.snapshot();
    assert_eq!(pressure_snapshot.evictions(), 2);
    assert_eq!(pressure_snapshot.pressure_events(), 2);

    let mut over_budget = GlyphAtlas::new(tiny_budget);
    let two_pixels = GlyphBitmap::new(two, one, vec![1, 2])?;
    assert!(matches!(
        over_budget.insert(GlyphKey::new(font()?, 3, 0), &two_pixels),
        Err(LayoutError::GlyphExceedsAtlasBudget { .. })
    ));
    let oversized_raster = RasterizedGlyph::new(Some(two_pixels.clone()), 0.0, 0.0)?;
    assert!(matches!(
        over_budget.insert_rasterized(GlyphKey::new(font()?, 52, 0), &oversized_raster),
        Err(LayoutError::GlyphExceedsAtlasBudget { .. })
    ));
    let mut empty_saturated = GlyphAtlas::new(tiny_budget);
    assert_eq!(
        empty_saturated.insert_rasterized(GlyphKey::new(font()?, 53, 0), &empty),
        Err(LayoutError::AtlasSaturated)
    );
    let mut empty_post_reservation_saturated = GlyphAtlas::new(tiny_budget);
    empty_post_reservation_saturated.index_slots = vec![None; MIN_ATLAS_INDEX_SLOTS];
    assert_eq!(
        empty_post_reservation_saturated
            .insert_empty_miss(GlyphKey::new(font()?, 57, 0), 0.0, 0.0,),
        Err(LayoutError::AtlasSaturated)
    );
    let exact_raster = RasterizedGlyph::new(Some(pixel.clone()), 0.0, 0.0)?;
    let mut exact_raster_budget = GlyphAtlas::new(tiny_budget);
    assert_eq!(
        exact_raster_budget.insert_rasterized(GlyphKey::new(font()?, 56, 0), &exact_raster),
        Err(LayoutError::AtlasSaturated)
    );
    let mut bitmap_post_reservation_saturated = GlyphAtlas::new(tiny_budget);
    bitmap_post_reservation_saturated.index_slots = vec![None; MIN_ATLAS_INDEX_SLOTS];
    assert_eq!(
        bitmap_post_reservation_saturated.insert_miss(
            GlyphKey::new(font()?, 58, 0),
            &pixel,
            0.0,
            0.0,
            1,
        ),
        Err(LayoutError::AtlasSaturated)
    );
    let mut metadata_saturated = GlyphAtlas::new(tiny_budget);
    assert_eq!(
        metadata_saturated.insert(GlyphKey::new(font()?, 30, 0), &pixel),
        Err(LayoutError::AtlasSaturated)
    );
    let modest_budget = NonZeroUsize::new(1024).ok_or("modest budget")?;
    let mut no_region = GlyphAtlas::new(modest_budget);
    assert_eq!(
        no_region.insert(GlyphKey::new(font()?, 4, 0), &pixel),
        Err(LayoutError::AtlasSaturated)
    );
    assert_eq!(
        GlyphAtlas::new(growth_budget).insert_miss(
            GlyphKey::new(font()?, 40, 0),
            &pixel,
            0.0,
            0.0,
            0,
        ),
        Err(LayoutError::AtlasSaturated)
    );
    assert_eq!(GlyphAtlas::new(modest_budget).evict_oldest(), Ok(false));
    assert_eq!(
        GlyphAtlas::new(modest_budget).grow(u32::MAX),
        Err(LayoutError::ArithmeticOverflow)
    );
    let mut invalid_peak = GlyphAtlas::new(tiny_budget);
    invalid_peak.pixels = vec![0; 2];
    assert_eq!(invalid_peak.update_peak(), Err(LayoutError::AtlasSaturated));
    let mut equal_peak = GlyphAtlas::new(tiny_budget);
    equal_peak.pixels = vec![0];
    assert_eq!(equal_peak.update_peak(), Ok(()));

    let invalid_rect = AtlasRect::new(1, 0, one, one);
    let mut invalid_pixels = GlyphAtlas::new(growth_budget);
    invalid_pixels.dimension = 1;
    invalid_pixels.pixels = vec![0];
    let invalid_revision = invalid_pixels.pixel_revision;
    assert_eq!(
        invalid_pixels.copy_bitmap(invalid_rect, &pixel),
        Err(LayoutError::ArithmeticOverflow)
    );
    assert_eq!(
        invalid_pixels.clear_rect(invalid_rect),
        Err(LayoutError::ArithmeticOverflow)
    );
    assert_eq!(invalid_pixels.pixel_revision, invalid_revision);

    let valid_rect = AtlasRect::new(0, 0, one, one);
    let mut exact_pixels = GlyphAtlas::new(growth_budget);
    exact_pixels.dimension = 1;
    exact_pixels.pixels = vec![0];
    assert_eq!(exact_pixels.copy_bitmap(valid_rect, &pixel), Ok(()));
    assert_eq!(exact_pixels.pixels(), &[1]);
    assert_eq!(exact_pixels.pixel_revision, 1);
    assert_eq!(exact_pixels.clear_rect(valid_rect), Ok(()));
    assert_eq!(exact_pixels.pixels(), &[0]);
    assert_eq!(exact_pixels.pixel_revision, 2);

    let mut positive_eviction = GlyphAtlas::new(growth_budget);
    positive_eviction.insert(GlyphKey::new(font()?, 54, 0), &pixel)?;
    assert_eq!(positive_eviction.evict_oldest(), Ok(true));
    assert!(positive_eviction.entries.is_empty());
    assert!(positive_eviction.pixels().iter().all(|pixel| *pixel == 0));

    let mut empty_eviction = GlyphAtlas::new(growth_budget);
    empty_eviction.insert_rasterized(GlyphKey::new(font()?, 55, 0), &empty)?;
    assert_eq!(empty_eviction.evict_oldest(), Ok(true));
    assert!(empty_eviction.entries.is_empty());
    assert!(empty_eviction.free.is_empty());

    let exact_bitmap_budget = NonZeroUsize::new(2).ok_or("exact bitmap budget")?;
    let mut exact_bitmap = GlyphAtlas::new(exact_bitmap_budget);
    assert_eq!(
        exact_bitmap.insert(GlyphKey::new(font()?, 31, 0), &two_pixels),
        Err(LayoutError::AtlasSaturated)
    );

    let mut geometry = GlyphAtlas::new(growth_budget);
    let eight = NonZeroU32::new(8).ok_or("eight")?;
    geometry.free.push(AtlasRect::new(0, 0, eight, eight));
    geometry
        .free
        .push(AtlasRect::new(10, 10, six_nonzero()?, six_nonzero()?));
    let allocated = geometry.allocate_rect(two, two).ok_or("allocated")?;
    assert_eq!(allocated, AtlasRect::new(10, 10, two, two));
    assert!(geometry.free.contains(&AtlasRect::new(12, 10, four, two)));
    assert!(
        geometry
            .free
            .contains(&AtlasRect::new(10, 12, six_nonzero()?, four))
    );
    let ten = NonZeroU32::new(10).ok_or("ten")?;
    let five = NonZeroU32::new(5).ok_or("five")?;
    let mut area_not_perimeter = GlyphAtlas::new(growth_budget);
    area_not_perimeter
        .free
        .push(AtlasRect::new(20, 0, two, ten));
    area_not_perimeter
        .free
        .push(AtlasRect::new(30, 0, five, five));
    assert_eq!(
        area_not_perimeter.allocate_rect(one, one),
        Some(AtlasRect::new(20, 0, one, one))
    );

    let mut exact_growth = GlyphAtlas::new(growth_budget);
    assert_eq!(exact_growth.grow(256), Ok(true));
    assert_eq!(exact_growth.dimension, 256);
    exact_growth.pixels[7] = 99;
    assert_eq!(exact_growth.grow(257), Ok(true));
    assert_eq!(exact_growth.dimension, 512);
    assert_eq!(exact_growth.pixels[7], 99);
    let dimension_256 = NonZeroU32::new(256).ok_or("256")?;
    let dimension_512 = NonZeroU32::new(512).ok_or("512")?;
    assert!(
        exact_growth
            .free
            .contains(&AtlasRect::new(256, 0, dimension_256, dimension_256,))
    );
    assert!(
        exact_growth
            .free
            .contains(&AtlasRect::new(0, 256, dimension_512, dimension_256,))
    );

    let mut clearing = GlyphAtlas::new(growth_budget);
    clearing.dimension = 4;
    clearing.pixels = vec![7; 16];
    clearing.clear_rect(AtlasRect::new(1, 1, two, two))?;
    assert_eq!(
        clearing.pixels,
        vec![7, 7, 7, 7, 7, 0, 0, 7, 7, 0, 0, 7, 7, 7, 7, 7]
    );

    let mut coalescing = GlyphAtlas::new(growth_budget);
    coalescing.free = vec![
        AtlasRect::new(0, 0, one, one),
        AtlasRect::new(1, 0, one, one),
        AtlasRect::new(2, 0, one, one),
    ];
    coalescing.coalesce_free();
    assert_eq!(coalescing.free, vec![AtlasRect::new(0, 0, three, one)]);

    let index_font = font()?;
    let full_index = (0..MIN_ATLAS_INDEX_SLOTS)
        .map(|index| {
            u32::try_from(index).map(|glyph_id| {
                Some(AtlasIndexSlot {
                    key: GlyphKey::new(index_font, glyph_id, 0),
                    entry_index: index,
                })
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let absent_key = GlyphKey::new(font()?, 99, 0);
    assert_eq!(atlas_index_slot(&full_index, absent_key), None);
    assert_eq!(atlas_index_insertion_slot(&full_index, absent_key), None);

    let mut empty_index = vec![None; MIN_ATLAS_INDEX_SLOTS];
    assert_eq!(
        remove_atlas_index_slot(&mut empty_index, 0),
        Err(LayoutError::ArithmeticOverflow)
    );
    let mut malformed_full_index = full_index;
    assert_eq!(
        remove_atlas_index_slot(&mut malformed_full_index, 0),
        Err(LayoutError::ArithmeticOverflow)
    );

    let text_error = match snapshot("").line_byte_range(9) {
        Ok(_) => return Err("expected text error".into()),
        Err(error) => error,
    };
    let wrapped = LayoutError::from(text_error);
    assert!(wrapped.source().is_some());
    assert!(!wrapped.to_string().is_empty());
    let errors = [
        LayoutError::EmptyDocument,
        LayoutError::InvalidScroll,
        LayoutError::ArithmeticOverflow,
        LayoutError::LineByteLimitExceeded {
            line: 1,
            bytes: 2,
            limit: 1,
        },
        LayoutError::GlyphLimitExceeded {
            glyphs: 2,
            limit: 1,
        },
        LayoutError::LayoutExceedsBudget {
            bytes: 2,
            budget: 1,
        },
        LayoutError::InvalidShaperOutput,
        LayoutError::InvalidGlyphBitmap {
            expected: 2,
            actual: 1,
        },
        LayoutError::GlyphExceedsAtlasBudget {
            bytes: 2,
            budget: 1,
        },
        LayoutError::AtlasSaturated,
        LayoutError::AllocationFailed,
        LayoutError::SequenceExhausted,
        LayoutError::UnsupportedPlatform,
        LayoutError::NativeFailure("fixture"),
    ];
    for error in errors {
        assert!(error.source().is_none());
        assert!(!error.to_string().is_empty());
    }
    Ok(())
}

fn six_nonzero() -> Result<NonZeroU32, Box<dyn Error>> {
    NonZeroU32::new(6).ok_or_else(|| "six".into())
}

#[test]
fn sequence_and_shaper_failures_are_atomic() -> Result<(), Box<dyn Error>> {
    let budget = NonZeroUsize::new(4096).ok_or("budget")?;
    let text = snapshot("x");
    let key = font()?;

    let mut failing = LineLayoutCache::new(budget);
    assert_eq!(
        failing.layout_line(&text, 0, key, wrap()?, &mut FailingShaper),
        Err(LayoutError::NativeFailure("injected shaper"))
    );
    assert!(failing.current.is_empty());

    let mut shaper = FixtureShaper::new();
    let mut begin_exhausted = LineLayoutCache::new(budget);
    begin_exhausted.previous.push(retained_entry(0)?);
    begin_exhausted.evictions = u64::MAX;
    assert_eq!(
        begin_exhausted.begin_frame(),
        Err(LayoutError::SequenceExhausted)
    );

    let mut current_hit = LineLayoutCache::new(budget);
    current_hit.layout_line(&text, 0, key, wrap()?, &mut shaper)?;
    current_hit.hits = u64::MAX;
    assert_eq!(
        current_hit.layout_line(&text, 0, key, wrap()?, &mut shaper),
        Err(LayoutError::SequenceExhausted)
    );

    let mut previous_hit = LineLayoutCache::new(budget);
    previous_hit.layout_line(&text, 0, key, wrap()?, &mut shaper)?;
    previous_hit.begin_frame()?;
    previous_hit.hits = u64::MAX;
    assert_eq!(
        previous_hit.layout_line(&text, 0, key, wrap()?, &mut shaper),
        Err(LayoutError::SequenceExhausted)
    );

    let mut miss_exhausted = LineLayoutCache::new(budget);
    miss_exhausted.misses = u64::MAX;
    assert_eq!(
        miss_exhausted.layout_line(&text, 0, key, wrap()?, &mut shaper),
        Err(LayoutError::SequenceExhausted)
    );

    let mut shaped_exhausted = LineLayoutCache::new(budget);
    shaped_exhausted.shaped_lines = u64::MAX;
    assert_eq!(
        shaped_exhausted.layout_line(&text, 0, key, wrap()?, &mut shaper),
        Err(LayoutError::SequenceExhausted)
    );
    assert!(shaped_exhausted.current.is_empty());

    let mut values = Vec::<u8>::new();
    assert_eq!(
        reserve_bytes_exact(&mut values, usize::MAX),
        Err(LayoutError::AllocationFailed)
    );
    let mut cache_entries = Vec::<CacheEntry>::new();
    assert_eq!(
        reserve_cache_entries(&mut cache_entries, usize::MAX),
        Err(LayoutError::AllocationFailed)
    );
    let mut atlas_entries = Vec::<AtlasEntry>::new();
    assert_eq!(
        reserve_atlas_entries(&mut atlas_entries, usize::MAX),
        Err(LayoutError::AllocationFailed)
    );
    let mut atlas_rects = Vec::<AtlasRect>::new();
    assert_eq!(
        reserve_atlas_rects(&mut atlas_rects, usize::MAX),
        Err(LayoutError::AllocationFailed)
    );
    Ok(())
}

#[test]
fn atlas_sequence_failures_preserve_owned_storage() -> Result<(), Box<dyn Error>> {
    let budget = NonZeroUsize::new(1024 * 1024).ok_or("budget")?;
    let one = NonZeroU32::new(1).ok_or("one")?;
    let bitmap = GlyphBitmap::new(one, one, vec![255])?;
    let first = GlyphKey::new(font()?, 1, 0);
    let second = GlyphKey::new(font()?, 2, 0);

    let mut tick = GlyphAtlas::new(budget);
    tick.tick = u64::MAX;
    assert_eq!(
        tick.insert(first, &bitmap),
        Err(LayoutError::SequenceExhausted)
    );
    assert!(tick.entries.is_empty());

    let mut misses = GlyphAtlas::new(budget);
    misses.misses = u64::MAX;
    assert_eq!(
        misses.insert(first, &bitmap),
        Err(LayoutError::SequenceExhausted)
    );
    assert!(misses.entries.is_empty());

    let mut hits = GlyphAtlas::new(budget);
    let retained = hits.insert(first, &bitmap)?;
    hits.hits = u64::MAX;
    assert_eq!(
        hits.insert(first, &bitmap),
        Err(LayoutError::SequenceExhausted)
    );
    assert_eq!(hits.entries[0].glyph.rect(), Some(retained));

    let mut pressure = GlyphAtlas::new(budget);
    pressure.pressure_events = u64::MAX;
    assert_eq!(pressure.pressure(), Err(LayoutError::SequenceExhausted));

    let mut pixel_revision = GlyphAtlas::new(budget);
    pixel_revision.insert(first, &bitmap)?;
    pixel_revision.pixel_revision = u64::MAX;
    let retained_snapshot = pixel_revision.snapshot();
    let retained_pixels = pixel_revision.pixels().to_vec();
    assert_eq!(
        pixel_revision.pressure(),
        Err(LayoutError::SequenceExhausted)
    );
    assert_eq!(pixel_revision.snapshot(), retained_snapshot);
    assert_eq!(pixel_revision.pixels(), retained_pixels);

    let mut pressure_evictions = GlyphAtlas::new(budget);
    pressure_evictions.insert(first, &bitmap)?;
    pressure_evictions.evictions = u64::MAX;
    assert_eq!(
        pressure_evictions.pressure(),
        Err(LayoutError::SequenceExhausted)
    );
    assert_eq!(pressure_evictions.entries.len(), 1);

    let mut eviction = GlyphAtlas::new(budget);
    eviction.insert(second, &bitmap)?;
    eviction.evictions = u64::MAX;
    assert_eq!(eviction.evict_oldest(), Err(LayoutError::SequenceExhausted));
    assert!(eviction.entries.is_empty());
    Ok(())
}
