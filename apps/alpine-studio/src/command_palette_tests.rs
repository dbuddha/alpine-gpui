use std::{
    fs,
    num::NonZeroU32,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::Instant,
};

use alpine_platform_macos::{
    EventTimestamp, ImeEvent, InputEpoch, KeyState, Modifiers, SurfaceEvent,
};
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
struct PaletteTextSystem {
    rejected_glyph: Option<u32>,
}

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
        glyph_id: u32,
        _subpixel_x: u8,
    ) -> Result<RasterizedGlyph, LayoutError> {
        if self.rejected_glyph == Some(glyph_id) {
            return Err(LayoutError::InvalidShaperOutput);
        }
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

fn logical_key(logical_key: &str, modifiers: Modifiers) -> SurfaceEvent {
    SurfaceEvent::Keyboard {
        timestamp: EventTimestamp::new(1),
        state: KeyState::Down,
        physical_key: u16::MAX,
        logical_key: logical_key.into(),
        modifiers,
        repeat: false,
    }
}

fn ime(event: ImeEvent) -> SurfaceEvent {
    SurfaceEvent::Ime {
        timestamp: EventTimestamp::new(2),
        input_epoch: InputEpoch::INITIAL,
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
    let mut app = StudioApp::open_file(PaletteTextSystem::default(), &file.path)?;
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
    #[cfg(not(target_os = "windows"))]
    assert_eq!(fs::read_to_string(&file.path)?, "after");
    #[cfg(target_os = "windows")]
    assert_eq!(fs::read_to_string(&file.path)?, "before");
    assert!(!app.command_palette.is_open());
    assert_eq!(app.command_palette.report().executions, 1);
    assert_eq!(app.command_palette.report().retained_bytes, 0);
    Ok(())
}

#[test]
fn command_palette_focus_cancel_and_scene_are_bounded() -> Result<(), Box<dyn std::error::Error>> {
    let mut app = StudioApp::new(PaletteTextSystem::default())?;
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
fn command_palette_shapes_shortcuts_from_the_authoritative_keymap()
-> Result<(), Box<dyn std::error::Error>> {
    let mut app = StudioApp::new(PaletteTextSystem {
        rejected_glyph: Some(u32::from('+')),
    })?;
    assert!(
        app.handle_event(&key(KEY_P, command_shift()))
            .visual_changed
    );
    assert!(matches!(
        app.try_scene(
            SceneRevision::new(902),
            Size::new(WINDOW_WIDTH, WINDOW_HEIGHT).ok_or("viewport")?,
        ),
        Err(StudioRenderError::Layout(LayoutError::InvalidShaperOutput))
    ));
    Ok(())
}

#[test]
#[cfg(not(target_os = "windows"))]
fn command_availability_refresh_prevents_stale_execution() -> Result<(), Box<dyn std::error::Error>>
{
    let file = TempFile::new("clean")?;
    let mut app = StudioApp::open_file(PaletteTextSystem::default(), &file.path)?;
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
#[cfg_attr(miri, ignore = "wall-clock qualification is not meaningful under Miri")]
fn command_palette_stage_measurements_are_separate_and_bounded()
-> Result<(), Box<dyn std::error::Error>> {
    let mut app = StudioApp::new(PaletteTextSystem::default())?;
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

#[test]
fn command_routes_cover_history_workspace_overlays_and_close()
-> Result<(), Box<dyn std::error::Error>> {
    let file = TempFile::new("first")?;
    let second = file.root.join("second.rs");
    fs::write(&second, "second")?;
    let mut app = StudioApp::open_workspace_lazy(PaletteTextSystem::default(), &file.root)?;
    app.open_workspace_path(&file.path, None)?;
    app.open_workspace_path(&second, None)?;

    let context = app.command_context();
    assert!(context.can_close_tab);
    assert!(context.can_navigate_back);
    assert!(!context.can_navigate_forward);
    let _ = app.dispatch_command(StudioCommand::SaveFile);
    let _ = app.dispatch_command(StudioCommand::NavigateBack);
    assert!(app.command_context().can_navigate_forward);
    let _ = app.dispatch_command(StudioCommand::NavigateForward);
    let _ = app.dispatch_command(StudioCommand::OpenFind);
    assert!(app.find.is_open());
    app.find.close();
    let _ = app.dispatch_command(StudioCommand::OpenReplace);
    assert!(app.find.is_open());
    app.find.close();
    let _ = app.dispatch_command(StudioCommand::OpenQuickOpen);
    assert!(app.quick_open.is_open());
    app.quick_open.close();
    let _ = app.dispatch_command(StudioCommand::ToggleFileTree);
    assert!(app.file_tree.is_visible());
    assert!(app.file_tree.is_focused());
    let _ = app.dispatch_command(StudioCommand::ToggleFileTree);
    assert!(!app.file_tree.is_visible());
    let before = app.tabs.len();
    let _ = app.dispatch_command(StudioCommand::CloseTab);
    assert_eq!(app.tabs.len(), before - 1);
    Ok(())
}

#[test]
fn command_focus_routes_every_key_ime_pointer_and_open_failure()
-> Result<(), Box<dyn std::error::Error>> {
    let mut app = StudioApp::new(PaletteTextSystem::default())?;
    app.command_palette.fail_next_open();
    assert!(app.open_command_palette().visual_changed);
    assert!(!app.command_palette.is_open());
    assert!(
        app.local_status
            .as_ref()
            .is_some_and(|status| status.message().contains("allocation"))
    );

    assert!(app.open_command_palette().visual_changed);
    assert!(app.handle_event(&ime(ImeEvent::Started)).visual_changed);
    assert!(
        app.handle_event(&ime(ImeEvent::Updated {
            text: "x".into(),
            selected_start_utf16: 0,
            selected_length_utf16: 2,
        }))
        .visual_changed
    );
    assert!(
        app.handle_event(&ime(ImeEvent::Updated {
            text: "find".into(),
            selected_start_utf16: 0,
            selected_length_utf16: 4,
        }))
        .visual_changed
    );
    assert!(
        !app.handle_event(&ime(ImeEvent::Updated {
            text: "find".into(),
            selected_start_utf16: 0,
            selected_length_utf16: 4,
        }))
        .visual_changed
    );
    assert!(app.handle_event(&ime(ImeEvent::Cancelled)).visual_changed);
    assert!(
        app.handle_event(&ime(ImeEvent::Committed("find".into())))
            .visual_changed
    );
    let _ = app.handle_event(&key(KEY_DOWN, Modifiers::default()));
    let _ = app.handle_event(&key(KEY_UP, Modifiers::default()));
    app.command_palette.fail_next_query_update();
    assert!(
        app.handle_event(&key(KEY_DELETE_BACKWARD, Modifiers::default()))
            .visual_changed
    );
    let _ = app.handle_event(&key(KEY_DELETE_BACKWARD, Modifiers::default()));
    assert!(
        !app.handle_event(&key(
            KEY_DELETE_BACKWARD,
            Modifiers::from_bits(Modifiers::COMMAND),
        ))
        .visual_changed
    );
    assert!(
        !app.handle_event(&key(u16::MAX, Modifiers::default()))
            .visual_changed
    );
    let position = Point::new(4.0, 4.0).ok_or("pointer")?;
    assert!(
        !app.handle_pointer(
            PointerAction::Down,
            position,
            PointerButton::Primary,
            Modifiers::default(),
        )
        .visual_changed
    );
    assert!(
        app.handle_event(&key(KEY_ESCAPE, Modifiers::default()))
            .visual_changed
    );
    assert!(
        app.dispatch_command(StudioCommand::OpenQuickOpen)
            .visual_changed
    );
    assert!(
        app.dispatch_command(StudioCommand::ToggleFileTree)
            .visual_changed
    );

    Ok(())
}

#[test]
fn command_dispatch_preserves_bounded_workspace_errors() -> Result<(), Box<dyn std::error::Error>> {
    let workspace = TempFile::new("workspace")?;
    let mut app = StudioApp::open_workspace_lazy(PaletteTextSystem::default(), &workspace.root)?;
    app.quick_open = crate::quick_open::QuickOpenState::with_test_limits(
        crate::quick_open::QuickOpenLimits::new(0, 1, 1, 1, 1, 1, 1),
    );
    assert!(
        app.dispatch_command(StudioCommand::OpenQuickOpen)
            .visual_changed
    );
    assert!(
        app.local_status
            .as_ref()
            .is_some_and(|status| status.message().contains("limits"))
    );

    app.file_tree = crate::file_tree::FileTreeState::with_test_limits(
        crate::file_tree::FileTreeLimits::new(0, 1, 1, 1, 1, 1, 1, 1, 1),
    );
    assert!(
        app.dispatch_command(StudioCommand::ToggleFileTree)
            .visual_changed
    );
    assert!(
        app.local_status
            .as_ref()
            .is_some_and(|status| status.message().contains("limits"))
    );
    Ok(())
}

fn glyph_count_at(scene: &Scene, x: f32, y: f32) -> usize {
    scene
        .glyphs()
        .iter()
        .filter(|glyph| {
            glyph.bounds().origin().x().to_bits() == x.to_bits()
                && glyph.bounds().origin().y().to_bits() == y.to_bits()
        })
        .count()
}

fn glyph_count_left_of(scene: &Scene, x: f32, y: f32) -> usize {
    scene
        .glyphs()
        .iter()
        .filter(|glyph| {
            glyph.bounds().origin().x() < x && glyph.bounds().origin().y().to_bits() == y.to_bits()
        })
        .count()
}

#[test]
fn command_palette_scene_geometry_is_exact_at_a_narrow_viewport()
-> Result<(), Box<dyn std::error::Error>> {
    let file = TempFile::new("first")?;
    let second = file.root.join("second.rs");
    fs::write(&second, "second")?;
    let mut app = StudioApp::open_workspace_lazy(PaletteTextSystem::default(), &file.root)?;
    app.open_workspace_path(&file.path, None)?;
    app.open_workspace_path(&second, None)?;
    assert!(app.open_command_palette().visual_changed);
    assert_eq!(app.command_palette.visible_commands()?.len(), 13);
    let shortcut_rows = app
        .command_palette
        .visible_commands()?
        .into_iter()
        .enumerate()
        .filter_map(|(row_index, row)| {
            app.settings
                .active()
                .keymap
                .shortcut_for(row.command)
                .map(|shortcut| (row_index, shortcut.chars().count()))
        })
        .collect::<Vec<_>>();
    assert!(!shortcut_rows.is_empty());

    let viewport = Size::new(300.0, 400.0).ok_or("viewport")?;
    let scene = app.try_scene(SceneRevision::new(906), viewport)?;
    let overlay = Rect::new(
        Point::new(24.0, 48.0).ok_or("overlay origin")?,
        Size::new(252.0, 346.0).ok_or("overlay size")?,
    );
    let selected = Rect::new(
        Point::new(24.0, 82.0).ok_or("selected origin")?,
        Size::new(252.0, 24.0).ok_or("selected size")?,
    );
    let overlay_clip = scene
        .clips()
        .iter()
        .position(|clip| clip.bounds() == overlay)
        .ok_or("command overlay clip")?;
    assert!(scene.quads().iter().any(|quad| quad.bounds() == overlay));
    assert!(scene.quads().iter().any(|quad| quad.bounds() == selected));
    for (x, y) in [(32.0, 67.0), (32.0, 98.0), (32.0, 122.0)] {
        let first_x = scene
            .glyphs()
            .iter()
            .filter(|glyph| {
                glyph
                    .clip()
                    .is_some_and(|clip| clip.index() == overlay_clip)
                    && glyph.bounds().origin().y().to_bits() == f32::to_bits(y)
            })
            .map(|glyph| glyph.bounds().origin().x())
            .reduce(f32::min)
            .ok_or("command row glyph")?;
        assert_eq!(first_x.to_bits(), f32::to_bits(x));
    }
    for &(row_index, shortcut_columns) in &shortcut_rows {
        let row_y = 98.0 + usize_as_f32(row_index) * COMMAND_PALETTE_ROW_HEIGHT;
        let shortcut_left = 268.0 - usize_as_f32(shortcut_columns) * 8.0;
        assert_eq!(glyph_count_at(&scene, shortcut_left, row_y), 1);
        assert_eq!(glyph_count_at(&scene, 260.0, row_y), 1);
    }

    let narrow_width = 80.0;
    let narrow_scene = app.try_scene(
        SceneRevision::new(907),
        Size::new(narrow_width, 400.0).ok_or("narrow viewport")?,
    )?;
    let overlay_width = COMMAND_PALETTE_WIDTH.min((narrow_width - CONTENT_INSET * 2.0).max(1.0));
    let overlay_left = ((narrow_width - overlay_width) * 0.5).max(0.0);
    let shortcut_floor = overlay_left + FIND_BAR_INSET;
    for &(row_index, _) in &shortcut_rows {
        let row_y = 98.0 + usize_as_f32(row_index) * COMMAND_PALETTE_ROW_HEIGHT;
        assert_eq!(glyph_count_at(&narrow_scene, shortcut_floor, row_y), 2);
        assert_eq!(glyph_count_left_of(&narrow_scene, shortcut_floor, row_y), 0);
    }
    Ok(())
}

#[test]
fn command_context_distinguishes_scratch_dirty_and_each_close_guard()
-> Result<(), Box<dyn std::error::Error>> {
    let mut scratch = StudioApp::new(PaletteTextSystem::default())?;
    assert!(
        scratch
            .handle_event(&ime(ImeEvent::Committed("dirty".into())))
            .visual_changed
    );
    let scratch_context = scratch.command_context();
    assert!(!scratch_context.can_save);
    assert!(!scratch_context.can_close_tab);

    let file = TempFile::new("first")?;
    let second = file.root.join("second.rs");
    fs::write(&second, "second")?;
    let mut app = StudioApp::open_workspace_lazy(PaletteTextSystem::default(), &file.root)?;
    app.open_workspace_path(&file.path, None)?;
    app.open_workspace_path(&second, None)?;
    assert!(app.command_context().can_close_tab);
    app.last_file_error = Some(FileError::InvalidUtf8);
    assert!(!app.command_context().can_close_tab);
    app.last_file_error = None;
    assert!(
        app.handle_event(&ime(ImeEvent::Committed("dirty".into())))
            .visual_changed
    );
    assert!(app.command_context().can_close_tab);
    let before = app.buffer().snapshot().text();
    assert!(app.dispatch_command(StudioCommand::CloseTab).visual_changed);
    assert!(app.document.is_dirty());
    assert_eq!(app.buffer().snapshot().text(), before);
    Ok(())
}

#[test]
fn command_focus_distinguishes_modifiers_and_suppresses_clipboard()
-> Result<(), Box<dyn std::error::Error>> {
    let mut app = StudioApp::new(PaletteTextSystem::default())?;
    app.selection = Selection::new(ByteOffset::new(0), ByteOffset::new(2));
    assert!(app.open_command_palette().visual_changed);
    let command = Modifiers::from_bits(Modifiers::COMMAND);
    let copy = app.handle_event_with_response(&logical_key("c", command));
    assert!(copy.clipboard_write.is_none());

    let selected = app
        .command_palette
        .visible_commands()?
        .into_iter()
        .find(|row| row.selected)
        .map(|row| row.command);
    assert!(!app.handle_event(&key(KEY_DOWN, command)).visual_changed);
    assert_eq!(
        app.command_palette
            .visible_commands()?
            .into_iter()
            .find(|row| row.selected)
            .map(|row| row.command),
        selected
    );
    assert!(
        app.handle_event(&key(KEY_DOWN, Modifiers::default()))
            .visual_changed
    );
    assert_ne!(
        app.command_palette
            .visible_commands()?
            .into_iter()
            .find(|row| row.selected)
            .map(|row| row.command),
        selected
    );
    let selected = app
        .command_palette
        .visible_commands()?
        .into_iter()
        .find(|row| row.selected)
        .map(|row| row.command);
    assert!(!app.handle_event(&key(KEY_UP, command)).visual_changed);
    assert_eq!(
        app.command_palette
            .visible_commands()?
            .into_iter()
            .find(|row| row.selected)
            .map(|row| row.command),
        selected
    );
    assert!(
        app.handle_event(&key(KEY_UP, Modifiers::default()))
            .visual_changed
    );
    assert_eq!(
        app.command_palette
            .visible_commands()?
            .into_iter()
            .find(|row| row.selected)
            .map(|row| row.command),
        Some(StudioCommand::OpenFind)
    );

    let executions = app.command_palette.report().executions;
    assert!(!app.handle_event(&key(KEY_RETURN, command)).visual_changed);
    assert!(app.command_palette.is_open());
    assert_eq!(app.command_palette.report().executions, executions);
    assert!(
        app.handle_event(&key(KEY_RETURN, Modifiers::default()))
            .visual_changed
    );
    assert!(!app.command_palette.is_open());
    assert_eq!(app.command_palette.report().executions, executions + 1);
    Ok(())
}

#[test]
fn command_dispatch_records_missing_workspace_and_queued_find_work()
-> Result<(), Box<dyn std::error::Error>> {
    let mut missing = StudioApp::new(PaletteTextSystem::default())?;
    let _ = missing.dispatch_command(StudioCommand::OpenQuickOpen);
    assert!(
        missing
            .local_status
            .as_ref()
            .is_some_and(|status| status.message().contains("requires one local workspace"))
    );
    missing.local_status = None;
    let _ = missing.dispatch_command(StudioCommand::ToggleFileTree);
    assert!(
        missing
            .local_status
            .as_ref()
            .is_some_and(|status| status.message().contains("requires one local workspace"))
    );

    for command in [StudioCommand::OpenFind, StudioCommand::OpenReplace] {
        let mut app = StudioApp::new(PaletteTextSystem::default())?;
        assert!(app.find.open(false));
        assert!(
            app.handle_event(&ime(ImeEvent::Committed("needle".into())))
                .visual_changed
        );
        app.find.close();
        app.find_needs_search = false;
        assert!(app.dispatch_command(command).visual_changed);
        assert!(app.find_needs_search);
    }
    Ok(())
}

#[test]
fn command_palette_render_errors_preserve_stage_and_source()
-> Result<(), Box<dyn std::error::Error>> {
    let converted = StudioRenderError::from(CommandPaletteError::InvalidComposition);
    assert!(converted.to_string().contains("command-palette rendering"));

    let mut query_failure = StudioApp::new(PaletteTextSystem {
        rejected_glyph: Some(u32::from('>')),
    })?;
    assert!(query_failure.open_command_palette().visual_changed);
    let query_result = query_failure.try_scene(
        SceneRevision::new(903),
        Size::new(WINDOW_WIDTH, WINDOW_HEIGHT).ok_or("viewport")?,
    );
    assert!(matches!(query_result, Err(StudioRenderError::Layout(_))));

    let mut row_failure = StudioApp::new(PaletteTextSystem {
        rejected_glyph: Some(u32::from('E')),
    })?;
    assert!(row_failure.open_command_palette().visual_changed);
    assert!(
        row_failure
            .handle_event(&ime(ImeEvent::Committed("find".into())))
            .visual_changed
    );
    let row_result = row_failure.try_scene(
        SceneRevision::new(904),
        Size::new(WINDOW_WIDTH, WINDOW_HEIGHT).ok_or("viewport")?,
    );
    assert!(matches!(row_result, Err(StudioRenderError::Layout(_))));

    let mut scene_failure = StudioApp::new(PaletteTextSystem::default())?;
    scene_failure.force_command_clip_failure = Some(());
    assert!(scene_failure.open_command_palette().visual_changed);
    let scene_result = scene_failure.try_scene(
        SceneRevision::new(905),
        Size::new(WINDOW_WIDTH, WINDOW_HEIGHT).ok_or("viewport")?,
    );
    assert!(matches!(
        scene_result,
        Err(StudioRenderError::Scene(SceneError::InvalidClip { .. }))
    ));
    Ok(())
}
