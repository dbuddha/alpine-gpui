#[cfg(test)]
use std::{cell::RefCell, rc::Rc};

use alpine_platform_macos::{StudioSignpost, StudioSignposts};
use alpine_text_layout::{
    FontKey, GlyphRasterizer, LayoutError, LineLayout, RasterizedGlyph, TextShaper,
};

use super::StudioTextSystem;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct TextSystemSnapshot {
    pub(super) shape_calls: u64,
    pub(super) rasterize_calls: u64,
}

pub(super) struct MeasuredTextSystem {
    inner: Box<dyn StudioTextSystem>,
    enabled: bool,
    shape_calls: u64,
    rasterize_calls: u64,
}

impl MeasuredTextSystem {
    pub(super) fn new(inner: impl StudioTextSystem + 'static, enabled: bool) -> Self {
        Self {
            inner: Box::new(inner),
            enabled,
            shape_calls: 0,
            rasterize_calls: 0,
        }
    }

    #[cfg(test)]
    pub(super) const fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    pub(super) const fn snapshot(&self) -> TextSystemSnapshot {
        TextSystemSnapshot {
            shape_calls: self.shape_calls,
            rasterize_calls: self.rasterize_calls,
        }
    }
}

impl TextShaper for MeasuredTextSystem {
    fn shape(&mut self, text: &str, font: FontKey) -> Result<LineLayout, LayoutError> {
        if self.enabled {
            self.shape_calls = self.shape_calls.saturating_add(1);
        }
        self.inner.shape(text, font)
    }
}

impl GlyphRasterizer for MeasuredTextSystem {
    fn rasterize(
        &mut self,
        font: FontKey,
        glyph_id: u32,
        subpixel_x: u8,
    ) -> Result<RasterizedGlyph, LayoutError> {
        if self.enabled {
            self.rasterize_calls = self.rasterize_calls.saturating_add(1);
        }
        self.inner.rasterize(font, glyph_id, subpixel_x)
    }
}

#[derive(Default)]
pub(super) struct StudioProfiler {
    native: StudioSignposts,
    #[cfg(test)]
    records: Option<Rc<RefCell<Vec<StudioSignpost>>>>,
    #[cfg(test)]
    enabled_override: Option<bool>,
}

impl StudioProfiler {
    pub(super) fn enabled(&self) -> bool {
        #[cfg(test)]
        if let Some(enabled) = self.enabled_override {
            return enabled;
        }
        #[cfg(test)]
        if self.records.is_some() {
            return true;
        }
        self.native.enabled()
    }

    pub(super) fn record(&self, point: StudioSignpost) {
        #[cfg(test)]
        if let Some(records) = self.records.as_ref() {
            records.borrow_mut().push(point);
            return;
        }
        let _ = self.native.emit(point);
    }

    #[cfg(test)]
    pub(super) fn recording() -> (Self, Rc<RefCell<Vec<StudioSignpost>>>) {
        let records = Rc::new(RefCell::new(Vec::new()));
        (
            Self {
                native: StudioSignposts::default(),
                records: Some(Rc::clone(&records)),
                enabled_override: None,
            },
            records,
        )
    }

    #[cfg(test)]
    pub(super) fn disabled() -> Self {
        Self {
            native: StudioSignposts::default(),
            records: None,
            enabled_override: Some(false),
        }
    }
}
