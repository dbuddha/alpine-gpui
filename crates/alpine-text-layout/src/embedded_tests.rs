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
    let budget = NonZeroUsize::new(256 * 256 + 32 * 1024).ok_or("budget")?;
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
fn atlas_lookup_retains_raster_bearings_and_empty_outcomes() -> Result<(), Box<dyn Error>> {
    let budget = NonZeroUsize::new(256 * 256 + 4096).ok_or("budget")?;
    let mut atlas = GlyphAtlas::new(budget);
    let two = NonZeroU32::new(2).ok_or("width")?;
    let bitmap = GlyphBitmap::new(two, two, vec![255; 4])?;
    let visible_key = GlyphKey::new(font()?, 10, 0);
    assert_eq!(atlas.lookup(visible_key)?, None);
    assert_eq!(atlas.snapshot().misses(), 0);

    let rasterized = RasterizedGlyph::new(Some(bitmap), -1.5, 3.25)?;
    let visible = atlas.insert_rasterized(visible_key, &rasterized)?;
    assert!(visible.rect().is_some());
    assert_eq!(visible.left(), -1.5);
    assert_eq!(visible.top(), 3.25);
    let visible_revision = atlas.snapshot().pixel_revision();
    assert_eq!(visible_revision, 2);
    assert_eq!(atlas.lookup(visible_key)?, Some(visible));
    assert_eq!(atlas.snapshot().pixel_revision(), visible_revision);

    let empty_key = GlyphKey::new(font()?, 11, 0);
    let empty = RasterizedGlyph::new(None, 0.5, 1.25)?;
    let retained_empty = atlas.insert_rasterized(empty_key, &empty)?;
    assert_eq!(retained_empty.rect(), None);
    assert_eq!(retained_empty.left(), 0.5);
    assert_eq!(retained_empty.top(), 1.25);
    assert_eq!(atlas.snapshot().pixel_revision(), visible_revision);
    assert_eq!(atlas.lookup(empty_key)?, Some(retained_empty));
    assert_eq!(atlas.snapshot().misses(), 2);
    assert_eq!(atlas.snapshot().hits(), 2);
    Ok(())
}

