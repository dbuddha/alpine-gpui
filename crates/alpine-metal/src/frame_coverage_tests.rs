//! Coverage-driving tests kept outside production source reporting.

#![allow(clippy::float_cmp)]

use std::{error::Error, num::NonZeroU32, sync::Arc};

use alpine_core::{LinearRgba, Point, Rect, Size};
use alpine_scene::{AtlasBounds, Glyph, GlyphAtlasImage, SceneBuilder, SceneRevision};

use super::{
    BGRA_BYTES_PER_PIXEL, MAX_GLYPH_ATLAS_BYTES, MAX_METAL3_TEXTURE_DIMENSION_2D,
    OffscreenDescriptor, OffscreenError, READBACK_ROW_ALIGNMENT, ValidatedFrame, lower_glyph,
    validate_glyph_atlas_contract,
};

fn color(red: f32, green: f32, blue: f32, alpha: f32) -> Result<LinearRgba, &'static str> {
    LinearRgba::new(red, green, blue, alpha).ok_or("valid color")
}

fn size(width: f32, height: f32) -> Result<Size, &'static str> {
    Size::new(width, height).ok_or("valid size")
}

fn point(x: f32, y: f32) -> Result<Point, &'static str> {
    Point::new(x, y).ok_or("valid point")
}

fn descriptor(width: u32, height: u32, scale: f32) -> Result<OffscreenDescriptor, Box<dyn Error>> {
    Ok(OffscreenDescriptor::new(
        width,
        height,
        scale,
        color(0.0, 0.0, 0.0, 0.0)?,
    )?)
}

#[test]
fn glyph_omission_precision_and_atlas_admission_are_checked() -> Result<(), Box<dyn Error>> {
    assert_eq!(BGRA_BYTES_PER_PIXEL, 4);
    assert_eq!(MAX_GLYPH_ATLAS_BYTES, 16_777_216);
    assert_eq!(READBACK_ROW_ALIGNMENT, 256);
    assert_eq!(
        validate_glyph_atlas_contract(
            MAX_METAL3_TEXTURE_DIMENSION_2D,
            MAX_METAL3_TEXTURE_DIMENSION_2D,
            MAX_GLYPH_ATLAS_BYTES,
        ),
        Ok(())
    );
    assert!(validate_glyph_atlas_contract(MAX_METAL3_TEXTURE_DIMENSION_2D + 1, 1, 1).is_err());
    assert!(validate_glyph_atlas_contract(1, MAX_METAL3_TEXTURE_DIMENSION_2D + 1, 1).is_err());
    assert!(validate_glyph_atlas_contract(1, 1, MAX_GLYPH_ATLAS_BYTES + 1).is_err());
    let one = NonZeroU32::new(1).ok_or("one")?;
    let atlas = GlyphAtlasImage::new(1, one, one, Arc::from([255_u8]))?;
    let source = AtlasBounds::new(0, 0, one, one);
    let paint = color(1.0, 1.0, 1.0, 1.0)?;

    let mut omitted = SceneBuilder::new(SceneRevision::new(12), size(2.0, 2.0)?);
    omitted.set_glyph_atlas(atlas.clone())?;
    let empty_bounds = Rect::new(point(0.0, 0.0)?, size(0.0, 1.0)?);
    omitted.push_glyph(Glyph::new(empty_bounds, source, paint))?;
    let outside_bounds = Rect::new(point(3.0, 3.0)?, size(1.0, 1.0)?);
    omitted.push_glyph(Glyph::new(outside_bounds, source, paint))?;
    let frame = ValidatedFrame::new(&omitted.finish(), descriptor(2, 2, 1.0)?)?;
    assert_eq!(frame.omitted_primitives(), 2);
    assert_eq!(frame.upload_bytes(), 0);
    assert_eq!(frame.glyph_atlas(), Some(&atlas));

    let mut precision = SceneBuilder::new(SceneRevision::new(13), size(16_777_218.0, 1.0)?);
    precision.set_glyph_atlas(atlas)?;
    let narrow = Rect::new(point(16_777_216.0, 0.0)?, size(1.0, 1.0)?);
    precision.push_glyph(Glyph::new(narrow, source, paint))?;
    assert_eq!(
        ValidatedFrame::new(&precision.finish(), descriptor(16_777_218, 1, 1.0)?),
        Err(OffscreenError::UnrepresentableQuad { primitive_index: 0 })
    );

    let too_wide = NonZeroU32::new(MAX_METAL3_TEXTURE_DIMENSION_2D + 1).ok_or("wide")?;
    let pixels = vec![0_u8; usize::try_from(too_wide.get())?];
    let oversized = GlyphAtlasImage::new(2, too_wide, one, Arc::from(pixels))?;
    let mut rejected = SceneBuilder::new(SceneRevision::new(14), size(1.0, 1.0)?);
    rejected.set_glyph_atlas(oversized)?;
    assert_eq!(
        ValidatedFrame::new(&rejected.finish(), descriptor(1, 1, 1.0)?),
        Err(OffscreenError::GlyphAtlasExtentUnsupported {
            width: MAX_METAL3_TEXTURE_DIMENSION_2D + 1,
            height: 1,
            limit: MAX_METAL3_TEXTURE_DIMENSION_2D,
        })
    );
    Ok(())
}

#[test]
fn glyph_lowering_crops_each_axis_and_uv_independently() -> Result<(), Box<dyn Error>> {
    let eight = NonZeroU32::new(8).ok_or("eight")?;
    let four = NonZeroU32::new(4).ok_or("four")?;
    let six = NonZeroU32::new(6).ok_or("six")?;
    let atlas = GlyphAtlasImage::new(3, eight, eight, Arc::from([255_u8; 64]))?;
    let glyph = Glyph::new(
        Rect::new(point(-1.0, -2.0)?, size(4.0, 6.0)?),
        AtlasBounds::new(2, 1, four, six),
        color(0.25, 0.5, 0.75, 1.0)?,
    );
    let lowered = lower_glyph(glyph, &atlas, 9.0, 9.0, 2.0, 7, [0.5, 0.25, 2.0, 3.0])?
        .ok_or("visible glyph")?;
    assert_eq!(lowered.bounds(), [1.0, 0.5, 4.0, 6.0]);
    assert_eq!(lowered.atlas_uv(), [0.4375, 0.40625, 0.625, 0.75]);

    let flat = Glyph::new(
        Rect::new(point(0.0, 0.0)?, size(1.0, 0.0)?),
        AtlasBounds::new(0, 0, four, six),
        color(1.0, 1.0, 1.0, 1.0)?,
    );
    assert_eq!(
        lower_glyph(flat, &atlas, 9.0, 9.0, 1.0, 8, [0.0, 0.0, 9.0, 9.0])?,
        None
    );

    let precise_x = Glyph::new(
        Rect::new(point(16_777_216.0, 0.0)?, size(1.0, 1.0)?),
        AtlasBounds::new(0, 0, four, six),
        color(1.0, 1.0, 1.0, 1.0)?,
    );
    assert_eq!(
        lower_glyph(
            precise_x,
            &atlas,
            16_777_218.0,
            2.0,
            1.0,
            9,
            [0.0, 0.0, 16_777_218.0, 2.0],
        ),
        Err(OffscreenError::UnrepresentableQuad { primitive_index: 9 })
    );
    Ok(())
}
