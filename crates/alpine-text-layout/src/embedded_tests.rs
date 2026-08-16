//! Legacy unit tests kept outside production source reporting.

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
    let viewport = PositiveFinite::new(60.0).ok_or(LayoutError::InvalidScroll)?;
    let line_height = PositiveFinite::new(20.0).ok_or(LayoutError::InvalidScroll)?;
    let lines = VisibleLines::new(100, 40.0, viewport, line_height, 2)?;
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

    let matches = entry.matches(
        entry.fingerprint,
        &different,
        range,
        entry.font,
        entry.wrap_width_bits,
    );
    assert_eq!(matches, Ok(false));
    Ok(())
}

#[test]
fn atlas_allocates_on_demand_reuses_evicts_and_drains() -> Result<(), Box<dyn Error>> {
    let budget = NonZeroUsize::new(256 * 256 + 4096).ok_or("budget")?;
    let mut atlas = GlyphAtlas::new(budget);
    assert_eq!(atlas.snapshot().pixel_bytes(), 0);
    let eight = NonZeroU32::new(8).ok_or("width")?;
    let bitmap = GlyphBitmap::new(eight, eight, vec![255; 64])?;
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

    let thirty_two = NonZeroU32::new(32).ok_or("width")?;
    let large = GlyphBitmap::new(thirty_two, thirty_two, vec![127; 32 * 32])?;
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
