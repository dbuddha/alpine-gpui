//! Bounded, validation-only exports from the production scene boundary.
//!
//! These are raw observations, not admitted renderer traces or timing samples.
//! The ordinary shipping build does not compile this module or its frame hook.

use std::{
    fs::{self, OpenOptions},
    io::{self, BufWriter, Write},
    path::Path,
    sync::atomic::{AtomicUsize, Ordering},
};

use alpine_core::{LinearRgba, Rect};
use alpine_scene::{GlyphAtlasImage, PaintOperation, Scene};

const MAX_CAPTURES: usize = 16;
const MAX_OPERATIONS: usize = 65_536;
const MAX_CLIPS: usize = 4_096;
const MAX_ATLAS_BYTES: usize = 16_777_216;
const MAX_GEOMETRY_BYTES: usize = 33_554_432;
const MAX_ROW_PATCHES: usize = 64;

#[cfg(any(alpine_native_validation, not(miri)))]
static NEXT_CAPTURE: AtomicUsize = AtomicUsize::new(0);

#[cfg(any(alpine_native_validation, not(miri)))]
pub(super) fn record_scene(scene: &Scene, visible_lines: usize) {
    let Some(directory) = std::env::var_os("ALPINE_STUDIO_NATIVE_SCENE_CAPTURE_DIR") else {
        return;
    };
    let Some(index) = next_index(&NEXT_CAPTURE) else {
        return;
    };
    if let Err(error) = write_capture(
        Path::new(&directory),
        scene,
        visible_lines,
        std::process::id(),
        index,
    ) {
        eprintln!("alpine-native-scene-capture-failed index={index} error={error}");
    }
}

fn next_index(counter: &AtomicUsize) -> Option<usize> {
    counter
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
            (value < MAX_CAPTURES).then(|| value + 1)
        })
        .ok()
}

fn validate_capture_shape(
    index: usize,
    visible_lines: usize,
    operations: usize,
    clips: usize,
) -> io::Result<()> {
    if index >= MAX_CAPTURES
        || visible_lines == 0
        || operations > MAX_OPERATIONS
        || clips > MAX_CLIPS
    {
        return Err(invalid(
            "scene is outside capture bounds or has no rendered editor lines",
        ));
    }
    Ok(())
}

fn write_capture(
    directory: &Path,
    scene: &Scene,
    visible_lines: usize,
    process_id: u32,
    index: usize,
) -> io::Result<()> {
    validate_capture_shape(
        index,
        visible_lines,
        scene.operations().len(),
        scene.clips().len(),
    )?;
    let stem = format!("scene-{process_id}-{index:04}");
    let atlas_name = format!("{stem}.a8");
    if let Some(atlas) = scene.glyph_atlas() {
        publish(directory, &atlas_name, MAX_ATLAS_BYTES, |writer| {
            write_atlas(writer, atlas)
        })?;
    }
    publish(
        directory,
        &format!("{stem}.json"),
        MAX_GEOMETRY_BYTES,
        |writer| write_geometry(writer, scene, visible_lines, process_id, index, &atlas_name),
    )
}

fn publish(
    directory: &Path,
    name: &str,
    limit: usize,
    write_contents: impl FnOnce(&mut BoundedWriter<BufWriter<std::fs::File>>) -> io::Result<()>,
) -> io::Result<()> {
    let partial = directory.join(format!("{name}.incomplete"));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let file = options.open(&partial)?;
    let mut writer = BoundedWriter {
        inner: BufWriter::with_capacity(32_768, file),
        written: 0,
        limit,
    };
    write_contents(&mut writer)?;
    writer.flush()?;
    writer.inner.get_ref().sync_all()?;
    drop(writer);
    // Unlike rename, hard-link publication cannot overwrite a prior capture.
    // On failure, preserve the incomplete file and the original destination.
    fs::hard_link(&partial, directory.join(name))?;
    fs::remove_file(partial)
}

struct BoundedWriter<W> {
    inner: W,
    written: usize,
    limit: usize,
}

impl<W: Write> Write for BoundedWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.written) {
            return Err(invalid("capture byte limit exceeded"));
        }
        let written = self.inner.write(bytes)?;
        self.written += written;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

fn validate_atlas_shape(
    width: usize,
    height: usize,
    backing: usize,
    patches: usize,
) -> io::Result<()> {
    let bytes = width
        .checked_mul(height)
        .filter(|bytes| *bytes <= MAX_ATLAS_BYTES)
        .ok_or_else(|| invalid("atlas dimensions exceed capture bounds"))?;
    if backing != bytes || patches > MAX_ROW_PATCHES {
        return Err(invalid("atlas backing or row patch count is invalid"));
    }
    Ok(())
}

fn write_atlas(writer: &mut impl Write, atlas: &GlyphAtlasImage) -> io::Result<()> {
    let width = usize::try_from(atlas.width().get()).map_err(io::Error::other)?;
    let height = usize::try_from(atlas.height().get()).map_err(io::Error::other)?;
    let base = atlas.pixels();
    validate_atlas_shape(width, height, base.len(), atlas.row_patches().len())?;
    let patches = atlas
        .row_patches()
        .iter()
        .map(|patch| {
            usize::try_from(patch.start_row())
                .map(|start| (start, patch.pixels()))
                .map_err(io::Error::other)
        })
        .collect::<io::Result<Vec<_>>>()?;
    // Select a patch once per row. Do not clone the atlas, expand A8 bytes into
    // JSON numbers, or use only the newest delta over an older base image.
    for row in 0..height {
        writer.write_all(atlas_row(base, width, row, &patches)?)?;
    }
    Ok(())
}

