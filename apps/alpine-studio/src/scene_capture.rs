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

#[cfg(alpine_native_validation)]
static NEXT_CAPTURE: AtomicUsize = AtomicUsize::new(0);

#[cfg(alpine_native_validation)]
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

fn write_capture(
    directory: &Path,
    scene: &Scene,
    visible_lines: usize,
    process_id: u32,
    index: usize,
) -> io::Result<()> {
    if index >= MAX_CAPTURES
        || visible_lines == 0
        || scene.operations().len() > MAX_OPERATIONS
        || scene.clips().len() > MAX_CLIPS
    {
        return Err(invalid(
            "scene is outside capture bounds or has no rendered editor lines",
        ));
    }
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

fn write_atlas(writer: &mut impl Write, atlas: &GlyphAtlasImage) -> io::Result<()> {
    let width = usize::try_from(atlas.width().get()).map_err(io::Error::other)?;
    let height = usize::try_from(atlas.height().get()).map_err(io::Error::other)?;
    let bytes = width
        .checked_mul(height)
        .filter(|bytes| *bytes <= MAX_ATLAS_BYTES)
        .ok_or_else(|| invalid("atlas dimensions exceed capture bounds"))?;
    let base = atlas.pixels();
    if base.len() != bytes || atlas.row_patches().len() > MAX_ROW_PATCHES {
        return Err(invalid("atlas backing or row patch count is invalid"));
    }
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
mod tests {
    use super::*;
    use alpine_core::{Point, Size};
    use alpine_scene::{Clip, Quad, SceneBuilder, SceneRevision};
    use std::{error::Error, path::PathBuf, time::SystemTime};

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Result<Self, Box<dyn Error>> {
            let nonce = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)?
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "alpine-scene-capture-{}-{nonce}",
                std::process::id()
            ));
            fs::create_dir(&path)?;
            Ok(Self(path))
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn scene() -> Result<Scene, Box<dyn Error>> {
        let size = Size::new(960.0, 540.0).ok_or("valid viewport")?;
        let rect = Rect::new(Point::new(3.25, 8.5).ok_or("valid point")?, size);
        let mut builder = SceneBuilder::new(SceneRevision::new(7), size);
        let clip = builder.push_clip(Clip::new(rect));
        builder.push_quad(
            Quad::new(
                rect,
                LinearRgba::new(0.25, 0.5, 0.75, 0.5).ok_or("valid color")?,
            )
            .clipped(clip),
        )?;
        builder.push_quad(Quad::new(
            Rect::new(Point::new(8.0, 11.0).ok_or("valid point")?, size),
            LinearRgba::new(0.0, 0.0, 0.0, 1.0).ok_or("valid color")?,
        ))?;
        Ok(builder.finish())
    }

    #[test]
    fn capture_admission_stops_at_the_exact_bound() {
        let counter = AtomicUsize::new(0);
        for expected in 0..MAX_CAPTURES {
            assert_eq!(next_index(&counter), Some(expected));
        }
        assert_eq!(next_index(&counter), None);
        assert_eq!(next_index(&counter), None);
        assert_eq!(counter.load(Ordering::Relaxed), MAX_CAPTURES);
        assert_eq!(next_index(&AtomicUsize::new(usize::MAX)), None);
    }

    #[test]
    fn byte_bound_rejects_the_next_write_without_partial_growth() -> io::Result<()> {
        let mut writer = BoundedWriter {
            inner: Vec::new(),
            written: 0,
            limit: 4,
        };
        writer.write_all(b"1234")?;
        assert!(writer.write_all(b"5").is_err());
        writer.flush()?;
        assert_eq!(writer.inner, b"1234");
        assert_eq!(writer.written, 4);
        Ok(())
    }

    #[test]
    fn cumulative_rows_override_the_base_without_losing_earlier_changes() -> io::Result<()> {
        let base = [0, 1, 2, 3, 4, 5, 6, 7];
        let first = [10, 11, 12, 13];
        let later = [20, 21];
        let patches = [(1, first.as_slice()), (3, later.as_slice())];
        let mut pixels = Vec::new();
        for row in 0..4 {
            pixels.extend_from_slice(atlas_row(&base, 2, row, &patches)?);
        }
        assert_eq!(pixels, [0, 1, 10, 11, 12, 13, 20, 21]);
        assert!(atlas_row(&base, 0, 0, &patches).is_err());
        assert!(atlas_row(&base, 3, 0, &patches).is_err());
        assert!(atlas_row(&base, 2, 4, &patches).is_err());
        assert!(atlas_row(&base, 2, 0, &[(0, &[1])]).is_err());
        Ok(())
    }

    #[test]
    fn geometry_preserves_order_clips_colors_and_explicit_omissions() -> Result<(), Box<dyn Error>>
    {
        let scene = scene()?;
        let mut bytes = Vec::new();
        write_geometry(&mut bytes, &scene, 12, 321, 2, "scene-321-0002.a8")?;
        let value: serde_json::Value = serde_json::from_slice(&bytes)?;
        assert_eq!(value["process_id"], 321);
        assert_eq!(value["capture_index"], 2);
        assert_eq!(value["scene_revision"], 7);
        assert_eq!(value["visible_editor_lines"], 12);
        assert_eq!(value["viewport"]["width"], 960.0);
        assert_eq!(value["viewport"]["height"], 540.0);
        assert!(value["viewport"]["backing_scale_factor"].is_null());
        assert_eq!(value["renderer_trace_admitted"], false);
        assert_eq!(value["timing_invalidated"], true);
        assert_eq!(value["counts"]["operations"], 2);
        assert_eq!(value["counts"]["quads"], 2);
        assert_eq!(value["counts"]["glyphs"], 0);
        assert_eq!(value["counts"]["clips"], 1);
        assert!(value["atlas"].is_null());
        assert_eq!(value["clips"][0]["x"], 3.25);
        assert_eq!(value["operations"][0]["sequence"], 0);
        assert_eq!(value["operations"][0]["source_id"], 0);
        assert_eq!(value["operations"][0]["kind"], "solid-quad");
        assert_eq!(value["operations"][0]["clip"], 0);
        assert_eq!(value["operations"][0]["x"], 3.25);
        assert_eq!(value["operations"][0]["y"], 8.5);
        assert_eq!(value["operations"][0]["red"], 0.25);
        assert_eq!(value["operations"][0]["green"], 0.5);
        assert_eq!(value["operations"][0]["blue"], 0.75);
        assert_eq!(value["operations"][0]["alpha"], 0.5);
        assert_eq!(value["operations"][1]["sequence"], 1);
        assert_eq!(value["operations"][1]["source_id"], 1);
        assert_eq!(value["operations"][1]["x"], 8.0);
        assert!(value["operations"][1]["clip"].is_null());
        assert_eq!(value["omissions"].as_array().ok_or("omissions")?.len(), 7);
        Ok(())
    }

    #[test]
    fn publication_rejects_existing_files_and_retains_failed_bytes() -> Result<(), Box<dyn Error>> {
        let root = TestDirectory::new()?;
        fs::write(root.0.join("existing"), b"original")?;
        assert!(publish(&root.0, "existing", 8, |writer| writer.write_all(b"new")).is_err());
        assert_eq!(fs::read(root.0.join("existing"))?, b"original");
        assert_eq!(fs::read(root.0.join("existing.incomplete"))?, b"new");
        assert!(publish(&root.0, "existing", 8, |writer| writer.write_all(b"other")).is_err());
        assert_eq!(fs::read(root.0.join("existing.incomplete"))?, b"new");
        assert!(
            publish(&root.0, "failed", 4, |writer| {
                writer.write_all(b"part")?;
                writer.write_all(b"overflow")
            })
            .is_err()
        );
        assert!(!root.0.join("failed").exists());
        assert!(root.0.join("failed.incomplete").exists());
        Ok(())
    }

    #[test]
    fn complete_capture_is_pid_bound_and_fallback_is_not_published() -> Result<(), Box<dyn Error>> {
        let root = TestDirectory::new()?;
        let scene = scene()?;
        assert!(write_capture(&root.0, &scene, 0, 123, 0).is_err());
        assert!(write_capture(&root.0, &scene, 1, 123, MAX_CAPTURES).is_err());
        assert_eq!(fs::read_dir(&root.0)?.count(), 0);
        write_capture(&root.0, &scene, 1, 123, 0)?;
        let path = root.0.join("scene-123-0000.json");
        let bytes = fs::read(&path)?;
        let value: serde_json::Value = serde_json::from_slice(&bytes)?;
        assert_eq!(value["process_id"], 123);
        assert!(!root.0.join("scene-123-0000.json.incomplete").exists());
        assert!(write_capture(&root.0, &scene, 1, 123, 0).is_err());
        assert_eq!(fs::read(path)?, bytes);
        Ok(())
    }

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    #[test]
    fn native_initial_scene_exports_real_glyphs_and_exact_atlas_rows() -> Result<(), Box<dyn Error>>
    {
        let root = TestDirectory::new()?;
        let scene = crate::initial_scene()?;
        let atlas = scene.glyph_atlas().ok_or("native glyph atlas")?;
        assert!(!scene.glyphs().is_empty());
        write_capture(&root.0, &scene, 1, 456, 0)?;
        let pixels = fs::read(root.0.join("scene-456-0000.a8"))?;
        assert_eq!(pixels.len(), atlas.pixels().len());
        let mut expected = atlas.pixels().to_vec();
        let width = usize::try_from(atlas.width().get())?;
        for patch in atlas.row_patches() {
            let start = usize::try_from(patch.start_row())? * width;
            let end = start + patch.pixels().len();
            expected
                .get_mut(start..end)
                .ok_or("patch range")?
                .copy_from_slice(patch.pixels());
        }
        assert_eq!(pixels, expected);
        let value: serde_json::Value =
            serde_json::from_slice(&fs::read(root.0.join("scene-456-0000.json"))?)?;
        assert_eq!(value["counts"]["glyphs"], scene.glyphs().len());
        assert_eq!(value["atlas"]["bytes"], pixels.len());
        let operations = value["operations"].as_array().ok_or("operations")?;
        for (actual, expected) in operations.iter().zip(scene.operations()) {
            if let PaintOperation::Glyph(id) = *expected {
                let glyph = scene.glyphs()[id.index()];
                assert_eq!(actual["kind"], "monochrome-glyph");
                assert_eq!(
                    serde_json::from_value::<f32>(actual["x"].clone())?.to_bits(),
                    glyph.bounds().origin().x().to_bits()
                );
                assert_eq!(actual["atlas_x"], glyph.atlas_bounds().x());
                assert_eq!(actual["atlas_y"], glyph.atlas_bounds().y());
                assert_eq!(actual["atlas_width"], glyph.atlas_bounds().width().get());
                assert_eq!(actual["atlas_height"], glyph.atlas_bounds().height().get());
            }
        }
        Ok(())
    }
}
