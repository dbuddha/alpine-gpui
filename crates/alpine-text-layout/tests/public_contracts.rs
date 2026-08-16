//! Consumer-side coverage for public text-layout and glyph-atlas contracts.

use std::{error::Error, num::NonZeroU32};

use alpine_text::Buffer;
use alpine_text_layout::{
    FontKey, GlyphAtlas, GlyphBitmap, GlyphKey, LayoutError, LineLayout, LineLayoutCache,
    PositiveFinite, RasterizedGlyph, ShapedGlyph, TextShaper,
};

struct ConsumerShaper;

impl TextShaper for ConsumerShaper {
    fn shape(&mut self, _text: &str, _font: FontKey) -> Result<LineLayout, LayoutError> {
        LineLayout::new(Vec::new(), 5.0, 3.0, 1.0, 1)
    }
}

#[test]
#[allow(clippy::float_cmp)]
fn public_text_layout_contracts_are_reachable_from_consumers() -> Result<(), Box<dyn Error>> {
    let size = PositiveFinite::new(13.0).ok_or("size")?;
    let scale = PositiveFinite::new(2.0).ok_or("scale")?;
    let tabs = NonZeroU32::new(4).ok_or("tabs")?;
    let font = FontKey::new(7, size, scale, tabs);
    assert_eq!(font.family(), 7);
    assert_eq!(font.size(), 13.0);
    assert_eq!(font.scale(), 2.0);
    assert_eq!(font.tab_columns(), tabs);
    let glyph = ShapedGlyph::new_resolved(1, 2.0, 3.0, 4.0, 5, 6)?;
    assert_eq!((glyph.glyph_id(), glyph.x(), glyph.y()), (1, 2.0, 3.0));
    assert_eq!(
        (
            glyph.advance(),
            glyph.source_utf16(),
            glyph.resolved_family()
        ),
        (4.0, 5, 6)
    );
    let layout = LineLayout::new(vec![glyph], 4.0, 3.0, 1.0, 1)?;
    assert_eq!(
        (layout.width(), layout.ascent(), layout.descent()),
        (4.0, 3.0, 1.0)
    );
    let mut cache = LineLayoutCache::new(std::num::NonZeroUsize::new(4096).ok_or("cache budget")?);
    let text = Buffer::new("hello").snapshot();
    let cached = cache.layout_line(
        &text,
        0,
        font,
        PositiveFinite::new(80.0).ok_or("wrap")?,
        &mut ConsumerShaper,
    )?;
    assert_eq!(cached.width(), 5.0);
    let one = NonZeroU32::new(1).ok_or("one")?;
    let bitmap = GlyphBitmap::new(one, one, vec![255])?;
    let raster = RasterizedGlyph::new(Some(bitmap.clone()), 1.0, 2.0)?;
    assert_eq!(
        (raster.bitmap(), raster.left(), raster.top()),
        (Some(&bitmap), 1.0, 2.0)
    );
    let budget = std::num::NonZeroUsize::new(70_000).ok_or("budget")?;
    let mut atlas = GlyphAtlas::new(budget);
    let rect = atlas.insert(GlyphKey::new(font, 1, 0), &bitmap)?;
    assert_eq!(
        (rect.x(), rect.y(), rect.width(), rect.height()),
        (0, 0, one, one)
    );
    let snapshot = atlas.snapshot();
    assert_eq!(snapshot.dimension(), 256);
    assert_eq!(snapshot.budget_bytes(), budget.get());
    Ok(())
}
