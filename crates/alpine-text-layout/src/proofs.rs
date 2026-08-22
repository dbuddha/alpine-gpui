use std::num::NonZeroU32;

use crate::{AtlasRect, DirtyAtlasRows, DirtyRowRange, atlas_probe_slot, merge_rects};

/// AEP-0141-C03, EV-0141-KANI03.
#[kani::proof]
fn adjacent_horizontal_rectangles_merge_without_loss() {
    let first_width = u32::from(kani::any::<u8>()).saturating_add(1);
    let second_width = u32::from(kani::any::<u8>()).saturating_add(1);
    let height = u32::from(kani::any::<u8>()).saturating_add(1);
    let Some(first_width) = NonZeroU32::new(first_width) else {
        return;
    };
    let Some(second_width) = NonZeroU32::new(second_width) else {
        return;
    };
    let Some(height) = NonZeroU32::new(height) else {
        return;
    };
    let second_x = first_width.get();
    let first = AtlasRect::new(0, 0, first_width, height);
    let second = AtlasRect::new(second_x, 0, second_width, height);
    let Some(merged) = merge_rects(first, second) else {
        assert!(false, "adjacent rectangles must merge");
        return;
    };

    assert_eq!(merged.x(), 0);
    assert_eq!(merged.y(), 0);
    assert_eq!(merged.width().get(), first_width.get() + second_width.get());
    assert_eq!(merged.height(), height);
    kani::cover!(first_width.get() == 1 && second_width.get() == 1);
    kani::cover!(first_width.get() == 256 && second_width.get() == 256);
}

/// AEP-0141-C07, EV-0141-KANI07.
#[kani::proof]
fn power_of_two_probe_never_escapes_the_index() {
    let exponent = kani::any::<u8>();
    kani::assume(exponent <= 7);
    let slot_count = 1_usize << exponent;
    let start = usize::from(kani::any::<u8>()) % slot_count;
    let probe = usize::from(kani::any::<u8>()) % slot_count;
    let slot = atlas_probe_slot(start, probe, slot_count);

    assert!(slot < slot_count);
    kani::cover!(slot_count == 1);
    kani::cover!(slot_count == 128 && start == 127 && probe == 127);
}

/// AEP-0141-C08, EV-0141-KANI08A.
#[kani::unwind(8)]
#[kani::proof]
fn dirty_row_small_actions_preserve_sorted_disjoint_storage() {
    const PROOF_CAPACITY: usize = 3;
    let mut dirty = DirtyAtlasRows::<PROOF_CAPACITY>::new();
    for _ in 0..PROOF_CAPACITY {
        let start = u32::from(kani::any::<u8>() & 7);
        let count = u32::from((kani::any::<u8>() & 3) + 1);
        dirty.insert(1, start, start + count);
        assert!(dirty.len <= PROOF_CAPACITY);
        for pair in dirty.ranges[..dirty.len].windows(2) {
            assert!(pair[0].start < pair[0].end);
            assert!(pair[0].end < pair[1].start);
        }
        if let Some(last) = dirty.ranges[..dirty.len].last() {
            assert!(last.start < last.end);
        }
    }
    kani::cover!(dirty.len > 1);
}

/// AEP-0141-C08, EV-0141-KANI08B.
#[kani::unwind(8)]
#[kani::proof]
fn dirty_row_capacity_merge_remains_bounded() {
    const PROOF_CAPACITY: usize = 3;
    let mut full = DirtyAtlasRows::<PROOF_CAPACITY>::new();
    full.ranges = [
        DirtyRowRange { start: 0, end: 1 },
        DirtyRowRange { start: 4, end: 5 },
        DirtyRowRange { start: 8, end: 9 },
    ];
    full.len = PROOF_CAPACITY;
    let start = u32::from(kani::any::<u8>() & 15);
    let count = u32::from((kani::any::<u8>() & 3) + 1);
    full.insert(1, start, start + count);
    assert!(full.len <= PROOF_CAPACITY);
    for pair in full.ranges[..full.len].windows(2) {
        assert!(pair[0].end < pair[1].start);
    }
    kani::cover!(full.len == PROOF_CAPACITY);
    kani::cover!(full.len < PROOF_CAPACITY);
}