#[test]
fn atlas_index_matches_independent_membership_and_lru_model() -> Result<(), Box<dyn Error>> {
    #[derive(Clone, Copy)]
    struct ReferenceEntry {
        key: GlyphKey,
        last_used: u64,
    }

    const fn reference_mix(state: u64, value: u64) -> u64 {
        let mut mixed = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
        mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        state ^ mixed ^ (mixed >> 31)
    }

    fn reference_hash(key: GlyphKey) -> u64 {
        let mut state = reference_mix(0x517c_c1b7_2722_0a95, key.font.family);
        state = reference_mix(state, u64::from(key.font.size_bits));
        state = reference_mix(state, u64::from(key.font.scale_bits));
        state = reference_mix(state, u64::from(key.font.tab_columns.get()));
        state = reference_mix(state, u64::from(key.glyph_id));
        reference_mix(state, u64::from(key.subpixel_x))
    }

    let budget = NonZeroUsize::new(512 * 512 + 64 * 1024).ok_or("budget")?;
    let one = NonZeroU32::new(1).ok_or("one")?;
    let bitmap = GlyphBitmap::new(one, one, vec![255])?;
    let font = font()?;
    let keys = (0..64)
        .map(|glyph_id| GlyphKey::new(font, glyph_id, 0))
        .collect::<Vec<_>>();

    let contract_key = (1..=u8::MAX)
        .map(|subpixel| GlyphKey::new(font, 0x5a5a_a5a5, subpixel))
        .find(|key| reference_hash(*key) & 7 > 1)
        .ok_or("hash contract key")?;
    assert_eq!(glyph_key_hash(contract_key), reference_hash(contract_key));
    assert_eq!(atlas_index_start(contract_key, 0), None);
    assert_eq!(atlas_index_start(contract_key, 3), None);
    assert_eq!(
        atlas_index_start(contract_key, MIN_ATLAS_INDEX_SLOTS),
        usize::try_from(reference_hash(contract_key) & 7).ok()
    );

    let mut repeated_slots = vec![None; MIN_ATLAS_INDEX_SLOTS];
    let repeated_slot = atlas_index_insertion_slot(&repeated_slots, contract_key)
        .ok_or("initial insertion slot")?;
    repeated_slots[repeated_slot] = Some(AtlasIndexSlot {
        key: contract_key,
        entry_index: 7,
    });
    assert_eq!(
        atlas_index_insertion_slot(&repeated_slots, contract_key),
        Some(repeated_slot)
    );

    let mut malformed_slots = vec![
        Some(AtlasIndexSlot {
            key: contract_key,
            entry_index: 0,
        }),
        None,
        None,
    ];
    assert_eq!(
        remove_atlas_index_slot(&mut malformed_slots, 0),
        Err(LayoutError::ArithmeticOverflow)
    );
    assert!(malformed_slots[0].is_some());

    let mut failed_reservation = Vec::<Option<AtlasIndexSlot>>::new();
    assert_eq!(
        reserve_atlas_index_slots(&mut failed_reservation, usize::MAX),
        Err(LayoutError::AllocationFailed)
    );

    let mut collision = None;
    for left in 0..keys.len() {
        for right in (left + 1)..keys.len() {
            if atlas_index_start(keys[left], MIN_ATLAS_INDEX_SLOTS)
                == atlas_index_start(keys[right], MIN_ATLAS_INDEX_SLOTS)
            {
                collision = Some((keys[left], keys[right]));
                break;
            }
        }
        if collision.is_some() {
            break;
        }
    }
    let (first_collision, second_collision) = collision.ok_or("collision fixture")?;
    let mut collision_atlas = GlyphAtlas::new(budget);
    collision_atlas.insert(first_collision, &bitmap)?;
    collision_atlas.insert(second_collision, &bitmap)?;
    assert!(collision_atlas.index_lookup(first_collision).is_some());
    assert!(collision_atlas.index_lookup(second_collision).is_some());

    let mut atlas = GlyphAtlas::new(budget);
    let mut reference = Vec::<ReferenceEntry>::new();
    let mut reference_tick = 0_u64;
    let mut random = 0x6a09_e667_f3bc_c909_u64;
    for _ in 0..2_048 {
        random = random
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let key = keys[usize::try_from(random % keys.len() as u64)?];
        match (random >> 32) % 7 {
            0..=2 => {
                atlas.insert(key, &bitmap)?;
                reference_tick = reference_tick.checked_add(1).ok_or("reference tick")?;
                if let Some(entry) = reference.iter_mut().find(|entry| entry.key == key) {
                    entry.last_used = reference_tick;
                } else {
                    reference.push(ReferenceEntry {
                        key,
                        last_used: reference_tick,
                    });
                }
            }
            3 | 4 => {
                let expected = reference.iter_mut().find(|entry| entry.key == key);
                let actual = atlas.lookup(key)?;
                assert_eq!(actual.is_some(), expected.is_some());
                if let Some(entry) = expected {
                    reference_tick = reference_tick.checked_add(1).ok_or("reference tick")?;
                    entry.last_used = reference_tick;
                }
            }
            5 => {
                let expected = reference
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, entry)| entry.last_used)
                    .map(|(index, entry)| (index, entry.key));
                assert_eq!(atlas.evict_oldest()?, expected.is_some());
                if let Some((index, key)) = expected {
                    assert_eq!(reference.swap_remove(index).key, key);
                }
            }
            6 => {
                atlas.pressure()?;
                reference.clear();
            }
            _ => return Err("operation escaped bounded selector".into()),
        }

        assert_eq!(atlas.tick, reference_tick);
        assert_eq!(atlas.entries.len(), reference.len());
        assert!(
            atlas.index_slots.is_empty()
                || atlas.entries.len().saturating_mul(2) <= atlas.index_slots.len()
        );
        for key in &keys {
            let expected = reference.iter().find(|entry| entry.key == *key);
            let actual = atlas
                .index_lookup(*key)
                .and_then(|index| atlas.entries.get(index));
            assert_eq!(actual.is_some(), expected.is_some());
            if let (Some(actual), Some(expected)) = (actual, expected) {
                assert_eq!(actual.key, expected.key);
                assert_eq!(actual.last_used, expected.last_used);
            }
        }
        let expected_metadata = atlas
            .entries
            .capacity()
            .saturating_mul(size_of::<AtlasEntry>())
            .saturating_add(atlas.free.capacity().saturating_mul(size_of::<AtlasRect>()))
            .saturating_add(
                atlas
                    .index_slots
                    .capacity()
                    .saturating_mul(size_of::<Option<AtlasIndexSlot>>()),
            );
        assert_eq!(atlas.snapshot().metadata_bytes(), expected_metadata);
        assert!(atlas.current_bytes() <= budget.get());
    }
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
