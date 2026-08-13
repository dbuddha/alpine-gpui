//! Bounded Kani proofs for Alpine core value contracts.

#![cfg(kani)]

use alpine_core::{LinearRgba, Point, Rect, Size};

#[kani::proof]
fn bounded_sizes_preserve_constructor_contract() {
    let width = f32::from(kani::any::<u16>());
    let height = f32::from(kani::any::<u16>());
    let size =
        Size::new(width, height).expect("bounded unsigned values are finite and non-negative");

    assert_eq!(size.width, width);
    assert_eq!(size.height, height);
    assert_eq!(size.is_empty(), width == 0.0 || height == 0.0);
}

#[kani::proof]
fn byte_colors_always_normalize_to_valid_channels() {
    let red = f32::from(kani::any::<u8>()) / 255.0;
    let green = f32::from(kani::any::<u8>()) / 255.0;
    let blue = f32::from(kani::any::<u8>()) / 255.0;
    let alpha = f32::from(kani::any::<u8>()) / 255.0;

    assert!(LinearRgba::new(red, green, blue, alpha).is_some());
}

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

    let first = Rect::new(
        Point {
            x: first_x,
            y: first_y,
        },
        Size {
            width: first_width,
            height: first_height,
        },
    );
    let second = Rect::new(
        Point {
            x: second_x,
            y: second_y,
        },
        Size {
            width: second_width,
            height: second_height,
        },
    );

    if let Some(intersection) = first.intersection(second) {
        assert!(intersection.size.width > 0.0);
        assert!(intersection.size.height > 0.0);
        assert!(intersection.origin.x >= first.origin.x);
        assert!(intersection.origin.x >= second.origin.x);
        assert!(intersection.origin.y >= first.origin.y);
        assert!(intersection.origin.y >= second.origin.y);
        assert!(
            intersection.origin.x + intersection.size.width <= first.origin.x + first.size.width
        );
        assert!(
            intersection.origin.x + intersection.size.width <= second.origin.x + second.size.width
        );
        assert!(
            intersection.origin.y + intersection.size.height <= first.origin.y + first.size.height
        );
        assert!(
            intersection.origin.y + intersection.size.height
                <= second.origin.y + second.size.height
        );
    }
}
