//! Backend-neutral geometry and color types for Alpine GPUI.

#[cfg(kani)]
mod proofs;

/// A two-dimensional point in logical pixels.
///
/// Fields are private so callers cannot bypass finite-value validation.
///
/// ```compile_fail
/// use alpine_core::Point;
///
/// let _ = Point { x: f32::NAN, y: 0.0 };
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    /// Horizontal coordinate.
    x: f32,
    /// Vertical coordinate.
    y: f32,
}

impl Point {
    /// Creates a point when both coordinates are finite.
    #[must_use]
    pub fn new(x: f32, y: f32) -> Option<Self> {
        (x.is_finite() && y.is_finite()).then_some(Self { x, y })
    }

    /// Returns the horizontal coordinate.
    #[must_use]
    pub const fn x(self) -> f32 {
        self.x
    }

    /// Returns the vertical coordinate.
    #[must_use]
    pub const fn y(self) -> f32 {
        self.y
    }
}

/// A two-dimensional extent in logical pixels.
///
/// Fields are private so callers cannot bypass finite, non-negative validation.
///
/// ```compile_fail
/// use alpine_core::Size;
///
/// let _ = Size { width: -1.0, height: f32::NAN };
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Size {
    /// Horizontal extent.
    width: f32,
    /// Vertical extent.
    height: f32,
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

    /// Returns the horizontal extent.
    #[must_use]
    pub const fn width(self) -> f32 {
        self.width
    }

    /// Returns the vertical extent.
    #[must_use]
    pub const fn height(self) -> f32 {
        self.height
    }
}

/// An axis-aligned rectangle in logical pixels.
///
/// Its components are already-validated [`Point`] and [`Size`] values.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    /// Top-left origin.
    origin: Point,
    /// Rectangle extent.
    size: Size,
}

impl Rect {
    /// Creates a rectangle from validated components.
    #[must_use]
    pub const fn new(origin: Point, size: Size) -> Self {
        Self { origin, size }
    }

    /// Returns the top-left origin.
    #[must_use]
    pub const fn origin(self) -> Point {
        self.origin
    }

    /// Returns the rectangle extent.
    #[must_use]
    pub const fn size(self) -> Size {
        self.size
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
///
/// Fields are private so callers cannot bypass finite, normalized validation.
///
/// ```compile_fail
/// use alpine_core::LinearRgba;
///
/// let _ = LinearRgba {
///     red: 2.0,
///     green: 0.0,
///     blue: 0.0,
///     alpha: 1.0,
/// };
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LinearRgba {
    /// Red channel.
    red: f32,
    /// Green channel.
    green: f32,
    /// Blue channel.
    blue: f32,
    /// Alpha channel.
    alpha: f32,
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

    /// Returns the red channel.
    #[must_use]
    pub const fn red(self) -> f32 {
        self.red
    }

    /// Returns the green channel.
    #[must_use]
    pub const fn green(self) -> f32 {
        self.green
    }

    /// Returns the blue channel.
    #[must_use]
    pub const fn blue(self) -> f32 {
        self.blue
    }

    /// Returns the alpha channel.
    #[must_use]
    pub const fn alpha(self) -> f32 {
        self.alpha
    }
}

#[cfg(test)]
mod tests {
    use super::{LinearRgba, Point, Rect, Size};

    fn valid_point(x: f32, y: f32) -> Result<Point, &'static str> {
        Point::new(x, y).ok_or("test point must be valid")
    }

    fn valid_size(width: f32, height: f32) -> Result<Size, &'static str> {
        Size::new(width, height).ok_or("test size must be valid")
    }

