use std::{
    fs,
    num::NonZeroU32,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::Instant,
};

use alpine_platform_macos::{EventTimestamp, ImeEvent, KeyState, Modifiers, SurfaceEvent};
use alpine_text::{ByteOffset, Selection};
use alpine_text_layout::{
    FontKey, GlyphBitmap, GlyphRasterizer, LayoutError, LineLayout, RasterizedGlyph, ShapedGlyph,
    TextShaper,
};

use super::*;

static NEXT_TEMP: AtomicU64 = AtomicU64::new(1);

struct TempFile {
    root: PathBuf,
    path: PathBuf,
}

impl TempFile {
    fn new(contents: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "alpine-command-palette-{}-{id}",
            std::process::id()
        ));
        fs::create_dir(&root)?;
        let path = root.join("main.rs");
        fs::write(&path, contents)?;
        Ok(Self { root, path })
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[derive(Default)]
struct PaletteTextSystem;

impl TextShaper for PaletteTextSystem {
    fn shape(&mut self, text: &str, _font: FontKey) -> Result<LineLayout, LayoutError> {
        let mut glyphs = Vec::new();
        let mut x = 0.0;
        let mut utf16 = 0_u32;
        for character in text.chars() {
            glyphs.push(ShapedGlyph::new_resolved(
                u32::from(character),
                x,
                0.0,
                8.0,
                utf16,
                FONT_FAMILY,
            )?);
            x += 8.0;
            utf16 = utf16
                .checked_add(
                    u32::try_from(character.len_utf16())
                        .map_err(|_| LayoutError::ArithmeticOverflow)?,
                )
                .ok_or(LayoutError::ArithmeticOverflow)?;
        }
        LineLayout::new(glyphs, x, 15.0, 4.0, 1_024)
    }
}

impl GlyphRasterizer for PaletteTextSystem {
    fn rasterize(
        &mut self,
        _font: FontKey,
        _glyph_id: u32,
        _subpixel_x: u8,
    ) -> Result<RasterizedGlyph, LayoutError> {
        let width = NonZeroU32::new(2).ok_or(LayoutError::InvalidShaperOutput)?;
        let height = NonZeroU32::new(3).ok_or(LayoutError::InvalidShaperOutput)?;
        let bitmap = GlyphBitmap::new(width, height, vec![255; 6])?;
        RasterizedGlyph::new(Some(bitmap), 0.0, 3.0)
    }
}

fn key(physical_key: u16, modifiers: Modifiers) -> SurfaceEvent {
    SurfaceEvent::Keyboard {
        timestamp: EventTimestamp::new(1),
        state: KeyState::Down,
        physical_key,
        logical_key: "test".into(),
        modifiers,
        repeat: false,
    }
}

fn ime(event: ImeEvent) -> SurfaceEvent {
    SurfaceEvent::Ime {
        timestamp: EventTimestamp::new(2),
        event,
    }
}

fn command_shift() -> Modifiers {
    Modifiers::from_bits(Modifiers::COMMAND | Modifiers::SHIFT)
}

#[test]
fn command_palette_routes_save_through_existing_editor_state()
-> Result<(), Box<dyn std::error::Error>> {
    let file = TempFile::new("before")?;
    let mut app = StudioApp::open_file(PaletteTextSystem, &file.path)?;
    app.selection = Selection::new(ByteOffset::new(0), ByteOffset::new(6));
    assert!(
        app.handle_event(&ime(ImeEvent::Committed("after".into())))
            .visual_changed
    );
    assert!(
        app.handle_event(&key(KEY_P, command_shift()))
            .visual_changed
    );
    let before = app.buffer().snapshot().text();
    assert!(
        app.handle_event(&ime(ImeEvent::Committed("save".into())))
            .visual_changed
    );
    assert_eq!(app.buffer().snapshot().text(), before);
    assert!(
        app.handle_event(&key(KEY_RETURN, Modifiers::default()))
            .visual_changed
    );
    assert_eq!(fs::read_to_string(&file.path)?, "after");
    assert!(!app.command_palette.is_open());
    assert_eq!(app.command_palette.report().executions, 1);
    assert_eq!(app.command_palette.report().retained_bytes, 0);
    Ok(())
}

