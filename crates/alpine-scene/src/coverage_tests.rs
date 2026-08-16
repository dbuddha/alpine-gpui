//! Coverage-driving tests kept outside production source reporting.

#![allow(clippy::too_many_lines)]

use std::{error::Error, num::NonZeroU32, sync::Arc};

use alpine_core::{LinearRgba, Point, Rect, Size};

use super::*;

fn point(x: f32, y: f32) -> Result<Point, &'static str> {
    Point::new(x, y).ok_or("valid point")
}

fn size(width: f32, height: f32) -> Result<Size, &'static str> {
    Size::new(width, height).ok_or("valid size")
}

fn color(red: f32, green: f32, blue: f32, alpha: f32) -> Result<LinearRgba, &'static str> {
    LinearRgba::new(red, green, blue, alpha).ok_or("valid color")
}

fn atlas() -> Result<GlyphAtlasImage, SceneError> {
    let four = NonZeroU32::new(4).ok_or(SceneError::ArithmeticOverflow)?;
    GlyphAtlasImage::new(4, four, four, Arc::from([255_u8; 16]))
}

#[test]
fn public_contracts_and_structured_failures_are_discriminating() -> Result<(), Box<dyn Error>> {
    let viewport = size(8.0, 8.0)?;
    let revision = SceneRevision::new(11);
    assert_eq!(revision.get(), 11);
    let mut builder = SceneBuilder::new(revision, viewport);
    let clip_value = Clip::new(Rect::new(point(1.0, 1.0)?, size(6.0, 6.0)?));
    let clip = builder.push_clip(clip_value);
    assert_eq!(
        clip_value.bounds(),
        Rect::new(point(1.0, 1.0)?, size(6.0, 6.0)?)
    );
    let quad = Quad::new(
        Rect::new(point(2.0, 2.0)?, size(2.0, 2.0)?),
        color(1.0, 0.0, 0.0, 1.0)?,
    )
    .clipped(clip);
    let quad_id = builder.push_quad(quad)?;
    assert_eq!(quad_id.index(), 0);
    let second_quad = builder.push_quad(quad)?;
    assert_eq!(second_quad.index(), 1);
    assert_eq!(quad.clip(), Some(clip));

    let image = atlas()?;
    assert_eq!(image.revision(), 4);
    assert_eq!(image.width().get(), 4);
    assert_eq!(image.height().get(), 4);
    assert_eq!(image.pixels().len(), 16);
    assert!(image.shares_storage_with(&image.clone()));
    builder.set_glyph_atlas(image.clone())?;
    let one = NonZeroU32::new(1).ok_or(SceneError::ArithmeticOverflow)?;
    let glyph = Glyph::new(
        Rect::new(point(3.0, 3.0)?, size(1.0, 1.0)?),
        AtlasBounds::new(0, 0, one, one),
        color(0.0, 1.0, 0.0, 1.0)?,
    )
    .clipped(clip);
    let glyph_id = builder.push_glyph(glyph)?;
    assert_eq!(glyph_id.index(), 0);
    let second_glyph = builder.push_glyph(glyph)?;
    assert_eq!(second_glyph.index(), 1);
    assert_eq!(glyph.bounds(), Rect::new(point(3.0, 3.0)?, size(1.0, 1.0)?));
    assert_eq!(glyph.color(), color(0.0, 1.0, 0.0, 1.0)?);
    assert_eq!(
        builder.set_glyph_atlas(image),
        Err(SceneError::AtlasAlreadyReferenced)
    );
    let scene = builder.finish();
    assert_eq!(scene.revision(), revision);
    assert_eq!(scene.viewport(), viewport);
    assert_eq!(scene.clips(), &[clip_value]);
    assert_eq!(scene.operation_count(), 4);
    assert_eq!(scene.glyph_atlas().map(GlyphAtlasImage::revision), Some(4));

    let invalid = ClipId(9);
    let mut invalid_builder = SceneBuilder::new(revision, viewport);
    assert_eq!(
        invalid_builder.push_quad(quad.clipped(invalid)),
        Err(SceneError::InvalidClip { index: 9 })
    );
    invalid_builder.set_glyph_atlas(atlas()?)?;
    assert_eq!(
        invalid_builder.push_glyph(glyph.clipped(invalid)),
        Err(SceneError::InvalidClip { index: 9 })
    );
    let overflow = Glyph::new(
        Rect::new(point(0.0, 0.0)?, size(1.0, 1.0)?),
        AtlasBounds::new(u32::MAX, 0, one, one),
        color(1.0, 1.0, 1.0, 1.0)?,
    );
    assert_eq!(
        invalid_builder.push_glyph(overflow),
        Err(SceneError::ArithmeticOverflow)
    );
    let two = NonZeroU32::new(2).ok_or(SceneError::ArithmeticOverflow)?;
    let offset = AtlasBounds::new(1, 2, two, one);
    assert_eq!(offset.y(), 2);
    let horizontal_only = Glyph::new(
        Rect::new(point(0.0, 0.0)?, size(1.0, 1.0)?),
        AtlasBounds::new(3, 0, two, one),
        color(1.0, 1.0, 1.0, 1.0)?,
    );
    assert_eq!(
        invalid_builder.push_glyph(horizontal_only),
        Err(SceneError::AtlasBoundsOutsideImage)
    );
    let vertical_only = Glyph::new(
        Rect::new(point(0.0, 0.0)?, size(1.0, 1.0)?),
        AtlasBounds::new(0, 4, one, one),
        color(1.0, 1.0, 1.0, 1.0)?,
    );
    assert_eq!(
        invalid_builder.push_glyph(vertical_only),
        Err(SceneError::AtlasBoundsOutsideImage)
    );

    let errors = [
        SceneError::ArithmeticOverflow,
        SceneError::InvalidAtlasLength {
            expected: 4,
            actual: 3,
        },
        SceneError::MissingGlyphAtlas,
        SceneError::AtlasAlreadyReferenced,
        SceneError::InvalidClip { index: 9 },
        SceneError::AtlasBoundsOutsideImage,
    ];
    for error in errors {
        assert!(!error.to_string().is_empty());
    }
    Ok(())
}
