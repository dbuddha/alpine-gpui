//! Coverage-driving tests kept outside production source reporting.

#![allow(clippy::float_cmp)]

use std::{error::Error, num::NonZeroU32, sync::Arc};

use alpine_core::{LinearRgba, Point, Rect, Size};
use alpine_scene::{AtlasBounds, Glyph, GlyphAtlasImage, SceneBuilder, SceneRevision};

use crate::{OffscreenDescriptor, ValidatedFrame};

use super::{composite_instance, sample_atlas};

fn color(red: f32, green: f32, blue: f32, alpha: f32) -> Result<LinearRgba, &'static str> {
    LinearRgba::new(red, green, blue, alpha).ok_or("valid color")
}

fn size(width: f32, height: f32) -> Result<Size, &'static str> {
    Size::new(width, height).ok_or("valid size")
}

fn point(x: f32, y: f32) -> Result<Point, &'static str> {
    Point::new(x, y).ok_or("valid point")
}

#[test]
fn glyph_sampling_covers_valid_missing_and_outside_coordinates() -> Result<(), Box<dyn Error>> {
    let one = NonZeroU32::new(1).ok_or("one")?;
    let two = NonZeroU32::new(2).ok_or("two")?;
    let atlas = GlyphAtlasImage::new(1, two, one, Arc::from([0_u8, 255]))?;
    let mut builder = SceneBuilder::new(SceneRevision::new(8), size(2.0, 1.0)?);
    builder.set_glyph_atlas(atlas)?;
    let bounds = Rect::new(point(0.0, 0.0)?, size(2.0, 1.0)?);
    let source = AtlasBounds::new(0, 0, two, one);
    builder.push_glyph(Glyph::new(bounds, source, color(1.0, 1.0, 1.0, 1.0)?))?;
    let descriptor = OffscreenDescriptor::new(2, 1, 1.0, color(0.0, 0.0, 0.0, 0.0)?)?;
    let frame = ValidatedFrame::new(&builder.finish(), descriptor)?;
    let image = frame.reference_image()?;
    assert_eq!(image.pixel(0, 0), Some([0, 0, 0, 0]));
    assert_eq!(image.pixel(1, 0), Some([255, 255, 255, 255]));
    assert_eq!(
        sample_atlas(
            &frame,
            [0.0, 0.0, 1.0, 1.0],
            [0.0, 0.0, 1.0, 1.0],
            -0.5,
            0.5,
        ),
        0.0
    );
    assert_eq!(
        sample_atlas(
            &frame,
            [0.0, 0.0, 1.0, 1.0],
            [0.0, 0.0, 1.0, 1.0],
            0.5,
            -0.5,
        ),
        0.0
    );
    assert_eq!(
        sample_atlas(&frame, [0.0, 0.0, 1.0, 1.0], [0.0, 0.0, 1.0, 1.0], 2.0, 0.5,),
        0.0
    );

    let empty_scene = SceneBuilder::new(SceneRevision::new(9), size(1.0, 1.0)?).finish();
    let empty_descriptor = OffscreenDescriptor::new(1, 1, 1.0, color(0.0, 0.0, 0.0, 0.0)?)?;
    let empty = ValidatedFrame::new(&empty_scene, empty_descriptor)?;
    assert_eq!(
        sample_atlas(&empty, [0.0, 0.0, 1.0, 1.0], [0.0; 4], 0.5, 0.5),
        0.0
    );
    Ok(())
}

#[test]
fn atlas_sampling_and_instance_edges_use_nontrivial_coordinates() -> Result<(), Box<dyn Error>> {
    let three = NonZeroU32::new(3).ok_or("three")?;
    let atlas = GlyphAtlasImage::new(
        2,
        three,
        three,
        Arc::from([0_u8, 32, 64, 96, 128, 160, 192, 224, 255]),
    )?;
    let mut builder = SceneBuilder::new(SceneRevision::new(10), size(12.0, 14.0)?);
    builder.set_glyph_atlas(atlas)?;
    builder.push_glyph(Glyph::new(
        Rect::new(point(2.0, 4.0)?, size(8.0, 8.0)?),
        AtlasBounds::new(0, 0, three, three),
        color(0.5, 0.25, 0.75, 0.5)?,
    ))?;
    let frame = ValidatedFrame::new(
        &builder.finish(),
        OffscreenDescriptor::new(12, 14, 1.0, color(0.0, 0.0, 0.0, 0.0)?)?,
    )?;
    assert_eq!(
        sample_atlas(
            &frame,
            [2.0, 4.0, 10.0, 12.0],
            [0.0, 0.0, 1.0, 1.0],
            6.0,
            8.0,
        ),
        128.0 / 255.0
    );
    assert_eq!(
        sample_atlas(
            &frame,
            [2.0, 4.0, 10.0, 12.0],
            [0.0, 0.0, 1.0, 1.0],
            9.5,
            11.5,
        ),
        255.0 / 255.0
    );
    assert_eq!(
        sample_atlas(
            &frame,
            [2.0, 4.0, 10.0, 12.0],
            [0.2, 0.25, 0.8, 0.75],
            6.0,
            8.8,
        ),
        128.0 / 255.0
    );
    assert_eq!(
        sample_atlas(&frame, [0.0, 0.0, 1.0, 1.0], [0.0, 0.0, 1.0, 1.0], 0.5, 2.0,),
        0.0
    );
    assert_eq!(
        sample_atlas(&frame, [0.0, 0.0, 1.0, 1.0], [0.0, 0.0, 1.0, 1.0], 1.0, 0.1,),
        0.0
    );

    let instance = frame.paints()[0];
    for (x, y) in [(1.999, 8.0), (10.0, 8.0), (6.0, 3.999), (6.0, 12.0)] {
        let mut destination = [0.1, 0.2, 0.3, 0.4];
        composite_instance(&frame, instance, x, y, &mut destination);
        assert_eq!(destination, [0.1, 0.2, 0.3, 0.4]);
    }
    let mut inside = [0.0; 4];
    composite_instance(&frame, instance, 6.0, 8.0, &mut inside);
    let expected_alpha = 0.5 * (128.0 / 255.0);
    assert!((inside[3] - expected_alpha).abs() < f32::EPSILON);
    Ok(())
}
