//! Consumer-side coverage for public scene contracts.

use std::{error::Error, num::NonZeroU32, sync::Arc};

use alpine_core::{LinearRgba, Point, Rect, Size};
use alpine_scene::{AtlasBounds, Clip, Glyph, GlyphAtlasImage, Quad, SceneBuilder, SceneRevision};

#[test]
fn public_scene_contracts_are_reachable_from_consumers() -> Result<(), Box<dyn Error>> {
    let viewport = Size::new(2.0, 2.0).ok_or("viewport")?;
    let bounds = Rect::new(Point::new(0.0, 0.0).ok_or("point")?, viewport);
    let color = LinearRgba::new(1.0, 1.0, 1.0, 1.0).ok_or("color")?;
    let mut builder = SceneBuilder::new(SceneRevision::new(3), viewport);
    let clip = builder.push_clip(Clip::new(bounds));
    let quad = builder.push_quad(Quad::new(bounds, color).clipped(clip))?;
    let one = NonZeroU32::new(1).ok_or("one")?;
    let atlas = GlyphAtlasImage::new(4, one, one, Arc::from([255_u8]))?;
    builder.set_glyph_atlas(atlas.clone())?;
    let glyph = builder.push_glyph(Glyph::new(bounds, AtlasBounds::new(0, 0, one, one), color))?;
    let scene = builder.finish();
    assert_eq!(scene.revision().get(), 3);
    assert_eq!(scene.viewport(), viewport);
    assert_eq!(scene.clips()[0].bounds(), bounds);
    assert_eq!(scene.quads()[quad.index()].clip(), Some(clip));
    assert_eq!(scene.glyphs()[glyph.index()].bounds(), bounds);
    assert_eq!(scene.glyphs()[glyph.index()].color(), color);
    assert_eq!(scene.operation_count(), 2);
    assert_eq!(atlas.revision(), 4);
    assert_eq!(atlas.pixels(), &[255]);
    assert!(atlas.shares_storage_with(&atlas.clone()));
    Ok(())
}
