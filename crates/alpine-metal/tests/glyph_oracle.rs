//! Consumer-side coverage for public glyph-frame and CPU-oracle contracts.

use std::{error::Error, num::NonZeroU32, sync::Arc};

use alpine_core::{LinearRgba, Point, Rect, Size};
use alpine_metal::{OffscreenDescriptor, ValidatedFrame};
use alpine_scene::{AtlasBounds, Glyph, GlyphAtlasImage, SceneBuilder, SceneRevision};

#[test]
fn public_glyph_frame_and_oracle_contracts_are_reachable() -> Result<(), Box<dyn Error>> {
    let viewport = Size::new(1.0, 1.0).ok_or("viewport")?;
    let bounds = Rect::new(Point::new(0.0, 0.0).ok_or("point")?, viewport);
    let white = LinearRgba::new(1.0, 1.0, 1.0, 1.0).ok_or("white")?;
    let clear = LinearRgba::new(0.0, 0.0, 0.0, 0.0).ok_or("clear")?;
    let one = NonZeroU32::new(1).ok_or("one")?;
    let atlas = GlyphAtlasImage::new(5, one, one, Arc::from([255_u8]))?;
    let mut builder = SceneBuilder::new(SceneRevision::new(6), viewport);
    builder.set_glyph_atlas(atlas.clone())?;
    builder.push_glyph(Glyph::new(bounds, AtlasBounds::new(0, 0, one, one), white))?;
    let descriptor = OffscreenDescriptor::new(1, 1, 1.0, clear)?;
    let frame = ValidatedFrame::new(&builder.finish(), descriptor)?;
    assert_eq!(frame.glyph_atlas(), Some(&atlas));
    assert_eq!(frame.upload_bytes(), std::mem::size_of_val(frame.paints()));
    assert_eq!(frame.readback_layout().compact_bytes_per_row(), 4);
    assert_eq!(frame.reference_image()?.pixel(0, 0), Some([255; 4]));
    Ok(())
}
