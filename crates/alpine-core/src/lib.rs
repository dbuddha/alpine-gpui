//! Backend-neutral geometry and color types for Alpine GPUI.

#[cfg(kani)]
mod proofs;

/// A two-dimensional point in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    /// Horizontal coordinate.
    pub x: f32,
    /// Vertical coordinate.
    pub y: f32,
}

impl Point {
    /// Creates a point when both coordinates are finite.
    #[must_use]
    pub fn new(x: f32, y: f32) -> Option<Self> {
        (x.is_finite() && y.is_finite()).then_some(Self { x, y })
    }
}

/// A two-dimensional extent in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Size {
    /// Horizontal extent.
    pub width: f32,
    /// Vertical extent.
    pub height: f32,
}

impl Size {
    /// Creates a non-negative finite size.
    #[must_use]
    pub fn new(width: f32, height: f32) -> Option<Self> {
        (width.is_finite() && height.is_finite() && width >= 0.0 && height >= 0.0)
            .then_some(Self { width, height })
    }

    /// Returns whether either extent is zero.
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.width == 0.0 || self.height == 0.0
    }
}

/// An axis-aligned rectangle in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    /// Top-left origin.
    pub origin: Point,
    /// Rectangle extent.
    pub size: Size,
}

impl Rect {
    /// Creates a rectangle from validated components.
    #[must_use]
    pub const fn new(origin: Point, size: Size) -> Self {
        Self { origin, size }
    }

    /// Returns the intersection of two rectangles.
    #[must_use]
    pub fn intersection(self, other: Self) -> Option<Self> {
        let left = self.origin.x.max(other.origin.x);
        let top = self.origin.y.max(other.origin.y);
        let right = (self.origin.x + self.size.width).min(other.origin.x + other.size.width);
        let bottom = (self.origin.y + self.size.height).min(other.origin.y + other.size.height);

        (right > left && bottom > top).then_some(Self {
            origin: Point { x: left, y: top },
            size: Size {
                width: right - left,
                height: bottom - top,
            },
        })
    }
}

/// Linear RGBA color with unpremultiplied channels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LinearRgba {
    /// Red channel.
    pub red: f32,
    /// Green channel.
    pub green: f32,
    /// Blue channel.
    pub blue: f32,
    /// Alpha channel.
    pub alpha: f32,
}

impl LinearRgba {
    /// Creates a color when every channel is finite and within `0.0..=1.0`.
    #[must_use]
    pub fn new(red: f32, green: f32, blue: f32, alpha: f32) -> Option<Self> {
        [red, green, blue, alpha]
            .into_iter()
            .all(|channel| channel.is_finite() && (0.0..=1.0).contains(&channel))
            .then_some(Self {
                red,
                green,
                blue,
                alpha,
            })
    }
}

#[cfg(test)]
mod tests {
    use super::{LinearRgba, Point, Rect, Size};

    #[test]
    fn rejects_non_finite_geometry() {
        assert!(Point::new(f32::NAN, 0.0).is_none());
        assert!(Point::new(0.0, f32::NEG_INFINITY).is_none());
        assert!(Size::new(1.0, f32::INFINITY).is_none());
        assert!(Size::new(-1.0, 1.0).is_none());
    }

    #[test]
    fn accepts_valid_geometry_and_identifies_empty_sizes() {
        assert_eq!(Point::new(-4.0, 8.5), Some(Point { x: -4.0, y: 8.5 }));

        assert_eq!(Size::new(12.0, 3.0).map(Size::is_empty), Some(false));
        assert_eq!(Size::new(0.0, 3.0).map(Size::is_empty), Some(true));
        assert_eq!(Size::new(3.0, 0.0).map(Size::is_empty), Some(true));
    }

    #[test]
    fn bounded_size_constructor_companion() {
        for width in [0_u16, 1, u16::MAX] {
            for height in [0_u16, 1, u16::MAX] {
                let width = f32::from(width);
                let height = f32::from(height);
                let size = Size::new(width, height);
                assert_eq!(
                    size.map(|value| {
                        (
                            value.width.to_bits(),
                            value.height.to_bits(),
                            value.is_empty(),
                        )
                    }),
                    Some((
                        width.to_bits(),
                        height.to_bits(),
                        width == 0.0 || height == 0.0,
                    ))
                );
            }
        }
    }

