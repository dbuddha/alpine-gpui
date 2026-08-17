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

use crate::project_search::ProjectSearchLimits;

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

#[derive(Default)]
struct FaultSearchTextSystem {
    rejected_glyph: Option<u32>,
}

impl TextShaper for FaultSearchTextSystem {
    fn shape(&mut self, text: &str, font: FontKey) -> Result<LineLayout, LayoutError> {
        SearchTextSystem.shape(text, font)
    }
}

impl GlyphRasterizer for FaultSearchTextSystem {
    fn rasterize(
        &mut self,
        font: FontKey,
        glyph_id: u32,
        subpixel_x: u8,
    ) -> Result<RasterizedGlyph, LayoutError> {
        if self.rejected_glyph == Some(glyph_id) {
            return Err(LayoutError::InvalidShaperOutput);
        }
        SearchTextSystem.rasterize(font, glyph_id, subpixel_x)
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

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "every project-search focus and selection route requires an independent control"
)]
fn project_search_focus_render_error_and_existing_tab_routes_are_discriminating()
-> Result<(), Box<dyn std::error::Error>> {
    let render: StudioRenderError = ProjectSearchError::InvalidLimits.into();
    assert!(
        render
            .to_string()
            .contains("project-search rendering failed")
    );
    assert!(render.source().is_none());
    let selection = WorkspaceSelectionError::ProjectSearch(ProjectSearchError::StaleMatch);
    assert!(selection.to_string().contains("project search failed"));
    assert!(selection.source().is_some());

    let mut missing = StudioApp::new(SearchTextSystem)?;
    assert!(missing.open_project_search().visual_changed);
    assert!(missing.project_search.open(1)?);
    assert!(matches!(
        missing.prepare_project_search_request(),
        Err(ProjectSearchError::NoWorkspace)
    ));

    let project = TempProject::new()?;
    let mut invalid = StudioApp::open_workspace_lazy(SearchTextSystem, &project.root)?;
    invalid.project_search = ProjectSearchState::with_test_limits(ProjectSearchLimits::new(
        0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    ));
    assert!(invalid.open_project_search().visual_changed);
    assert!(
        invalid
            .local_status
            .as_ref()
            .is_some_and(|status| status.message().contains("limits are invalid"))
    );

    let mut app = StudioApp::open_workspace_lazy(SearchTextSystem, &project.root)?;
    assert!(app.open_project_search().visual_changed);
    assert!(app.handle_event(&ime("needle")).visual_changed);
    let inventory = app.prepare_project_search_request()?.ok_or("inventory")?;
    assert!(
        app.apply_project_search_output(inventory.execute())
            .visual_changed
    );
    let stale = app
        .prepare_project_search_request()?
        .ok_or("stale search")?;
    assert!(app.handle_event(&ime("x")).visual_changed);
    assert!(
        !app.apply_project_search_output(stale.execute())
            .visual_changed
    );
    assert!(app.project_search.report().stale_rejections > 0);
    assert!(
        app.handle_event(&key(KEY_ESCAPE, Modifiers::default()))
            .visual_changed
    );

    open_and_search(&mut app, &project.root)?;
    assert!(
        app.handle_event(&key(KEY_DOWN, Modifiers::default()))
            .visual_changed
    );
    assert!(
        app.handle_event(&key(KEY_UP, Modifiers::default()))
            .visual_changed
    );
    assert!(
        !app.handle_event(&key(KEY_A, Modifiers::default()))
            .visual_changed
    );
    let scene = app.scene(
        SceneRevision::new(907),
        Size::new(WINDOW_WIDTH, WINDOW_HEIGHT).ok_or("viewport")?,
    );
    assert!(!scene.glyphs().is_empty());
    assert!(
        !app.handle_pointer(
            PointerAction::Down,
            Point::new(100.0, 100.0).ok_or("point")?,
            PointerButton::Primary,
            Modifiers::default(),
        )
        .visual_changed
    );
    assert!(
        app.handle_event(&key(KEY_DELETE_BACKWARD, Modifiers::default()))
            .visual_changed
    );
    assert!(
        app.handle_event(&SurfaceEvent::Ime {
            timestamp: EventTimestamp::new(3),
            event: ImeEvent::Started,
        })
        .visual_changed
    );
    assert!(
        app.handle_event(&SurfaceEvent::Ime {
            timestamp: EventTimestamp::new(4),
            event: ImeEvent::Updated {
                text: "é".into(),
                selected_start_utf16: 1,
                selected_length_utf16: 0,
            },
        })
        .visual_changed
    );
    assert!(
        app.handle_event(&SurfaceEvent::Ime {
            timestamp: EventTimestamp::new(5),
            event: ImeEvent::Cancelled,
        })
        .visual_changed
    );
    assert!(
        app.handle_event(&SurfaceEvent::Ime {
            timestamp: EventTimestamp::new(6),
            event: ImeEvent::Updated {
                text: "x".into(),
                selected_start_utf16: 2,
                selected_length_utf16: 0,
            },
        })
        .visual_changed
    );

    app.project_search.close();
    open_and_search(&mut app, &project.root)?;
    assert!(
        app.handle_event(&key(KEY_RETURN, Modifiers::default()))
            .document_changed
    );
    open_and_search(&mut app, &project.root)?;
    assert!(
        app.handle_event(&key(KEY_RETURN, Modifiers::default()))
            .visual_changed
    );
    let beta = project.root.join("beta.rs");
    app.open_workspace_path(&beta, None)?;
    open_and_search(&mut app, &project.root)?;
    assert!(
        app.handle_event(&key(KEY_RETURN, Modifiers::default()))
            .document_changed
    );
    assert_eq!(
        app.buffer().snapshot().slice(app.selection.range())?,
        "needle"
    );

    app.open_workspace_path(&beta, None)?;
    open_and_search(&mut app, &project.root)?;
    let alpha = fs::canonicalize(project.root.join("alpha.rs"))?;
    assert!(
        !app.tabs
            .clear_inactive_document_for_test(&project.root.join("missing.rs"))
    );
    assert!(app.tabs.clear_inactive_document_for_test(&alpha));
    let tabs = app.tabs.len();
    let effect = app.handle_event(&key(KEY_RETURN, Modifiers::default()));
    assert!(effect.visual_changed);
    assert!(!effect.document_changed);
    assert_eq!(app.tabs.len(), tabs);

    let mut exhausted = StudioApp::open_workspace_lazy(SearchTextSystem, &project.root)?;
    assert!(exhausted.open_project_search().visual_changed);
    assert!(exhausted.handle_event(&ime("needle")).visual_changed);
    exhausted.project_search.exhaust_generations_for_test();
    assert!(
        exhausted
            .handle_event(&key(KEY_DELETE_BACKWARD, Modifiers::default()))
            .visual_changed
    );
    Ok(())
}