#[test]
fn command_palette_focus_cancel_and_scene_are_bounded() -> Result<(), Box<dyn std::error::Error>> {
    let mut app = StudioApp::new(PaletteTextSystem)?;
    app.selection = Selection::new(ByteOffset::new(0), ByteOffset::new(2));
    let before = app.buffer().snapshot().text();
    assert!(
        app.handle_event(&key(KEY_P, command_shift()))
            .visual_changed
    );
    assert!(app.handle_event(&ime(ImeEvent::Started)).visual_changed);
    assert!(
        app.handle_event(&ime(ImeEvent::Updated {
            text: "find".into(),
            selected_start_utf16: 0,
            selected_length_utf16: 4,
        }))
        .visual_changed
    );
    assert_eq!(app.buffer().snapshot().text(), before);
    let scene = app.try_scene(
        SceneRevision::new(901),
        Size::new(WINDOW_WIDTH, WINDOW_HEIGHT).ok_or("viewport")?,
    )?;
    assert!(!scene.glyphs().is_empty());
    assert!(
        app.command_palette.report().visible_rows
            <= commands::MAX_VISIBLE_COMMANDS + commands::MAX_VISIBLE_OVERSCAN * 2
    );
    assert!(
        app.handle_event(&key(KEY_ESCAPE, Modifiers::default()))
            .visual_changed
    );
    assert_eq!(app.buffer().snapshot().text(), before);
    assert_eq!(app.command_palette.report().retained_bytes, 0);
    assert_eq!(app.command_palette.report().cancellations, 1);
    Ok(())
}

#[test]
fn command_availability_refresh_prevents_stale_execution() -> Result<(), Box<dyn std::error::Error>>
{
    let file = TempFile::new("clean")?;
    let mut app = StudioApp::open_file(PaletteTextSystem, &file.path)?;
    app.selection = Selection::new(ByteOffset::new(0), ByteOffset::new(5));
    assert!(
        app.handle_event(&ime(ImeEvent::Committed("dirty".into())))
            .visual_changed
    );
    assert!(
        app.handle_event(&key(KEY_P, command_shift()))
            .visual_changed
    );
    assert!(
        app.handle_event(&ime(ImeEvent::Committed("save".into())))
            .visual_changed
    );
    assert_eq!(
        app.command_palette.visible_commands()?[0].command,
        StudioCommand::SaveFile
    );
    app.document.save()?;
    let effect = app.handle_event(&key(KEY_RETURN, Modifiers::default()));
    assert!(effect.visual_changed);
    assert!(app.command_palette.is_open());
    assert_eq!(app.command_palette.report().executions, 0);
    assert_eq!(fs::read_to_string(&file.path)?, "dirty");
    Ok(())
}

#[test]
fn command_palette_stage_measurements_are_separate_and_bounded()
-> Result<(), Box<dyn std::error::Error>> {
    let mut app = StudioApp::new(PaletteTextSystem)?;
    let open_start = Instant::now();
    assert!(
        app.handle_event(&key(KEY_P, command_shift()))
            .visual_changed
    );
    let open = open_start.elapsed();
    let match_start = Instant::now();
    assert!(
        app.handle_event(&ime(ImeEvent::Committed("find".into())))
            .visual_changed
    );
    let matching = match_start.elapsed();
    let projection_start = Instant::now();
    let rows = app.command_palette.visible_commands()?;
    let projection = projection_start.elapsed();
    let scene_start = Instant::now();
    let scene = app.try_scene(
        SceneRevision::new(902),
        Size::new(WINDOW_WIDTH, WINDOW_HEIGHT).ok_or("viewport")?,
    )?;
    let scene_build = scene_start.elapsed();
    assert!(!rows.is_empty());
    assert!(!scene.glyphs().is_empty());
    for elapsed in [open, matching, projection, scene_build] {
        assert!(elapsed < std::time::Duration::from_secs(5));
    }
    let report = app.command_palette.report();
    assert!(report.query_bytes <= commands::MAX_QUERY_BYTES);
    assert!(report.retained_matches <= commands::MAX_COMMANDS);
    eprintln!(
        "command-palette stages: open={open:?} match={matching:?} projection={projection:?} scene={scene_build:?}"
    );
    Ok(())
}
