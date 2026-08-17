use std::{
    fs,
    num::NonZeroU32,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

use alpine_platform_macos::{EventTimestamp, ImeEvent, KeyState, Modifiers, SurfaceEvent};
use alpine_text_layout::{
    FontKey, GlyphBitmap, GlyphRasterizer, LayoutError, LineLayout, RasterizedGlyph, ShapedGlyph,
    TextShaper,
};

use super::*;

static NEXT_TEMP: AtomicU64 = AtomicU64::new(1);

struct TempProject {
    root: PathBuf,
}

impl TempProject {
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "alpine-project-search-integration-{}-{id}",
            std::process::id()
        ));
        fs::create_dir(&root)?;
        fs::write(root.join("alpha.rs"), "zero\nneedle alpha\n")?;
        fs::write(root.join("beta.rs"), "needle beta\n")?;
        Ok(Self { root })
    }
}

impl Drop for TempProject {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[derive(Default)]
struct SearchTextSystem;

impl TextShaper for SearchTextSystem {
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
        LineLayout::new(glyphs, x, 15.0, 4.0, 8_192)
    }
}

impl GlyphRasterizer for SearchTextSystem {
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

fn ime(text: &str) -> SurfaceEvent {
    SurfaceEvent::Ime {
        timestamp: EventTimestamp::new(2),
        event: ImeEvent::Committed(text.into()),
    }
}

fn open_and_search(app: &mut StudioApp, root: &std::path::Path) -> Result<(), Box<dyn Error>> {
    let command_shift = Modifiers::from_bits(Modifiers::COMMAND | Modifiers::SHIFT);
    assert!(app.handle_event(&key(KEY_F, command_shift)).visual_changed);
    assert!(app.project_search.is_open());
    assert!(app.handle_event(&ime("needle")).visual_changed);
    for _ in 0..64 {
        let Some(request) = app.prepare_project_search_request()? else {
            break;
        };
        let effect = app.apply_project_search_output(request.execute());
        assert!(effect.visual_changed);
    }
    assert!(app.project_search.report().terminal);
    assert_eq!(app.project_search.report().retained_matches, 2);
    let canonical_root = fs::canonicalize(root)?;
    assert_eq!(
        app.workspace.as_ref().map(Workspace::root),
        Some(canonical_root.as_path())
    );
    Ok(())
}

#[test]
fn project_search_is_off_startup_streams_and_opens_the_exact_current_match()
-> Result<(), Box<dyn std::error::Error>> {
    let project = TempProject::new()?;
    let mut app = StudioApp::open_workspace_lazy(SearchTextSystem, &project.root)?;
    assert_eq!(app.project_search.report().retained_bytes, 0);
    assert!(app.prepare_project_search_request()?.is_none());
    let before_scene = app.scene(
        SceneRevision::new(1),
        Size::new(WINDOW_WIDTH, WINDOW_HEIGHT).ok_or("viewport")?,
    );
    open_and_search(&mut app, &project.root)?;
    let search_scene = app.scene(
        SceneRevision::new(2),
        Size::new(WINDOW_WIDTH, WINDOW_HEIGHT).ok_or("viewport")?,
    );
    assert!(search_scene.operations().len() > before_scene.operations().len());
    assert!(
        app.handle_event(&key(KEY_RETURN, Modifiers::default()))
            .document_changed
    );
    assert!(!app.project_search.is_open());
    assert_eq!(
        app.buffer().snapshot().slice(app.selection.range())?,
        "needle"
    );
    assert_eq!(app.tabs.len(), 2);
    Ok(())
}

#[test]
fn stale_selected_file_preserves_the_current_document_and_tab_identity()
-> Result<(), Box<dyn std::error::Error>> {
    let project = TempProject::new()?;
    let mut app = StudioApp::open_workspace_lazy(SearchTextSystem, &project.root)?;
    open_and_search(&mut app, &project.root)?;
    let before_snapshot = app.buffer().snapshot();
    let before = before_snapshot.slice(0..before_snapshot.len_bytes())?;
    let tabs = app.tabs.len();
    fs::write(project.root.join("alpha.rs"), "changed after search\n")?;
    let effect = app.handle_event(&key(KEY_RETURN, Modifiers::default()));
    assert!(effect.visual_changed);
    assert!(!effect.document_changed);
    let after = app.buffer().snapshot();
    assert_eq!(after.slice(0..after.len_bytes())?, before);
    assert_eq!(app.tabs.len(), tabs);
    assert!(app.project_search.is_open());
    assert!(
        app.local_status
            .as_ref()
            .is_some_and(|status| status.message().contains("no longer current"))
    );
    Ok(())
}

#[test]
fn project_search_stages_are_separate_bounded_and_diagnostic_only()
-> Result<(), Box<dyn std::error::Error>> {
    let project = TempProject::new()?;
    for index in 0..128 {
        fs::write(
            project.root.join(format!("extra-{index:03}.txt")),
            format!("line {index}\nneedle {index}\n"),
        )?;
    }
    let mut app = StudioApp::open_workspace_lazy(SearchTextSystem, &project.root)?;
    let open_start = Instant::now();
    assert!(app.open_project_search().visual_changed);
    let open_elapsed = open_start.elapsed();
    assert!(app.handle_event(&ime("needle")).visual_changed);
    let worker_start = Instant::now();
    while let Some(request) = app.prepare_project_search_request()? {
        let _ = app.apply_project_search_output(request.execute());
    }
    let worker_elapsed = worker_start.elapsed();
    let projection_start = Instant::now();
    let rows = app
        .project_search
        .visible_results(PROJECT_SEARCH_VISIBLE_ROWS, PROJECT_SEARCH_OVERSCAN_ROWS)?;
    let projection_elapsed = projection_start.elapsed();
    let scene_start = Instant::now();
    let _ = app.scene(
        SceneRevision::new(3),
        Size::new(WINDOW_WIDTH, WINDOW_HEIGHT).ok_or("viewport")?,
    );
    let scene_elapsed = scene_start.elapsed();
    let report = app.project_search.report();
    assert_eq!(report.retained_matches, 130);
    assert!(report.result_bytes <= project_search::MAX_RESULT_BYTES);
    assert!(report.inventory_bytes <= project_search::MAX_INVENTORY_BYTES);
    assert!(rows.len() <= PROJECT_SEARCH_VISIBLE_ROWS + PROJECT_SEARCH_OVERSCAN_ROWS * 2);
    for elapsed in [
        open_elapsed,
        worker_elapsed,
        projection_elapsed,
        scene_elapsed,
    ] {
        assert!(elapsed < Duration::from_secs(5));
    }
    Ok(())
}

#[test]
fn command_palette_and_missing_workspace_route_the_typed_project_search_command()
-> Result<(), Box<dyn std::error::Error>> {
    let mut missing = StudioApp::new(SearchTextSystem)?;
    let effect = missing.dispatch_command(StudioCommand::OpenProjectSearch);
    assert!(effect.visual_changed);
    assert!(
        missing
            .local_status
            .as_ref()
            .is_some_and(|status| status.message().contains("requires one local workspace"))
    );

    let project = TempProject::new()?;
    let mut app = StudioApp::open_workspace_lazy(SearchTextSystem, &project.root)?;
    assert!(app.open_command_palette().visual_changed);
    assert!(app.handle_event(&ime("project search")).visual_changed);
    assert!(
        app.handle_event(&key(KEY_RETURN, Modifiers::default()))
            .visual_changed
    );
    assert!(app.project_search.is_open());
    assert!(!app.command_palette.is_open());
    Ok(())
}
