//! Keep glyph batches in painter order while atlas admission is still pending.

use alpine_core::{LinearRgba, Size};
use alpine_scene::{
    Clip, ClipId, Glyph, GlyphAtlasImage, Quad, QuadId, Scene, SceneBuilder, SceneError,
    SceneRevision,
};

use crate::{EditorRenderError, PendingGlyph};

pub(super) struct DeferredScene {
    base: SceneBuilder,
    quad_count: usize,
    barriers: Vec<(usize, usize)>,
}

impl DeferredScene {
    pub(super) fn new(revision: SceneRevision, viewport: Size) -> Self {
        Self {
            base: SceneBuilder::new(revision, viewport),
            quad_count: 0,
            barriers: Vec::new(),
        }
    }

    pub(super) fn push_clip(&mut self, clip: Clip) -> ClipId {
        self.base.push_clip(clip)
    }

    pub(super) fn push_quad(&mut self, quad: Quad) -> Result<QuadId, SceneError> {
        let id = self.base.push_quad(quad)?;
        self.quad_count += 1;
        Ok(id)
    }

    /// Place all glyphs collected so far before subsequent overlay backgrounds.
    pub(super) fn flush_glyphs(&mut self, pending: &[PendingGlyph]) {
        if self.barriers.last().map_or(0, |(_, end)| *end) != pending.len() {
            self.barriers.push((self.quad_count, pending.len()));
        }
    }

    pub(super) fn finish(
        mut self,
        atlas: Option<GlyphAtlasImage>,
        pending: &[PendingGlyph],
        default_color: LinearRgba,
    ) -> Result<Scene, EditorRenderError> {
        self.flush_glyphs(pending);
        let base = self.base.finish();
        let mut scene = SceneBuilder::new(base.revision(), base.viewport());
        for clip in base.clips() {
            scene.push_clip(*clip);
        }
        if !pending.is_empty() {
            scene.set_glyph_atlas(atlas.ok_or(EditorRenderError::Domain)?)?;
        }
        let mut quad_start = 0;
        let mut glyph_start = 0;
        for (quad_end, glyph_end) in self.barriers {
            for quad in &base.quads()[quad_start..quad_end] {
                scene.push_quad(*quad)?;
            }
            for glyph in &pending[glyph_start..glyph_end] {
                scene.push_glyph(
                    Glyph::new(
                        glyph.bounds,
                        glyph.atlas_bounds,
                        glyph.color.unwrap_or(default_color),
                    )
                    .clipped(glyph.clip),
                )?;
            }
            quad_start = quad_end;
            glyph_start = glyph_end;
        }
        for quad in &base.quads()[quad_start..] {
            scene.push_quad(*quad)?;
        }
        Ok(scene.finish())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EditorApp, tests::TestTextSystem};
    use alpine_scene::PaintOperation;

    #[test]
    fn palette_background_covers_document_text_and_precedes_its_own_text()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut app = EditorApp::new(TestTextSystem)?;
        let viewport = app.last_viewport;
        let document = app.try_scene(SceneRevision::new(1), viewport)?;
        let document_glyphs = document.glyphs().len();
        assert!(document_glyphs > 0);
        app.command_palette.open(app.command_context())?;
        let scene = app.try_scene(SceneRevision::new(2), viewport)?;
        let background = scene.operations().iter().position(|operation| {
            matches!(operation, PaintOperation::Quad(id)
                if scene.quads()[id.index()].color() == app.settings.active().theme.command_palette_background)
        }).ok_or("palette background")?;
        let last_document = scene.operations().iter().position(|operation| {
            matches!(operation, PaintOperation::Glyph(id) if id.index() + 1 == document_glyphs)
        }).ok_or("document glyph")?;
        let first_overlay = scene.operations().iter().position(|operation| {
            matches!(operation, PaintOperation::Glyph(id) if id.index() == document_glyphs)
        }).ok_or("overlay glyph")?;
        assert!(
            last_document < background,
            "editor glyphs must be occluded by the palette"
        );
        assert!(
            background < first_overlay,
            "palette text must cover its background"
        );
        Ok(())
    }
}
