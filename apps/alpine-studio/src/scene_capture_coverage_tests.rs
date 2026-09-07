use super::*;
use alpine_core::{Point, Size};
use alpine_scene::{Clip, Quad, SceneBuilder, SceneRevision};
use std::{error::Error, path::PathBuf, time::SystemTime};

struct TestDirectory(PathBuf, bool);

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
        Ok(Self(path, false))
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        if self.1 || std::thread::panicking() {
            eprintln!(
                "scene capture failure artifacts retained at {}",
                self.0.display()
            );
        } else {
            let _ = fs::remove_dir_all(&self.0);
        }
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
fn geometry_preserves_order_clips_colors_and_explicit_omissions() -> Result<(), Box<dyn Error>> {
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
fn native_initial_scene_exports_real_glyphs_and_exact_atlas_rows() -> Result<(), Box<dyn Error>> {
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

#[test]
fn exact_capture_and_atlas_limits_are_checked_without_large_allocations() -> io::Result<()> {
    validate_capture_shape(MAX_CAPTURES - 1, 1, MAX_OPERATIONS, MAX_CLIPS)?;
    for (index, lines, operations, clips) in [
        (MAX_CAPTURES, 1, 0, 0),
        (0, 0, 0, 0),
        (0, 1, MAX_OPERATIONS + 1, 0),
        (0, 1, 0, MAX_CLIPS + 1),
    ] {
        assert!(validate_capture_shape(index, lines, operations, clips).is_err());
    }
    validate_atlas_shape(1, MAX_ATLAS_BYTES, MAX_ATLAS_BYTES, MAX_ROW_PATCHES)?;
    validate_atlas_shape(3, 4, 12, 0)?;
    for (width, height, backing, patches) in [
        (1, MAX_ATLAS_BYTES + 1, MAX_ATLAS_BYTES + 1, 0),
        (usize::MAX, 2, 0, 0),
        (3, 4, 11, 0),
        (3, 4, 12, MAX_ROW_PATCHES + 1),
    ] {
        assert!(validate_atlas_shape(width, height, backing, patches).is_err());
    }
    Ok(())
}

#[test]
fn row_access_rejects_each_invalid_shape_and_preserves_nonzero_base_rows() -> io::Result<()> {
    let base = [0, 1, 2, 3, 4, 5, 6, 7];
    assert_eq!(atlas_row(&base, 2, 1, &[])?, [2, 3]);
    assert_eq!(atlas_row(&base, 2, 3, &[])?, [6, 7]);
    for length in 0..=base.len() {
        for width in 1..=4 {
            for row in 0..=5 {
                let actual = atlas_row(&base[..length], width, row, &[]);
                if length % width == 0 && row < length / width {
                    assert_eq!(actual?, &base[row * width..(row + 1) * width]);
                } else {
                    assert!(actual.is_err());
                }
            }
        }
    }
    assert!(atlas_row(&base, 0, 0, &[]).is_err());
    assert!(atlas_row(&base, 2, 0, &[(3, &[1])]).is_err());
    Ok(())
}

#[test]
fn bounded_writer_delegates_flush_and_accounts_for_short_writes() -> io::Result<()> {
    struct ShortWriter {
        bytes: Vec<u8>,
        flushes: usize,
    }
    impl Write for ShortWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            let accepted = bytes.len().min(2);
            self.bytes.extend_from_slice(&bytes[..accepted]);
            Ok(accepted)
        }
        fn flush(&mut self) -> io::Result<()> {
            self.flushes += 1;
            Err(io::Error::other("injected flush failure"))
        }
    }
    let mut writer = BoundedWriter {
        inner: ShortWriter {
            bytes: Vec::new(),
            flushes: 0,
        },
        written: 0,
        limit: 4,
    };
    assert_eq!(writer.write(b"1234")?, 2);
    assert_eq!(writer.written, 2);
    assert_eq!(writer.inner.bytes, b"12");
    writer.write_all(b"34")?;
    assert_eq!(writer.written, 4);
    assert!(writer.write_all(b"5").is_err());
    assert_eq!(writer.inner.bytes, b"1234");
    assert_eq!(
        writer.flush().err().map(|e| e.to_string()),
        Some("injected flush failure".to_owned())
    );
    assert_eq!(writer.inner.flushes, 1);
    Ok(())
}