fn atlas_row<'a>(
    base: &'a [u8],
    width: usize,
    row: usize,
    patches: &[(usize, &'a [u8])],
) -> io::Result<&'a [u8]> {
    if width == 0 || !base.len().is_multiple_of(width) || row >= base.len() / width {
        return Err(invalid("atlas row is outside its backing"));
    }
    for &(start, pixels) in patches {
        if !pixels.len().is_multiple_of(width) {
            return Err(invalid("atlas patch does not contain complete rows"));
        }
        if let Some(offset) = row.checked_sub(start) {
            let rows = pixels.len() / width;
            if offset < rows {
                return Ok(&pixels[offset * width..(offset + 1) * width]);
            }
        }
    }
    Ok(&base[row * width..(row + 1) * width])
}

fn write_geometry(
    writer: &mut impl Write,
    scene: &Scene,
    visible_lines: usize,
    process_id: u32,
    index: usize,
    atlas_name: &str,
) -> io::Result<()> {
    writeln!(
        writer,
        "{{\"schema\":\"alpine-studio-scene-capture/v1\",\"origin\":\"studio-app-delegate-frame\",\"process_id\":{process_id},\"capture_index\":{index},\"capture_limit\":{MAX_CAPTURES},\"scene_revision\":{},\"visible_editor_lines\":{visible_lines},\"renderer_trace_admitted\":false,\"timing_invalidated\":true,",
        scene.revision().get()
    )?;
    writeln!(
        writer,
        "\"viewport\":{{\"width\":{},\"height\":{},\"backing_scale_factor\":null}},\"counts\":{{\"clips\":{},\"quads\":{},\"glyphs\":{},\"operations\":{}}},",
        scene.viewport().width(),
        scene.viewport().height(),
        scene.clips().len(),
        scene.quads().len(),
        scene.glyphs().len(),
        scene.operations().len()
    )?;
    writer.write_all(b"\"omissions\":[\"backing-scale-factor\",\"font-file-identities\",\"source-and-executable-identities\",\"document-identity-and-input-history\",\"native-window-identity\",\"renderer-trace-admission\",\"presentation-evidence\"],\n\"atlas\":")?;
    if let Some(atlas) = scene.glyph_atlas() {
        write!(
            writer,
            "{{\"file\":\"{atlas_name}\",\"width\":{},\"height\":{},\"revision\":{},\"bytes\":{},\"cumulative_row_patches\":{}}}",
            atlas.width().get(),
            atlas.height().get(),
            atlas.revision(),
            atlas.pixels().len(),
            atlas.row_patches().len()
        )?;
    } else {
        writer.write_all(b"null")?;
    }
    writer.write_all(b",\n\"clips\":[")?;
    for (index, clip) in scene.clips().iter().enumerate() {
        separator(writer, index)?;
        write!(writer, "{{\"id\":{index},")?;
        write_rect(writer, clip.bounds())?;
        writer.write_all(b"}")?;
    }
    writer.write_all(b"],\n\"operations\":[")?;
    for (sequence, operation) in scene.operations().iter().enumerate() {
        separator(writer, sequence)?;
        write!(writer, "{{\"sequence\":{sequence},")?;
        match *operation {
            PaintOperation::Quad(id) => {
                let quad = scene
                    .quads()
                    .get(id.index())
                    .ok_or_else(|| invalid("paint references a missing quad"))?;
                write!(
                    writer,
                    "\"kind\":\"solid-quad\",\"source_id\":{},",
                    id.index()
                )?;
                write_rect(writer, quad.bounds())?;
                write_color(writer, quad.color())?;
                write_clip(writer, quad.clip().map(alpine_scene::ClipId::index))?;
            }
            PaintOperation::Glyph(id) => {
                let glyph = scene
                    .glyphs()
                    .get(id.index())
                    .ok_or_else(|| invalid("paint references a missing glyph"))?;
                write!(
                    writer,
                    "\"kind\":\"monochrome-glyph\",\"source_id\":{},",
                    id.index()
                )?;
                write_rect(writer, glyph.bounds())?;
                write_color(writer, glyph.color())?;
                write_clip(writer, glyph.clip().map(alpine_scene::ClipId::index))?;
                let bounds = glyph.atlas_bounds();
                write!(
                    writer,
                    ",\"atlas_x\":{},\"atlas_y\":{},\"atlas_width\":{},\"atlas_height\":{}",
                    bounds.x(),
                    bounds.y(),
                    bounds.width().get(),
                    bounds.height().get()
                )?;
            }
        }
        writer.write_all(b"}")?;
    }
    writer.write_all(b"]\n}\n")
}

fn separator(writer: &mut impl Write, index: usize) -> io::Result<()> {
    if index != 0 {
        writer.write_all(b",")?;
    }
    Ok(())
}

fn write_rect(writer: &mut impl Write, rect: Rect) -> io::Result<()> {
    write!(
        writer,
        "\"x\":{},\"y\":{},\"width\":{},\"height\":{}",
        rect.origin().x(),
        rect.origin().y(),
        rect.size().width(),
        rect.size().height()
    )
}

fn write_color(writer: &mut impl Write, color: LinearRgba) -> io::Result<()> {
    write!(
        writer,
        ",\"red\":{},\"green\":{},\"blue\":{},\"alpha\":{}",
        color.red(),
        color.green(),
        color.blue(),
        color.alpha()
    )
}

fn write_clip(writer: &mut impl Write, clip: Option<usize>) -> io::Result<()> {
    writer.write_all(b",\"clip\":")?;
    if let Some(index) = clip {
        write!(writer, "{index}")
    } else {
        writer.write_all(b"null")
    }
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
#[path = "scene_capture_coverage_tests.rs"]
mod tests;
