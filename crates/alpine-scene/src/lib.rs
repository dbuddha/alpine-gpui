//! Immutable renderer input for Alpine GPUI.

use alpine_core::{LinearRgba, Rect, Size};

/// Monotonically increasing identity for a scene snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SceneRevision(u64);

impl SceneRevision {
    /// Creates a revision from its persisted integer value.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the underlying integer value.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// A renderer primitive in painter's order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Primitive {
    /// A solid axis-aligned rectangle.
    Quad {
        /// Bounds in logical pixels.
        bounds: Rect,
        /// Linear unpremultiplied color.
        color: LinearRgba,
    },
}

/// An immutable snapshot consumed by a renderer.
#[derive(Clone, Debug, PartialEq)]
pub struct Scene {
    revision: SceneRevision,
    viewport: Size,
    primitives: Box<[Primitive]>,
}

impl Scene {
    /// Returns this snapshot's revision.
    #[must_use]
    pub const fn revision(&self) -> SceneRevision {
        self.revision
    }

    /// Returns the logical viewport size.
    #[must_use]
    pub const fn viewport(&self) -> Size {
        self.viewport
    }

    /// Returns primitives in painter's order.
    #[must_use]
    pub fn primitives(&self) -> &[Primitive] {
        &self.primitives
    }
}

/// Single-use builder for an immutable scene.
#[derive(Debug)]
pub struct SceneBuilder {
    revision: SceneRevision,
    viewport: Size,
    primitives: Vec<Primitive>,
}

impl SceneBuilder {
    /// Starts a scene for the given revision and viewport.
    #[must_use]
    pub const fn new(revision: SceneRevision, viewport: Size) -> Self {
        Self {
            revision,
            viewport,
            primitives: Vec::new(),
        }
    }

    /// Appends a primitive in painter's order.
    pub fn push(&mut self, primitive: Primitive) {
        self.primitives.push(primitive);
    }

    /// Freezes the scene for renderer consumption.
    #[must_use]
    pub fn finish(self) -> Scene {
        Scene {
            revision: self.revision,
            viewport: self.viewport,
            primitives: self.primitives.into_boxed_slice(),
        }
    }
}

#[cfg(test)]
mod tests {
    use alpine_core::{LinearRgba, Point, Rect, Size};

    use super::{Primitive, SceneBuilder, SceneRevision};

    fn valid_point(x: f32, y: f32) -> Result<Point, &'static str> {
        Point::new(x, y).ok_or("test point must be valid")
    }

    fn valid_size(width: f32, height: f32) -> Result<Size, &'static str> {
        Size::new(width, height).ok_or("test size must be valid")
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
    fn freezes_primitives_in_painter_order() -> Result<(), &'static str> {
        let viewport = valid_size(100.0, 100.0)?;
        let mut builder = SceneBuilder::new(SceneRevision::new(7), viewport);
        let first = Primitive::Quad {
            bounds: Rect::new(valid_point(0.0, 0.0)?, valid_size(10.0, 10.0)?),
            color: valid_color(1.0, 0.0, 0.0, 1.0)?,
        };
        let second = Primitive::Quad {
            bounds: Rect::new(valid_point(5.0, 5.0)?, valid_size(10.0, 10.0)?),
            color: valid_color(0.0, 0.0, 1.0, 0.5)?,
        };
        builder.push(first);
        builder.push(second);

        let scene = builder.finish();

        assert_eq!(scene.revision().get(), 7);
        assert_eq!(scene.viewport(), viewport);
        assert_eq!(scene.primitives(), &[first, second]);
        Ok(())
    }

    #[test]
    fn freezes_an_empty_scene_without_allocated_primitives() -> Result<(), &'static str> {
        let viewport = valid_size(0.0, 0.0)?;
        let scene = SceneBuilder::new(SceneRevision::new(0), viewport).finish();

        assert_eq!(scene.revision().get(), 0);
        assert_eq!(scene.viewport(), viewport);
        assert!(scene.primitives().is_empty());
        Ok(())
    }
}
