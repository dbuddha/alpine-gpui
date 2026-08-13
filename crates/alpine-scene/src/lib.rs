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

    #[test]
    fn freezes_primitives_in_painter_order() {
        let viewport = Size {
            width: 100.0,
            height: 100.0,
        };
        let mut builder = SceneBuilder::new(SceneRevision::new(7), viewport);
        let first = Primitive::Quad {
            bounds: Rect::new(
                Point { x: 0.0, y: 0.0 },
                Size {
                    width: 10.0,
                    height: 10.0,
                },
            ),
            color: LinearRgba {
                red: 1.0,
                green: 0.0,
                blue: 0.0,
                alpha: 1.0,
            },
        };
        let second = Primitive::Quad {
            bounds: Rect::new(
                Point { x: 5.0, y: 5.0 },
                Size {
                    width: 10.0,
                    height: 10.0,
                },
            ),
            color: LinearRgba {
                red: 0.0,
                green: 0.0,
                blue: 1.0,
                alpha: 0.5,
            },
        };
        builder.push(first);
        builder.push(second);

        let scene = builder.finish();

        assert_eq!(scene.revision().get(), 7);
        assert_eq!(scene.viewport(), viewport);
        assert_eq!(scene.primitives(), &[first, second]);
    }

    #[test]
    fn freezes_an_empty_scene_without_allocated_primitives() {
        let viewport = Size {
            width: 0.0,
            height: 0.0,
        };
        let scene = SceneBuilder::new(SceneRevision::new(0), viewport).finish();

        assert_eq!(scene.revision().get(), 0);
        assert_eq!(scene.viewport(), viewport);
        assert!(scene.primitives().is_empty());
    }
}
