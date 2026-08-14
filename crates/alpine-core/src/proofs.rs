//! Bounded Kani proofs for Alpine core value contracts.

use crate::{LinearRgba, Point, Rect, Size};

/// AEP-0016-C01, EV-0016-KANI01.
#[kani::proof]
fn bounded_sizes_preserve_constructor_contract() {
    let width = f32::from(kani::any::<u16>());
    let height = f32::from(kani::any::<u16>());
    let Some(size) = Size::new(width, height) else {
        unreachable!("u16 values are finite and non-negative");
    };

    kani::cover!(width == 0.0 && height == 0.0, "empty boundary is reachable");
    kani::cover!(
        width > 0.0 && height > 0.0,
        "non-empty values are reachable"
    );
    assert_eq!(size.width(), width);
    assert_eq!(size.height(), height);
    assert_eq!(size.is_empty(), width == 0.0 || height == 0.0);
}

/// AEP-0016-C02, EV-0016-KANI02.
#[kani::proof]
fn byte_colors_always_normalize_to_valid_channels() {
    let red = f32::from(kani::any::<u8>()) / 255.0;
    let green = f32::from(kani::any::<u8>()) / 255.0;
    let blue = f32::from(kani::any::<u8>()) / 255.0;
    let alpha = f32::from(kani::any::<u8>()) / 255.0;

    kani::cover!(red == 0.0 && alpha == 0.0, "zero endpoints are reachable");
    kani::cover!(red == 1.0 && alpha == 1.0, "one endpoints are reachable");
    assert!(LinearRgba::new(red, green, blue, alpha).is_some());
}

/// AEP-0016-C03, EV-0016-KANI03.
#[kani::proof]
fn bounded_intersections_remain_inside_both_inputs() {
    let first_x = f32::from(kani::any::<u8>());
    let first_y = f32::from(kani::any::<u8>());
    let second_x = f32::from(kani::any::<u8>());
    let second_y = f32::from(kani::any::<u8>());
    let first_width = f32::from(kani::any::<u8>());
    let first_height = f32::from(kani::any::<u8>());
    let second_width = f32::from(kani::any::<u8>());
    let second_height = f32::from(kani::any::<u8>());

    let Some(first_origin) = Point::new(first_x, first_y) else {
        unreachable!("u8 coordinates are finite");
    };
    let Some(first_size) = Size::new(first_width, first_height) else {
        unreachable!("u8 extents are finite and non-negative");
    };
    let Some(second_origin) = Point::new(second_x, second_y) else {
        unreachable!("u8 coordinates are finite");
    };
    let Some(second_size) = Size::new(second_width, second_height) else {
        unreachable!("u8 extents are finite and non-negative");
    };
    let first = Rect::new(first_origin, first_size);
    let second = Rect::new(second_origin, second_size);
    let intersection = first.intersection(second);

    kani::cover!(intersection.is_some(), "overlap is reachable");
    kani::cover!(intersection.is_none(), "empty intersection is reachable");
    if let Some(intersection) = intersection {
        assert!(intersection.size().width() > 0.0);
        assert!(intersection.size().height() > 0.0);
        assert!(intersection.origin().x() >= first.origin().x());
        assert!(intersection.origin().x() >= second.origin().x());
        assert!(intersection.origin().y() >= first.origin().y());
        assert!(intersection.origin().y() >= second.origin().y());
        assert!(
            intersection.origin().x() + intersection.size().width()
                <= first.origin().x() + first.size().width()
        );
        assert!(
            intersection.origin().x() + intersection.size().width()
                <= second.origin().x() + second.size().width()
        );
        assert!(
            intersection.origin().y() + intersection.size().height()
                <= first.origin().y() + first.size().height()
        );
        assert!(
            intersection.origin().y() + intersection.size().height()
                <= second.origin().y() + second.size().height()
        );
    }
}
