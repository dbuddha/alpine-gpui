use std::num::NonZeroU32;

use crate::{AtlasRect, merge_rects};

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