fn glyph_scene() -> Result<Scene, Box<dyn Error>> {
    use alpine_scene::{AtlasBounds, Glyph, GlyphAtlasRowPatch};
    use std::{num::NonZeroU32, sync::Arc};
    let one = NonZeroU32::new(1).ok_or("one")?;
    let two = NonZeroU32::new(2).ok_or("two")?;
    let three = NonZeroU32::new(3).ok_or("three")?;
    let four = NonZeroU32::new(4).ok_or("four")?;
    let base = GlyphAtlasImage::new(
        11,
        three,
        four,
        Arc::from([0_u8, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]),
    )?;
    let first = base.with_row_patches(
        11,
        12,
        Arc::from([GlyphAtlasRowPatch::new(1, one, Arc::from([21_u8, 22, 23]))]),
    )?;
    let atlas = first.advance_with_row_patches(
        12,
        13,
        Arc::from([GlyphAtlasRowPatch::new(3, one, Arc::from([31_u8, 32, 33]))]),
    )?;
    let size = Size::new(19.0, 17.0).ok_or("viewport")?;
    let bounds = Rect::new(
        Point::new(4.25, 5.5).ok_or("origin")?,
        Size::new(7.0, 6.0).ok_or("size")?,
    );
    let color = LinearRgba::new(0.125, 0.25, 0.5, 0.75).ok_or("color")?;
    let mut builder = SceneBuilder::new(SceneRevision::new(9), size);
    let clip = builder.push_clip(Clip::new(bounds));
    builder.set_glyph_atlas(atlas)?;
    builder.push_quad(Quad::new(bounds, color))?;
    builder
        .push_glyph(Glyph::new(bounds, AtlasBounds::new(1, 1, two, three), color).clipped(clip))?;
    builder.push_quad(Quad::new(bounds, color).clipped(clip))?;
    builder.push_glyph(Glyph::new(bounds, AtlasBounds::new(0, 0, one, two), color))?;
    Ok(builder.finish())
}