    fn valid_rect(x: f32, y: f32, width: f32, height: f32) -> Result<Rect, &'static str> {
        Ok(Rect::new(valid_point(x, y)?, valid_size(width, height)?))
    }

    fn valid_color(
        red: f32,
        green: f32,
        blue: f32,
        alpha: f32,
    ) -> Result<LinearRgba, &'static str> {
        LinearRgba::new(red, green, blue, alpha).ok_or("test color must be valid")
    }

    #[test]
    fn rejects_non_finite_geometry() {
        assert!(Point::new(f32::NAN, 0.0).is_none());
        assert!(Point::new(0.0, f32::NEG_INFINITY).is_none());
        assert!(Size::new(1.0, f32::INFINITY).is_none());
        assert!(Size::new(-1.0, 1.0).is_none());
    }

    #[test]
    fn accepts_valid_geometry_and_identifies_empty_sizes() {
        assert_eq!(
            Point::new(-4.0, 8.5).map(|point| (point.x(), point.y())),
            Some((-4.0, 8.5))
        );

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
                            value.width().to_bits(),
                            value.height().to_bits(),
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
    fn intersects_overlapping_rectangles() -> Result<(), &'static str> {
        let first = valid_rect(0.0, 0.0, 10.0, 10.0)?;
        let second = valid_rect(5.0, 4.0, 10.0, 2.0)?;

        assert_eq!(
            first.intersection(second),
            Some(valid_rect(5.0, 4.0, 5.0, 2.0)?)
        );
        Ok(())
    }

    #[test]
    fn intersection_is_symmetric_and_excludes_empty_contact() -> Result<(), &'static str> {
        let first = valid_rect(-2.0, -1.0, 4.0, 3.0)?;
        let contained = valid_rect(-1.0, 0.0, 1.0, 1.0)?;
        let touching = valid_rect(2.0, -1.0, 2.0, 3.0)?;
        let touching_below = valid_rect(-2.0, 2.0, 4.0, 1.0)?;

        assert_eq!(first.intersection(contained), Some(contained));
        assert_eq!(contained.intersection(first), Some(contained));
        assert_eq!(first.intersection(touching), None);
        assert_eq!(touching.intersection(first), None);
        assert_eq!(first.intersection(touching_below), None);
        assert_eq!(touching_below.intersection(first), None);
        Ok(())
    }

    #[test]
    fn rejects_invalid_color_channels() {
        assert!(LinearRgba::new(1.1, 0.0, 0.0, 1.0).is_none());
        assert!(LinearRgba::new(-0.1, 0.0, 0.0, 1.0).is_none());
        assert!(LinearRgba::new(0.0, 0.0, 0.0, f32::NAN).is_none());
    }

    #[test]
    fn accepts_color_range_endpoints() -> Result<(), &'static str> {
        let color = LinearRgba::new(0.0, 0.25, 1.0, 1.0);
        assert_eq!(color, Some(valid_color(0.0, 0.25, 1.0, 1.0)?));
        assert_eq!(
            color.map(|value| (value.red(), value.green(), value.blue(), value.alpha())),
            Some((0.0, 0.25, 1.0, 1.0))
        );
        Ok(())
    }

    #[test]
    fn accessors_preserve_validated_components() -> Result<(), &'static str> {
        let point = valid_point(-3.5, 7.25)?;
        let size = valid_size(11.5, 19.25)?;
        let rect = Rect::new(point, size);
        let color = valid_color(0.125, 0.25, 0.75, 0.875)?;

        assert_eq!((point.x(), point.y()), (-3.5, 7.25));
        assert_eq!((size.width(), size.height()), (11.5, 19.25));
        assert_eq!((rect.origin(), rect.size()), (point, size));
        assert_eq!(
            (color.red(), color.green(), color.blue(), color.alpha()),
            (0.125, 0.25, 0.75, 0.875)
        );
        Ok(())
    }

    #[test]
    fn byte_color_constructor_companion() {
        for byte in [0_u8, 1, 127, 254, u8::MAX] {
            let channel = f32::from(byte) / 255.0;
            assert!(LinearRgba::new(channel, channel, channel, channel).is_some());
        }
    }

    #[test]
    fn bounded_intersection_companion() -> Result<(), &'static str> {
        let samples = [
            valid_rect(0.0, 0.0, 0.0, 0.0)?,
            valid_rect(0.0, 0.0, 1.0, 1.0)?,
            valid_rect(f32::from(u8::MAX - 1), f32::from(u8::MAX - 1), 1.0, 1.0)?,
        ];

        for first in samples {
            for second in samples {
                if let Some(intersection) = first.intersection(second) {
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
        }
        Ok(())
    }
}
