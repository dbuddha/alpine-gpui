//! Public contract tests for safe offscreen frame planning and lifecycle behavior.

use std::error::Error;

use alpine_core::{LinearRgba, Point, Rect, Size};
use alpine_metal::{
    FrameLifecycle, FrameOutcome, LifecycleAction, OffscreenDescriptor, ValidatedFrame,
};
use alpine_scene::{Primitive, SceneBuilder, SceneRevision};

fn color(red: f32, green: f32, blue: f32, alpha: f32) -> Result<LinearRgba, &'static str> {
    LinearRgba::new(red, green, blue, alpha).ok_or("valid contract color")
}

fn size(width: f32, height: f32) -> Result<Size, &'static str> {
    Size::new(width, height).ok_or("valid contract size")
}

fn point(x: f32, y: f32) -> Result<Point, &'static str> {
    Point::new(x, y).ok_or("valid contract point")
}

#[test]
fn public_offscreen_contract_renders_reference() -> Result<(), Box<dyn Error>> {
    let mut builder = SceneBuilder::new(SceneRevision::new(19), size(2.0, 2.0)?);
    builder.push(Primitive::Quad {
        bounds: Rect::new(point(0.0, 0.0)?, size(2.0, 2.0)?),
        color: color(0.0, 1.0, 0.0, 1.0)?,
    });
    let descriptor = OffscreenDescriptor::new(4, 4, 2.0, color(0.0, 0.0, 0.0, 0.0)?)?;
    let frame = ValidatedFrame::new(&builder.finish(), descriptor)?;
    let image = frame.reference_image()?;

    assert_eq!(frame.revision(), SceneRevision::new(19));
    assert_eq!(frame.consumed_primitives(), 1);
    assert_eq!(frame.omitted_primitives(), 0);
    assert_eq!(image.width(), 4);
    assert_eq!(image.height(), 4);
    assert_eq!(image.pixel(0, 0), Some([0, 255, 0, 255]));
    assert_eq!(image.pixel(3, 3), Some([0, 255, 0, 255]));
    assert_eq!(image.bytes().len(), 64);
    Ok(())
}

#[test]
fn public_lifecycle_rejects_submission_before_encoding() {
    let mut lifecycle = FrameLifecycle::new();
    let before = lifecycle;
    assert!(lifecycle.apply(LifecycleAction::Submit).is_err());
    assert_eq!(lifecycle, before);
    assert_eq!(lifecycle.outcome(), FrameOutcome::Pending);
    assert_eq!(lifecycle.submit_count(), 0);
    assert!(lifecycle.invariants_hold());
}