#[test]
fn portable_glyph_capture_preserves_pixels_and_complete_operation_fields()
-> Result<(), Box<dyn Error>> {
    let root = TestDirectory::new()?;
    let scene = glyph_scene()?;
    write_capture(&root.0, &scene, 2, 987, 1)?;
    let pixels = fs::read(root.0.join("scene-987-0001.a8"))?;
    assert_eq!(pixels, [0, 1, 2, 21, 22, 23, 6, 7, 8, 31, 32, 33]);
    let value: serde_json::Value =
        serde_json::from_slice(&fs::read(root.0.join("scene-987-0001.json"))?)?;
    assert_eq!(
        value["atlas"],
        serde_json::json!({"file":"scene-987-0001.a8","width":3,"height":4,"revision":13,"bytes":12,"cumulative_row_patches":2})
    );
    assert_eq!(
        value["counts"],
        serde_json::json!({"clips":1,"quads":2,"glyphs":2,"operations":4})
    );
    assert_eq!(
        value["operations"],
        serde_json::json!([
            {"sequence":0,"kind":"solid-quad","source_id":0,"x":4.25,"y":5.5,"width":7,"height":6,"red":0.125,"green":0.25,"blue":0.5,"alpha":0.75,"clip":null},
            {"sequence":1,"kind":"monochrome-glyph","source_id":0,"x":4.25,"y":5.5,"width":7,"height":6,"red":0.125,"green":0.25,"blue":0.5,"alpha":0.75,"clip":0,"atlas_x":1,"atlas_y":1,"atlas_width":2,"atlas_height":3},
            {"sequence":2,"kind":"solid-quad","source_id":1,"x":4.25,"y":5.5,"width":7,"height":6,"red":0.125,"green":0.25,"blue":0.5,"alpha":0.75,"clip":0},
            {"sequence":3,"kind":"monochrome-glyph","source_id":1,"x":4.25,"y":5.5,"width":7,"height":6,"red":0.125,"green":0.25,"blue":0.5,"alpha":0.75,"clip":null,"atlas_x":0,"atlas_y":0,"atlas_width":1,"atlas_height":2}
        ])
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(
            fs::metadata(root.0.join("scene-987-0001.a8"))?
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(root.0.join("scene-987-0001.json"))?
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    Ok(())
}

#[test]
fn every_geometry_and_atlas_write_failure_is_propagated() -> Result<(), Box<dyn Error>> {
    struct FailWriter {
        calls: usize,
        fail_at: usize,
    }
    impl Write for FailWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            let attempt = self.calls;
            self.calls += 1;
            if attempt == self.fail_at {
                return Err(io::Error::other("injected capture write failure"));
            }
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    // A swallowed error must not cause another failure on the next write.
    let mut one_shot = FailWriter {
        calls: 0,
        fail_at: 0,
    };
    assert!(one_shot.write(b"first").is_err());
    assert_eq!(one_shot.write(b"next")?, 4);
    assert_eq!(one_shot.calls, 2);
    let scene = glyph_scene()?;
    let mut complete = FailWriter {
        calls: 0,
        fail_at: usize::MAX,
    };
    write_geometry(&mut complete, &scene, 2, 123, 1, "scene-123-0001.a8")?;
    assert!(complete.calls > 0);
    for fail_at in 0..complete.calls {
        let mut failing = FailWriter { calls: 0, fail_at };
        let error = write_geometry(&mut failing, &scene, 2, 123, 1, "scene-123-0001.a8")
            .err()
            .ok_or("geometry write failure was swallowed")?;
        assert_eq!(error.to_string(), "injected capture write failure");
        assert_eq!(failing.calls, fail_at + 1, "writes continued after failure");
    }
    let atlas = scene.glyph_atlas().ok_or("atlas")?;
    let mut complete = FailWriter {
        calls: 0,
        fail_at: usize::MAX,
    };
    write_atlas(&mut complete, atlas)?;
    assert_eq!(complete.calls, 4);
    for fail_at in 0..complete.calls {
        let mut failing = FailWriter { calls: 0, fail_at };
        let error = write_atlas(&mut failing, atlas)
            .err()
            .ok_or("atlas write failure was swallowed")?;
        assert_eq!(error.to_string(), "injected capture write failure");
        assert_eq!(failing.calls, fail_at + 1, "writes continued after failure");
    }
    Ok(())
}

// Subprocess execution is not a Miri-supported operation. The pure validation,
// row publication and serializer tests remain in the existing Miri partition.
#[cfg(not(miri))]
#[test]
fn recording_entrypoint_uses_environment_and_bounds_failed_attempts() -> Result<(), Box<dyn Error>>
{
    const CHILD: &str = "ALPINE_SCENE_CAPTURE_REGRESSION_CHILD";
    const DIRECTORY: &str = "ALPINE_STUDIO_NATIVE_SCENE_CAPTURE_DIR";
    if let Ok(mode) = std::env::var(CHILD) {
        let scene = scene()?;
        for _ in 0..=MAX_CAPTURES {
            record_scene(&scene, 1);
        }
        if mode == "disabled" {
            assert_eq!(NEXT_CAPTURE.load(Ordering::Relaxed), 0);
        } else {
            assert_eq!(NEXT_CAPTURE.load(Ordering::Relaxed), MAX_CAPTURES);
            let path = PathBuf::from(std::env::var_os(DIRECTORY).ok_or("child capture directory")?);
            if mode == "enabled" {
                assert_eq!(fs::read_dir(&path)?.count(), MAX_CAPTURES);
                for index in 0..MAX_CAPTURES {
                    let name = format!("scene-{}-{index:04}.json", std::process::id());
                    let value: serde_json::Value =
                        serde_json::from_slice(&fs::read(path.join(name))?)?;
                    assert_eq!(value["process_id"], std::process::id());
                    assert_eq!(value["capture_index"], index);
                }
            } else {
                assert_eq!(mode, "failure");
                assert!(!path.exists());
            }
        }
        return Ok(());
    }
    let mut root = TestDirectory::new()?;
    for mode in ["disabled", "enabled", "failure"] {
        let path = root.0.join(mode);
        let mut command = std::process::Command::new(std::env::current_exe()?);
        command.args(["--exact", "scene_capture::tests::recording_entrypoint_uses_environment_and_bounds_failed_attempts", "--nocapture", "--test-threads=1"])
            .env(CHILD, mode).env_remove(DIRECTORY)
            .stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
        if mode != "disabled" {
            command.env(DIRECTORY, &path);
        }
        if mode == "enabled" {
            fs::create_dir(&path)?;
        }
        let mut child = command.spawn()?;
        let started = std::time::Instant::now();
        let mut timed_out = false;
        while child.try_wait()?.is_none() {
            if started.elapsed() > std::time::Duration::from_secs(10) {
                root.1 = true;
                child.kill()?;
                timed_out = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let output = child.wait_with_output()?;
        if timed_out || !output.status.success() {
            root.1 = true;
            fs::write(root.0.join(format!("{mode}.stdout")), &output.stdout)?;
            fs::write(root.0.join(format!("{mode}.stderr")), &output.stderr)?;
            let status = format!(
                "mode={mode} timed_out={timed_out} status={}\n",
                output.status
            );
            fs::write(root.0.join(format!("{mode}.status")), &status)?;
            return Err(format!(
                "capture regression child failed: {}; stdout={:?}; stderr={:?}; artifacts={}",
                status.trim(),
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
                root.0.display()
            )
            .into());
        }
        if mode == "failure" {
            assert_eq!(
                String::from_utf8_lossy(&output.stderr)
                    .lines()
                    .filter(|line| line.starts_with("alpine-native-scene-capture-failed index="))
                    .count(),
                MAX_CAPTURES
            );
        }
    }
    Ok(())
}