#[test]
fn project_search_render_failures_preserve_layout_and_scene_stages()
-> Result<(), Box<dyn std::error::Error>> {
    let project = TempProject::new()?;
    fs::write(project.root.join("alpha.rs"), "zero\nneedle row-only~\n")?;
    for rejected_glyph in [u32::from('P'), u32::from('~')] {
        let mut app = StudioApp::open_workspace_lazy(
            FaultSearchTextSystem {
                rejected_glyph: Some(rejected_glyph),
            },
            &project.root,
        )?;
        open_and_search(&mut app, &project.root)?;
        assert!(matches!(
            app.try_scene(
                SceneRevision::new(908),
                Size::new(WINDOW_WIDTH, WINDOW_HEIGHT).ok_or("viewport")?,
            ),
            Err(StudioRenderError::Layout(_))
        ));
    }

    let mut clip = StudioApp::open_workspace_lazy(FaultSearchTextSystem::default(), &project.root)?;
    open_and_search(&mut clip, &project.root)?;
    clip.force_project_search_clip_failure = Some(());
    assert!(matches!(
        clip.try_scene(
            SceneRevision::new(909),
            Size::new(WINDOW_WIDTH, WINDOW_HEIGHT).ok_or("viewport")?,
        ),
        Err(StudioRenderError::Scene(SceneError::InvalidClip { .. }))
    ));
    Ok(())
}

#[test]
fn runtime_project_search_admits_workers_and_rolls_back_submission_failures()
-> Result<(), Box<dyn std::error::Error>> {
    let clear = LinearRgba::new(0.02, 0.02, 0.02, 1.0).ok_or("clear")?;
    let viewport = Size::new(WINDOW_WIDTH, WINDOW_HEIGHT).ok_or("viewport")?;
    let project = TempProject::new()?;
    let app = StudioApp::open_workspace_lazy(SearchTextSystem, &project.root)?;
    let mut runtime = Application::new(app, viewport, clear, WorkerConfig::default())?;
    let command_shift = Modifiers::from_bits(Modifiers::COMMAND | Modifiers::SHIFT);
    assert!(runtime.dispatch(&key(KEY_F, command_shift)).is_some());
    assert!(runtime.dispatch(&ime("needle")).is_some());
    let mut admitted = false;
    for timestamp in 100..612 {
        std::thread::sleep(Duration::from_millis(1));
        if runtime
            .dispatch(&SurfaceEvent::Wake {
                timestamp: EventTimestamp::new(timestamp),
            })
            .is_some()
        {
            admitted = true;
        }
    }
    assert!(admitted);

    let worker_config = WorkerConfig::new(
        std::num::NonZeroUsize::MIN,
        std::num::NonZeroUsize::MIN,
        std::num::NonZeroUsize::MIN,
    );
    let mut rejected = StudioApp::open_workspace_lazy(SearchTextSystem, &project.root)?;
    assert!(rejected.open_project_search().visual_changed);
    assert!(rejected.handle_event(&ime("needle")).visual_changed);
    rejected.force_project_search_submission_failure = Some(());
    let mut rejected_runtime = Application::new(rejected, viewport, clear, worker_config)?;
    assert!(
        rejected_runtime
            .dispatch(&SurfaceEvent::Wake {
                timestamp: EventTimestamp::new(700),
            })
            .is_some()
    );

    let mut missing = StudioApp::new(SearchTextSystem)?;
    assert!(missing.project_search.open(1)?);
    assert!(missing.project_search.commit_text("needle")?);
    let mut missing_runtime = Application::new(missing, viewport, clear, worker_config)?;
    assert!(
        missing_runtime
            .dispatch(&SurfaceEvent::Wake {
                timestamp: EventTimestamp::new(701),
            })
            .is_some()
    );
    Ok(())
}
