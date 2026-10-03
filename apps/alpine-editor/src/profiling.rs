#[cfg(test)]
use std::{cell::RefCell, rc::Rc};
use std::{
    ffi::OsString,
    fs::{self, File},
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use alpine_platform_macos::{EditorSignpost, EditorSignposts, RecorderSnapshot};
use alpine_text_layout::{
    FontKey, GlyphRasterizer, LayoutError, LineLayout, RasterizedGlyph, TextShaper,
};

use super::EditorTextSystem;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct TextSystemSnapshot {
    pub(super) shape_calls: u64,
    pub(super) rasterize_calls: u64,
}

pub(super) struct MeasuredTextSystem {
    inner: Box<dyn EditorTextSystem>,
    enabled: bool,
    shape_calls: u64,
    rasterize_calls: u64,
}

impl MeasuredTextSystem {
    pub(super) fn new(inner: impl EditorTextSystem + 'static, enabled: bool) -> Self {
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
    fn caret_offset(
        &mut self,
        text: &str,
        font: FontKey,
        index: usize,
    ) -> Result<f32, LayoutError> {
        if self.enabled {
            self.shape_calls = self.shape_calls.saturating_add(1);
        }
        self.inner.caret_offset(text, font, index)
    }

    fn index_at_x(
        &mut self,
        text: &str,
        font: FontKey,
        x: f32,
    ) -> Result<Option<usize>, LayoutError> {
        if self.enabled {
            self.shape_calls = self.shape_calls.saturating_add(1);
        }
        self.inner.index_at_x(text, font, x)
    }

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
pub(super) struct EditorProfiler {
    native: EditorSignposts,
    #[cfg(test)]
    records: Option<Rc<RefCell<Vec<EditorSignpost>>>>,
    #[cfg(test)]
    enabled_override: Option<bool>,
}

impl EditorProfiler {
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

    pub(super) fn record(&self, point: EditorSignpost) {
        #[cfg(test)]
        if let Some(records) = self.records.as_ref() {
            records.borrow_mut().push(point);
            return;
        }
        let _ = self.native.emit(point);
    }

    #[cfg(test)]
    pub(super) fn recording() -> (Self, Rc<RefCell<Vec<EditorSignpost>>>) {
        let records = Rc::new(RefCell::new(Vec::new()));
        (
            Self {
                native: EditorSignposts::default(),
                records: Some(Rc::clone(&records)),
                enabled_override: None,
            },
            records,
        )
    }

    #[cfg(test)]
    pub(super) fn disabled() -> Self {
        Self {
            native: EditorSignposts::default(),
            records: None,
            enabled_override: Some(false),
        }
    }
}

/// Writes `snapshot` as `perf-<unix s>-frames.tsv` and `-samples.tsv` under
/// `~/Library/Logs/Alpine Editor/`, creating the folder, and returns a status
/// line naming each file saved, even when the samples file then fails.
pub(super) fn save_performance_log(
    home: Option<OsString>,
    now: SystemTime,
    snapshot: &RecorderSnapshot,
) -> String {
    let directory = match log_directory(home) {
        Ok(directory) => directory,
        Err(error) => return format!("Could not save the performance log: {error}"),
    };
    let stamp = now
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    let frames = format!("perf-{stamp}-frames.tsv");
    let samples = format!("perf-{stamp}-samples.tsv");
    let path = |name: &str| directory.join(name);
    if let Err(error) = write_tsv(&path(&frames), |out| snapshot.write_frames_tsv(out)) {
        return format!("Could not save the performance log: {error}");
    }
    let shown = directory.display();
    match write_tsv(&path(&samples), |out| snapshot.write_samples_tsv(out)) {
        Ok(()) => format!("Saved {frames} and {samples} in {shown}"),
        Err(error) => format!("Saved {frames} in {shown}, but not {samples}: {error}"),
    }
}

fn log_directory(home: Option<OsString>) -> io::Result<PathBuf> {
    let home = home
        .filter(|home| !home.is_empty())
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is not set"))?;
    let directory = PathBuf::from(home).join("Library/Logs/Alpine Editor");
    fs::create_dir_all(&directory)?;
    Ok(directory)
}

fn write_tsv(
    path: &Path,
    write: impl FnOnce(&mut BufWriter<File>) -> io::Result<()>,
) -> io::Result<()> {
    let mut out = BufWriter::new(File::create(path)?);
    write(&mut out)?;
    out.flush()
}