    #[test]
    fn intersects_overlapping_rectangles() {
        let first = Rect::new(
            Point { x: 0.0, y: 0.0 },
            Size {
                width: 10.0,
                height: 10.0,
            },
        );
        let second = Rect::new(
            Point { x: 5.0, y: 4.0 },
            Size {
                width: 10.0,
                height: 2.0,
            },
        );

        assert_eq!(
            first.intersection(second),
            Some(Rect::new(
                Point { x: 5.0, y: 4.0 },
                Size {
                    width: 5.0,
                    height: 2.0,
                },
            ))
        );
    }

    #[test]
    fn intersection_is_symmetric_and_excludes_empty_contact() {
        let first = Rect::new(
            Point { x: -2.0, y: -1.0 },
            Size {
                width: 4.0,
                height: 3.0,
            },
        );
        let contained = Rect::new(
            Point { x: -1.0, y: 0.0 },
            Size {
                width: 1.0,
                height: 1.0,
            },
        );
        let touching = Rect::new(
            Point { x: 2.0, y: -1.0 },
            Size {
                width: 2.0,
                height: 3.0,
            },
        );
        let touching_below = Rect::new(
            Point { x: -2.0, y: 2.0 },
            Size {
                width: 4.0,
                height: 1.0,
            },
        );

        assert_eq!(first.intersection(contained), Some(contained));
        assert_eq!(contained.intersection(first), Some(contained));
        assert_eq!(first.intersection(touching), None);
        assert_eq!(touching.intersection(first), None);
        assert_eq!(first.intersection(touching_below), None);
        assert_eq!(touching_below.intersection(first), None);
    }

    #[test]
    fn rejects_invalid_color_channels() {
        assert!(LinearRgba::new(1.1, 0.0, 0.0, 1.0).is_none());
        assert!(LinearRgba::new(-0.1, 0.0, 0.0, 1.0).is_none());
        assert!(LinearRgba::new(0.0, 0.0, 0.0, f32::NAN).is_none());
    }

    #[test]
    fn accepts_color_range_endpoints() {
        assert_eq!(
            LinearRgba::new(0.0, 0.25, 1.0, 1.0),
            Some(LinearRgba {
                red: 0.0,
                green: 0.25,
                blue: 1.0,
                alpha: 1.0,
            })
        );
    }

    #[test]
    fn byte_color_constructor_companion() {
        for byte in [0_u8, 1, 127, 254, u8::MAX] {
            let channel = f32::from(byte) / 255.0;
            assert!(LinearRgba::new(channel, channel, channel, channel).is_some());
        }
    }

    #[test]
    fn bounded_intersection_companion() {
        let samples = [
            Rect::new(
                Point { x: 0.0, y: 0.0 },
                Size {
                    width: 0.0,
                    height: 0.0,
                },
            ),
            Rect::new(
                Point { x: 0.0, y: 0.0 },
                Size {
                    width: 1.0,
                    height: 1.0,
                },
            ),
            Rect::new(
                Point {
                    x: f32::from(u8::MAX - 1),
                    y: f32::from(u8::MAX - 1),
                },
                Size {
                    width: 1.0,
                    height: 1.0,
                },
            ),
        ];

        for first in samples {
            for second in samples {
                if let Some(intersection) = first.intersection(second) {
                    assert!(intersection.size.width > 0.0);
                    assert!(intersection.size.height > 0.0);
                    assert!(intersection.origin.x >= first.origin.x);
                    assert!(intersection.origin.x >= second.origin.x);
                    assert!(intersection.origin.y >= first.origin.y);
                    assert!(intersection.origin.y >= second.origin.y);
                    assert!(
                        intersection.origin.x + intersection.size.width
                            <= first.origin.x + first.size.width
                    );
                    assert!(
                        intersection.origin.x + intersection.size.width
                            <= second.origin.x + second.size.width
                    );
                    assert!(
                        intersection.origin.y + intersection.size.height
                            <= first.origin.y + first.size.height
                    );
                    assert!(
                        intersection.origin.y + intersection.size.height
                            <= second.origin.y + second.size.height
                    );
                }
            }
        }
    }
}
