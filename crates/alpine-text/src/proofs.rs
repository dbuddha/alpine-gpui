use crate::{Edit, transform_offset};

/// AEP-0139-C01, EV-0139-KANI01.
#[kani::proof]
fn selection_transform_stays_at_or_after_replacement_start() {
    let start = usize::from(kani::any::<u8>());
    let removed = usize::from(kani::any::<u8>());
    let inserted = usize::from(kani::any::<u8>() % 4);
    let Some(end) = start.checked_add(removed) else {
        return;
    };
    let offset = usize::from(kani::any::<u8>());
    let edit = Edit {
        range: start..end,
        replacement: ["", "x", "xx", "xxx"][inserted].to_owned(),
    };
    let transformed = transform_offset(offset, &[edit]);
    if offset == start && removed > 0 {
        assert_eq!(transformed, start);
    } else if offset >= start && offset <= end {
        assert_eq!(transformed, start + inserted);
    } else if offset < start {
        assert_eq!(transformed, offset);
    }
}
