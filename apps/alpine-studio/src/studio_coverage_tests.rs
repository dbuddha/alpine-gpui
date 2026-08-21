use std::{
    cell::Cell,
    fs,
    num::NonZeroU32,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use alpine_platform_macos::{CloseDisposition, EventTimestamp, ScrollPhase};
use alpine_text_layout::{GlyphBitmap, RasterizedGlyph, ShapedGlyph};

use super::*;

static NEXT_TEST_FILE: AtomicU64 = AtomicU64::new(1);

thread_local! {
    static TEST_SHAPE_CALLS: Cell<u64> = const { Cell::new(0) };
}

struct TestFile {
    path: PathBuf,
}

impl TestFile {
    fn new(bytes: impl AsRef<[u8]>) -> std::io::Result<Self> {
        let sequence = NEXT_TEST_FILE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "alpine-studio-{}-{sequence}.txt",
            std::process::id()
        ));
        fs::write(&path, bytes)?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

struct TestWorkspace {
    path: PathBuf,
}

impl TestWorkspace {
    fn new() -> std::io::Result<Self> {
        let sequence = NEXT_TEST_FILE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "alpine-studio-workspace-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path)?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn write(&self, name: &str, bytes: impl AsRef<[u8]>) -> std::io::Result<()> {
        fs::write(self.path.join(name), bytes)
    }

    fn create_dir(&self, name: &str) -> std::io::Result<()> {
        fs::create_dir(self.path.join(name))
    }
}

impl Drop for TestWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[derive(Default)]
pub(super) struct TestTextSystem;

impl TextShaper for TestTextSystem {
    fn shape(&mut self, text: &str, _font: FontKey) -> Result<LineLayout, LayoutError> {
        TEST_SHAPE_CALLS.with(|calls| calls.set(calls.get().saturating_add(1)));
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

impl GlyphRasterizer for TestTextSystem {
    fn rasterize(
        &mut self,
        _font: FontKey,
        _glyph_id: u32,
        _subpixel_x: u8,
    ) -> Result<alpine_text_layout::RasterizedGlyph, LayoutError> {
        let width = NonZeroU32::new(2).ok_or(LayoutError::InvalidShaperOutput)?;
        let height = NonZeroU32::new(3).ok_or(LayoutError::InvalidShaperOutput)?;
        let bitmap = GlyphBitmap::new(width, height, vec![255; 6])?;
        RasterizedGlyph::new(Some(bitmap), 0.0, 3.0)
    }
}

struct UnresolvedEmptyTextSystem;

impl TextShaper for UnresolvedEmptyTextSystem {
    fn shape(&mut self, text: &str, _font: FontKey) -> Result<LineLayout, LayoutError> {
        let mut glyphs = Vec::new();
        let mut x = 0.0;
        let mut utf16 = 0_u32;
        for character in text.chars() {
            glyphs.push(ShapedGlyph::new(u32::from(character), x, 0.0, 8.0, utf16)?);
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

impl GlyphRasterizer for UnresolvedEmptyTextSystem {
    fn rasterize(
        &mut self,
        _font: FontKey,
        _glyph_id: u32,
        _subpixel_x: u8,
    ) -> Result<RasterizedGlyph, LayoutError> {
        RasterizedGlyph::new(None, 0.0, 0.0)
    }
}

struct FailingTextSystem;

impl TextShaper for FailingTextSystem {
    fn shape(&mut self, _text: &str, _font: FontKey) -> Result<LineLayout, LayoutError> {
        Err(LayoutError::NativeFailure("injected shaping failure"))
    }
}

impl GlyphRasterizer for FailingTextSystem {
    fn rasterize(
        &mut self,
        _font: FontKey,
        _glyph_id: u32,
        _subpixel_x: u8,
    ) -> Result<RasterizedGlyph, LayoutError> {
        Err(LayoutError::NativeFailure("injected raster failure"))
    }
}

struct FailingRasterTextSystem;

impl TextShaper for FailingRasterTextSystem {
    fn shape(&mut self, text: &str, font: FontKey) -> Result<LineLayout, LayoutError> {
        TestTextSystem.shape(text, font)
    }
}

impl GlyphRasterizer for FailingRasterTextSystem {
    fn rasterize(
        &mut self,
        _font: FontKey,
        _glyph_id: u32,
        _subpixel_x: u8,
    ) -> Result<RasterizedGlyph, LayoutError> {
        Err(LayoutError::NativeFailure("injected status raster failure"))
    }
}

fn test_app() -> Result<StudioApp, SurfaceError> {
    StudioApp::new(TestTextSystem)
}

fn viewport() -> Result<Size, SurfaceError> {
    Size::new(WINDOW_WIDTH, WINDOW_HEIGHT).ok_or(SurfaceError::invariant(
        alpine_platform_macos::SurfaceOperation::Application,
    ))
}

fn ime(event: ImeEvent) -> SurfaceEvent {
    SurfaceEvent::Ime {
        timestamp: EventTimestamp::new(1),
        input_epoch: InputEpoch::INITIAL,
        event,
    }
}

fn ime_at(input_epoch: InputEpoch, event: ImeEvent) -> SurfaceEvent {
    SurfaceEvent::Ime {
        timestamp: EventTimestamp::new(input_epoch.get()),
        input_epoch,
        event,
    }
}

fn assert_focus_epoch_cancels_owner(app: &mut StudioApp) -> Result<(), StudioRenderError> {
    let next_epoch = InputEpoch::INITIAL
        .checked_next()
        .ok_or(StudioRenderError::Domain)?;
    assert!(app.handle_event(&ime(ImeEvent::Started)).visual_changed);
    assert!(
        app.handle_event(&ime(ImeEvent::Updated {
            text: "é".into(),
            selected_start_utf16: 1,
            selected_length_utf16: 0,
        }))
        .visual_changed
    );

    let document_revision = app.buffer().revision();
    let stale_before = app.rejected_stale_input_events;
    let future_before = app.rejected_future_input_events;
    assert!(
        app.handle_event(&SurfaceEvent::Focus {
            timestamp: EventTimestamp::new(10),
            input_epoch: next_epoch,
            focused: false,
        })
        .visual_changed
    );
    assert_eq!(app.input_epoch, next_epoch);
    assert!(!app.focused);
    assert!(
        !app.handle_event(&ime_at(
            InputEpoch::INITIAL,
            ImeEvent::Committed("obsolete".into()),
        ))
        .visual_changed
    );
    assert_eq!(app.buffer().revision(), document_revision);
    assert_eq!(app.rejected_stale_input_events, stale_before + 1);

    let future_epoch = next_epoch.checked_next().ok_or(StudioRenderError::Domain)?;
    assert!(
        !app.handle_event(&ime_at(future_epoch, ImeEvent::Committed("future".into()),))
            .visual_changed
    );
    assert_eq!(app.buffer().revision(), document_revision);
    assert_eq!(app.rejected_future_input_events, future_before + 1);
    assert!(
        !app.handle_event(&SurfaceEvent::Focus {
            timestamp: EventTimestamp::new(11),
            input_epoch: next_epoch,
            focused: false,
        })
        .visual_changed
    );
    assert!(
        app.handle_event(&SurfaceEvent::Focus {
            timestamp: EventTimestamp::new(12),
            input_epoch: next_epoch,
            focused: true,
        })
        .visual_changed
    );
    let stale_focus_before = app.rejected_stale_input_events;
    assert!(
        !app.handle_event(&SurfaceEvent::Focus {
            timestamp: EventTimestamp::new(13),
            input_epoch: InputEpoch::INITIAL,
            focused: false,
        })
        .visual_changed
    );
    assert_eq!(app.input_epoch, next_epoch);
    assert!(app.focused);
    assert_eq!(app.rejected_stale_input_events, stale_focus_before + 1);
    assert!(
        !app.handle_event(&ime_at(InputEpoch::INITIAL, ImeEvent::Started))
            .visual_changed
    );
    assert!(
        !app.handle_event(&ime_at(next_epoch, ImeEvent::Cancelled))
            .visual_changed
    );
    assert_eq!(app.buffer().revision(), document_revision);
    Ok(())
}

#[test]
fn focus_epoch_admission_preserves_current_and_cancels_future_sessions()
-> Result<(), StudioRenderError> {
    let mut app = test_app().map_err(|_| StudioRenderError::Domain)?;
    assert!(app.handle_event(&ime(ImeEvent::Started)).visual_changed);
    assert!(app.composition.is_some());

    assert!(
        !app.handle_event(&SurfaceEvent::Focus {
            timestamp: EventTimestamp::new(1),
            input_epoch: InputEpoch::INITIAL,
            focused: true,
        })
        .visual_changed
    );
    assert!(app.focused);
    assert!(app.composition.is_some());

    let stale_before = app.rejected_stale_input_events;
    assert!(
        !app.handle_event(&SurfaceEvent::Focus {
            timestamp: EventTimestamp::new(2),
            input_epoch: InputEpoch::INITIAL,
            focused: false,
        })
        .visual_changed
    );
    assert!(app.focused);
    assert!(app.composition.is_some());
    assert_eq!(app.rejected_stale_input_events, stale_before + 1);

    let migrated_epoch = InputEpoch::INITIAL
        .checked_next()
        .ok_or(StudioRenderError::Domain)?;
    assert!(
        app.handle_event(&SurfaceEvent::Focus {
            timestamp: EventTimestamp::new(3),
            input_epoch: migrated_epoch,
            focused: true,
        })
        .visual_changed
    );
    assert_eq!(app.input_epoch, migrated_epoch);
    assert!(app.focused);
    assert!(app.composition.is_none());

    assert!(
        app.handle_event(&ime_at(migrated_epoch, ImeEvent::Started))
            .visual_changed
    );
    let suspended_epoch = migrated_epoch
        .checked_next()
        .ok_or(StudioRenderError::Domain)?;
    assert!(
        app.handle_event(&SurfaceEvent::Focus {
            timestamp: EventTimestamp::new(4),
            input_epoch: suspended_epoch,
            focused: false,
        })
        .visual_changed
    );
    assert!(!app.focused);
    assert!(app.composition.is_none());

    let revision = app.buffer().revision();
    let rejected_before = app.rejected_stale_input_events;
    assert!(
        !app.handle_event(&ime_at(
            suspended_epoch,
            ImeEvent::Committed("unfocused".into()),
        ))
        .visual_changed
    );
    assert_eq!(app.buffer().revision(), revision);
    assert_eq!(app.rejected_stale_input_events, rejected_before + 1);
    Ok(())
}

#[test]
fn focus_epochs_cancel_every_composing_owner_and_reject_obsolete_mutation()
-> Result<(), StudioRenderError> {
    let mut editor = test_app().map_err(|_| StudioRenderError::Domain)?;
    assert_focus_epoch_cancels_owner(&mut editor)?;

    let mut find = test_app().map_err(|_| StudioRenderError::Domain)?;
    assert!(find.find.open(false));
    assert_focus_epoch_cancels_owner(&mut find)?;

    let mut quick_open = test_app().map_err(|_| StudioRenderError::Domain)?;
    assert!(
        quick_open
            .quick_open
            .open(1)
            .map_err(|_| StudioRenderError::Domain)?
    );
    assert_focus_epoch_cancels_owner(&mut quick_open)?;

    let mut command_palette = test_app().map_err(|_| StudioRenderError::Domain)?;
    assert!(command_palette.open_command_palette().visual_changed);
    assert_focus_epoch_cancels_owner(&mut command_palette)?;

    let mut project_search = test_app().map_err(|_| StudioRenderError::Domain)?;
    assert!(
        project_search
            .project_search
            .open(1)
            .map_err(|_| StudioRenderError::Domain)?
    );
    assert_focus_epoch_cancels_owner(&mut project_search)?;
    Ok(())
}

fn key(physical_key: u16, modifiers: Modifiers) -> SurfaceEvent {
    SurfaceEvent::Keyboard {
        timestamp: EventTimestamp::new(1),
        state: KeyState::Down,
        physical_key,
        logical_key: Box::default(),
        modifiers,
        repeat: false,
    }
}

fn clipboard_key(logical_key: &str) -> SurfaceEvent {
    SurfaceEvent::Keyboard {
        timestamp: EventTimestamp::new(1),
        state: KeyState::Down,
        physical_key: 0,
        logical_key: logical_key.into(),
        modifiers: Modifiers::from_bits(Modifiers::COMMAND),
        repeat: false,
    }
}

fn clipboard_event(event: ClipboardEvent) -> SurfaceEvent {
    SurfaceEvent::Clipboard {
        timestamp: EventTimestamp::new(2),
        event,
    }
}

#[test]
fn clipboard_policy_controls_distinguish_each_response_boundary() -> Result<(), SurfaceError> {
    assert_eq!(
        EventEffect::visual().merge(EventEffect::default()),
        EventEffect::visual()
    );
    assert_eq!(
        EventEffect::default().merge(EventEffect::visual()),
        EventEffect::visual()
    );
    assert_eq!(
        EventEffect::document().merge(EventEffect::default()),
        EventEffect::document()
    );
    assert_eq!(
        EventEffect::default().merge(EventEffect::document()),
        EventEffect::document()
    );
    assert_eq!(
        EventEffect::document_replacement().merge(EventEffect::default()),
        EventEffect::document_replacement()
    );
    assert_eq!(
        EventEffect::default().merge(EventEffect::document_replacement()),
        EventEffect::document_replacement()
    );

    let shortcut = |modifiers: u8| SurfaceEvent::Keyboard {
        timestamp: EventTimestamp::new(1),
        state: KeyState::Down,
        physical_key: 0,
        logical_key: "c".into(),
        modifiers: Modifiers::from_bits(modifiers),
        repeat: false,
    };
    assert_eq!(
        studio_clipboard_shortcut(&shortcut(Modifiers::COMMAND)),
        Some(ClipboardOperation::Copy)
    );
    for modifiers in [
        0,
        Modifiers::COMMAND | Modifiers::CONTROL,
        Modifiers::COMMAND | Modifiers::OPTION,
        Modifiers::COMMAND | Modifiers::SHIFT,
    ] {
        assert_eq!(studio_clipboard_shortcut(&shortcut(modifiers)), None);
    }

    let mut app = test_app()?;
    let pending = PendingCut {
        revision: app.buffer().revision().get(),
        selection: Selection::new(ByteOffset::new(0), ByteOffset::new(1)),
    };
    app.pending_cut = Some(pending);
    let rejected_copy = app.reject_clipboard_response(ClipboardOperation::Copy);
    assert!(rejected_copy.visual_changed);
    assert_eq!(app.pending_cut, Some(pending));
    assert_eq!(app.clipboard_failures, 1);
    assert!(matches!(app.local_status, Some(LocalStatus::Clipboard(_))));
    assert!(app.clear_clipboard_status().visual_changed);

    let rejected_cut = app.reject_clipboard_response(ClipboardOperation::Cut);
    assert!(rejected_cut.visual_changed);
    assert_eq!(app.pending_cut, None);
    assert_eq!(app.clipboard_failures, 2);

    app.last_clipboard_error = Some(ClipboardError::Unavailable);
    let cleared = app.clear_clipboard_status();
    assert!(cleared.visual_changed);
    assert_eq!(app.local_status, None);
    assert_eq!(app.last_clipboard_error, None);
    assert!(!app.clear_clipboard_status().visual_changed);

    let failures = app.input_failures;
    app.resolve_close_admission(false, false);
    app.resolve_close_admission(true, true);
    assert_eq!(app.input_failures, failures);
    app.resolve_close_admission(true, false);
    assert_eq!(app.input_failures, failures + 1);

    let admitted =
        app.resolve_clipboard_admission(EventEffect::document(), ClipboardOperation::Copy, true);
    assert_eq!(admitted, EventEffect::document());
    let rejected =
        app.resolve_clipboard_admission(EventEffect::default(), ClipboardOperation::Copy, false);
    assert!(rejected.visual_changed);
    Ok(())
}

#[test]
fn clipboard_defensive_paths_preserve_document_state() -> Result<(), SurfaceError> {
    let mut app = test_app()?;
    *app.buffer_mut() = Buffer::new("abc");

    let empty = app.begin_clipboard_operation(ClipboardOperation::Copy);
    assert!(empty.clipboard_write.is_none());

    app.selection = Selection::new(ByteOffset::new(4), ByteOffset::new(5));
    let invalid = app.begin_clipboard_operation(ClipboardOperation::Copy);
    assert!(invalid.clipboard_write.is_none());
    assert!(invalid.effect.visual_changed);

    app.selection = Selection::new(ByteOffset::new(0), ByteOffset::new(1));
    let invalid_operation = app.begin_clipboard_operation(ClipboardOperation::Paste);
    assert!(invalid_operation.clipboard_write.is_none());
    assert!(invalid_operation.effect.visual_changed);
    assert!(app.last_clipboard_error.is_some());

    let copy_failure = app.handle_event_with_response(&clipboard_event(
        ClipboardEvent::CopyCompleted(Err(ClipboardError::WriteRejected)),
    ));
    assert!(copy_failure.effect.visual_changed);
    assert_eq!(
        app.last_clipboard_error,
        Some(ClipboardError::WriteRejected)
    );
    let copy_success =
        app.handle_event_with_response(&clipboard_event(ClipboardEvent::CopyCompleted(Ok(()))));
    assert!(copy_success.effect.visual_changed);
    assert_eq!(app.last_clipboard_error, None);

    let missing =
        app.handle_event_with_response(&clipboard_event(ClipboardEvent::CutCompleted(Ok(()))));
    assert!(missing.effect.visual_changed);
    assert!(!missing.effect.document_changed);

    let invalid_selection = Selection::new(ByteOffset::new(4), ByteOffset::new(5));
    app.selection = invalid_selection;
    app.pending_cut = Some(PendingCut {
        revision: app.buffer().revision().get(),
        selection: invalid_selection,
    });
    let before = app.buffer().snapshot().text();
    let atomic_failure =
        app.handle_event_with_response(&clipboard_event(ClipboardEvent::CutCompleted(Ok(()))));
    assert!(atomic_failure.effect.visual_changed);
    assert!(!atomic_failure.effect.document_changed);
    assert_eq!(app.buffer().snapshot().text(), before);

    app.selection = Selection::new(ByteOffset::new(0), ByteOffset::new(1));
    let paste_shortcut = app.handle_event_with_response(&clipboard_key("v"));
    assert!(paste_shortcut.clipboard_write.is_none());
    assert!(!paste_shortcut.effect.document_changed);
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "the 64 MiB ownership boundary is covered outside Miri")]
fn oversized_copy_selection_is_rejected_before_response_ownership() -> Result<(), SurfaceError> {
    let mut app = test_app()?;
    let oversized = "x".repeat(alpine_platform_macos::MAX_CLIPBOARD_TEXT_BYTES + 1);
    *app.buffer_mut() = Buffer::new(&oversized);
    app.selection = Selection::new(ByteOffset::new(0), ByteOffset::new(oversized.len()));
    let response = app.begin_clipboard_operation(ClipboardOperation::Copy);
    assert!(response.clipboard_write.is_none());
    assert!(response.effect.visual_changed);
    assert!(matches!(
        app.last_clipboard_error,
        Some(ClipboardError::TooLarge { .. })
    ));
    Ok(())
}

#[test]
fn editor_scene_contains_clipped_glyphs_atlas_and_caret() -> Result<(), StudioRenderError> {
    let mut app = test_app().map_err(|_| StudioRenderError::Domain)?;
    let scene = app.try_scene(
        SceneRevision::new(1),
        viewport().map_err(|_| StudioRenderError::Domain)?,
    )?;
    assert_eq!(scene.revision(), SceneRevision::new(1));
    assert_eq!(scene.clips().len(), 2);
    assert!(!scene.glyphs().is_empty());
    assert!(scene.glyph_atlas().is_some());
    assert!(scene.quads().len() >= 3);
    assert_eq!(app.render_failures, 0);
    Ok(())
}

#[test]
fn editor_scene_projects_compiled_rust_syntax_onto_visible_glyphs() -> Result<(), StudioRenderError>
{
    let document = StudioDocument::scratch("pub fn Main() { let count = 42; // local\n}");
    let mut app = StudioApp::from_document(TestTextSystem, document, Some(Path::new("main.rs")))
        .map_err(|_| StudioRenderError::Domain)?;
    let scene = app.try_scene(
        SceneRevision::new(1),
        viewport().map_err(|_| StudioRenderError::Domain)?,
    )?;
    let mut colors = Vec::new();
    for glyph in scene.glyphs() {
        if !colors.contains(&glyph.color()) {
            colors.push(glyph.color());
        }
    }
    assert!(
        colors.len() >= 5,
        "expected plain text plus four syntax classes"
    );
    let first_cache = app.syntax_cache.snapshot();
    assert!(first_cache.misses() > 0);
    let _second = app.try_scene(
        SceneRevision::new(2),
        viewport().map_err(|_| StudioRenderError::Domain)?,
    )?;
    let second_cache = app.syntax_cache.snapshot();
    assert!(second_cache.hits() > first_cache.hits());
    assert!(second_cache.current_bytes() <= second_cache.budget_bytes());

    let palette = app.settings.active().theme.syntax;
    let classes = [
        SyntaxClass::Comment,
        SyntaxClass::Keyword,
        SyntaxClass::String,
        SyntaxClass::Number,
        SyntaxClass::Type,
        SyntaxClass::Property,
        SyntaxClass::Heading,
        SyntaxClass::Code,
    ];
    let palette_colors = classes.map(|class| palette.color(class));
    assert!(
        palette_colors
            .iter()
            .enumerate()
            .all(|(index, color)| !palette_colors[..index].contains(color))
    );

    let render_error = StudioRenderError::from(SyntaxError::InvalidBudget);
    assert_eq!(
        render_error.to_string(),
        "syntax rendering failed: syntax cache budget must be nonzero"
    );
    Ok(())
}

#[test]
fn runtime_builds_only_after_an_accepted_editor_change() -> Result<(), RuntimeError> {
    let viewport = viewport()?;
    let clear = LinearRgba::new(0.02, 0.02, 0.02, 1.0).ok_or(SurfaceError::invariant(
        alpine_platform_macos::SurfaceOperation::Application,
    ))?;
    let mut application = Application::new(test_app()?, viewport, clear, WorkerConfig::default())?;
    let first = application.frame_if_dirty().ok_or(SurfaceError::invariant(
        alpine_platform_macos::SurfaceOperation::Application,
    ))?;
    assert!(
        application
            .dispatch(&SurfaceEvent::Wake {
                timestamp: alpine_platform_macos::EventTimestamp::new(1),
            })
            .is_none()
    );
    let changed = application
        .dispatch(&ime(ImeEvent::Committed("x".into())))
        .ok_or(SurfaceError::invariant(
            alpine_platform_macos::SurfaceOperation::Application,
        ))?;
    assert!(changed.scene().glyphs().len() > first.scene().glyphs().len());
    assert_eq!(application.snapshot().document_revision().get(), 1);
    assert!(
        application
            .dispatch(&SurfaceEvent::Wake {
                timestamp: EventTimestamp::new(2),
            })
            .is_none()
    );
    Ok(())
}

#[test]
fn runtime_find_worker_admits_current_results_and_schedules_replacement()
-> Result<(), Box<dyn std::error::Error>> {
    let viewport = viewport()?;
    let clear = LinearRgba::new(0.02, 0.02, 0.02, 1.0).ok_or(SurfaceError::invariant(
        alpine_platform_macos::SurfaceOperation::Application,
    ))?;
    let mut app = test_app()?;
    *app.buffer_mut() = Buffer::new("alpha beta alpha");
    let mut runtime = Application::new(app, viewport, clear, WorkerConfig::default())?;
    let command = Modifiers::from_bits(Modifiers::COMMAND);
    let command_option = Modifiers::from_bits(Modifiers::COMMAND | Modifiers::OPTION);

    runtime.dispatch(&key(KEY_F, command)).ok_or("find frame")?;
    let pending = runtime
        .dispatch(&ime(ImeEvent::Committed("alpha".into())))
        .ok_or("query frame")?;
    let pending_quads = pending.scene().quads().len();
    let mut admitted = false;
    for timestamp in 10..266 {
        std::thread::sleep(std::time::Duration::from_millis(1));
        if let Some(frame) = runtime.dispatch(&SurfaceEvent::Wake {
            timestamp: EventTimestamp::new(timestamp),
        }) && frame.scene().quads().len() > pending_quads
        {
            admitted = true;
            break;
        }
    }
    assert!(admitted);

    runtime
        .dispatch(&key(KEY_F, command_option))
        .ok_or("replace frame")?;
    runtime
        .dispatch(&ime(ImeEvent::Committed("x".into())))
        .ok_or("replacement frame")?;
    runtime
        .dispatch(&key(KEY_RETURN, command))
        .ok_or("replace current frame")?;
    assert_eq!(runtime.snapshot().document_revision().get(), 1);

    let root = TestWorkspace::new()?;
    root.write("alpha.rs", "alpha")?;
    root.write("beta.rs", "beta")?;
    let mut tab_app = StudioApp::open_workspace(TestTextSystem, root.path())?;
    let workspace = tab_app.workspace.as_ref().ok_or("workspace")?;
    let alpha = workspace.index_named("alpha.rs").ok_or("alpha")?;
    let beta = workspace.index_named("beta.rs").ok_or("beta")?;
    tab_app.open_workspace_entry(alpha)?;
    tab_app.open_workspace_entry(beta)?;
    let mut tab_runtime = Application::new(tab_app, viewport, clear, WorkerConfig::default())?;
    let before = tab_runtime.snapshot().document_revision().get();
    tab_runtime
        .dispatch(&key(KEY_LEFT_BRACKET, command))
        .ok_or("tab frame")?;
    assert!(tab_runtime.snapshot().document_revision().get() > before);
    Ok(())
}

#[test]
fn find_failure_paths_fail_closed_without_mutating_the_document()
-> Result<(), Box<dyn std::error::Error>> {
    let command = Modifiers::from_bits(Modifiers::COMMAND);
    let command_option = Modifiers::from_bits(Modifiers::COMMAND | Modifiers::OPTION);

    let mut failed = test_app()?;
    assert!(failed.handle_event(&key(KEY_F, command)).visual_changed);
    assert!(
        failed
            .handle_event(&ime(ImeEvent::Committed("alpha".into())))
            .visual_changed
    );
    let request = failed
        .prepare_find_request()?
        .ok_or(StudioRenderError::Domain)?;
    let identity = request.identity();
    assert!(
        failed
            .apply_find_output(FindWorkerOutput::failure_for_test(
                identity,
                FindError::IncompleteResult,
            ))
            .visual_changed
    );

    for ranges in [
        std::iter::once(usize::MAX..usize::MAX).collect(),
        vec![0..2, 1..3],
        std::iter::once(std::ops::Range { start: 3, end: 2 }).collect(),
        std::iter::once(usize::from(u16::MAX)..usize::from(u16::MAX) + 1).collect(),
    ] {
        let mut app = test_app()?;
        *app.buffer_mut() = Buffer::new("alpha beta alpha");
        assert!(app.handle_event(&key(KEY_F, command)).visual_changed);
        assert!(
            app.handle_event(&ime(ImeEvent::Committed("alpha".into())))
                .visual_changed
        );
        assert!(app.complete_pending_find_for_test()?.visual_changed);
        app.find.replace_ranges_for_test(ranges);
        assert!(app.handle_event(&key(KEY_F, command_option)).visual_changed);
        assert!(
            app.handle_event(&ime(ImeEvent::Committed("x".into())))
                .visual_changed
        );
        let before = app.buffer().snapshot().text();
        let effect = app.handle_event(&key(KEY_RETURN, command_option));
        assert!(!effect.document_changed);
        assert_eq!(app.buffer().snapshot().text(), before);
    }

    let mut oversized = test_app()?;
    oversized.find.oversize_query_for_test();
    oversized.find_needs_search = true;
    assert!(matches!(
        oversized.prepare_find_request(),
        Err(FindError::QueryTooLong { .. })
    ));

    let mut invalid_navigation = test_app()?;
    *invalid_navigation.buffer_mut() = Buffer::new("alpha");
    invalid_navigation.handle_event(&key(KEY_F, command));
    invalid_navigation.handle_event(&ime(ImeEvent::Committed("alpha".into())));
    invalid_navigation.complete_pending_find_for_test()?;
    invalid_navigation
        .find
        .replace_ranges_for_test(std::iter::once(usize::MAX..usize::MAX).collect());
    assert!(
        !invalid_navigation
            .apply_find_navigation(FindNavigation::new(usize::MAX, false))
            .visual_changed
    );
    assert!(
        invalid_navigation
            .handle_event(&key(KEY_RETURN, Modifiers::default()))
            .visual_changed
    );

    let mut scrolled_navigation = test_app()?;
    let deep_source = format!("{}target{}", "\n".repeat(40), "\n".repeat(40));
    *scrolled_navigation.buffer_mut() = Buffer::new(&deep_source);
    scrolled_navigation.try_scene(
        SceneRevision::new(1),
        viewport().map_err(|_| StudioRenderError::Domain)?,
    )?;
    scrolled_navigation.handle_event(&key(KEY_F, command));
    scrolled_navigation.handle_event(&ime(ImeEvent::Committed("target".into())));
    assert!(
        scrolled_navigation
            .complete_pending_find_for_test()?
            .visual_changed
    );
    assert_eq!(scrolled_navigation.scroll_y.to_bits(), 410.0_f32.to_bits());
    let highlighted = scrolled_navigation.try_scene(
        SceneRevision::new(2),
        viewport().map_err(|_| StudioRenderError::Domain)?,
    )?;
    assert!(!highlighted.glyphs().is_empty());
    assert!(highlighted.quads().len() > 3);

    Ok(())
}

#[test]
fn malformed_find_highlight_fails_before_scene_publication()
-> Result<(), Box<dyn std::error::Error>> {
    let mut app = test_app()?;
    *app.buffer_mut() = Buffer::new("é");
    let command = Modifiers::from_bits(Modifiers::COMMAND);
    app.handle_event(&key(KEY_F, command));
    app.handle_event(&ime(ImeEvent::Committed("é".into())));
    app.complete_pending_find_for_test()?;
    app.find
        .replace_ranges_for_test(std::iter::once(1..2).collect());
    assert!(matches!(
        app.try_scene(
            SceneRevision::new(3),
            viewport().map_err(|_| StudioRenderError::Domain)?,
        ),
        Err(StudioRenderError::Text(_))
    ));
    Ok(())
}

#[test]
fn runtime_find_failures_are_bounded_and_visible() -> Result<(), Box<dyn std::error::Error>> {
    let viewport = viewport()?;
    let clear = LinearRgba::new(0.02, 0.02, 0.02, 1.0).ok_or(SurfaceError::invariant(
        alpine_platform_macos::SurfaceOperation::Application,
    ))?;
    let command = Modifiers::from_bits(Modifiers::COMMAND);
    let command_option = Modifiers::from_bits(Modifiers::COMMAND | Modifiers::OPTION);

    let mut exhausted = test_app()?;
    *exhausted.buffer_mut() = Buffer::new("alpha beta alpha");
    exhausted.handle_event(&key(KEY_F, command));
    exhausted.handle_event(&ime(ImeEvent::Committed("alpha".into())));
    exhausted.find.exhaust_generation_for_test();
    exhausted.complete_pending_find_for_test()?;
    exhausted.handle_event(&key(KEY_F, command_option));
    exhausted.handle_event(&ime(ImeEvent::Committed("x".into())));
    let mut exhausted_runtime =
        Application::new(exhausted, viewport, clear, WorkerConfig::default())?;
    assert!(
        exhausted_runtime
            .dispatch(&key(KEY_RETURN, command))
            .is_some()
    );

    let mut oversized = test_app()?;
    oversized.find.oversize_query_for_test();
    oversized.find_needs_search = true;
    let mut oversized_runtime =
        Application::new(oversized, viewport, clear, WorkerConfig::default())?;
    assert!(
        oversized_runtime
            .dispatch(&SurfaceEvent::Wake {
                timestamp: EventTimestamp::new(1),
            })
            .is_some()
    );

    #[cfg(not(miri))]
    {
        let mut saturated = test_app()?;
        let saturated_source = "x".repeat(16 * 1024 * 1024);
        *saturated.buffer_mut() = Buffer::new(&saturated_source);
        let mut saturated_runtime =
            Application::new(saturated, viewport, clear, WorkerConfig::default())?;
        saturated_runtime.dispatch(&key(KEY_F, command));
        for query_byte in ["y", "y", "y"] {
            assert!(
                saturated_runtime
                    .dispatch(&ime(ImeEvent::Committed(query_byte.into())))
                    .is_some()
            );
        }
    }
    Ok(())
}

#[test]
fn find_scroll_and_replacement_budget_boundaries_are_exact()
-> Result<(), Box<dyn std::error::Error>> {
    let mut scrolling = test_app()?;
    let scrolling_source = "\n".repeat(80);
    *scrolling.buffer_mut() = Buffer::new(&scrolling_source);
    scrolling.scroll_y = 0.0;
    scrolling.select_find_range(22..22);
    assert_eq!(scrolling.scroll_y.to_bits(), 14.0_f32.to_bits());
    scrolling.scroll_y = 10.0;
    scrolling.select_find_range(21..21);
    assert_eq!(scrolling.scroll_y.to_bits(), 10.0_f32.to_bits());
    scrolling.scroll_y = 400.0;
    scrolling.select_find_range(40..40);
    assert_eq!(scrolling.scroll_y.to_bits(), 410.0_f32.to_bits());
    scrolling.scroll_y = 500.0;
    scrolling.select_find_range(44..44);
    assert_eq!(scrolling.scroll_y.to_bits(), 500.0_f32.to_bits());

    let command = Modifiers::from_bits(Modifiers::COMMAND);
    let command_option = Modifiers::from_bits(Modifiers::COMMAND | Modifiers::OPTION);
    let mut budget = test_app()?;
    *budget.buffer_mut() = Buffer::new("x");
    budget.handle_event(&key(KEY_F, command));
    budget.handle_event(&ime(ImeEvent::Committed("x".into())));
    budget.complete_pending_find_for_test()?;
    budget.find.replace_ranges_for_test(
        std::iter::once(0..MAX_REPLACEMENT_TRANSACTION_BYTES - crate::find::MAX_QUERY_BYTES)
            .collect(),
    );
    budget.handle_event(&key(KEY_F, command_option));
    budget.handle_event(&ime(ImeEvent::Committed(
        "r".repeat(crate::find::MAX_QUERY_BYTES).into(),
    )));
    let failures = budget.input_failures;
    let effect = budget.handle_event(&key(KEY_RETURN, command_option));
    assert!(!effect.document_changed);
    assert_eq!(budget.input_failures, failures + 1);
    Ok(())
}

#[test]
fn clipboard_copy_cut_and_paste_preserve_revision_and_selection_identity()
-> Result<(), Box<dyn std::error::Error>> {
    let mut app = test_app()?;
    *app.buffer_mut() = Buffer::new("alpha");
    app.selection = Selection::new(ByteOffset::new(1), ByteOffset::new(4));

    let before_copy = app.buffer().snapshot().text();
    let copy = app.handle_event_with_response(&clipboard_key("c"));
    let (operation, text) = copy
        .clipboard_write
        .ok_or("copy did not produce a response")?
        .into_parts();
    assert_eq!(operation, ClipboardOperation::Copy);
    assert_eq!(text.as_str(), "lph");
    assert_eq!(app.buffer().snapshot().text(), before_copy);
    assert_eq!(app.selection.range(), 1..4);

    let cut = app.handle_event_with_response(&clipboard_key("x"));
    let (operation, text) = cut
        .clipboard_write
        .ok_or("cut did not produce a response")?
        .into_parts();
    assert_eq!(operation, ClipboardOperation::Cut);
    assert_eq!(text.as_str(), "lph");
    assert_eq!(app.buffer().snapshot().text(), "alpha");
    let completed =
        app.handle_event_with_response(&clipboard_event(ClipboardEvent::CutCompleted(Ok(()))));
    assert!(completed.effect.document_changed);
    assert_eq!(app.buffer().snapshot().text(), "aa");
    assert_eq!(app.selection, Selection::caret(ByteOffset::new(1)));

    app.selection = Selection::new(ByteOffset::new(0), ByteOffset::new(1));
    let pasted = app.handle_event_with_response(&clipboard_event(ClipboardEvent::PasteCompleted(
        Ok(ClipboardText::new("z")?),
    )));
    assert!(pasted.effect.document_changed);
    assert_eq!(app.buffer().snapshot().text(), "za");
    assert_eq!(app.selection, Selection::caret(ByteOffset::new(1)));
    assert_eq!(app.clipboard_failures, 0);
    Ok(())
}

#[test]
fn cut_completion_rejects_stale_selection_revision_and_native_failure()
-> Result<(), Box<dyn std::error::Error>> {
    let mut app = test_app()?;
    *app.buffer_mut() = Buffer::new("alpha");
    app.selection = Selection::new(ByteOffset::new(0), ByteOffset::new(2));

    let selection_cut = app.handle_event_with_response(&clipboard_key("x"));
    assert!(selection_cut.clipboard_write.is_some());
    app.selection = Selection::new(ByteOffset::new(1), ByteOffset::new(2));
    let before_selection_completion = app.buffer().snapshot().text();
    let stale_selection =
        app.handle_event_with_response(&clipboard_event(ClipboardEvent::CutCompleted(Ok(()))));
    assert!(!stale_selection.effect.document_changed);
    assert_eq!(app.buffer().snapshot().text(), before_selection_completion);

    app.selection = Selection::new(ByteOffset::new(0), ByteOffset::new(2));
    let revision_cut = app.handle_event_with_response(&clipboard_key("x"));
    assert!(revision_cut.clipboard_write.is_some());
    assert!(app.replace_range(5..5, "!").document_changed);
    app.selection = Selection::new(ByteOffset::new(0), ByteOffset::new(2));
    let before_revision_completion = app.buffer().snapshot().text();
    let stale_revision =
        app.handle_event_with_response(&clipboard_event(ClipboardEvent::CutCompleted(Ok(()))));
    assert!(!stale_revision.effect.document_changed);
    assert_eq!(app.buffer().snapshot().text(), before_revision_completion);

    let failure_cut = app.handle_event_with_response(&clipboard_key("x"));
    assert!(failure_cut.clipboard_write.is_some());
    let before_failure = app.buffer().snapshot().text();
    let selection_before_failure = app.selection;
    let failed = app.handle_event_with_response(&clipboard_event(ClipboardEvent::CutCompleted(
        Err(ClipboardError::WriteRejected),
    )));
    assert!(failed.effect.visual_changed);
    assert!(!failed.effect.document_changed);
    assert_eq!(app.buffer().snapshot().text(), before_failure);
    assert_eq!(app.selection, selection_before_failure);
    assert_eq!(
        app.last_clipboard_error,
        Some(ClipboardError::WriteRejected)
    );
    assert!(matches!(app.local_status, Some(LocalStatus::Clipboard(_))));
    assert_eq!(app.clipboard_failures, 3);
    Ok(())
}

#[test]
fn clipboard_failure_is_visible_and_paste_failure_is_atomic()
-> Result<(), Box<dyn std::error::Error>> {
    let mut app = test_app()?;
    *app.buffer_mut() = Buffer::new("stable");
    app.selection = Selection::new(ByteOffset::new(1), ByteOffset::new(4));
    let before = app.buffer().snapshot().text();
    let before_selection = app.selection;
    let failed = app.handle_event_with_response(&clipboard_event(ClipboardEvent::PasteCompleted(
        Err(ClipboardError::Unavailable),
    )));
    assert!(failed.effect.visual_changed);
    assert!(!failed.effect.document_changed);
    assert_eq!(app.buffer().snapshot().text(), before);
    assert_eq!(app.selection, before_selection);
    assert_eq!(app.last_clipboard_error, Some(ClipboardError::Unavailable));

    let baseline_quads = test_app()?
        .try_scene(SceneRevision::new(1), viewport()?)?
        .quads()
        .len();
    let failure_scene = app.try_scene(SceneRevision::new(1), viewport()?)?;
    assert!(failure_scene.quads().len() > baseline_quads);
    assert!(failure_scene.glyphs().len() > before.len());
    Ok(())
}

#[test]
fn status_raster_failure_is_structured_after_empty_document_layout() -> Result<(), StudioRenderError>
{
    let mut app = StudioApp::new(FailingRasterTextSystem).map_err(|_| StudioRenderError::Domain)?;
    *app.buffer_mut() = Buffer::new("");
    app.local_status = Some(LocalStatus::Clipboard(Arc::from("status")));
    assert!(matches!(
        app.try_scene(
            SceneRevision::new(1),
            viewport().map_err(|_| StudioRenderError::Domain)?,
        ),
        Err(StudioRenderError::Layout(LayoutError::NativeFailure(
            "injected status raster failure"
        )))
    ));

    let mut overlay =
        StudioApp::new(FailingRasterTextSystem).map_err(|_| StudioRenderError::Domain)?;
    *overlay.buffer_mut() = Buffer::new("");
    assert!(
        overlay
            .handle_event(&key(KEY_F, Modifiers::from_bits(Modifiers::COMMAND),))
            .visual_changed
    );
    assert!(matches!(
        overlay.try_scene(
            SceneRevision::new(2),
            viewport().map_err(|_| StudioRenderError::Domain)?,
        ),
        Err(StudioRenderError::Layout(LayoutError::NativeFailure(
            "injected status raster failure"
        )))
    ));
    Ok(())
}

#[test]
fn studio_runtime_returns_clipboard_and_dirty_close_responses()
-> Result<(), Box<dyn std::error::Error>> {
    let viewport = viewport()?;
    let clear = LinearRgba::new(0.02, 0.02, 0.02, 1.0).ok_or(SurfaceError::invariant(
        alpine_platform_macos::SurfaceOperation::Application,
    ))?;
    let mut copy_app = test_app()?;
    copy_app.selection = Selection::new(ByteOffset::new(0), ByteOffset::new(2));
    let mut copy_runtime = Application::new(copy_app, viewport, clear, WorkerConfig::default())?;
    let (_, write, close) = copy_runtime
        .dispatch_with_response(&clipboard_key("c"))
        .into_parts();
    let (operation, text) = write
        .ok_or("runtime omitted clipboard response")?
        .into_parts();
    assert_eq!(operation, ClipboardOperation::Copy);
    assert_eq!(text.as_str(), &INITIAL_TEXT[..2]);
    assert_eq!(close, CloseDisposition::NotRequested);

    let mut dirty_app = test_app()?;
    assert!(
        dirty_app
            .handle_event(&ime(ImeEvent::Committed("x".into())))
            .document_changed
    );
    assert!(dirty_app.document.is_dirty());
    let mut dirty_runtime = Application::new(dirty_app, viewport, clear, WorkerConfig::default())?;
    let (_, _, close) = dirty_runtime
        .dispatch_with_response(&SurfaceEvent::CloseRequested {
            timestamp: EventTimestamp::new(3),
        })
        .into_parts();
    assert_eq!(close, CloseDisposition::Cancel);
    assert!(!dirty_runtime.snapshot().is_shutting_down());

    let mut clean_runtime =
        Application::new(test_app()?, viewport, clear, WorkerConfig::default())?;
    let (_, _, close) = clean_runtime
        .dispatch_with_response(&SurfaceEvent::CloseRequested {
            timestamp: EventTimestamp::new(4),
        })
        .into_parts();
    assert_eq!(close, CloseDisposition::Allow);
    assert!(clean_runtime.snapshot().is_shutting_down());
    Ok(())
}

#[test]
#[cfg(not(target_family = "windows"))]
fn dirty_file_close_is_blocked_until_atomic_save_succeeds() -> Result<(), Box<dyn std::error::Error>>
{
    let file = TestFile::new("before")?;
    let mut app = StudioApp::open_file(TestTextSystem, file.path())?;
    assert!(
        !app.handle_event_with_response(&SurfaceEvent::CloseRequested {
            timestamp: EventTimestamp::new(1),
        })
        .cancel_close
    );
    assert!(
        app.handle_event(&ime(ImeEvent::Committed("x".into())))
            .document_changed
    );
    let blocked = app.handle_event_with_response(&SurfaceEvent::CloseRequested {
        timestamp: EventTimestamp::new(2),
    });
    assert!(blocked.cancel_close);
    assert_eq!(app.local_status, Some(LocalStatus::CloseBlocked));
    assert!(
        app.handle_event(&key(KEY_S, Modifiers::from_bits(Modifiers::COMMAND)))
            .visual_changed
    );
    assert!(!app.document.is_dirty());
    assert_eq!(app.local_status, None);
    assert!(
        !app.handle_event_with_response(&SurfaceEvent::CloseRequested {
            timestamp: EventTimestamp::new(3),
        })
        .cancel_close
    );
    Ok(())
}

#[test]
#[cfg(not(target_family = "windows"))]
fn real_file_open_edit_save_and_conflicts_are_atomic() -> Result<(), Box<dyn std::error::Error>> {
    let file = TestFile::new("before")?;
    let mut app = StudioApp::open_file(TestTextSystem, file.path())?;
    assert_eq!(app.buffer().snapshot().text(), "before");
    assert!(!app.document.is_dirty());

    assert!(
        app.handle_event(&ime(ImeEvent::Committed("x".into())))
            .document_changed
    );
    assert!(app.document.is_dirty());
    assert!(
        !app.handle_event(&key(KEY_S, Modifiers::from_bits(Modifiers::COMMAND)))
            .visual_changed
    );
    assert_eq!(fs::read_to_string(file.path())?, "xbefore");
    assert!(!app.document.is_dirty());
    let report = app.last_save.ok_or("missing save report")?;
    assert_eq!(report.revision(), app.buffer().revision());
    assert_eq!(report.bytes_written(), "xbefore".len());
    assert_eq!(app.save_failures, 0);
    assert_eq!(app.last_file_error, None);

    assert!(
        app.handle_event(&ime(ImeEvent::Committed("y".into())))
            .document_changed
    );
    fs::write(file.path(), "external")?;
    let _effect = app.handle_event(&key(KEY_S, Modifiers::from_bits(Modifiers::COMMAND)));
    assert_eq!(fs::read_to_string(file.path())?, "external");
    assert!(app.document.is_dirty());
    assert_eq!(app.save_failures, 1);
    assert_eq!(
        app.last_file_error,
        Some(FileError::Conflict(alpine_text::ExternalChange::Modified))
    );

    fs::remove_file(file.path())?;
    let _effect = app.handle_event(&key(KEY_S, Modifiers::from_bits(Modifiers::COMMAND)));
    assert_eq!(app.save_failures, 2);
    assert_eq!(
        app.last_file_error,
        Some(FileError::Conflict(alpine_text::ExternalChange::Deleted))
    );
    Ok(())
}

#[test]
#[cfg(target_family = "windows")]
fn windows_file_save_fails_structurally_without_touching_disk()
-> Result<(), Box<dyn std::error::Error>> {
    let file = TestFile::new("before")?;
    let mut app = StudioApp::open_file(TestTextSystem, file.path())?;
    assert!(
        app.handle_event(&ime(ImeEvent::Committed("x".into())))
            .document_changed
    );
    let effect = app.handle_event(&key(KEY_S, Modifiers::from_bits(Modifiers::COMMAND)));
    assert!(!effect.visual_changed);
    assert!(!effect.document_changed);
    assert_eq!(fs::read_to_string(file.path())?, "before");
    assert!(app.document.is_dirty());
    assert_eq!(app.save_failures, 1);
    assert_eq!(
        app.last_file_error,
        Some(FileError::UnsupportedAtomicReplace)
    );
    assert_eq!(app.last_save, None);
    Ok(())
}

#[test]
fn file_launch_rejects_invalid_utf8_and_scratch_save_is_isolated()
-> Result<(), Box<dyn std::error::Error>> {
    let invalid = TestFile::new([0xff])?;
    assert!(matches!(
        StudioApp::open_file(TestTextSystem, invalid.path()),
        Err(StudioError::File(FileError::InvalidUtf8))
    ));

    let mut scratch = test_app()?;
    let before = scratch.buffer().snapshot().text();
    assert!(!scratch.document.is_dirty());
    let effect = scratch.handle_event(&key(KEY_S, Modifiers::from_bits(Modifiers::COMMAND)));
    assert_eq!(scratch.buffer().snapshot().text(), before);
    assert!(!effect.visual_changed);
    assert!(!effect.document_changed);
    assert_eq!(scratch.last_save, None);
    assert_eq!(scratch.save_failures, 0);
    assert_eq!(scratch.last_file_error, None);

    let usage = StudioError::Usage;
    assert_eq!(usage.to_string(), "usage: alpine-studio [path]");
    assert!(usage.source().is_none());
    let file_error = StudioError::from(FileError::InvalidUtf8);
    assert!(file_error.to_string().contains("Studio file failed"));
    assert!(file_error.source().is_some());
    let runtime_error = StudioError::from(RuntimeError::Surface(SurfaceError::UnsupportedPlatform));
    assert!(runtime_error.to_string().contains("Studio runtime failed"));
    assert!(runtime_error.source().is_some());
    Ok(())
}

#[test]
fn bounded_workspace_is_sorted_capped_and_projects_only_visible_rows()
-> Result<(), Box<dyn std::error::Error>> {
    let root = TestWorkspace::new()?;
    root.write("z.rs", "z")?;
    root.write("a.rs", "a")?;
    root.write("long-name.rs", "long")?;
    root.create_dir("src")?;
    let limits = WorkspaceLimits::new(16, 3, 64, 9);
    let workspace = Workspace::open(root.path(), limits)?;
    let snapshot = workspace.snapshot();
    assert_eq!(snapshot.scanned_entries, 4);
    assert_eq!(snapshot.retained_entries, 2);
    assert_eq!(snapshot.retained_name_bytes, 7);
    assert_eq!(snapshot.omitted_entries, 2);
    assert_eq!(snapshot.scan_limit, 16);
    assert_eq!(snapshot.entry_limit, 3);
    assert_eq!(snapshot.name_byte_limit, 9);
    assert_eq!(
        workspace
            .entry(0)
            .map(super::workspace::WorkspaceEntry::name)
            .as_deref(),
        Some("src")
    );
    assert_eq!(
        workspace
            .entry(1)
            .map(super::workspace::WorkspaceEntry::name)
            .as_deref(),
        Some("a.rs")
    );
    assert_eq!(workspace.visible_range(1, 1, 0), 1..2);
    assert_eq!(workspace.visible_range(9, 2, 1), 2..2);
    assert_eq!(workspace.len(), 2);

    let defaults = Workspace::open(root.path(), WorkspaceLimits::default())?;
    assert_eq!(defaults.snapshot().name_byte_limit, 256 * 1_024);

    let exact_scan = Workspace::open(root.path(), WorkspaceLimits::new(4, 4, 64, 64))?;
    assert_eq!(exact_scan.snapshot().scanned_entries, 4);

    let exact_name = Workspace::open(root.path(), WorkspaceLimits::new(4, 4, 3, 64))?;
    assert_eq!(exact_name.index_named("src"), Some(0));
    assert_eq!(exact_name.len(), 1);

    let exact_aggregate = Workspace::open(root.path(), WorkspaceLimits::new(4, 4, 64, 7))?;
    assert_eq!(exact_aggregate.len(), 2);
    assert_eq!(exact_aggregate.snapshot().retained_name_bytes, 7);

    let exact_capacity = Workspace::open(root.path(), WorkspaceLimits::new(4, 2, 64, 64))?;
    assert_eq!(exact_capacity.len(), 2);

    let truncated_app = StudioApp::from_workspace(TestTextSystem, workspace)?;
    assert_eq!(
        truncated_app
            .local_status
            .as_ref()
            .map(LocalStatus::message),
        Some("Workspace tree truncated: 2 entries omitted.")
    );

    let too_small = WorkspaceLimits::new(1, 1, 64, 64);
    assert!(matches!(
        Workspace::open(root.path(), too_small),
        Err(WorkspaceError::ScanLimitExceeded { limit: 1, .. })
    ));

    #[cfg(all(target_os = "linux", not(miri)))]
    {
        use std::os::unix::{ffi::OsStrExt, fs::symlink};

        let omitted_root = TestWorkspace::new()?;
        fs::write(
            omitted_root
                .path()
                .join(std::ffi::OsStr::from_bytes(b"invalid-\xff")),
            "invalid name",
        )?;
        let outside = TestFile::new("outside")?;
        symlink(outside.path(), omitted_root.path().join("link"))?;
        let omitted = Workspace::open(omitted_root.path(), WorkspaceLimits::new(2, 2, 64, 64))?;
        assert_eq!(omitted.snapshot().scanned_entries, 2);
        assert_eq!(omitted.snapshot().omitted_entries, 2);
        assert_eq!(omitted.len(), 0);
    }

    let rendered_root = TestWorkspace::new()?;
    for index in 0..100 {
        rendered_root.write(&format!("file-{index:03}.rs"), "x")?;
    }
    let mut app = StudioApp::open_workspace(TestTextSystem, rendered_root.path())?;
    let viewport = viewport().map_err(|_| StudioRenderError::Domain)?;
    let visible_rows = floor_f32_to_usize(viewport.height() / TREE_ROW_HEIGHT)
        .ok_or("invalid visible row count")?
        .saturating_add(1);
    let projected_tree_rows = app
        .file_tree
        .visible_rows(0, visible_rows, TREE_OVERSCAN_ROWS)?
        .len();
    let editor_rows = app.buffer().snapshot().line_count();
    TEST_SHAPE_CALLS.with(|calls| calls.set(0));
    let _scene = app.try_scene(SceneRevision::new(1), viewport)?;
    let expected_shapes = u64::try_from(
        projected_tree_rows
            .saturating_add(app.tabs.len())
            .saturating_add(editor_rows),
    )?;
    TEST_SHAPE_CALLS.with(|calls| assert_eq!(calls.get(), expected_shapes));
    Ok(())
}

#[test]
fn workspace_errors_and_statuses_preserve_exact_sources_and_messages()
-> Result<(), Box<dyn std::error::Error>> {
    let missing = std::env::temp_dir().join("alpine-studio-definitely-missing-workspace");
    let io_error = Workspace::open(&missing, WorkspaceLimits::default())
        .err()
        .ok_or("missing workspace unexpectedly opened")?;
    assert!(std::error::Error::source(&io_error).is_some());
    assert!(io_error.to_string().contains("canonicalize"));
    let file = TestFile::new("not a directory")?;
    assert!(matches!(
        Workspace::open(file.path(), WorkspaceLimits::default()),
        Err(WorkspaceError::NotDirectory(_))
    ));

    let variants = [
        WorkspaceError::NotDirectory(PathBuf::from("file")),
        WorkspaceError::UnsupportedTarget(PathBuf::from("target")),
        WorkspaceError::ScanLimitExceeded {
            root: PathBuf::from("root"),
            limit: 7,
        },
        WorkspaceError::AllocationFailed,
        WorkspaceError::EntryNotFound(9),
        WorkspaceError::NotRegularFile(PathBuf::from("directory")),
        WorkspaceError::EscapesRoot(PathBuf::from("outside")),
        WorkspaceError::InvalidRelativePath(PathBuf::from("invalid")),
        WorkspaceError::Symlink(PathBuf::from("link")),
        WorkspaceError::PathDepthExceeded {
            actual: 2,
            limit: 1,
        },
    ];
    for error in variants {
        assert!(!error.to_string().is_empty());
        assert!(std::error::Error::source(&error).is_none());
    }

    let wrapped = StudioError::from(WorkspaceError::AllocationFailed);
    assert!(wrapped.to_string().contains("Studio workspace failed"));
    assert!(std::error::Error::source(&wrapped).is_some());
    let workspace_selection = WorkspaceSelectionError::Workspace(WorkspaceError::AllocationFailed);
    assert!(std::error::Error::source(&workspace_selection).is_some());
    let quick_open_selection = WorkspaceSelectionError::QuickOpen(QuickOpenError::MissingSelection);
    assert!(
        quick_open_selection
            .to_string()
            .contains("quick open failed")
    );
    assert!(std::error::Error::source(&quick_open_selection).is_some());
    let quick_open_render = StudioRenderError::from(QuickOpenError::InvalidLimits);
    assert!(
        quick_open_render
            .to_string()
            .contains("quick-open rendering failed")
    );
    let invalid_file = TestFile::new([0xff])?;
    let file_error = StudioDocument::open(invalid_file.path())
        .err()
        .ok_or("invalid UTF-8 file unexpectedly opened")?;
    let file_selection = WorkspaceSelectionError::File(file_error);
    assert!(std::error::Error::source(&file_selection).is_some());
    for error in [
        WorkspaceSelectionError::NoWorkspace,
        WorkspaceSelectionError::DirtyDocument,
        WorkspaceSelectionError::RevisionExhausted,
    ] {
        assert!(!error.to_string().is_empty());
        assert!(std::error::Error::source(&error).is_none());
    }
    assert_eq!(
        LocalStatus::CloseBlocked.message(),
        "Save changes before closing."
    );
    assert_eq!(
        LocalStatus::Workspace(Arc::from("workspace status")).message(),
        "workspace status"
    );
    Ok(())
}

#[test]
#[allow(
    clippy::float_cmp,
    clippy::too_many_lines,
    reason = "exact geometry values distinguish the workspace painter and routing mutants"
)]
fn workspace_scene_geometry_and_scroll_routing_are_exact() -> Result<(), Box<dyn std::error::Error>>
{
    let root = TestWorkspace::new()?;
    for index in 0..40 {
        root.write(&format!("file-{index:03}.rs"), "line\n".repeat(100))?;
    }
    let mut app = StudioApp::open_workspace(TestTextSystem, root.path())?;
    let viewport = viewport().map_err(|_| StudioRenderError::Domain)?;
    assert_eq!(app.sidebar_width(viewport), SIDEBAR_WIDTH);
    assert_eq!(app.maximum_workspace_scroll(), 364.0);
    let scene = app.try_scene(SceneRevision::new(1), viewport)?;
    let expected_editor = Rect::new(
        Point::new(260.0, 24.0).ok_or("editor origin")?,
        Size::new(676.0, 492.0).ok_or("editor size")?,
    );
    let expected_sidebar = Rect::new(
        Point::new(0.0, 0.0).ok_or("sidebar origin")?,
        Size::new(236.0, 540.0).ok_or("sidebar size")?,
    );
    assert_eq!(scene.clips()[0].bounds(), expected_editor);
    assert_eq!(scene.clips()[1].bounds(), expected_sidebar);
    assert_eq!(scene.quads()[1].bounds(), expected_editor);
    assert_eq!(scene.quads()[2].bounds(), expected_sidebar);
    assert_eq!(scene.glyphs()[0].bounds().origin().x(), CONTENT_INSET);
    assert_eq!(scene.glyphs()[0].bounds().origin().y(), 36.0);

    let file_index = app
        .workspace
        .as_ref()
        .and_then(|workspace| workspace.index_named("file-010.rs"))
        .ok_or("missing file")?;
    let opened = app.open_workspace_entry(file_index)?;
    assert_eq!(opened, EventEffect::document_replacement());
    let active_scene = app.try_scene(SceneRevision::new(2), viewport)?;
    let expected_active = Rect::new(
        Point::new(
            0.0,
            CONTENT_INSET + usize_as_f32(file_index) * TREE_ROW_HEIGHT,
        )
        .ok_or("active origin")?,
        Size::new(SIDEBAR_WIDTH, TREE_ROW_HEIGHT).ok_or("active size")?,
    );
    assert_eq!(active_scene.quads()[3].bounds(), expected_active);

    app.workspace_scroll_y = 110.0;
    let scrolled_scene = app.try_scene(SceneRevision::new(3), viewport)?;
    assert_eq!(scrolled_scene.glyphs()[0].bounds().origin().y(), -30.0);

    app.workspace_scroll_y = 0.0;
    app.composition = Some(Composition {
        replacement: 1..1,
        text: "q".into(),
        selected_start_utf16: 0,
        selected_length_utf16: 0,
    });
    let composition_scene = app.try_scene(SceneRevision::new(4), viewport)?;
    assert_eq!(
        composition_scene
            .glyphs()
            .last()
            .ok_or("missing composition glyph")?
            .bounds()
            .origin()
            .x(),
        268.0
    );
    app.composition = None;
    app.local_status = Some(LocalStatus::Workspace(Arc::from("s")));
    let status_scene = app.try_scene(SceneRevision::new(5), viewport)?;
    assert_eq!(
        status_scene
            .glyphs()
            .last()
            .ok_or("missing status glyph")?
            .bounds()
            .origin()
            .x(),
        266.0
    );
    assert_eq!(
        status_scene
            .glyphs()
            .last()
            .ok_or("missing status glyph")?
            .bounds()
            .origin()
            .y(),
        506.0
    );
    app.local_status = None;

    app.last_pointer_position = Point::new(SIDEBAR_WIDTH - 1.0, CONTENT_INSET);
    assert!(
        app.handle_event(&SurfaceEvent::Scroll {
            timestamp: EventTimestamp::new(1),
            delta_x: 0.0,
            delta_y: -22.0,
            phase: ScrollPhase::Changed,
            precise: true,
            modifiers: Modifiers::default(),
        })
        .visual_changed
    );
    assert_eq!(app.workspace_scroll_y, 22.0);
    assert_eq!(app.scroll_y, 0.0);

    app.last_pointer_position = Point::new(SIDEBAR_WIDTH, CONTENT_INSET);
    assert!(
        app.handle_event(&SurfaceEvent::Scroll {
            timestamp: EventTimestamp::new(2),
            delta_x: 0.0,
            delta_y: -22.0,
            phase: ScrollPhase::Changed,
            precise: true,
            modifiers: Modifiers::default(),
        })
        .visual_changed
    );
    assert_eq!(app.workspace_scroll_y, 22.0);
    assert_eq!(app.scroll_y, 22.0);

    assert_eq!(
        app.offset_at_point(Point::new(259.0, CONTENT_INSET).ok_or("outside editor")?),
        None
    );
    app.scroll_y = 0.0;
    assert_eq!(
        app.offset_at_point(Point::new(260.0, CONTENT_INSET).ok_or("editor edge")?),
        Some(ByteOffset::new(0))
    );
    assert_eq!(
        app.offset_at_point(Point::new(268.0, CONTENT_INSET).ok_or("editor glyph")?),
        Some(ByteOffset::new(1))
    );
    let tiny = Size::new(100.0, WINDOW_HEIGHT).ok_or("tiny viewport")?;
    assert_eq!(app.sidebar_width(tiny), 99.0);

    let mut pointer_app = StudioApp::open_workspace(TestTextSystem, root.path())?;
    let failures = pointer_app.workspace_failures;
    let secondary = pointer_app.handle_pointer(
        PointerAction::Down,
        Point::new(1.0, CONTENT_INSET + 1.0).ok_or("secondary point")?,
        PointerButton::Secondary,
        Modifiers::default(),
    );
    assert_eq!(secondary, EventEffect::default());
    assert_eq!(pointer_app.workspace_failures, failures);
    let edge = pointer_app.handle_pointer(
        PointerAction::Down,
        Point::new(SIDEBAR_WIDTH, CONTENT_INSET + 1.0).ok_or("sidebar edge")?,
        PointerButton::Primary,
        Modifiers::default(),
    );
    assert_eq!(edge, EventEffect::default());
    assert_eq!(pointer_app.active_workspace_entry, None);
    let unrepresentable_row = pointer_app.handle_pointer(
        PointerAction::Down,
        Point::new(1.0, CONTENT_INSET + 20_000_000.0 * TREE_ROW_HEIGHT)
            .ok_or("unrepresentable row")?,
        PointerButton::Primary,
        Modifiers::default(),
    );
    assert_eq!(unrepresentable_row, EventEffect::default());
    pointer_app.workspace_scroll_y = TREE_ROW_HEIGHT;
    let row_one = pointer_app.handle_pointer(
        PointerAction::Down,
        Point::new(1.0, CONTENT_INSET + 1.0).ok_or("row one")?,
        PointerButton::Primary,
        Modifiers::default(),
    );
    assert_eq!(row_one, EventEffect::document_replacement());
    assert_eq!(pointer_app.active_workspace_entry, None);
    let selected = pointer_app
        .file_tree
        .visible_rows(1, 1, 0)?
        .into_iter()
        .next()
        .ok_or("selected tree row")?;
    assert!(selected.selected);
    assert_eq!(selected.path.as_ref(), "file-001.rs");

    let mut exact_paint = test_app()?;
    exact_paint.selection = Selection::new(ByteOffset::new(1), ByteOffset::new(2));
    let exact_scene = exact_paint.try_scene(SceneRevision::new(1), viewport)?;
    let expected_selection = Rect::new(
        Point::new(32.0, CONTENT_INSET).ok_or("selection origin")?,
        Size::new(8.0, LINE_HEIGHT).ok_or("selection size")?,
    );
    let expected_caret = Rect::new(
        Point::new(40.0, CONTENT_INSET).ok_or("caret origin")?,
        Size::new(CARET_WIDTH, LINE_HEIGHT).ok_or("caret size")?,
    );
    assert_eq!(exact_scene.quads()[2].bounds(), expected_selection);
    assert_eq!(
        exact_scene.quads().last().ok_or("missing caret")?.bounds(),
        expected_caret
    );

    let mut exhausted = test_app()?;
    exhausted.runtime_document_revision = u64::MAX;
    let failures = exhausted.input_failures;
    exhausted.advance_runtime_document_identity(false);
    assert_eq!(exhausted.runtime_document_revision, u64::MAX);
    assert_eq!(exhausted.input_failures, failures + 1);
    exhausted.advance_runtime_document_identity(true);
    assert_eq!(exhausted.input_failures, failures + 1);
    Ok(())
}

#[test]
#[allow(
    clippy::float_cmp,
    clippy::too_many_lines,
    reason = "exact split geometry and retained scroll distinguish pane ownership mutations"
)]
fn split_views_render_focus_and_close_with_bounded_independent_scroll()
-> Result<(), Box<dyn std::error::Error>> {
    let text = "line\n".repeat(100);
    let mut app = StudioApp::from_document(TestTextSystem, StudioDocument::scratch(&text), None)?;
    let viewport = viewport()?;
    app.last_viewport = viewport;
    let region = app.editor_region(viewport)?;
    assert_eq!(app.panes.len(), 1);
    assert!(app.command_context().can_split_right);
    assert!(!app.command_context().can_close_pane);

    assert!(
        app.dispatch_command(StudioCommand::SplitRight)
            .visual_changed
    );
    assert_eq!(app.panes.len(), 2);
    assert!(app.command_context().can_close_pane);
    app.scroll_y = 44.0;
    app.selection = Selection::new(ByteOffset::new(5), ByteOffset::new(9));
    let scene = app.try_scene(SceneRevision::new(70), viewport)?;
    let layout = app.panes.layout(region)?;
    let entries: Vec<_> = layout.iter().collect();
    assert_eq!(entries.len(), 2);
    assert_eq!(scene.clips()[0].bounds(), entries[0].bounds);
    assert_eq!(scene.clips()[1].bounds(), entries[1].bounds);
    assert!(scene.glyphs().iter().any(|glyph| {
        let x = glyph.bounds().origin().x();
        x >= entries[0].bounds.origin().x()
            && x < entries[0].bounds.origin().x() + entries[0].bounds.size().width()
    }));
    assert!(scene.glyphs().iter().any(|glyph| {
        let x = glyph.bounds().origin().x();
        x >= entries[1].bounds.origin().x()
            && x < entries[1].bounds.origin().x() + entries[1].bounds.size().width()
    }));
    let expected_tops = [
        entries[0].bounds.origin().y() + LINE_HEIGHT,
        entries[1].bounds.origin().y() + LINE_HEIGHT - 44.0,
    ];
    for (entry, expected_top) in entries.iter().zip(expected_tops) {
        let expected_selection = Rect::new(
            Point::new(entry.bounds.origin().x(), expected_top).ok_or("pane selection origin")?,
            Size::new(32.0, LINE_HEIGHT).ok_or("pane selection size")?,
        );
        assert!(
            scene
                .quads()
                .iter()
                .any(|quad| quad.bounds() == expected_selection)
        );
        assert!(scene.glyphs().iter().any(|glyph| {
            glyph.bounds().origin().x() >= entry.bounds.origin().x()
                && glyph.bounds().origin().x()
                    < entry.bounds.origin().x() + entry.bounds.size().width()
                && glyph.bounds().origin().y().to_bits() == (expected_top + 12.0).to_bits()
        }));
    }

    let left_point = Point::new(
        entries[0].bounds.origin().x() + 4.0,
        entries[0].bounds.origin().y() + 4.0,
    )
    .ok_or("left pane point")?;
    let active = app.panes.active_id();
    assert_eq!(
        app.handle_pointer(
            PointerAction::Down,
            left_point,
            PointerButton::Secondary,
            Modifiers::default(),
        ),
        EventEffect::default()
    );
    assert_eq!(app.panes.active_id(), active);
    assert!(
        app.handle_pointer(
            PointerAction::Down,
            left_point,
            PointerButton::Primary,
            Modifiers::default(),
        )
        .visual_changed
    );
    assert_eq!(app.panes.active_id(), entries[0].id);
    assert_eq!(app.scroll_y, 0.0);
    app.scroll_y = 22.0;

    let right_point = Point::new(
        entries[1].bounds.origin().x() + 4.0,
        entries[1].bounds.origin().y() + 4.0,
    )
    .ok_or("right pane point")?;
    assert!(
        app.handle_pointer(
            PointerAction::Down,
            right_point,
            PointerButton::Primary,
            Modifiers::default(),
        )
        .visual_changed
    );
    assert_eq!(app.panes.active_id(), entries[1].id);
    assert_eq!(app.scroll_y, 44.0);

    assert!(
        app.dispatch_command(StudioCommand::SplitDown)
            .visual_changed
    );
    assert_eq!(app.panes.len(), 3);
    assert!(
        app.dispatch_command(StudioCommand::FocusNextPane)
            .visual_changed
    );
    assert!(
        app.dispatch_command(StudioCommand::ClosePane)
            .visual_changed
    );
    assert_eq!(app.panes.len(), 2);
    Ok(())
}

#[test]
fn pane_projection_uses_half_open_bounds_and_relative_lines()
-> Result<(), Box<dyn std::error::Error>> {
    let text = "zero\none\ntwo\n";
    let viewport = viewport()?;
    let mut app = StudioApp::from_document(TestTextSystem, StudioDocument::scratch(text), None)?;
    app.last_viewport = viewport;
    app.split_active_pane(SplitAxis::Columns);
    app.try_scene(SceneRevision::new(72), viewport)?;
    let active = app
        .panes
        .layout(app.editor_region(viewport)?)?
        .active()
        .ok_or("active pane")?;
    let origin = active.bounds.origin();
    let size = active.bounds.size();
    let inside_x = origin.x() + 0.5;

    assert_eq!(
        app.offset_at_point(Point::new(origin.x() - 0.5, origin.y()).ok_or("left outside")?),
        None
    );
    assert_eq!(
        app.offset_at_point(Point::new(origin.x(), origin.y()).ok_or("top left")?),
        Some(ByteOffset::new(0))
    );
    assert_eq!(
        app.offset_at_point(Point::new(origin.x() + size.width(), origin.y()).ok_or("right edge")?),
        None
    );
    assert!(
        app.offset_at_point(
            Point::new(origin.x() + size.width() - 0.5, origin.y()).ok_or("right inside")?
        )
        .is_some()
    );
    assert_eq!(
        app.offset_at_point(Point::new(inside_x, origin.y() + size.height()).ok_or("bottom edge")?),
        None
    );
    assert_eq!(
        app.offset_at_point(Point::new(inside_x, origin.y() - 0.5).ok_or("top outside")?),
        None
    );
    assert_eq!(
        app.offset_at_point(
            Point::new(inside_x, origin.y() + LINE_HEIGHT * 2.0 + 1.0).ok_or("third line")?
        ),
        Some(ByteOffset::new(9))
    );

    let mut single = StudioApp::from_document(TestTextSystem, StudioDocument::scratch(text), None)?;
    single.last_viewport = viewport;
    single.try_scene(SceneRevision::new(73), viewport)?;
    let single_bounds = single.active_pane_bounds()?;
    assert_eq!(
        single.offset_at_point(
            Point::new(
                single_bounds.origin().x() + 0.5,
                single_bounds.origin().y() - 0.5,
            )
            .ok_or("single pane overscroll")?
        ),
        Some(ByteOffset::new(0))
    );
    Ok(())
}

#[test]
fn pane_command_pointer_and_projection_failures_are_structured()
-> Result<(), Box<dyn std::error::Error>> {
    let mut app = test_app()?;
    app.last_viewport = viewport()?;

    assert!(!app.focus_next_pane().visual_changed);
    let failures = app.workspace_failures;
    assert!(app.close_active_pane().visual_changed);
    assert_eq!(app.workspace_failures, failures + 1);

    for _ in 0..MAX_PANES - 1 {
        assert!(app.split_active_pane(SplitAxis::Columns).visual_changed);
    }
    let failures = app.workspace_failures;
    app.split_active_pane(SplitAxis::Columns);
    assert_eq!(app.workspace_failures, failures + 1);

    app.scroll_y = f32::NAN;
    let failures = app.workspace_failures;
    app.focus_next_pane();
    assert_eq!(app.workspace_failures, failures + 1);
    let point = app.editor_region(app.last_viewport)?.origin();
    app.handle_pointer(
        PointerAction::Down,
        point,
        PointerButton::Primary,
        Modifiers::default(),
    );
    assert_eq!(app.workspace_failures, failures + 2);

    let mut projection = test_app()?;
    let viewport = viewport()?;
    projection.last_viewport = viewport;
    projection.split_active_pane(SplitAxis::Columns);
    let layout = projection
        .panes
        .layout(projection.editor_region(viewport)?)?;
    let inactive = layout
        .iter()
        .find(|pane| !pane.active)
        .ok_or("inactive pane")?;
    projection
        .panes
        .inject_scroll_fault(inactive.id, f32::MAX)?;
    assert!(
        projection
            .try_scene(SceneRevision::new(71), viewport)
            .is_err()
    );
    Ok(())
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "each explicit pane and tab corruption boundary is proven independently"
)]
fn pane_document_failure_boundaries_fail_closed() -> Result<(), Box<dyn std::error::Error>> {
    let mut split = test_app()?;
    split.last_viewport = viewport()?;
    split.scroll_y = f32::NAN;
    let failures = split.workspace_failures;
    split.split_active_pane(SplitAxis::Columns);
    assert_eq!(split.workspace_failures, failures + 1);

    let mut close = test_app()?;
    close.scroll_y = f32::NAN;
    let failures = close.workspace_failures;
    close.close_active_pane();
    assert_eq!(close.workspace_failures, failures + 1);

    let mut focus = test_app()?;
    focus.panes.inject_layout_fault();
    let failures = focus.workspace_failures;
    focus.focus_next_pane();
    assert_eq!(focus.workspace_failures, failures + 1);

    let mut missing_document = test_app()?;
    missing_document.panes.inject_active_document_fault()?;
    let failures = missing_document.workspace_failures;
    missing_document.apply_focused_pane_document();
    assert_eq!(missing_document.workspace_failures, failures + 1);

    let mut invalid_active_tab = test_app()?;
    invalid_active_tab.tabs.inject_active_index_fault();
    let failures = invalid_active_tab.workspace_failures;
    invalid_active_tab.apply_focused_pane_document();
    assert_eq!(invalid_active_tab.workspace_failures, failures + 1);

    let mut missing_tab = test_app()?;
    missing_tab.panes.sync_active_document(
        crate::documents::DocumentTabId(u64::MAX),
        missing_tab.active_document_view(),
    )?;
    let failures = missing_tab.workspace_failures;
    missing_tab.apply_focused_pane_document();
    assert_eq!(missing_tab.workspace_failures, failures + 1);

    let root = TestWorkspace::new()?;
    root.write("alpha.rs", "alpha")?;
    root.write("beta.rs", "beta")?;
    let mut invalid_switch = StudioApp::open_workspace(TestTextSystem, root.path())?;
    let alpha = invalid_switch
        .workspace
        .as_ref()
        .and_then(|workspace| workspace.index_named("alpha.rs"))
        .ok_or("alpha entry")?;
    let beta = invalid_switch
        .workspace
        .as_ref()
        .and_then(|workspace| workspace.index_named("beta.rs"))
        .ok_or("beta entry")?;
    invalid_switch.open_workspace_entry(alpha)?;
    let alpha_tab = invalid_switch.tabs.active_id()?;
    invalid_switch.open_workspace_entry(beta)?;
    invalid_switch
        .panes
        .sync_active_document(alpha_tab, invalid_switch.active_document_view())?;
    invalid_switch
        .tabs
        .inject_active_payload_for_test(StudioDocument::scratch("duplicate active payload"));
    let failures = invalid_switch.workspace_failures;
    invalid_switch.apply_focused_pane_document();
    assert_eq!(invalid_switch.workspace_failures, failures + 1);

    let mut invalid_restored_view = test_app()?;
    invalid_restored_view
        .panes
        .inject_scroll_fault(invalid_restored_view.panes.active_id(), f32::NAN)?;
    let failures = invalid_restored_view.workspace_failures;
    invalid_restored_view.apply_focused_pane_document();
    assert_eq!(invalid_restored_view.workspace_failures, failures + 1);

    let mut pointer = test_app()?;
    pointer.last_viewport = viewport()?;
    pointer.panes.inject_layout_fault();
    let failures = pointer.workspace_failures;
    let point = pointer.editor_region(pointer.last_viewport)?.origin();
    pointer.focus_pane_for_pointer(PointerAction::Down, PointerButton::Primary, point);
    assert_eq!(pointer.workspace_failures, failures + 1);
    Ok(())
}

#[test]
fn pane_focus_restores_exact_document_and_view_identity() -> Result<(), Box<dyn std::error::Error>>
{
    let root = TestWorkspace::new()?;
    let alpha_text = "alpha\n".repeat(100);
    let beta_text = "beta\n".repeat(100);
    root.write("alpha.rs", &alpha_text)?;
    root.write("beta.rs", &beta_text)?;
    let mut app = StudioApp::open_workspace(TestTextSystem, root.path())?;
    let alpha = app
        .workspace
        .as_ref()
        .and_then(|workspace| workspace.index_named("alpha.rs"))
        .ok_or("alpha entry")?;
    let beta = app
        .workspace
        .as_ref()
        .and_then(|workspace| workspace.index_named("beta.rs"))
        .ok_or("beta entry")?;
    app.open_workspace_entry(alpha)?;
    app.selection = Selection::caret(ByteOffset::new(2));
    app.scroll_y = 11.0;
    app.last_viewport = viewport()?;
    assert!(app.split_active_pane(SplitAxis::Columns).visual_changed);

    app.open_workspace_entry(beta)?;
    app.selection = Selection::caret(ByteOffset::new(1));
    app.scroll_y = 33.0;
    app.sync_active_pane_document()?;
    let layout = app.panes.layout(app.editor_region(app.last_viewport)?)?;
    let inactive = layout
        .iter()
        .find(|pane| !pane.active)
        .ok_or("inactive pane")?;
    let active = app.panes.active_id();
    let point = Point::new(
        inactive.bounds.origin().x() + 1.0,
        inactive.bounds.origin().y() + 1.0,
    )
    .ok_or("inactive pane point")?;
    assert_eq!(
        app.focus_pane_for_pointer(PointerAction::Down, PointerButton::Secondary, point),
        EventEffect::default()
    );
    assert_eq!(app.panes.active_id(), active);

    assert!(
        app.handle_pointer(
            PointerAction::Down,
            point,
            PointerButton::Primary,
            Modifiers::default(),
        )
        .document_identity_advanced
    );
    assert!(app.buffer().snapshot().text().starts_with("alpha\n"));
    assert_eq!(app.selection, Selection::caret(ByteOffset::new(2)));
    assert_eq!(app.scroll_y.to_bits(), 11.0_f32.to_bits());
    assert!(app.focus_next_pane().document_identity_advanced);
    assert!(app.buffer().snapshot().text().starts_with("beta\n"));
    assert_eq!(app.selection, Selection::caret(ByteOffset::new(1)));
    assert_eq!(app.scroll_y.to_bits(), 33.0_f32.to_bits());
    Ok(())
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "the adversarial workspace selection journey keeps all atomicity checks together"
)]
fn workspace_click_revalidates_target_and_preserves_current_document_on_failure()
-> Result<(), Box<dyn std::error::Error>> {
    let root = TestWorkspace::new()?;
    root.write("alpha.rs", "alpha")?;
    root.write("invalid.rs", [0xff])?;
    root.write("replace.rs", "replace")?;
    root.create_dir("src")?;
    root.write("src/nested.rs", "nested")?;
    let mut app = StudioApp::open_workspace(TestTextSystem, root.path())?;
    let viewport = viewport().map_err(|_| StudioRenderError::Domain)?;
    let _scene = app.try_scene(SceneRevision::new(1), viewport)?;
    let workspace = app.workspace.as_ref().ok_or("missing workspace")?;
    let alpha = workspace.index_named("alpha.rs").ok_or("missing alpha")?;
    let invalid = workspace
        .index_named("invalid.rs")
        .ok_or("missing invalid")?;
    let replacement = workspace
        .index_named("replace.rs")
        .ok_or("missing replacement")?;
    let directory = workspace.index_named("src").ok_or("missing directory")?;

    let click = |index: usize, timestamp: u64| -> Result<SurfaceEvent, StudioRenderError> {
        Ok(SurfaceEvent::Pointer {
            timestamp: EventTimestamp::new(timestamp),
            action: PointerAction::Down,
            position: Point::new(
                CONTENT_INSET,
                CONTENT_INSET + usize_as_f32(index) * TREE_ROW_HEIGHT + 1.0,
            )
            .ok_or(StudioRenderError::Domain)?,
            button: PointerButton::Primary,
            modifiers: Modifiers::default(),
        })
    };
    let opened = app.handle_event(&click(alpha, 1)?);
    assert!(opened.visual_changed);
    assert!(opened.document_changed);
    assert!(opened.document_identity_advanced);
    assert_eq!(app.buffer().snapshot().text(), "alpha");
    assert_eq!(app.active_workspace_entry, None);
    let selected = app
        .file_tree
        .visible_rows(alpha, 1, 0)?
        .into_iter()
        .next()
        .ok_or("selected alpha row")?;
    assert!(selected.selected);
    assert_eq!(selected.path.as_ref(), "alpha.rs");
    let accepted_revision = app.runtime_document_revision;

    let failed = app.handle_event(&click(invalid, 2)?);
    assert!(failed.visual_changed);
    assert!(!failed.document_changed);
    assert_eq!(app.buffer().snapshot().text(), "alpha");
    assert_eq!(app.runtime_document_revision, accepted_revision);
    assert_eq!(app.workspace_failures, 1);
    assert!(app.last_workspace_error.is_some());

    let directory_changed = app.handle_event(&click(directory, 3)?);
    assert!(directory_changed.visual_changed);
    assert!(!directory_changed.document_changed);
    let nested_request = app
        .prepare_file_tree_request()?
        .ok_or("nested directory request")?;
    assert!(
        app.apply_file_tree_output(nested_request.execute())
            .visual_changed
    );
    let nested = app
        .file_tree
        .visible_rows(directory.saturating_add(1), 1, 0)?
        .into_iter()
        .next()
        .ok_or("nested directory row")?;
    assert_eq!(nested.path.as_ref(), "src/nested.rs");
    assert_eq!(app.buffer().snapshot().text(), "alpha");
    assert_eq!(app.runtime_document_revision, accepted_revision);
    assert_eq!(app.workspace_failures, 1);
    assert!(app.handle_event(&click(directory, 4)?).visual_changed);
    assert_eq!(app.file_tree.total_rows(), 4);

    fs::remove_file(root.path().join("replace.rs"))?;
    let missing_failed = app.handle_event(&click(replacement, 5)?);
    assert!(missing_failed.visual_changed);
    assert_eq!(app.buffer().snapshot().text(), "alpha");
    assert_eq!(app.runtime_document_revision, accepted_revision);
    assert_eq!(app.workspace_failures, 2);

    #[cfg(unix)]
    {
        let outside = TestFile::new("outside")?;
        std::os::unix::fs::symlink(outside.path(), root.path().join("replace.rs"))?;
        let symlink_failed = app.handle_event(&click(replacement, 6)?);
        assert!(symlink_failed.visual_changed);
        assert!(matches!(
            app.workspace
                .as_ref()
                .ok_or("missing workspace")?
                .path_for_file(replacement),
            Err(WorkspaceError::NotRegularFile(_))
        ));
        assert_eq!(app.buffer().snapshot().text(), "alpha");
        assert_eq!(app.runtime_document_revision, accepted_revision);
        assert_eq!(app.workspace_failures, 3);
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;

        let race_root = TestWorkspace::new()?;
        race_root.write("race.rs", "inside")?;
        let race_workspace = Workspace::open(race_root.path(), WorkspaceLimits::default())?;
        let race_index = race_workspace
            .index_named("race.rs")
            .ok_or("missing race")?;
        let outside = TestFile::new("outside")?;
        let outside_path = outside.path().to_path_buf();
        super::workspace::set_revalidation_hook(move |candidate| {
            assert!(fs::remove_file(candidate).is_ok(), "remove raced target");
            assert!(
                symlink(&outside_path, candidate).is_ok(),
                "replace raced target"
            );
        });
        assert!(matches!(
            race_workspace.path_for_file(race_index),
            Err(WorkspaceError::EscapesRoot(_))
        ));
    }

    assert!(
        app.handle_event(&ime(ImeEvent::Committed("x".into())))
            .document_changed
    );
    let dirty_before = app.buffer().snapshot().text();
    let failures_before_dirty = app.workspace_failures;
    let dirty_failed = app.handle_event(&click(invalid, 6)?);
    assert!(dirty_failed.visual_changed);
    assert_eq!(app.buffer().snapshot().text(), dirty_before);
    assert_eq!(
        app.workspace_failures,
        failures_before_dirty.saturating_add(1)
    );
    Ok(())
}

#[test]
fn bounded_tabs_preserve_dirty_documents_and_refuse_dirty_close()
-> Result<(), Box<dyn std::error::Error>> {
    let root = TestWorkspace::new()?;
    root.write("alpha.rs", "alpha")?;
    root.write("beta.rs", "beta")?;
    let mut app = StudioApp::open_workspace(TestTextSystem, root.path())?;
    let workspace = app.workspace.as_ref().ok_or("workspace")?;
    let alpha = workspace.index_named("alpha.rs").ok_or("alpha")?;
    let beta = workspace.index_named("beta.rs").ok_or("beta")?;

    assert!(app.open_workspace_entry(alpha)?.document_changed);
    assert!(
        app.handle_event(&ime(ImeEvent::Committed("x".into())))
            .document_changed
    );
    let dirty_alpha = app.buffer().snapshot().text();
    assert!(app.open_workspace_entry(beta)?.document_changed);
    assert_eq!(app.buffer().snapshot().text(), "beta");
    let tab_count = app.tabs.len();
    assert!(app.handle_close_request().cancel_close);

    assert!(app.open_workspace_entry(alpha)?.document_changed);
    assert_eq!(app.tabs.len(), tab_count);
    assert_eq!(app.buffer().snapshot().text(), dirty_alpha);
    assert!(matches!(
        app.close_active_tab(),
        Err(WorkspaceSelectionError::DirtyDocument)
    ));
    assert_eq!(app.tabs.len(), tab_count);
    assert_eq!(app.buffer().snapshot().text(), dirty_alpha);

    let _save_effect = app.save_document();
    #[cfg(not(target_family = "windows"))]
    {
        assert!(!app.document.is_dirty());
        assert!(app.close_active_tab()?.document_changed);
        assert_eq!(app.tabs.len(), tab_count - 1);
        assert_eq!(
            fs::read_to_string(root.path().join("alpha.rs"))?,
            dirty_alpha
        );
    }
    #[cfg(target_family = "windows")]
    {
        assert!(app.document.is_dirty());
        assert_eq!(
            app.last_file_error,
            Some(FileError::UnsupportedAtomicReplace)
        );
        assert!(matches!(
            app.close_active_tab(),
            Err(WorkspaceSelectionError::DirtyDocument)
        ));
        assert_eq!(app.tabs.len(), tab_count);
        assert_eq!(fs::read_to_string(root.path().join("alpha.rs"))?, "alpha");
    }
    Ok(())
}

#[test]
fn tab_fault_paths_are_structured_and_non_destructive() -> Result<(), Box<dyn std::error::Error>> {
    let root = TestWorkspace::new()?;
    root.write("alpha.rs", "alpha")?;
    root.write("beta.rs", "beta")?;
    let mut app = StudioApp::open_workspace(TestTextSystem, root.path())?;
    let workspace = app.workspace.as_ref().ok_or("workspace")?;
    let alpha = workspace.index_named("alpha.rs").ok_or("alpha")?;
    let beta = workspace.index_named("beta.rs").ok_or("beta")?;
    app.open_workspace_entry(alpha)?;
    app.open_workspace_entry(beta)?;

    assert_eq!(
        app.activate_document_tab(app.tabs.active_index())?,
        EventEffect::default()
    );
    let tabs_error = WorkspaceSelectionError::Tabs(DocumentTabError::LastTab);
    assert_eq!(
        tabs_error.to_string(),
        "document tabs failed: the final document tab cannot be closed"
    );
    assert!(tabs_error.source().is_some());

    app.last_viewport = viewport()?;
    app.tab_scroll_x = f32::NAN;
    let tab_point =
        Point::new(app.sidebar_width(app.last_viewport) + 1.0, 1.0).ok_or("tab point")?;
    assert_eq!(
        app.handle_pointer(
            PointerAction::Down,
            tab_point,
            PointerButton::Primary,
            Modifiers::default()
        ),
        EventEffect::default()
    );
    app.tab_scroll_x = TAB_WIDTH * 100.0;
    let failures = app.workspace_failures;
    assert!(
        app.handle_pointer(
            PointerAction::Down,
            tab_point,
            PointerButton::Primary,
            Modifiers::default()
        )
        .visual_changed
    );
    assert_eq!(app.workspace_failures, failures + 1);

    app.runtime_document_revision = u64::MAX;
    let failures = app.workspace_failures;
    assert!(app.navigate_document_history(false).visual_changed);
    assert_eq!(app.workspace_failures, failures + 1);
    app.runtime_document_revision = 10;

    assert!(
        app.handle_event(&ime(ImeEvent::Committed("x".into())))
            .document_changed
    );
    let failures = app.workspace_failures;
    assert!(app.close_active_tab_or_record().visual_changed);
    assert_eq!(app.workspace_failures, failures + 1);

    app.tabs
        .inject_active_payload_for_test(StudioDocument::Scratch {
            buffer: Buffer::new("fault"),
            clean_revision: 0,
            recovery_base: Buffer::new("fault").snapshot(),
        });
    let active_before = app.buffer().snapshot().text();
    let failures = app.workspace_failures;
    assert!(app.navigate_document_history(false).visual_changed);
    assert_eq!(app.workspace_failures, failures + 1);
    assert_eq!(app.buffer().snapshot().text(), active_before);
    Ok(())
}

#[test]
fn tab_history_branches_and_pointer_activation_restore_exact_view_state()
-> Result<(), Box<dyn std::error::Error>> {
    let root = TestWorkspace::new()?;
    root.write("alpha.rs", "alpha")?;
    root.write("beta.rs", "beta")?;
    root.write("gamma.rs", "gamma")?;
    let mut app = StudioApp::open_workspace(TestTextSystem, root.path())?;
    let workspace = app.workspace.as_ref().ok_or("workspace")?;
    let alpha = workspace.index_named("alpha.rs").ok_or("alpha")?;
    let beta = workspace.index_named("beta.rs").ok_or("beta")?;
    let gamma = workspace.index_named("gamma.rs").ok_or("gamma")?;
    app.open_workspace_entry(alpha)?;
    app.selection = Selection::caret(ByteOffset::new(2));
    app.scroll_y = 11.0;
    app.open_workspace_entry(beta)?;
    app.selection = Selection::caret(ByteOffset::new(3));
    app.open_workspace_entry(gamma)?;

    assert!(
        app.navigate_document_history(false)
            .document_identity_advanced
    );
    assert_eq!(app.buffer().snapshot().text(), "beta");
    assert_eq!(app.selection, Selection::caret(ByteOffset::new(3)));
    assert!(
        app.navigate_document_history(false)
            .document_identity_advanced
    );
    assert_eq!(app.buffer().snapshot().text(), "alpha");
    assert_eq!(app.selection, Selection::caret(ByteOffset::new(2)));
    assert!((app.scroll_y - 11.0).abs() < f32::EPSILON);
    assert!(
        app.navigate_document_history(true)
            .document_identity_advanced
    );
    assert_eq!(app.buffer().snapshot().text(), "beta");
    assert!(app.open_workspace_entry(alpha)?.document_identity_advanced);
    assert_eq!(app.navigate_document_history(true), EventEffect::default());

    let viewport = viewport().map_err(|_| StudioRenderError::Domain)?;
    let _scene = app.try_scene(SceneRevision::new(9), viewport)?;
    let canonical_beta = fs::canonicalize(root.path().join("beta.rs"))?;
    let beta_tab = app.tabs.index_for_path(&canonical_beta).ok_or("beta tab")?;
    let pointer = app.handle_pointer(
        PointerAction::Down,
        Point::new(
            app.sidebar_width(viewport) + usize_as_f32(beta_tab) * TAB_WIDTH + 1.0,
            1.0,
        )
        .ok_or("tab point")?,
        PointerButton::Primary,
        Modifiers::default(),
    );
    assert!(pointer.document_identity_advanced);
    assert_eq!(app.buffer().snapshot().text(), "beta");
    Ok(())
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "exact tab geometry and routing controls distinguish projection mutations"
)]
fn tab_projection_scroll_keyboard_and_pointer_boundaries_are_exact()
-> Result<(), Box<dyn std::error::Error>> {
    let mut single = test_app()?;
    single.tab_scroll_x = 100.0;
    single.ensure_active_tab_visible();
    assert_eq!(single.tab_scroll_x.to_bits(), 0.0_f32.to_bits());

    let root = TestWorkspace::new()?;
    let names = [
        "a.rs", "b.rs", "c.rs", "d.rs", "e.rs", "f.rs", "g.rs", "h.rs", "i.rs", "j.rs",
    ];
    for name in names {
        root.write(name, name)?;
    }
    let mut app = StudioApp::open_workspace(TestTextSystem, root.path())?;
    for name in names {
        let index = app
            .workspace
            .as_ref()
            .and_then(|workspace| workspace.index_named(name))
            .ok_or("workspace entry")?;
        app.open_workspace_entry(index)?;
    }
    let small_viewport = Size::new(556.0, WINDOW_HEIGHT).ok_or("viewport")?;
    app.last_viewport = small_viewport;
    app.ensure_active_tab_visible();
    assert_eq!(app.tabs.active_index(), 10);
    assert_eq!(app.tab_scroll_x.to_bits(), 1_440.0_f32.to_bits());

    TEST_SHAPE_CALLS.with(|calls| calls.set(0));
    let scene = app.try_scene(SceneRevision::new(20), small_viewport)?;
    TEST_SHAPE_CALLS.with(|calls| assert_eq!(calls.get(), 15));
    let expected_tab_bounds = Rect::new(
        Point::new(SIDEBAR_WIDTH, 0.0).ok_or("tab origin")?,
        Size::new(320.0, TAB_BAR_HEIGHT).ok_or("tab size")?,
    );
    assert_eq!(scene.clips()[2].bounds(), expected_tab_bounds);
    assert!(
        scene
            .quads()
            .iter()
            .any(|quad| quad.bounds() == expected_tab_bounds)
    );
    let expected_active = Rect::new(
        Point::new(396.0, 0.0).ok_or("active tab origin")?,
        Size::new(TAB_WIDTH, TAB_BAR_HEIGHT).ok_or("active tab size")?,
    );
    assert!(
        scene
            .quads()
            .iter()
            .any(|quad| quad.bounds() == expected_active)
    );
    let tab_glyphs: Vec<_> = scene
        .glyphs()
        .iter()
        .filter(|glyph| glyph.bounds().origin().y() < TAB_BAR_HEIGHT)
        .collect();
    assert!(!tab_glyphs.is_empty());
    assert!(tab_glyphs.iter().any(|glyph| {
        glyph.bounds().origin().x().to_bits() == (-76.0_f32).to_bits()
            && glyph.bounds().origin().y().to_bits() == 16.0_f32.to_bits()
    }));
    assert!(tab_glyphs.iter().any(|glyph| {
        glyph.bounds().origin().x().to_bits() == 404.0_f32.to_bits()
            && glyph.bounds().origin().y().to_bits() == 16.0_f32.to_bits()
    }));

    app.tab_scroll_x = 320.0;
    TEST_SHAPE_CALLS.with(|calls| calls.set(0));
    let mid_scene = app.try_scene(SceneRevision::new(21), small_viewport)?;
    TEST_SHAPE_CALLS.with(|calls| assert_eq!(calls.get(), 17));
    let first_mid_tab = mid_scene
        .glyphs()
        .iter()
        .find(|glyph| glyph.bounds().origin().y() < TAB_BAR_HEIGHT)
        .ok_or("first mid tab glyph")?;
    assert_eq!(
        first_mid_tab.bounds().origin().x().to_bits(),
        (-76.0_f32).to_bits()
    );
    assert_eq!(
        first_mid_tab.bounds().origin().y().to_bits(),
        16.0_f32.to_bits()
    );
    assert_eq!(
        mid_scene
            .quads()
            .iter()
            .filter(|quad| {
                quad.bounds().origin().y().to_bits() == 0.0_f32.to_bits()
                    && quad.bounds().size().height().to_bits() == TAB_BAR_HEIGHT.to_bits()
            })
            .count(),
        1
    );
    app.ensure_active_tab_visible();
    assert_eq!(app.tab_scroll_x.to_bits(), 1_440.0_f32.to_bits());

    assert!(app.activate_document_tab(2)?.document_identity_advanced);
    assert_eq!(app.tab_scroll_x.to_bits(), 320.0_f32.to_bits());
    app.tab_scroll_x = 170.0;
    app.ensure_active_tab_visible();
    assert_eq!(app.tab_scroll_x.to_bits(), 170.0_f32.to_bits());
    assert!(app.activate_document_tab(1)?.document_identity_advanced);
    assert_eq!(app.tab_scroll_x.to_bits(), 170.0_f32.to_bits());
    app.tab_scroll_x = 0.0;
    assert!(app.activate_document_tab(10)?.document_identity_advanced);
    assert_eq!(app.tab_scroll_x.to_bits(), 1_440.0_f32.to_bits());

    let tab_five = app.handle_pointer(
        PointerAction::Down,
        Point::new(SIDEBAR_WIDTH + 1.0, 1.0).ok_or("tab five")?,
        PointerButton::Primary,
        Modifiers::default(),
    );
    assert!(tab_five.document_identity_advanced);
    assert_eq!(app.tabs.active_index(), 9);
    let boundary = app.handle_pointer(
        PointerAction::Down,
        Point::new(SIDEBAR_WIDTH + TAB_WIDTH + 1.0, TAB_BAR_HEIGHT).ok_or("tab boundary")?,
        PointerButton::Primary,
        Modifiers::default(),
    );
    assert!(!boundary.document_identity_advanced);
    assert_eq!(app.tabs.active_index(), 9);

    assert_eq!(
        app.handle_key(KEY_LEFT_BRACKET, Modifiers::default()),
        EventEffect::default()
    );
    assert_eq!(app.tabs.active_index(), 9);
    assert_eq!(
        app.handle_key(KEY_RETURN, Modifiers::from_bits(Modifiers::COMMAND)),
        EventEffect::default()
    );
    assert_eq!(app.tabs.active_index(), 9);
    assert!(
        app.handle_key(KEY_LEFT_BRACKET, Modifiers::from_bits(Modifiers::COMMAND))
            .document_identity_advanced
    );
    assert_eq!(app.tabs.active_index(), 10);
    assert_eq!(
        app.handle_key(KEY_RIGHT_BRACKET, Modifiers::default()),
        EventEffect::default()
    );
    assert_eq!(app.tabs.active_index(), 10);
    assert!(
        app.handle_key(KEY_RIGHT_BRACKET, Modifiers::from_bits(Modifiers::COMMAND))
            .document_identity_advanced
    );
    assert_eq!(app.tabs.active_index(), 9);

    let before_close = app.tabs.len();
    assert!(
        app.handle_key(KEY_W, Modifiers::from_bits(Modifiers::COMMAND))
            .document_identity_advanced
    );
    assert_eq!(app.tabs.len(), before_close - 1);
    Ok(())
}

#[test]
fn ime_preview_is_non_destructive_and_commit_is_atomic() -> Result<(), StudioRenderError> {
    let mut app = test_app().map_err(|_| StudioRenderError::Domain)?;
    let before = app.buffer().snapshot().text();
    assert!(app.handle_event(&ime(ImeEvent::Started)).visual_changed);
    assert!(
        app.handle_event(&ime(ImeEvent::Updated {
            text: "é".into(),
            selected_start_utf16: 1,
            selected_length_utf16: 0,
        }))
        .visual_changed
    );
    assert_eq!(app.buffer().snapshot().text(), before);
    let preview = app.try_scene(
        SceneRevision::new(1),
        viewport().map_err(|_| StudioRenderError::Domain)?,
    )?;
    assert!(preview.quads().len() >= 4);
    let effect = app.handle_event(&ime(ImeEvent::Committed("é".into())));
    assert!(effect.document_changed);
    assert!(app.buffer().snapshot().text().starts_with('é'));
    assert!(app.composition.is_none());
    Ok(())
}

#[test]
fn grapheme_delete_and_command_undo_restore_text() -> Result<(), SurfaceError> {
    let mut app = test_app()?;
    let original = app.buffer().snapshot().text();
    assert!(
        app.handle_event(&ime(ImeEvent::Committed("🦀".into())))
            .document_changed
    );
    let inserted = app.buffer().snapshot().text();
    assert!(inserted.starts_with('🦀'));
    assert!(
        app.handle_event(&key(KEY_DELETE_BACKWARD, Modifiers::default()))
            .document_changed
    );
    assert_eq!(app.buffer().snapshot().text(), original);
    assert!(
        app.handle_event(&key(KEY_Z, Modifiers::from_bits(Modifiers::COMMAND),))
            .document_changed
    );
    assert_eq!(app.buffer().snapshot().text(), inserted);
    Ok(())
}

#[test]
fn scroll_and_pointer_selection_use_rendered_visible_lines() -> Result<(), StudioRenderError> {
    let mut app = test_app().map_err(|_| StudioRenderError::Domain)?;
    *app.buffer_mut() = Buffer::new(&"line\n".repeat(100));
    app.selection = Selection::caret(ByteOffset::new(0));
    let _scene = app.try_scene(
        SceneRevision::new(1),
        viewport().map_err(|_| StudioRenderError::Domain)?,
    )?;
    let scrolled = app.handle_event(&SurfaceEvent::Scroll {
        timestamp: EventTimestamp::new(1),
        delta_x: 0.0,
        delta_y: -220.0,
        phase: ScrollPhase::Changed,
        precise: true,
        modifiers: Modifiers::default(),
    });
    assert!(scrolled.visual_changed);
    assert!((app.scroll_y - 220.0).abs() < f32::EPSILON);
    let _scene = app.try_scene(
        SceneRevision::new(2),
        viewport().map_err(|_| StudioRenderError::Domain)?,
    )?;
    let pointer = app.handle_event(&SurfaceEvent::Pointer {
        timestamp: EventTimestamp::new(2),
        action: PointerAction::Down,
        position: Point::new(CONTENT_INSET + 9.0, CONTENT_INSET + 2.0)
            .ok_or(StudioRenderError::Domain)?,
        button: PointerButton::Primary,
        modifiers: Modifiers::default(),
    });
    assert!(pointer.visual_changed);
    assert!(app.selection.head().get() > 0);
    Ok(())
}

#[test]
fn selection_spans_lines_and_unchanged_atlas_storage_is_reused() -> Result<(), StudioRenderError> {
    let mut app = test_app().map_err(|_| StudioRenderError::Domain)?;
    app.selection = Selection::new(
        ByteOffset::new(0),
        ByteOffset::new(app.buffer().snapshot().len_bytes()),
    );
    let first = app.try_scene(
        SceneRevision::new(1),
        viewport().map_err(|_| StudioRenderError::Domain)?,
    )?;
    let first_atlas = first
        .glyph_atlas()
        .cloned()
        .ok_or(StudioRenderError::Domain)?;
    assert!(first.quads().len() > 5);
    let second = app.try_scene(
        SceneRevision::new(2),
        viewport().map_err(|_| StudioRenderError::Domain)?,
    )?;
    let second_atlas = second.glyph_atlas().ok_or(StudioRenderError::Domain)?;
    assert!(first_atlas.shares_storage_with(second_atlas));
    app.selection = Selection::new(ByteOffset::new(0), ByteOffset::new(1));
    let third = app.try_scene(
        SceneRevision::new(3),
        viewport().map_err(|_| StudioRenderError::Domain)?,
    )?;
    assert!(third.quads().len() >= 3);
    Ok(())
}

#[test]
fn empty_raster_output_and_render_failure_degrade_without_invalid_scene()
-> Result<(), StudioRenderError> {
    let mut empty =
        StudioApp::new(UnresolvedEmptyTextSystem).map_err(|_| StudioRenderError::Domain)?;
    let scene = empty.try_scene(
        SceneRevision::new(1),
        viewport().map_err(|_| StudioRenderError::Domain)?,
    )?;
    assert!(scene.glyphs().is_empty());
    assert!(scene.glyph_atlas().is_none());

    let mut failing = StudioApp::new(FailingTextSystem).map_err(|_| StudioRenderError::Domain)?;
    let fallback = failing.scene(
        SceneRevision::new(2),
        viewport().map_err(|_| StudioRenderError::Domain)?,
    );
    assert_eq!(fallback.quads().len(), 1);
    assert_eq!(fallback.operation_count(), 1);
    assert_eq!(failing.render_failures, 1);
    assert!(failing.rendered_lines.is_empty());

    let text_error: StudioRenderError = TextError::EmptySelectionSet.into();
    let find_error: StudioRenderError = FindError::IncompleteResult.into();
    let layout_error: StudioRenderError = LayoutError::InvalidScroll.into();
    let scene_error: StudioRenderError = SceneError::MissingGlyphAtlas.into();
    assert!(
        text_error
            .to_string()
            .starts_with("text layout input failed")
    );
    assert!(find_error.to_string().starts_with("find rendering failed"));
    assert!(
        layout_error
            .to_string()
            .starts_with("visible layout failed")
    );
    assert!(
        scene_error
            .to_string()
            .starts_with("scene construction failed")
    );
    assert_eq!(
        StudioRenderError::Domain.to_string(),
        "invalid Studio render domain value"
    );
    let font = StudioApp::font()?;
    let mut failing_system = FailingTextSystem;
    assert!(failing_system.rasterize(font, 1, 0).is_err());
    Ok(())
}

#[test]
fn keyboard_commands_cover_selection_navigation_and_history() -> Result<(), SurfaceError> {
    let mut app = test_app()?;
    assert!(!app.undo().visual_changed);
    assert!(!app.redo().visual_changed);

    assert!(
        app.handle_event(&key(KEY_A, Modifiers::from_bits(Modifiers::COMMAND)))
            .visual_changed
    );
    assert_eq!(
        app.selection.range(),
        0..app.buffer().snapshot().len_bytes()
    );
    assert!(
        app.handle_event(&key(KEY_LEFT, Modifiers::default()))
            .visual_changed
    );
    assert_eq!(app.selection.head(), ByteOffset::new(0));

    app.selection = Selection::new(
        ByteOffset::new(0),
        ByteOffset::new(app.buffer().snapshot().len_bytes()),
    );
    assert!(
        app.handle_event(&key(KEY_RIGHT, Modifiers::default()))
            .visual_changed
    );
    assert_eq!(
        app.selection.head().get(),
        app.buffer().snapshot().len_bytes()
    );

    *app.buffer_mut() = Buffer::new("ab\néx\nlast");
    app.selection = Selection::caret(ByteOffset::new(1));
    assert!(
        app.handle_event(&key(KEY_DOWN, Modifiers::default()))
            .visual_changed
    );
    assert_eq!(app.selection.head(), ByteOffset::new(3));
    assert!(
        app.handle_event(&key(KEY_UP, Modifiers::from_bits(Modifiers::SHIFT)))
            .visual_changed
    );
    assert!(!app.selection.range().is_empty());
    assert!(
        app.handle_event(&key(KEY_HOME, Modifiers::default()))
            .visual_changed
    );
    assert!(
        app.handle_event(&key(KEY_END, Modifiers::from_bits(Modifiers::SHIFT)))
            .visual_changed
    );

    app.selection = Selection::caret(ByteOffset::new(0));
    assert!(
        app.handle_event(&key(KEY_DELETE_FORWARD, Modifiers::default()))
            .document_changed
    );
    app.selection = Selection::new(ByteOffset::new(0), ByteOffset::new(1));
    assert!(
        app.handle_event(&key(KEY_DELETE_FORWARD, Modifiers::default()))
            .document_changed
    );
    assert!(
        app.handle_event(&key(KEY_RETURN, Modifiers::default()))
            .document_changed
    );
    assert!(
        app.handle_event(&key(KEY_TAB, Modifiers::default()))
            .document_changed
    );
    assert!(app.undo().document_changed);
    assert!(app.redo().document_changed);
    assert!(!app.redo().visual_changed);
    assert!(
        app.handle_event(&key(KEY_Z, Modifiers::from_bits(Modifiers::COMMAND)))
            .document_changed
    );
    assert!(
        app.handle_event(&key(
            KEY_Z,
            Modifiers::from_bits(Modifiers::COMMAND | Modifiers::SHIFT),
        ))
        .document_changed
    );

    app.composition = Some(Composition {
        replacement: app.selection.range(),
        text: "marked".into(),
        selected_start_utf16: 0,
        selected_length_utf16: 0,
    });
    assert!(
        app.handle_event(&key(KEY_ESCAPE, Modifiers::default()))
            .visual_changed
    );
    assert!(app.composition.is_none());
    assert!(
        !app.handle_event(&key(999, Modifiers::default()))
            .visual_changed
    );
    assert!(
        !app.handle_event(&key(KEY_RETURN, Modifiers::from_bits(Modifiers::COMMAND)))
            .visual_changed
    );
    Ok(())
}

#[test]
#[allow(
    clippy::reversed_empty_ranges,
    reason = "the malformed range is the exact rejected input under test"
)]
fn input_edges_are_bounded_and_failed_edits_are_atomic() -> Result<(), StudioRenderError> {
    let mut app = test_app().map_err(|_| StudioRenderError::Domain)?;
    let next_epoch = InputEpoch::INITIAL
        .checked_next()
        .ok_or(StudioRenderError::Domain)?;
    assert!(
        app.handle_event(&SurfaceEvent::Focus {
            timestamp: EventTimestamp::new(1),
            input_epoch: next_epoch,
            focused: false,
        })
        .visual_changed
    );
    assert!(
        !app.handle_event(&SurfaceEvent::Focus {
            timestamp: EventTimestamp::new(2),
            input_epoch: next_epoch,
            focused: false,
        })
        .visual_changed
    );

    assert!(
        !app.handle_event(&ime(ImeEvent::Updated {
            text: "x".into(),
            selected_start_utf16: 2,
            selected_length_utf16: 0,
        }))
        .visual_changed
    );
    assert!(
        app.handle_event(&SurfaceEvent::Focus {
            timestamp: EventTimestamp::new(3),
            input_epoch: next_epoch,
            focused: true,
        })
        .visual_changed
    );
    assert!(
        !app.handle_event(&ime_at(
            next_epoch,
            ImeEvent::Updated {
                text: "x".into(),
                selected_start_utf16: 2,
                selected_length_utf16: 0,
            },
        ))
        .visual_changed
    );
    assert!(
        app.handle_event(&ime_at(next_epoch, ImeEvent::Started))
            .visual_changed
    );
    assert!(
        app.handle_event(&ime_at(next_epoch, ImeEvent::Cancelled))
            .visual_changed
    );
    assert!(
        !app.handle_event(&ime_at(next_epoch, ImeEvent::Cancelled))
            .visual_changed
    );

    let failures = app.input_failures;
    assert!(
        !app.replace_range(usize::MAX..usize::MAX, "x")
            .visual_changed
    );
    assert!(!app.replace_range(2..1, "").visual_changed);
    assert!(!app.replace_range(9_999..9_999, "").visual_changed);
    assert_eq!(app.input_failures, failures + 3);

    *app.buffer_mut() = Buffer::new("🦀x");
    app.selection = Selection::caret(ByteOffset::new(1));
    assert!(!app.delete_backward().visual_changed);
    assert!(!app.delete_forward().visual_changed);
    assert!(!app.move_horizontal(true, false).visual_changed);
    assert!(app.move_vertical(1, false).visual_changed);
    app.selection = Selection::caret(ByteOffset::new(usize::MAX));
    assert!(!app.move_vertical(1, false).visual_changed);
    assert!(!app.move_to_line_edge(true, false).visual_changed);
    assert!(app.input_failures >= failures + 6);

    app.selection = Selection::caret(ByteOffset::new(0));
    assert!(!app.delete_backward().visual_changed);
    assert!(!app.move_horizontal(false, false).visual_changed);
    app.selection = Selection::caret(ByteOffset::new(app.buffer().snapshot().len_bytes()));
    assert!(!app.delete_forward().visual_changed);
    assert!(!app.move_horizontal(true, false).visual_changed);

    app.selection = Selection::new(ByteOffset::new(0), ByteOffset::new(4));
    assert!(app.delete_backward().document_changed);
    Ok(())
}

#[test]
fn pointer_drag_focus_and_outside_geometry_are_deterministic() -> Result<(), StudioRenderError> {
    let mut app = test_app().map_err(|_| StudioRenderError::Domain)?;
    let _scene = app.try_scene(
        SceneRevision::new(1),
        viewport().map_err(|_| StudioRenderError::Domain)?,
    )?;
    let down = SurfaceEvent::Pointer {
        timestamp: EventTimestamp::new(1),
        action: PointerAction::Down,
        position: Point::new(CONTENT_INSET + 1.0, CONTENT_INSET + 1.0)
            .ok_or(StudioRenderError::Domain)?,
        button: PointerButton::Primary,
        modifiers: Modifiers::from_bits(Modifiers::SHIFT),
    };
    assert!(app.handle_event(&down).visual_changed || app.pointer_selecting);
    assert!(
        app.handle_event(&SurfaceEvent::Pointer {
            timestamp: EventTimestamp::new(2),
            action: PointerAction::Moved,
            position: Point::new(CONTENT_INSET + 18.0, CONTENT_INSET + LINE_HEIGHT + 1.0)
                .ok_or(StudioRenderError::Domain)?,
            button: PointerButton::None,
            modifiers: Modifiers::default(),
        })
        .visual_changed
    );
    assert!(
        !app.handle_event(&SurfaceEvent::Pointer {
            timestamp: EventTimestamp::new(3),
            action: PointerAction::Up,
            position: Point::new(0.0, 0.0).ok_or(StudioRenderError::Domain)?,
            button: PointerButton::Primary,
            modifiers: Modifiers::default(),
        })
        .visual_changed
    );
    assert!(!app.pointer_selecting);
    assert!(
        !app.handle_event(&SurfaceEvent::Pointer {
            timestamp: EventTimestamp::new(4),
            action: PointerAction::Moved,
            position: Point::new(0.0, 0.0).ok_or(StudioRenderError::Domain)?,
            button: PointerButton::None,
            modifiers: Modifiers::default(),
        })
        .visual_changed
    );
    assert!(
        !app.handle_event(&SurfaceEvent::Pointer {
            timestamp: EventTimestamp::new(5),
            action: PointerAction::Down,
            position: Point::new(0.0, -1.0).ok_or(StudioRenderError::Domain)?,
            button: PointerButton::Secondary,
            modifiers: Modifiers::default(),
        })
        .visual_changed
    );
    let huge_y = CONTENT_INSET + 20_000_000.0 * LINE_HEIGHT;
    assert!(
        !app.handle_event(&SurfaceEvent::Pointer {
            timestamp: EventTimestamp::new(6),
            action: PointerAction::Down,
            position: Point::new(CONTENT_INSET, huge_y).ok_or(StudioRenderError::Domain)?,
            button: PointerButton::Primary,
            modifiers: Modifiers::default(),
        })
        .visual_changed
    );
    app.pointer_selecting = true;
    assert!(
        !app.handle_event(&SurfaceEvent::Pointer {
            timestamp: EventTimestamp::new(7),
            action: PointerAction::Moved,
            position: Point::new(CONTENT_INSET, huge_y).ok_or(StudioRenderError::Domain)?,
            button: PointerButton::None,
            modifiers: Modifiers::default(),
        })
        .visual_changed
    );
    app.pointer_selecting = false;
    assert_eq!(
        app.offset_at_point(
            Point::new(CONTENT_INSET, CONTENT_INSET - 1.0).ok_or(StudioRenderError::Domain)?,
        ),
        Some(ByteOffset::new(0))
    );
    Ok(())
}

#[test]
fn coordinate_helpers_cover_clusters_bounds_and_binary_line_search() -> Result<(), StudioRenderError>
{
    let mut system = TestTextSystem;
    let font = StudioApp::font()?;
    let layout = system.shape("aé", font)?;
    assert!((x_for_utf16(&layout, 0) - 0.0).abs() < f32::EPSILON);
    assert!((x_for_utf16(&layout, 1) - 8.0).abs() < f32::EPSILON);
    assert!((x_for_utf16(&layout, 3) - layout.width()).abs() < f32::EPSILON);
    assert_eq!(utf16_at_x(&layout, 0.0), 0);
    assert_eq!(utf16_at_x(&layout, 7.0), 1);
    assert_eq!(utf16_at_x(&layout, 100.0), 2);
    assert_eq!(byte_at_utf16("aé", 0), Some(0));
    assert_eq!(byte_at_utf16("aé", 1), Some(1));
    assert_eq!(byte_at_utf16("aé", 2), Some(3));
    assert_eq!(byte_at_utf16("aé", 3), None);
    assert_eq!(byte_at_utf16("a🦀", 2), None);
    assert_eq!(byte_at_utf16("a🦀", 3), Some(5));
    assert_eq!(floor_f32_to_usize(3.9), Some(3));
    assert_eq!(floor_f32_to_usize(20_000_000.0), None);
    assert!((usize_as_f32(7) - 7.0).abs() < f32::EPSILON);
    assert!((u32_as_f32(9) - 9.0).abs() < f32::EPSILON);

    let app = test_app().map_err(|_| StudioRenderError::Domain)?;
    let snapshot = Buffer::new("a\nb\nc\nd\n").snapshot();
    assert_eq!(StudioApp::line_for_offset(&snapshot, 0)?, Some(0));
    assert_eq!(StudioApp::line_for_offset(&snapshot, 4)?, Some(2));
    assert_eq!(
        StudioApp::line_for_offset(&snapshot, snapshot.len_bytes())?,
        Some(snapshot.line_count() - 1)
    );
    assert_eq!(
        StudioApp::line_for_offset(&snapshot, snapshot.len_bytes() + 1)?,
        None
    );
    assert_eq!(local_utf16(&Buffer::new("aé").snapshot(), 0, 3)?, 2);
    assert!(local_utf16(&Buffer::new("é").snapshot(), 0, 1).is_err());
    assert!(app.maximum_scroll() >= 0.0);
    Ok(())
}

#[test]
fn offscreen_caret_and_invalid_scroll_use_safe_scene_fallback() -> Result<(), StudioRenderError> {
    let mut app = test_app().map_err(|_| StudioRenderError::Domain)?;
    *app.buffer_mut() = Buffer::new(&"line\n".repeat(100));
    app.selection = Selection::caret(ByteOffset::new(0));
    app.scroll_y = app.maximum_scroll();
    let snapshot = app.buffer().snapshot();
    assert!(app.caret_bounds(&snapshot, &[], CONTENT_INSET)?.is_none());
    app.selection = Selection::caret(ByteOffset::new(usize::MAX));
    assert!(app.caret_bounds(&snapshot, &[], CONTENT_INSET)?.is_none());
    app.selection = Selection::caret(ByteOffset::new(0));
    let scene = app.try_scene(
        SceneRevision::new(1),
        viewport().map_err(|_| StudioRenderError::Domain)?,
    )?;
    assert!(scene.quads().len() >= 2);

    app.scroll_y = f32::NAN;
    let fallback = app.scene(
        SceneRevision::new(2),
        viewport().map_err(|_| StudioRenderError::Domain)?,
    );
    assert_eq!(fallback.quads().len(), 1);
    assert_eq!(app.render_failures, 1);
    Ok(())
}

#[test]
fn bounded_find_routes_ime_selects_matches_and_paints_only_after_admission()
-> Result<(), StudioRenderError> {
    let mut app = test_app().map_err(|_| StudioRenderError::Domain)?;
    *app.buffer_mut() = Buffer::new("alpha beta alpha gamma alpha\n");
    let original = app.buffer().snapshot().text();

    assert!(
        app.handle_event(&key(KEY_F, Modifiers::from_bits(Modifiers::COMMAND),))
            .visual_changed
    );
    assert!(app.handle_event(&ime(ImeEvent::Started)).visual_changed);
    assert!(
        app.handle_event(&ime(ImeEvent::Updated {
            text: "alpha".into(),
            selected_start_utf16: 5,
            selected_length_utf16: 0,
        }))
        .visual_changed
    );
    assert!(
        app.handle_event(&ime(ImeEvent::Committed("alpha".into())))
            .visual_changed
    );
    app.find_needs_search = false;
    assert!(
        app.handle_event(&key(KEY_DELETE_BACKWARD, Modifiers::default()))
            .visual_changed
    );
    assert_eq!(app.find.query(), "alph");
    assert!(app.find_needs_search);
    assert!(
        app.handle_event(&ime(ImeEvent::Committed("a".into())))
            .visual_changed
    );
    assert_eq!(app.buffer().snapshot().text(), original);

    let scene_viewport = viewport().map_err(|_| StudioRenderError::Domain)?;
    let before = app.try_scene(SceneRevision::new(1), scene_viewport)?;
    let expected_overlay = Rect::new(
        Point::new(
            WINDOW_WIDTH - CONTENT_INSET - FIND_BAR_WIDTH,
            TAB_BAR_HEIGHT + FIND_BAR_INSET,
        )
        .ok_or(StudioRenderError::Domain)?,
        Size::new(FIND_BAR_WIDTH, FIND_BAR_HEIGHT).ok_or(StudioRenderError::Domain)?,
    );
    assert_eq!(
        before
            .clips()
            .last()
            .ok_or(StudioRenderError::Domain)?
            .bounds(),
        expected_overlay
    );
    let overlay_clip_index = before.clips().len() - 1;
    let first_overlay_glyph = before
        .glyphs()
        .iter()
        .filter(|glyph| glyph.clip().map(alpine_scene::ClipId::index) == Some(overlay_clip_index))
        .min_by(|left, right| {
            left.bounds()
                .origin()
                .x()
                .total_cmp(&right.bounds().origin().x())
        })
        .ok_or(StudioRenderError::Domain)?;
    let overlay_glyph_origin = first_overlay_glyph.bounds().origin();
    assert_eq!(
        overlay_glyph_origin.x().to_bits(),
        (WINDOW_WIDTH - CONTENT_INSET - FIND_BAR_WIDTH + FIND_BAR_INSET).to_bits()
    );
    assert_eq!(
        overlay_glyph_origin.y().to_bits(),
        (TAB_BAR_HEIGHT + FIND_BAR_INSET + 15.0 + 6.0 - 3.0).to_bits()
    );
    assert!(app.complete_pending_find_for_test()?.visual_changed);
    assert_eq!(app.selection.range(), 0..5);
    let after = app.try_scene(
        SceneRevision::new(2),
        viewport().map_err(|_| StudioRenderError::Domain)?,
    )?;
    assert!(after.quads().len() > before.quads().len());

    assert!(
        app.handle_event(&key(KEY_RETURN, Modifiers::from_bits(Modifiers::SHIFT),))
            .visual_changed
    );
    assert_eq!(app.selection.range(), 23..28);
    assert!(
        app.handle_event(&key(KEY_RETURN, Modifiers::default()))
            .visual_changed
    );
    assert_eq!(app.selection.range(), 0..5);
    assert_eq!(app.buffer().snapshot().text(), original);
    assert!(
        app.handle_event(&key(KEY_ESCAPE, Modifiers::default()))
            .visual_changed
    );
    app.find_needs_search = false;
    assert!(
        app.handle_event(&key(KEY_F, Modifiers::from_bits(Modifiers::COMMAND)))
            .visual_changed
    );
    assert!(app.find_needs_search);
    Ok(())
}

#[test]
fn find_command_modifiers_do_not_edit_or_switch_fields() -> Result<(), StudioRenderError> {
    let mut app = test_app().map_err(|_| StudioRenderError::Domain)?;
    let command = Modifiers::from_bits(Modifiers::COMMAND);
    let command_option = Modifiers::from_bits(Modifiers::COMMAND | Modifiers::OPTION);
    app.handle_event(&key(KEY_F, command));
    app.handle_event(&ime(ImeEvent::Committed("alpha".into())));
    assert!(app.handle_event(&key(KEY_F, command_option)).visual_changed);
    assert!(
        app.handle_event(&ime(ImeEvent::Committed("replacement".into())))
            .visual_changed
    );
    let replacement = app.find.replacement().to_owned();
    assert!(!app.handle_event(&key(KEY_TAB, command)).visual_changed);
    assert_eq!(app.find.field(), crate::find::FindField::Replacement);
    assert!(
        !app.handle_event(&key(KEY_DELETE_BACKWARD, command))
            .visual_changed
    );
    assert_eq!(app.find.replacement(), replacement);
    Ok(())
}

#[test]
fn stale_find_completion_preserves_the_current_selection() -> Result<(), StudioRenderError> {
    let mut app = test_app().map_err(|_| StudioRenderError::Domain)?;
    *app.buffer_mut() = Buffer::new("alpha beta alpha");
    assert!(
        app.handle_event(&key(KEY_F, Modifiers::from_bits(Modifiers::COMMAND),))
            .visual_changed
    );
    assert!(
        app.handle_event(&ime(ImeEvent::Committed("alpha".into())))
            .visual_changed
    );
    let stale = app
        .prepare_find_request()?
        .ok_or(StudioRenderError::Domain)?
        .execute();
    app.selection = Selection::caret(ByteOffset::new(3));
    assert!(
        app.handle_event(&ime(ImeEvent::Committed("x".into())))
            .visual_changed
    );
    assert!(!app.apply_find_output(stale).visual_changed);
    assert_eq!(app.selection, Selection::caret(ByteOffset::new(3)));
    Ok(())
}

#[test]
fn current_and_all_replacement_are_atomic_and_undoable() -> Result<(), StudioRenderError> {
    let original = "alpha beta alpha";
    let command = Modifiers::from_bits(Modifiers::COMMAND);
    let command_option = Modifiers::from_bits(Modifiers::COMMAND | Modifiers::OPTION);

    let mut current = test_app().map_err(|_| StudioRenderError::Domain)?;
    *current.buffer_mut() = Buffer::new(original);
    assert!(current.handle_event(&key(KEY_F, command)).visual_changed);
    assert!(
        current
            .handle_event(&ime(ImeEvent::Committed("alpha".into())))
            .visual_changed
    );
    assert!(current.complete_pending_find_for_test()?.visual_changed);
    assert!(
        current
            .handle_event(&key(KEY_F, command_option))
            .visual_changed
    );
    assert!(
        current
            .handle_event(&ime(ImeEvent::Committed("omega".into())))
            .visual_changed
    );
    assert!(
        current
            .handle_event(&key(KEY_RETURN, command))
            .document_changed
    );
    assert_eq!(current.buffer().snapshot().text(), "omega beta alpha");
    current.find_needs_search = false;
    assert!(!current.update_find_after_document_change().visual_changed);
    assert!(current.find_needs_search);
    assert!(current.complete_pending_find_for_test()?.visual_changed);
    assert_eq!(current.selection.range(), 11..16);
    assert!(
        current
            .handle_event(&key(KEY_ESCAPE, Modifiers::default()))
            .visual_changed
    );
    assert!(current.handle_event(&key(KEY_Z, command)).document_changed);
    assert_eq!(current.buffer().snapshot().text(), original);

    let mut all = test_app().map_err(|_| StudioRenderError::Domain)?;
    *all.buffer_mut() = Buffer::new(original);
    assert!(all.handle_event(&key(KEY_F, command)).visual_changed);
    assert!(
        all.handle_event(&ime(ImeEvent::Committed("alpha".into())))
            .visual_changed
    );
    assert!(all.complete_pending_find_for_test()?.visual_changed);
    assert!(all.handle_event(&key(KEY_F, command_option)).visual_changed);
    assert!(
        all.handle_event(&ime(ImeEvent::Committed("x".into())))
            .visual_changed
    );
    assert!(
        all.handle_event(&key(KEY_RETURN, command_option))
            .document_changed
    );
    assert_eq!(all.buffer().snapshot().text(), "x beta x");
    assert!(
        all.handle_event(&key(KEY_ESCAPE, Modifiers::default()))
            .visual_changed
    );
    assert!(all.handle_event(&key(KEY_Z, command)).document_changed);
    assert_eq!(all.buffer().snapshot().text(), original);
    Ok(())
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one application-level matrix discriminates coupled find focus and rejection paths"
)]
fn find_focus_rejections_and_empty_actions_remain_bounded() -> Result<(), StudioRenderError> {
    let mut app = test_app().map_err(|_| StudioRenderError::Domain)?;
    let command = Modifiers::from_bits(Modifiers::COMMAND);
    let command_option = Modifiers::from_bits(Modifiers::COMMAND | Modifiers::OPTION);
    assert!(app.handle_event(&key(KEY_F, command)).visual_changed);
    assert!(matches!(
        app.handle_event_with_response(&clipboard_key("c")),
        StudioTransition {
            effect: EventEffect {
                visual_changed: false,
                document_changed: false,
                document_identity_advanced: false,
            },
            clipboard_write: None,
            cancel_close: false,
        }
    ));
    assert!(
        !app.handle_event(&key(KEY_TAB, Modifiers::default()))
            .visual_changed
    );
    assert!(
        !app.handle_event(&key(KEY_LEFT, Modifiers::default()))
            .visual_changed
    );
    assert!(
        app.handle_event(&key(KEY_DELETE_BACKWARD, Modifiers::default()))
            .visual_changed
    );
    assert!(
        !app.handle_event(&ime(ImeEvent::Updated {
            text: "x".into(),
            selected_start_utf16: 2,
            selected_length_utf16: 0,
        }))
        .visual_changed
    );
    assert!(app.handle_event(&ime(ImeEvent::Started)).visual_changed);
    assert!(app.handle_event(&ime(ImeEvent::Cancelled)).visual_changed);
    assert!(!app.handle_event(&ime(ImeEvent::Cancelled)).visual_changed);

    let oversized = "x".repeat(find::MAX_QUERY_BYTES + 1);
    assert!(
        app.handle_event(&ime(ImeEvent::Committed(oversized.into())))
            .visual_changed
    );
    assert!(app.handle_event(&key(KEY_RETURN, command)).visual_changed);
    assert!(
        app.handle_event(&key(KEY_RETURN, command_option))
            .visual_changed
    );
    assert!(!app.navigate_find(true).visual_changed);
    assert!(!app.complete_pending_find_for_test()?.visual_changed);

    assert!(app.handle_event(&key(KEY_F, command_option)).visual_changed);
    assert!(
        app.handle_event(&key(KEY_TAB, Modifiers::default()))
            .visual_changed
    );
    assert!(
        app.handle_event(&key(KEY_TAB, Modifiers::default()))
            .visual_changed
    );

    *app.buffer_mut() = Buffer::new(&"line\n".repeat(100));
    app.last_viewport = viewport().map_err(|_| StudioRenderError::Domain)?;
    app.scroll_y = 50.0;
    assert!(app.select_find_range(0..1).visual_changed);
    assert!(app.scroll_y.abs() < f32::EPSILON);
    app.scroll_y = 0.0;
    let last = app.buffer().snapshot().len_bytes().saturating_sub(2);
    assert!(app.select_find_range(last..last + 1).visual_changed);
    assert!(app.scroll_y > 0.0);

    let mut empty = test_app().map_err(|_| StudioRenderError::Domain)?;
    *empty.buffer_mut() = Buffer::new("alpha");
    assert!(empty.handle_event(&key(KEY_F, command)).visual_changed);
    assert!(
        empty
            .handle_event(&ime(ImeEvent::Committed("missing".into())))
            .visual_changed
    );
    assert!(empty.complete_pending_find_for_test()?.visual_changed);
    assert!(
        empty
            .handle_event(&key(KEY_F, command_option))
            .visual_changed
    );
    assert!(
        !empty
            .handle_event(&key(KEY_RETURN, command_option))
            .visual_changed
    );

    let mut budget = test_app().map_err(|_| StudioRenderError::Domain)?;
    *budget.buffer_mut() = Buffer::new(&"x".repeat(5_000));
    assert!(budget.handle_event(&key(KEY_F, command)).visual_changed);
    assert!(
        budget
            .handle_event(&ime(ImeEvent::Committed("x".into())))
            .visual_changed
    );
    assert!(budget.complete_pending_find_for_test()?.visual_changed);
    assert!(
        budget
            .handle_event(&key(KEY_F, command_option))
            .visual_changed
    );
    assert!(
        budget
            .handle_event(&ime(ImeEvent::Committed(
                "r".repeat(find::MAX_QUERY_BYTES).into()
            )))
            .visual_changed
    );
    let before = budget.buffer().snapshot().text();
    assert!(
        budget
            .handle_event(&key(KEY_RETURN, command_option))
            .visual_changed
    );
    assert_eq!(budget.buffer().snapshot().text(), before);
    assert!(
        budget
            .find
            .display_text()?
            .contains("replacement transaction")
    );

    let mut exhausted = test_app().map_err(|_| StudioRenderError::Domain)?;
    assert!(exhausted.handle_event(&key(KEY_F, command)).visual_changed);
    assert!(
        exhausted
            .handle_event(&ime(ImeEvent::Committed("x".into())))
            .visual_changed
    );
    exhausted.find.exhaust_generation_for_test();
    let query = exhausted.find.query().to_owned();
    assert!(
        exhausted
            .handle_event(&key(KEY_DELETE_BACKWARD, Modifiers::default()))
            .visual_changed
    );
    assert_eq!(exhausted.find.query(), query);
    Ok(())
}

#[test]
fn runtime_quick_open_worker_admits_inventory_and_ranked_results()
-> Result<(), Box<dyn std::error::Error>> {
    let root = TestWorkspace::new()?;
    root.write("alpha.rs", "alpha")?;
    let app = StudioApp::open_workspace(TestTextSystem, root.path())?;
    let viewport = viewport()?;
    let clear = LinearRgba::new(0.02, 0.02, 0.02, 1.0).ok_or(SurfaceError::invariant(
        alpine_platform_macos::SurfaceOperation::Application,
    ))?;
    let mut runtime = Application::new(app, viewport, clear, WorkerConfig::default())?;
    let command = Modifiers::from_bits(Modifiers::COMMAND);

    let pending = runtime
        .dispatch(&key(KEY_P, command))
        .ok_or("quick-open frame")?;
    let pending_quads = pending.scene().quads().len();
    let mut admitted = false;
    for timestamp in 300..812 {
        std::thread::sleep(std::time::Duration::from_millis(1));
        if let Some(frame) = runtime.dispatch(&SurfaceEvent::Wake {
            timestamp: EventTimestamp::new(timestamp),
        }) && frame.scene().quads().len() > pending_quads
        {
            admitted = true;
            break;
        }
    }
    assert!(admitted);
    Ok(())
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one end-to-end journey distinguishes the quick-open painter, input, and selection mutants"
)]
fn quick_open_lazily_indexes_renders_and_opens_a_nested_file()
-> Result<(), Box<dyn std::error::Error>> {
    let root = TestWorkspace::new()?;
    root.write(".gitignore", "ignored/\n")?;
    root.create_dir("src")?;
    root.write("src/main.rs", "fn nested() {}\n")?;
    root.create_dir("ignored")?;
    root.write("ignored/lost.rs", "lost")?;
    let mut app = StudioApp::open_workspace(TestTextSystem, root.path())?;
    assert!(app.quick_open.inventory_report().is_none());

    let command = Modifiers::from_bits(Modifiers::COMMAND);
    assert!(app.handle_event(&key(KEY_P, command)).visual_changed);
    let inventory = app
        .prepare_quick_open_request()?
        .ok_or("inventory request")?;
    assert!(
        app.apply_quick_open_output(inventory.execute())
            .visual_changed
    );
    let initial_query = app
        .prepare_quick_open_request()?
        .ok_or("initial query request")?;
    assert!(
        app.apply_quick_open_output(initial_query.execute())
            .visual_changed
    );
    let report = app
        .quick_open
        .inventory_report()
        .ok_or("inventory report")?;
    assert_eq!(report.paths, 2);
    assert_eq!(report.errors, 0);

    let first_path = app.quick_open.selected_path()?;
    assert!(
        app.handle_event(&key(KEY_DOWN, Modifiers::default()))
            .visual_changed
    );
    let second_path = app.quick_open.selected_path()?;
    assert_ne!(first_path, second_path);
    assert_eq!(
        app.handle_event(&key(KEY_UP, command)),
        EventEffect::default()
    );
    assert_eq!(app.quick_open.selected_path()?, second_path);

    let narrow_viewport = Size::new(300.0, 180.0).ok_or("narrow viewport")?;
    let scene = app.try_scene(SceneRevision::new(90), narrow_viewport)?;
    let overlay_width = QUICK_OPEN_WIDTH.min(narrow_viewport.width() - CONTENT_INSET * 2.0);
    let overlay_left = (narrow_viewport.width() - overlay_width) * 0.5;
    let overlay_top = TAB_BAR_HEIGHT + CONTENT_INSET;
    let overlay_height = QUICK_OPEN_QUERY_HEIGHT + QUICK_OPEN_ROW_HEIGHT * 2.0;
    let expected_overlay = Rect::new(
        Point::new(overlay_left, overlay_top).ok_or("overlay origin")?,
        Size::new(overlay_width, overlay_height).ok_or("overlay size")?,
    );
    let overlay_clip = scene
        .clips()
        .iter()
        .position(|clip| clip.bounds() == expected_overlay)
        .ok_or("quick-open clip")?;
    let expected_selected = Rect::new(
        Point::new(
            overlay_left,
            overlay_top + QUICK_OPEN_QUERY_HEIGHT + QUICK_OPEN_ROW_HEIGHT,
        )
        .ok_or("selected origin")?,
        Size::new(overlay_width, QUICK_OPEN_ROW_HEIGHT).ok_or("selected size")?,
    );
    assert!(scene.quads().iter().any(|quad| {
        quad.clip().is_some_and(|clip| clip.index() == overlay_clip)
            && quad.bounds() == expected_selected
    }));
    let query_y = overlay_top + 19.0;
    let query_glyph = scene
        .glyphs()
        .iter()
        .find(|glyph| {
            glyph
                .clip()
                .is_some_and(|clip| clip.index() == overlay_clip)
                && glyph.bounds().origin().y().to_bits() == query_y.to_bits()
        })
        .ok_or("query glyph")?;
    assert_eq!(
        query_glyph.bounds().origin().x().to_bits(),
        (overlay_left + FIND_BAR_INSET).to_bits()
    );
    let second_row_y = overlay_top + QUICK_OPEN_QUERY_HEIGHT + QUICK_OPEN_ROW_HEIGHT + 16.0;
    let second_row_glyph = scene
        .glyphs()
        .iter()
        .find(|glyph| {
            glyph
                .clip()
                .is_some_and(|clip| clip.index() == overlay_clip)
                && glyph.bounds().origin().y().to_bits() == second_row_y.to_bits()
        })
        .ok_or("second-row glyph")?;
    assert_eq!(
        second_row_glyph.bounds().origin().x().to_bits(),
        (overlay_left + FIND_BAR_INSET).to_bits()
    );

    assert!(
        app.handle_event(&key(KEY_UP, Modifiers::default()))
            .visual_changed
    );
    assert_eq!(app.quick_open.selected_path()?, first_path);
    assert_eq!(
        app.handle_event(&key(KEY_DOWN, command)),
        EventEffect::default()
    );
    assert_eq!(app.quick_open.selected_path()?, first_path);

    assert!(
        app.handle_event(&ime(ImeEvent::Committed("main".into())))
            .visual_changed
    );
    let query = app
        .prepare_quick_open_request()?
        .ok_or("filtered query request")?;
    assert!(app.apply_quick_open_output(query.execute()).visual_changed);
    assert_eq!(app.quick_open.selected_path()?.as_ref(), "src/main.rs");
    let result = app.quick_open.result_report().ok_or("query result")?;
    assert_eq!(result.0, 1);
    assert!(result.1 <= quick_open::MAX_RESULT_METADATA_BYTES);

    assert_eq!(
        app.handle_event(&key(KEY_DELETE_BACKWARD, command)),
        EventEffect::default()
    );
    assert_eq!(app.quick_open.query(), "main");
    assert!(
        app.handle_event(&key(KEY_DELETE_BACKWARD, Modifiers::default()))
            .visual_changed
    );
    assert_eq!(app.quick_open.query(), "mai");
    assert!(
        app.handle_event(&ime(ImeEvent::Committed("n".into())))
            .visual_changed
    );
    let restored_query = app
        .prepare_quick_open_request()?
        .ok_or("restored query request")?;
    assert!(
        app.apply_quick_open_output(restored_query.execute())
            .visual_changed
    );
    assert_eq!(
        app.handle_event(&key(KEY_RETURN, command)),
        EventEffect::default()
    );
    assert!(app.quick_open.is_open());
    let effect = app.handle_event(&key(KEY_RETURN, Modifiers::default()));
    assert!(effect.document_changed);
    assert!(effect.document_identity_advanced);
    assert!(!app.quick_open.is_open());
    assert_eq!(app.buffer().snapshot().text(), "fn nested() {}\n");
    Ok(())
}

#[test]
fn quick_open_focus_suppresses_editor_input_and_requires_a_workspace()
-> Result<(), Box<dyn std::error::Error>> {
    let command = Modifiers::from_bits(Modifiers::COMMAND);
    let mut scratch = test_app()?;
    assert!(scratch.handle_event(&key(KEY_P, command)).visual_changed);
    assert!(scratch.last_workspace_error.is_some());

    let root = TestWorkspace::new()?;
    root.write("alpha.rs", "alpha")?;
    let mut app = StudioApp::open_workspace(TestTextSystem, root.path())?;
    *app.buffer_mut() = Buffer::new("alpha");
    app.selection = Selection::new(ByteOffset::new(0), ByteOffset::new(5));
    let before = app.buffer().snapshot().text();
    assert!(app.handle_event(&key(KEY_P, command)).visual_changed);
    assert!(app.handle_event(&ime(ImeEvent::Started)).visual_changed);
    assert!(
        app.handle_event(&ime(ImeEvent::Updated {
            text: "alpha".into(),
            selected_start_utf16: 0,
            selected_length_utf16: 1,
        }))
        .visual_changed
    );
    assert!(
        app.handle_event(&ime(ImeEvent::Updated {
            text: "alphab".into(),
            selected_start_utf16: 6,
            selected_length_utf16: 0,
        }))
        .visual_changed
    );
    assert!(
        app.handle_event(&ime(ImeEvent::Committed("alpha".into())))
            .visual_changed
    );
    assert_eq!(app.buffer().snapshot().text(), before);
    assert!(matches!(
        app.handle_event_with_response(&clipboard_key("c")),
        StudioTransition {
            clipboard_write: None,
            ..
        }
    ));
    assert!(
        app.handle_event(&key(KEY_ESCAPE, Modifiers::default()))
            .visual_changed
    );
    assert_eq!(app.buffer().snapshot().text(), before);
    Ok(())
}

#[test]
fn quick_open_path_revalidation_accepts_the_limit_and_rejects_the_next_component()
-> Result<(), Box<dyn std::error::Error>> {
    let root = TestWorkspace::new()?;
    let mut parent = PathBuf::new();
    for _ in 0..quick_open::MAX_DEPTH.saturating_sub(1) {
        parent.push("d");
    }
    fs::create_dir_all(root.path().join(&parent))?;

    let accepted = parent.join("accepted.rs");
    root.write(accepted.to_str().ok_or("accepted UTF-8 path")?, "accepted")?;
    let workspace = Workspace::open(root.path(), WorkspaceLimits::default())?;
    assert_eq!(
        workspace.path_for_relative_file(&accepted)?,
        fs::canonicalize(root.path().join(&accepted))?
    );

    let overflow_parent = parent.join("overflow");
    fs::create_dir(root.path().join(&overflow_parent))?;
    let overflow = overflow_parent.join("rejected.rs");
    root.write(overflow.to_str().ok_or("overflow UTF-8 path")?, "rejected")?;
    assert!(matches!(
        workspace.path_for_relative_file(&overflow),
        Err(WorkspaceError::PathDepthExceeded { actual, limit })
            if actual == quick_open::MAX_DEPTH + 1 && limit == quick_open::MAX_DEPTH
    ));
    Ok(())
}

#[test]
fn quick_open_submission_rollback_only_rejects_failed_admission()
-> Result<(), Box<dyn std::error::Error>> {
    let root = TestWorkspace::new()?;
    root.write("alpha.rs", "alpha")?;
    let mut app = StudioApp::open_workspace(TestTextSystem, root.path())?;
    let command = Modifiers::from_bits(Modifiers::COMMAND);
    assert!(app.handle_event(&key(KEY_P, command)).visual_changed);
    let request = app
        .prepare_quick_open_request()?
        .ok_or("inventory request")?;
    let identity = request.identity();
    assert!(!app.reject_failed_quick_open_submission(identity, false));
    assert!(app.reject_failed_quick_open_submission(identity, true));
    assert!(app.prepare_quick_open_request()?.is_some());
    Ok(())
}

#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
#[test]
fn run_rejects_an_unsupported_host() {
    assert!(matches!(
        initial_scene(),
        Err(SurfaceError::UnsupportedPlatform)
    ));
    assert!(matches!(
        run(),
        Err(RuntimeError::Surface(SurfaceError::UnsupportedPlatform))
    ));
    assert!(matches!(
        run_file("missing.txt"),
        Err(StudioError::Runtime(RuntimeError::Surface(
            SurfaceError::UnsupportedPlatform
        )))
    ));
    assert!(matches!(
        run_path("."),
        Err(StudioError::Runtime(RuntimeError::Surface(
            SurfaceError::UnsupportedPlatform
        )))
    ));
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn native_file_constructor_rejects_a_missing_file_before_native_setup()
-> Result<(), Box<dyn std::error::Error>> {
    let root = TestWorkspace::new()?;
    assert!(matches!(
        native_file_app(&root.path().join("missing.rs")),
        Err(StudioError::File(_))
    ));
    Ok(())
}

struct SelectiveFailingRasterTextSystem {
    glyph_id: u32,
}

impl TextShaper for SelectiveFailingRasterTextSystem {
    fn shape(&mut self, text: &str, font: FontKey) -> Result<LineLayout, LayoutError> {
        TestTextSystem.shape(text, font)
    }
}

impl GlyphRasterizer for SelectiveFailingRasterTextSystem {
    fn rasterize(
        &mut self,
        font: FontKey,
        glyph_id: u32,
        subpixel_x: u8,
    ) -> Result<RasterizedGlyph, LayoutError> {
        if glyph_id == self.glyph_id {
            Err(LayoutError::NativeFailure(
                "injected selective raster failure",
            ))
        } else {
            TestTextSystem.rasterize(font, glyph_id, subpixel_x)
        }
    }
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one corpus distinguishes independent quick-open orchestration failures"
)]
fn quick_open_orchestration_failures_are_bounded_and_visible()
-> Result<(), Box<dyn std::error::Error>> {
    let command = Modifiers::from_bits(Modifiers::COMMAND);

    let mut scratch = test_app()?;
    assert!(scratch.quick_open.open(1)?);
    assert!(matches!(
        scratch.prepare_quick_open_request(),
        Err(QuickOpenError::NoWorkspace)
    ));
    assert!(scratch.quick_open.close());

    let root = TestWorkspace::new()?;
    root.write("alpha.rs", "alpha")?;

    let mut exhausted_open = StudioApp::open_workspace(TestTextSystem, root.path())?;
    exhausted_open.quick_open.exhaust_generations_for_test();
    assert!(
        exhausted_open
            .handle_event(&key(KEY_P, command))
            .visual_changed
    );
    assert!(exhausted_open.last_workspace_error.is_some());

    let mut missing = StudioApp::open_workspace(TestTextSystem, root.path())?;
    assert!(missing.handle_event(&key(KEY_P, command)).visual_changed);
    assert!(
        missing
            .handle_event(&key(KEY_RETURN, Modifiers::default()))
            .visual_changed
    );
    assert!(missing.last_workspace_error.is_some());
    let position = Point::new(20.0, 20.0).ok_or(StudioRenderError::Domain)?;
    assert_eq!(
        missing.handle_pointer(
            PointerAction::Down,
            position,
            PointerButton::Primary,
            Modifiers::default(),
        ),
        EventEffect::default()
    );

    let mut ime_app = StudioApp::open_workspace(TestTextSystem, root.path())?;
    assert!(ime_app.handle_event(&key(KEY_P, command)).visual_changed);
    let failures = ime_app.input_failures;
    assert_eq!(
        ime_app.handle_event(&ime(ImeEvent::Updated {
            text: "a".into(),
            selected_start_utf16: u32::MAX,
            selected_length_utf16: 1,
        })),
        EventEffect::default()
    );
    assert_eq!(
        ime_app.handle_event(&ime(ImeEvent::Updated {
            text: "a".into(),
            selected_start_utf16: 2,
            selected_length_utf16: 0,
        })),
        EventEffect::default()
    );
    assert_eq!(ime_app.input_failures, failures + 2);
    assert!(ime_app.handle_event(&ime(ImeEvent::Started)).visual_changed);
    assert!(
        ime_app
            .handle_event(&ime(ImeEvent::Cancelled))
            .visual_changed
    );
    ime_app.quick_open.exhaust_generations_for_test();
    assert!(
        ime_app
            .handle_event(&ime(ImeEvent::Committed("x".into())))
            .visual_changed
    );

    let mut delete_error = StudioApp::open_workspace(TestTextSystem, root.path())?;
    assert!(
        delete_error
            .handle_event(&key(KEY_P, command))
            .visual_changed
    );
    assert!(delete_error.quick_open.commit_text("x")?);
    delete_error.quick_open.exhaust_generations_for_test();
    assert!(
        delete_error
            .handle_event(&key(KEY_DELETE_BACKWARD, Modifiers::default()))
            .visual_changed
    );

    let mut stale = StudioApp::open_workspace(TestTextSystem, root.path())?;
    assert!(stale.handle_event(&key(KEY_P, command)).visual_changed);
    let request = stale
        .prepare_quick_open_request()?
        .ok_or("stale inventory request")?;
    assert!(stale.quick_open.close());
    assert_eq!(
        stale.apply_quick_open_output(request.execute()),
        EventEffect::default()
    );
    Ok(())
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "filesystem identity defenses require independent malformed and race fixtures"
)]
fn quick_open_path_revalidation_rejects_every_identity_break()
-> Result<(), Box<dyn std::error::Error>> {
    let root = TestWorkspace::new()?;
    root.write("plain.rs", "plain")?;
    root.create_dir("directory")?;
    let workspace = Workspace::open(root.path(), WorkspaceLimits::default())?;

    for relative in [Path::new(""), root.path(), Path::new("../outside.rs")] {
        assert!(matches!(
            workspace.path_for_relative_file(relative),
            Err(WorkspaceError::InvalidRelativePath(_))
        ));
    }
    assert!(matches!(
        workspace.path_for_relative_file(Path::new("missing.rs")),
        Err(WorkspaceError::Io { .. })
    ));
    assert!(matches!(
        workspace.path_for_relative_file(Path::new("plain.rs/child")),
        Err(WorkspaceError::NotRegularFile(_))
    ));
    assert!(matches!(
        workspace.path_for_relative_file(Path::new("directory")),
        Err(WorkspaceError::NotRegularFile(_))
    ));

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;

        root.create_dir("real-directory")?;
        symlink(
            root.path().join("real-directory"),
            root.path().join("linked-directory"),
        )?;
        assert!(matches!(
            workspace.path_for_relative_file(Path::new("linked-directory/file.rs")),
            Err(WorkspaceError::Symlink(_))
        ));

        root.write("real.rs", "real")?;
        symlink(root.path().join("real.rs"), root.path().join("linked.rs"))?;
        assert!(matches!(
            workspace.path_for_relative_file(Path::new("linked.rs")),
            Err(WorkspaceError::Symlink(_))
        ));

        root.write("metadata-race.rs", "inside")?;
        let metadata_outside = TestFile::new("outside")?;
        let metadata_outside_path = metadata_outside.path().to_path_buf();
        super::workspace::set_revalidation_hook(move |candidate| {
            assert!(
                fs::remove_file(candidate).is_ok(),
                "remove metadata race target"
            );
            assert!(
                symlink(&metadata_outside_path, candidate).is_ok(),
                "replace metadata race target"
            );
        });
        assert!(matches!(
            workspace.path_for_relative_file(Path::new("metadata-race.rs")),
            Err(WorkspaceError::Symlink(_))
        ));

        root.write("final-metadata-race.rs", "inside")?;
        let final_outside = TestFile::new("outside")?;
        let final_outside_path = final_outside.path().to_path_buf();
        super::workspace::set_revalidation_hook(move |_| {
            super::workspace::set_revalidation_hook(move |candidate| {
                assert!(
                    fs::remove_file(candidate).is_ok(),
                    "remove final metadata race target"
                );
                assert!(
                    symlink(&final_outside_path, candidate).is_ok(),
                    "replace final metadata race target"
                );
            });
        });
        assert!(matches!(
            workspace.path_for_relative_file(Path::new("final-metadata-race.rs")),
            Err(WorkspaceError::Symlink(_))
        ));

        root.write("outside-race.rs", "inside")?;
        let outside = TestFile::new("outside")?;
        let outside_path = outside.path().to_path_buf();
        super::workspace::set_revalidation_hook(move |_| {
            super::workspace::set_revalidation_hook(move |_| {
                super::workspace::set_revalidation_hook(move |candidate| {
                    assert!(
                        fs::remove_file(candidate).is_ok(),
                        "remove outside race target"
                    );
                    assert!(
                        symlink(&outside_path, candidate).is_ok(),
                        "replace outside race target"
                    );
                });
            });
        });
        assert!(matches!(
            workspace.path_for_relative_file(Path::new("outside-race.rs")),
            Err(WorkspaceError::EscapesRoot(_))
        ));

        root.write("inside-target.rs", "target")?;
        root.write("inside-race.rs", "inside")?;
        let inside_target = root.path().join("inside-target.rs");
        super::workspace::set_revalidation_hook(move |_| {
            super::workspace::set_revalidation_hook(move |_| {
                super::workspace::set_revalidation_hook(move |candidate| {
                    assert!(
                        fs::remove_file(candidate).is_ok(),
                        "remove inside race target"
                    );
                    assert!(
                        symlink(&inside_target, candidate).is_ok(),
                        "replace inside race target"
                    );
                });
            });
        });
        assert!(matches!(
            workspace.path_for_relative_file(Path::new("inside-race.rs")),
            Err(WorkspaceError::Symlink(_))
        ));
    }
    Ok(())
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one rendering corpus proves successful rows and independent query and row failures"
)]
fn quick_open_overlay_raster_failures_preserve_scene_atomicity()
-> Result<(), Box<dyn std::error::Error>> {
    let command = Modifiers::from_bits(Modifiers::COMMAND);
    let root = TestWorkspace::new()?;
    root.write("alpha.rs", "alpha")?;
    let mut query_app = StudioApp::open_workspace(
        SelectiveFailingRasterTextSystem {
            glyph_id: u32::from('Q'),
        },
        root.path(),
    )?;
    assert!(query_app.handle_event(&key(KEY_P, command)).visual_changed);
    let inventory = query_app
        .prepare_quick_open_request()?
        .ok_or("query raster inventory")?;
    assert!(
        query_app
            .apply_quick_open_output(inventory.execute())
            .visual_changed
    );
    let query = query_app
        .prepare_quick_open_request()?
        .ok_or("query raster ranking")?;
    assert!(
        query_app
            .apply_quick_open_output(query.execute())
            .visual_changed
    );
    assert!(query_app.quick_open.commit_text("Q")?);
    assert!(matches!(
        query_app.try_scene(SceneRevision::new(1), viewport()?),
        Err(StudioRenderError::Layout(LayoutError::NativeFailure(_)))
    ));

    let nested = TestWorkspace::new()?;
    nested.create_dir("src")?;
    nested.write("src/main.rs", "main")?;
    let mut success_app = StudioApp::open_workspace(TestTextSystem, nested.path())?;
    assert!(
        success_app
            .handle_event(&key(KEY_P, command))
            .visual_changed
    );
    let inventory = success_app
        .prepare_quick_open_request()?
        .ok_or("successful row inventory")?;
    assert!(
        success_app
            .apply_quick_open_output(inventory.execute())
            .visual_changed
    );
    let query = success_app
        .prepare_quick_open_request()?
        .ok_or("successful row ranking")?;
    assert!(
        success_app
            .apply_quick_open_output(query.execute())
            .visual_changed
    );
    assert!(
        !success_app
            .quick_open
            .visible_results(QUICK_OPEN_VISIBLE_ROWS, QUICK_OPEN_OVERSCAN_ROWS)
            .is_empty()
    );
    assert!(
        !success_app
            .try_scene(SceneRevision::new(1), viewport()?)?
            .quads()
            .is_empty()
    );

    let mut row_app = StudioApp::open_workspace(
        SelectiveFailingRasterTextSystem {
            glyph_id: u32::from('/'),
        },
        nested.path(),
    )?;
    assert!(row_app.handle_event(&key(KEY_P, command)).visual_changed);
    let inventory = row_app
        .prepare_quick_open_request()?
        .ok_or("row raster inventory")?;
    assert!(
        row_app
            .apply_quick_open_output(inventory.execute())
            .visual_changed
    );
    let query = row_app
        .prepare_quick_open_request()?
        .ok_or("row raster ranking")?;
    assert!(
        row_app
            .apply_quick_open_output(query.execute())
            .visual_changed
    );
    assert!(
        !row_app
            .quick_open
            .visible_results(QUICK_OPEN_VISIBLE_ROWS, QUICK_OPEN_OVERSCAN_ROWS)
            .is_empty()
    );
    assert!(matches!(
        row_app.try_scene(SceneRevision::new(1), viewport()?),
        Err(StudioRenderError::Layout(LayoutError::NativeFailure(_)))
    ));
    Ok(())
}

#[test]
fn runtime_quick_open_submission_failures_invalidate_without_blocking()
-> Result<(), Box<dyn std::error::Error>> {
    let clear = LinearRgba::new(0.02, 0.02, 0.02, 1.0).ok_or(SurfaceError::invariant(
        alpine_platform_macos::SurfaceOperation::Application,
    ))?;
    let worker_config = WorkerConfig::new(
        std::num::NonZeroUsize::MIN,
        std::num::NonZeroUsize::MIN,
        std::num::NonZeroUsize::MIN,
    );

    let mut no_workspace = test_app()?;
    assert!(no_workspace.quick_open.open(1)?);
    let mut no_workspace_runtime =
        Application::new(no_workspace, viewport()?, clear, worker_config)?;
    assert!(
        no_workspace_runtime
            .dispatch(&SurfaceEvent::Wake {
                timestamp: EventTimestamp::new(900),
            })
            .is_some()
    );

    let root = TestWorkspace::new()?;
    root.write("alpha.rs", "alpha")?;
    let mut rejected = StudioApp::open_workspace(TestTextSystem, root.path())?;
    let command = Modifiers::from_bits(Modifiers::COMMAND);
    assert!(rejected.handle_event(&key(KEY_P, command)).visual_changed);
    rejected.force_quick_open_submission_failure = Some(());
    let mut rejected_runtime = Application::new(rejected, viewport()?, clear, worker_config)?;
    assert!(
        rejected_runtime
            .dispatch(&SurfaceEvent::Wake {
                timestamp: EventTimestamp::new(901),
            })
            .is_some()
    );
    Ok(())
}

#[test]
fn file_tree_lazily_loads_and_opens_a_nested_file() -> Result<(), Box<dyn std::error::Error>> {
    let root = TestWorkspace::new()?;
    root.create_dir("src")?;
    root.write("src/main.rs", "fn nested() {}\n")?;
    let mut app = StudioApp::open_workspace_lazy(TestTextSystem, root.path())?;
    assert!(!app.file_tree.is_active());
    assert!(app.prepare_file_tree_request()?.is_none());
    let _first_scene = app.try_scene(SceneRevision::new(1), viewport()?)?;
    assert!(app.prepare_file_tree_request()?.is_none());

    let command_shift = Modifiers::from_bits(Modifiers::COMMAND | Modifiers::SHIFT);
    assert!(app.handle_event(&key(KEY_E, command_shift)).visual_changed);
    let root_request = app
        .prepare_file_tree_request()?
        .ok_or("root tree request")?;
    assert!(
        app.apply_file_tree_output(root_request.execute())
            .visual_changed
    );
    let rows = app.file_tree.visible_rows(0, 8, TREE_OVERSCAN_ROWS)?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].path.as_ref(), "src");
    assert!(matches!(
        app.file_tree.activate_row(0)?,
        FileTreeAction::Changed
    ));
    let nested_request = app
        .prepare_file_tree_request()?
        .ok_or("nested tree request")?;
    assert!(
        app.apply_file_tree_output(nested_request.execute())
            .visual_changed
    );
    assert_eq!(app.file_tree.total_rows(), 2);
    assert!(app.file_tree.navigate(true, 8));
    let action = app.file_tree.activate_selected()?;
    assert!(matches!(
        &action,
        FileTreeAction::Open(path) if path.as_ref() == "src/main.rs"
    ));
    let effect = app.apply_file_tree_action(action);
    assert!(effect.document_changed);
    assert!(effect.document_identity_advanced);
    assert_eq!(app.buffer().snapshot().text(), "fn nested() {}\n");
    assert!(!app.file_tree.is_focused());
    Ok(())
}

#[test]
fn file_tree_keyboard_geometry_and_error_routing_are_discriminating()
-> Result<(), Box<dyn std::error::Error>> {
    let command = Modifiers::from_bits(Modifiers::COMMAND);
    let shift = Modifiers::from_bits(Modifiers::SHIFT);
    let command_shift = Modifiers::from_bits(Modifiers::COMMAND | Modifiers::SHIFT);
    let mut no_workspace = test_app()?;
    let missing = no_workspace.handle_event(&key(KEY_E, command_shift));
    assert!(missing.visual_changed);
    assert_eq!(no_workspace.workspace_failures, 1);
    assert!(
        no_workspace
            .last_workspace_error
            .as_deref()
            .is_some_and(|message| message.contains("requires one local workspace"))
    );

    let root = TestWorkspace::new()?;
    root.write("alpha.rs", "alpha")?;
    root.write("beta.rs", "beta")?;
    let mut app = StudioApp::open_workspace_lazy(TestTextSystem, root.path())?;
    assert_eq!(
        app.handle_event(&key(KEY_E, command)),
        EventEffect::default()
    );
    assert_eq!(app.handle_event(&key(KEY_E, shift)), EventEffect::default());
    assert!(!app.file_tree.is_active());
    assert!(app.handle_event(&key(KEY_E, command_shift)).visual_changed);
    let text_before_ignored_ime = app.buffer().snapshot().text().clone();
    assert_eq!(
        app.handle_event(&ime(ImeEvent::Committed("ignored".into()))),
        EventEffect::default()
    );
    assert_eq!(app.buffer().snapshot().text(), text_before_ignored_ime);
    let no_selection = app.handle_event(&key(KEY_RETURN, Modifiers::default()));
    assert!(no_selection.visual_changed);
    assert_eq!(app.workspace_failures, 1);
    let request = app
        .prepare_file_tree_request()?
        .ok_or("root file-tree request")?;
    assert!(app.apply_file_tree_output(request.execute()).visual_changed);

    let tree_viewport = Size::new(640.0, 440.0).ok_or("tree viewport")?;
    let _scene = app.try_scene(SceneRevision::new(20), tree_viewport)?;
    assert_eq!(app.visible_tree_rows(), 20);
    assert_eq!(app.file_tree.total_rows(), 2);

    assert!(
        app.handle_event(&key(KEY_DOWN, Modifiers::default()))
            .visual_changed
    );
    let selected = app
        .file_tree
        .visible_rows(0, 2, 0)?
        .into_iter()
        .find(|row| row.selected)
        .ok_or("down selection")?;
    assert_eq!(selected.path.as_ref(), "beta.rs");
    assert_eq!(
        app.handle_event(&key(KEY_UP, command)),
        EventEffect::default()
    );
    assert!(
        app.handle_event(&key(KEY_UP, Modifiers::default()))
            .visual_changed
    );
    let selected = app
        .file_tree
        .visible_rows(0, 2, 0)?
        .into_iter()
        .find(|row| row.selected)
        .ok_or("up selection")?;
    assert_eq!(selected.path.as_ref(), "alpha.rs");
    assert_eq!(
        app.handle_event(&key(KEY_DOWN, command)),
        EventEffect::default()
    );
    assert!(
        app.handle_event(&key(KEY_DOWN, Modifiers::default()))
            .visual_changed
    );

    assert!(
        app.handle_event(&key(KEY_ESCAPE, Modifiers::default()))
            .visual_changed
    );
    assert!(!app.file_tree.is_focused());
    assert!(app.handle_event(&key(KEY_E, command_shift)).visual_changed);
    assert!(app.file_tree.is_focused());
    assert!(app.handle_event(&key(KEY_E, command_shift)).visual_changed);
    assert!(!app.file_tree.is_visible());
    assert!(app.handle_event(&key(KEY_E, command_shift)).visual_changed);
    assert!(app.file_tree.is_focused());
    let before_open = app.buffer().snapshot().text().clone();
    assert_eq!(
        app.handle_event(&key(KEY_RETURN, command)),
        EventEffect::default()
    );
    assert_eq!(app.buffer().snapshot().text(), before_open);
    let opened = app.handle_event(&key(KEY_RETURN, Modifiers::default()));
    assert!(opened.document_changed);
    assert!(opened.document_identity_advanced);
    assert_eq!(app.buffer().snapshot().text(), "beta");
    assert!(!app.file_tree.is_focused());
    Ok(())
}

#[test]
fn runtime_file_tree_submission_admits_and_forced_failure_rolls_back()
-> Result<(), Box<dyn std::error::Error>> {
    let clear = LinearRgba::new(0.02, 0.02, 0.02, 1.0).ok_or(SurfaceError::invariant(
        alpine_platform_macos::SurfaceOperation::Application,
    ))?;
    let command_shift = Modifiers::from_bits(Modifiers::COMMAND | Modifiers::SHIFT);

    let admitted_root = TestWorkspace::new()?;
    admitted_root.write("alpha.rs", "alpha")?;
    let mut admitted = StudioApp::open_workspace_lazy(TestTextSystem, admitted_root.path())?;
    assert!(
        admitted
            .handle_event(&key(KEY_E, command_shift))
            .visual_changed
    );
    let mut admitted_runtime =
        Application::new(admitted, viewport()?, clear, WorkerConfig::default())?;
    let pending = admitted_runtime
        .dispatch(&SurfaceEvent::Wake {
            timestamp: EventTimestamp::new(920),
        })
        .ok_or("pending file-tree frame")?;
    let pending_glyphs = pending.scene().glyphs().len();
    let mut published = false;
    for timestamp in 921..1_433 {
        std::thread::sleep(std::time::Duration::from_millis(1));
        if let Some(frame) = admitted_runtime.dispatch(&SurfaceEvent::Wake {
            timestamp: EventTimestamp::new(timestamp),
        }) && frame.scene().glyphs().len() > pending_glyphs
        {
            published = true;
            break;
        }
    }
    assert!(published);

    let rejected_root = TestWorkspace::new()?;
    rejected_root.write("alpha.rs", "alpha")?;
    let mut rejected = StudioApp::open_workspace_lazy(TestTextSystem, rejected_root.path())?;
    assert!(
        rejected
            .handle_event(&key(KEY_E, command_shift))
            .visual_changed
    );
    rejected.force_file_tree_submission_failure = Some(());
    let mut rejected_runtime =
        Application::new(rejected, viewport()?, clear, WorkerConfig::default())?;
    let rejected_frame = rejected_runtime
        .dispatch(&SurfaceEvent::Wake {
            timestamp: EventTimestamp::new(1_434),
        })
        .ok_or("rejected file-tree frame")?;
    let rejected_glyphs = rejected_frame.scene().glyphs().len();
    for timestamp in 1_435..1_499 {
        std::thread::sleep(std::time::Duration::from_millis(1));
        if let Some(frame) = rejected_runtime.dispatch(&SurfaceEvent::Wake {
            timestamp: EventTimestamp::new(timestamp),
        }) {
            assert_eq!(frame.scene().glyphs().len(), rejected_glyphs);
        }
    }
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "wall-clock qualification is not meaningful under Miri")]
fn file_tree_stage_measurements_are_separate_and_bounded() -> Result<(), Box<dyn std::error::Error>>
{
    let root = TestWorkspace::new()?;
    for index in 0..1_024 {
        root.write(&format!("file-{index:04}.rs"), "x")?;
    }
    let mut app = StudioApp::open_workspace_lazy(TestTextSystem, root.path())?;
    let command_shift = Modifiers::from_bits(Modifiers::COMMAND | Modifiers::SHIFT);

    let activation_start = std::time::Instant::now();
    assert!(app.handle_event(&key(KEY_E, command_shift)).visual_changed);
    let activation = activation_start.elapsed();

    let request = app
        .prepare_file_tree_request()?
        .ok_or("stage directory request")?;
    let enumeration_start = std::time::Instant::now();
    let output = request.execute();
    let enumeration = enumeration_start.elapsed();
    assert!(app.apply_file_tree_output(output).visual_changed);

    let flatten_start = std::time::Instant::now();
    let rows = app.file_tree.visible_rows(0, 40, TREE_OVERSCAN_ROWS)?;
    let flatten = flatten_start.elapsed();
    assert_eq!(rows.len(), 46);
    assert_eq!(app.file_tree.snapshot().1, 1_024);

    let scene_start = std::time::Instant::now();
    let scene = app.try_scene(SceneRevision::new(200), viewport()?)?;
    let scene_build = scene_start.elapsed();
    assert!(!scene.glyphs().is_empty());
    for elapsed in [activation, enumeration, flatten, scene_build] {
        assert!(elapsed < std::time::Duration::from_secs(5));
    }
    eprintln!(
        "file-tree stages: activation={activation:?} enumeration={enumeration:?} flatten={flatten:?} scene={scene_build:?}"
    );
    Ok(())
}

#[test]
fn file_tree_error_and_generation_failure_remain_structured()
-> Result<(), Box<dyn std::error::Error>> {
    let render_error = StudioRenderError::from(FileTreeError::NoWorkspace);
    assert!(
        render_error
            .to_string()
            .contains("file-tree rendering failed")
    );

    let root = TestWorkspace::new()?;
    root.write("a.rs", "a")?;
    let mut app = StudioApp::open_workspace_lazy(TestTextSystem, root.path())?;
    app.file_tree.exhaust_tree_generation();
    let command_shift = Modifiers::from_bits(Modifiers::COMMAND | Modifiers::SHIFT);
    let effect = app.handle_event(&key(KEY_E, command_shift));
    assert!(effect.visual_changed);
    assert_eq!(app.workspace_failures, 1);
    assert!(app.last_workspace_error.is_some());
    Ok(())
}

#[test]
fn file_tree_pointer_activation_failed_output_and_stale_output_are_admitted_correctly()
-> Result<(), Box<dyn std::error::Error>> {
    let root = TestWorkspace::new()?;
    root.write("a.rs", "a")?;
    let mut pointer_app = StudioApp::open_workspace_lazy(TestTextSystem, root.path())?;
    let point = Point::new(8.0, CONTENT_INSET + 1.0).ok_or("tree point")?;
    let activated = pointer_app.handle_pointer(
        PointerAction::Down,
        point,
        PointerButton::Primary,
        Modifiers::default(),
    );
    assert!(activated.visual_changed);
    assert!(pointer_app.file_tree.is_active());
    let missing_row = pointer_app.handle_pointer(
        PointerAction::Down,
        point,
        PointerButton::Primary,
        Modifiers::default(),
    );
    assert!(missing_row.visual_changed);
    assert_eq!(pointer_app.workspace_failures, 1);

    let mut exhausted = StudioApp::open_workspace_lazy(TestTextSystem, root.path())?;
    exhausted.file_tree.exhaust_tree_generation();
    let exhausted_effect = exhausted.handle_pointer(
        PointerAction::Down,
        point,
        PointerButton::Primary,
        Modifiers::default(),
    );
    assert!(exhausted_effect.visual_changed);
    assert_eq!(exhausted.workspace_failures, 1);

    let stale_root = TestWorkspace::new()?;
    stale_root.write("a.rs", "a")?;
    let mut stale = StudioApp::open_workspace_lazy(TestTextSystem, stale_root.path())?;
    assert!(stale.file_tree.activate(1)?);
    let stale_request = stale
        .prepare_file_tree_request()?
        .ok_or("stale root request")?;
    let stale_output = stale_request.execute();
    assert!(stale.file_tree.hide());
    assert_eq!(
        stale.apply_file_tree_output(stale_output),
        EventEffect::default()
    );

    let failed_root = TestWorkspace::new()?;
    failed_root.write("a.rs", "a")?;
    let mut failed = StudioApp::open_workspace_lazy(TestTextSystem, failed_root.path())?;
    assert!(failed.file_tree.activate(1)?);
    let failed_request = failed
        .prepare_file_tree_request()?
        .ok_or("failed root request")?;
    std::fs::remove_dir_all(failed_root.path())?;
    let failed_effect = failed.apply_file_tree_output(failed_request.execute());
    assert!(failed_effect.visual_changed);
    assert_eq!(failed.workspace_failures, 1);
    assert!(failed.last_workspace_error.is_some());
    Ok(())
}

#[test]
fn active_tree_without_workspace_fails_before_worker_submission()
-> Result<(), Box<dyn std::error::Error>> {
    let mut app = test_app()?;
    assert!(app.file_tree.activate(1)?);
    assert!(matches!(
        app.prepare_file_tree_request(),
        Err(FileTreeError::NoWorkspace)
    ));

    let clear = LinearRgba::new(0.02, 0.02, 0.02, 1.0).ok_or(SurfaceError::invariant(
        alpine_platform_macos::SurfaceOperation::Application,
    ))?;
    let mut runtime = Application::new(app, viewport()?, clear, WorkerConfig::default())?;
    assert!(
        runtime
            .dispatch(&SurfaceEvent::Wake {
                timestamp: EventTimestamp::new(2_000),
            })
            .is_some()
    );
    Ok(())
}

#[test]
fn workspace_root_and_eager_directory_selection_reject_non_files()
-> Result<(), Box<dyn std::error::Error>> {
    let root = TestWorkspace::new()?;
    root.write("not-a-directory", "x")?;
    assert!(matches!(
        Workspace::open_root(&root.path().join("not-a-directory")),
        Err(workspace::WorkspaceError::NotDirectory(_))
    ));

    std::fs::create_dir(root.path().join("directory"))?;
    root.write("directory/file.rs", "x")?;
    let workspace = Workspace::open(root.path(), WorkspaceLimits::default())?;
    let snapshot = workspace.snapshot();
    let rejected_directory = (0..snapshot.retained_entries).any(|index| {
        matches!(
            workspace.path_for_file(index),
            Err(workspace::WorkspaceError::NotRegularFile(_))
        )
    });
    assert!(rejected_directory);
    Ok(())
}

fn assert_single_accessibility_focus(snapshot: &AccessibilitySnapshot) {
    assert_eq!(
        snapshot
            .nodes()
            .iter()
            .filter(|node| node.is_focused())
            .count(),
        1
    );
}

fn assert_initial_accessibility_snapshot(
    snapshot: &AccessibilitySnapshot,
) -> Result<(), AccessibilityError> {
    assert_eq!(snapshot.revision().document(), 0);
    assert_eq!(snapshot.revision().buffer(), 0);
    assert_eq!(snapshot.text_len_utf16(), 4);
    assert_eq!(snapshot.line_count(), 1);
    assert!(!snapshot.is_dirty());
    assert_eq!(snapshot.selection().anchor_utf16(), 3);
    assert_eq!(snapshot.selection().head_utf16(), 1);
    let selection_range = snapshot.selection().range();
    assert_eq!(selection_range, AccessibilityTextRange::new(1, 2));
    assert_eq!(selection_range.length_utf16(), 2);
    assert_eq!(snapshot.text(AccessibilityTextRange::new(1, 2))?, "🦀");
    assert!(snapshot.nodes().iter().any(|node| {
        node.role() == AccessibilityRole::CodeEditor
            && node.name() == "Untitled"
            && node.is_focused()
            && node.parent().is_some()
    }));
    assert_single_accessibility_focus(snapshot);
    assert!(!snapshot.nodes()[0].is_focused());
    assert_eq!(
        snapshot
            .nodes()
            .iter()
            .filter(|node| node.role() == AccessibilityRole::Tab && node.is_selected())
            .count(),
        1
    );
    assert_eq!(
        snapshot
            .nodes()
            .iter()
            .filter(|node| node.is_selected())
            .count(),
        1
    );
    assert_eq!(
        snapshot
            .nodes()
            .iter()
            .filter(|node| node.announces())
            .count(),
        0
    );
    let report = snapshot.report();
    assert_eq!(report.node_count(), snapshot.nodes().len());
    let node_bytes = std::mem::size_of::<accessibility::AccessibilityNode>();
    assert!(report.owned_node_bytes() >= report.node_count() * node_bytes);
    assert_eq!(report.owned_node_bytes() % node_bytes, 0);
    assert_eq!(
        report.referenced_name_bytes(),
        snapshot
            .nodes()
            .iter()
            .map(|node| node.name().len())
            .sum::<usize>()
    );
    assert_eq!(report.max_nodes(), accessibility::MAX_ACCESSIBILITY_NODES);
    assert_eq!(
        report.max_text_request_bytes(),
        accessibility::MAX_ACCESSIBILITY_TEXT_REQUEST_BYTES
    );
    Ok(())
}

#[test]
fn accessibility_snapshot_preserves_unicode_revision_focus_and_bounded_text()
-> Result<(), Box<dyn std::error::Error>> {
    let mut app = StudioApp::from_document(TestTextSystem, StudioDocument::scratch("a🦀é"), None)?;
    app.selection = Selection::new(ByteOffset::new(5), ByteOffset::new(1));

    let snapshot = app.accessibility_snapshot()?;
    assert_initial_accessibility_snapshot(&snapshot)?;

    let request = AccessibilityRequest::snapshot(AccessibilityRequestId::new(1))?;
    let (response, effect) = accessibility::respond(&mut app, &request);
    assert_eq!(response.validate_for(&request), Ok(()));
    assert!(!effect.visual_changed);
    assert!(matches!(
        response.result(),
        Ok(AccessibilityPayload::Snapshot(value))
            if value.revision() == snapshot.revision()
                && value.nodes().len() == snapshot.nodes().len()
                && value.text_len_utf16() == snapshot.text_len_utf16()
    ));

    let text_request = AccessibilityRequest::text(
        AccessibilityRequestId::new(2),
        snapshot.revision(),
        AccessibilityTextRange::new(1, 2),
    )?;
    let (text_response, text_effect) = accessibility::respond(&mut app, &text_request);
    assert!(!text_effect.visual_changed);
    assert!(matches!(
        text_response.result(),
        Ok(AccessibilityPayload::Text(text)) if text.as_str() == "🦀"
    ));

    assert!(matches!(
        app.handle_accessibility_action(AccessibilityAction::set_selection(
            snapshot.revision(),
            2,
            2,
        )),
        Err(AccessibilityError::Text(TextError::InvalidUtf16Boundary {
            offset: 2
        }))
    ));
    let effect = app.handle_accessibility_action(AccessibilityAction::set_selection(
        snapshot.revision(),
        4,
        3,
    ))?;
    assert!(effect.visual_changed);
    assert_eq!(app.selection.anchor().get(), 7);
    assert_eq!(app.selection.head().get(), 5);

    assert!(app.open_command_palette().visual_changed);
    let palette = app.accessibility_snapshot()?;
    assert!(palette.nodes().iter().any(|node| {
        node.role() == AccessibilityRole::Dialog
            && node.name() == "Command palette"
            && node.is_focused()
    }));
    assert_single_accessibility_focus(&palette);
    assert!(
        !palette
            .nodes()
            .iter()
            .any(|node| { node.role() == AccessibilityRole::CodeEditor && node.is_focused() })
    );
    app.set_local_status(LocalStatus::Command(Arc::from("Selection changed.")));
    let announced = app.accessibility_snapshot()?;
    assert!(announced.nodes().iter().any(|node| {
        node.role() == AccessibilityRole::Status
            && node.name() == "Selection changed."
            && node.announces()
            && node.id() != snapshot.nodes()[0].id()
    }));
    assert_eq!(
        announced
            .nodes()
            .iter()
            .filter(|node| node.announces())
            .count(),
        1
    );

    app.command_palette.cancel();
    assert!(app.replace_selection("x").document_changed);
    let actual = accessibility::revision(&app);
    assert!(matches!(
        app.handle_accessibility_action(AccessibilityAction::set_selection(
            snapshot.revision(),
            0,
            0,
        )),
        Err(AccessibilityError::StaleRevision { expected, actual: found })
            if expected == snapshot.revision() && found == actual
    ));
    Ok(())
}

#[test]
fn accessibility_text_mapping_dispatches_exact_unicode_results()
-> Result<(), Box<dyn std::error::Error>> {
    let mut app = StudioApp::from_document(TestTextSystem, StudioDocument::scratch("a🦀é"), None)?;
    let revision = app.accessibility_snapshot()?.revision();
    let line_request =
        AccessibilityRequest::line_for_index(AccessibilityRequestId::new(3), revision, 3)?;
    let (line_response, line_effect) = accessibility::respond(&mut app, &line_request);
    assert!(!line_effect.visual_changed);
    assert_eq!(line_response.result(), &Ok(AccessibilityPayload::Line(0)));

    let line_range_request =
        AccessibilityRequest::range_for_line(AccessibilityRequestId::new(4), revision, 0)?;
    let (line_range_response, line_range_effect) =
        accessibility::respond(&mut app, &line_range_request);
    assert!(!line_range_effect.visual_changed);
    assert_eq!(
        line_range_response.result(),
        &Ok(AccessibilityPayload::Range(AccessibilityTextRange::new(
            0, 4
        )))
    );

    let grapheme_request =
        AccessibilityRequest::range_for_index(AccessibilityRequestId::new(5), revision, 1)?;
    let (grapheme_response, grapheme_effect) = accessibility::respond(&mut app, &grapheme_request);
    assert!(!grapheme_effect.visual_changed);
    assert_eq!(
        grapheme_response.result(),
        &Ok(AccessibilityPayload::Range(AccessibilityTextRange::new(
            1, 2
        )))
    );
    Ok(())
}

#[test]
fn accessibility_runtime_dispatch_is_exact_dirty_neutral_and_revision_checked()
-> Result<(), Box<dyn std::error::Error>> {
    assert_eq!(accessibility_admission_failures(4, true), 4);
    assert_eq!(accessibility_admission_failures(4, false), 5);
    assert_eq!(accessibility_admission_failures(u64::MAX, false), u64::MAX);
    let app = StudioApp::from_document(TestTextSystem, StudioDocument::scratch("a🦀é"), None)?;
    let clear = LinearRgba::new(0.02, 0.02, 0.02, 1.0).ok_or("clear color")?;
    let mut runtime = Application::new(app, viewport()?, clear, WorkerConfig::default())?;
    assert!(runtime.frame_if_dirty().is_some());
    assert!(runtime.frame_if_dirty().is_none());

    let snapshot_request = AccessibilityRequest::snapshot(AccessibilityRequestId::new(10))?;
    let snapshot_response = runtime.dispatch_with_response(&SurfaceEvent::Accessibility {
        timestamp: EventTimestamp::new(10),
        request: snapshot_request.clone(),
    });
    assert!(snapshot_response.frame().is_none());
    let snapshot_response = snapshot_response
        .accessibility_response()
        .ok_or("snapshot response")?;
    assert_eq!(snapshot_response.validate_for(&snapshot_request), Ok(()));
    let revision = match snapshot_response.result() {
        Ok(AccessibilityPayload::Snapshot(snapshot)) => snapshot.revision(),
        _ => return Err("snapshot payload".into()),
    };

    let selection_request =
        AccessibilityRequest::selection(AccessibilityRequestId::new(11), revision)?;
    let selection_response = runtime.dispatch_with_response(&SurfaceEvent::Accessibility {
        timestamp: EventTimestamp::new(11),
        request: selection_request,
    });
    assert!(selection_response.frame().is_none());
    assert!(matches!(
        selection_response
            .accessibility_response()
            .ok_or("selection response")?
            .result(),
        Ok(AccessibilityPayload::Selection(selection))
            if selection.anchor_utf16() == 0 && selection.head_utf16() == 0
    ));

    let unchanged_request = AccessibilityRequest::action(
        AccessibilityRequestId::new(12),
        AccessibilityAction::set_selection(revision, 0, 0),
    )?;
    let unchanged_response = runtime.dispatch_with_response(&SurfaceEvent::Accessibility {
        timestamp: EventTimestamp::new(12),
        request: unchanged_request,
    });
    assert!(unchanged_response.frame().is_none());
    assert!(matches!(
        unchanged_response
            .accessibility_response()
            .ok_or("unchanged action response")?
            .result(),
        Ok(AccessibilityPayload::Action(
            alpine_platform_macos::AccessibilityActionResult::Unchanged
        ))
    ));

    let applied_request = AccessibilityRequest::action(
        AccessibilityRequestId::new(13),
        AccessibilityAction::set_selection(revision, 0, 1),
    )?;
    let applied_response = runtime.dispatch_with_response(&SurfaceEvent::Accessibility {
        timestamp: EventTimestamp::new(13),
        request: applied_request,
    });
    assert!(applied_response.frame().is_some());
    assert!(matches!(
        applied_response
            .accessibility_response()
            .ok_or("applied action response")?
            .result(),
        Ok(AccessibilityPayload::Action(
            alpine_platform_macos::AccessibilityActionResult::Applied
        ))
    ));
    Ok(())
}

#[test]
fn accessibility_runtime_rejects_stale_mapping_and_oversized_text()
-> Result<(), Box<dyn std::error::Error>> {
    let app = StudioApp::from_document(TestTextSystem, StudioDocument::scratch("a🦀é"), None)?;
    let revision = accessibility::revision(&app);
    let clear = LinearRgba::new(0.02, 0.02, 0.02, 1.0).ok_or("clear color")?;
    let mut runtime = Application::new(app, viewport()?, clear, WorkerConfig::default())?;
    assert!(runtime.frame_if_dirty().is_some());
    let stale = alpine_platform_macos::AccessibilityRevision::new(
        revision.document(),
        revision.buffer() + 1,
    );
    let stale_request = AccessibilityRequest::text(
        AccessibilityRequestId::new(14),
        stale,
        AccessibilityTextRange::new(0, 0),
    )?;
    let stale_response = runtime.dispatch_with_response(&SurfaceEvent::Accessibility {
        timestamp: EventTimestamp::new(14),
        request: stale_request,
    });
    assert!(stale_response.frame().is_none());
    assert!(matches!(
        stale_response
            .accessibility_response()
            .ok_or("stale response")?
            .result(),
        Err(alpine_platform_macos::AccessibilityError::StaleRevision {
            expected,
            actual,
        }) if *expected == stale && *actual == revision
    ));

    let stale_mapping_request =
        AccessibilityRequest::line_for_index(AccessibilityRequestId::new(17), stale, 0)?;
    let stale_mapping_response = runtime.dispatch_with_response(&SurfaceEvent::Accessibility {
        timestamp: EventTimestamp::new(17),
        request: stale_mapping_request,
    });
    assert!(matches!(
        stale_mapping_response
            .accessibility_response()
            .ok_or("stale mapping response")?
            .result(),
        Err(alpine_platform_macos::AccessibilityError::StaleRevision {
            expected,
            actual,
        }) if *expected == stale && *actual == revision
    ));

    let invalid_mapping_request = AccessibilityRequest::action(
        AccessibilityRequestId::new(15),
        AccessibilityAction::set_selection(revision, 2, 2),
    )?;
    let invalid_mapping_response = runtime.dispatch_with_response(&SurfaceEvent::Accessibility {
        timestamp: EventTimestamp::new(15),
        request: invalid_mapping_request,
    });
    assert!(invalid_mapping_response.frame().is_none());
    assert_eq!(
        invalid_mapping_response
            .accessibility_response()
            .ok_or("mapping response")?
            .result(),
        &Err(alpine_platform_macos::AccessibilityError::TextMappingFailed)
    );

    let mut oversized = StudioApp::from_document(
        TestTextSystem,
        StudioDocument::scratch(&"é".repeat(32_769)),
        None,
    )?;
    let oversized_revision = accessibility::revision(&oversized);
    let oversized_request = AccessibilityRequest::text(
        AccessibilityRequestId::new(16),
        oversized_revision,
        AccessibilityTextRange::new(0, 32_769),
    )?;
    let (oversized_response, effect) = accessibility::respond(&mut oversized, &oversized_request);
    assert!(!effect.visual_changed);
    assert!(matches!(
        oversized_response.result(),
        Err(alpine_platform_macos::AccessibilityError::TextResponseTooLarge {
            actual,
            limit,
        }) if *actual == 65_538
            && *limit == accessibility::MAX_ACCESSIBILITY_TEXT_REQUEST_BYTES
    ));
    Ok(())
}

fn assert_completion_shutdown_and_idle_are_bounded(
    path: &std::path::Path,
    diagnostics: &serde_json::value::RawValue,
    completion: &serde_json::value::RawValue,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut app = StudioApp::open_file(TestTextSystem, path)?;
    let input = app.active_rust_document().ok_or("Rust document")?;
    app.rust_diagnostics.install_for_test(
        input,
        diagnostics,
        rust_diagnostics::tests::mock_executable(),
    )?;
    app.rust_diagnostics
        .install_completion_for_test(5, app.language_identity(), completion)?;
    assert!(
        app.rust_diagnostics
            .completion_is_open(app.language_identity())
    );
    let shutdown = app.rust_diagnostics.shutdown();
    assert!(!shutdown.active);
    assert!(!shutdown.completion_pending);
    assert_eq!(shutdown.completion_items, 0);
    assert_eq!(shutdown.completion_bytes, 0);

    let mut idle_app = StudioApp::open_file(TestTextSystem, path)?;
    let idle_input = idle_app.active_rust_document().ok_or("Rust document")?;
    idle_app.rust_diagnostics.install_for_test(
        idle_input,
        diagnostics,
        rust_diagnostics::tests::mock_executable(),
    )?;
    idle_app.rust_diagnostics.install_completion_for_test(
        6,
        idle_app.language_identity(),
        completion,
    )?;
    let clear = LinearRgba::new(0.02, 0.02, 0.02, 1.0).ok_or("clear color")?;
    let mut runtime = Application::new(idle_app, viewport()?, clear, WorkerConfig::default())?;
    assert!(runtime.frame_if_dirty().is_some());
    assert!(runtime.frame_if_dirty().is_none());
    Ok(())
}

fn assert_completion_trigger_guards(app: &mut StudioApp) {
    app.composition = Some(Composition {
        replacement: app.selection.range(),
        text: Box::default(),
        selected_start_utf16: 0,
        selected_length_utf16: 0,
    });
    assert_eq!(app.trigger_rust_completion(), EventEffect::default());
    app.composition = None;

    assert!(
        app.dispatch_command(StudioCommand::TriggerCompletion)
            .visual_changed
    );
    assert!(!app.rust_diagnostics.snapshot().completion_pending);
    assert!(!app.rust_diagnostics.cancel_completion());

    let original_selection = app.selection;
    app.selection = Selection::caret(ByteOffset::new(app.buffer().snapshot().len_bytes() + 1));
    let failures = app.input_failures;
    assert_eq!(app.trigger_rust_completion(), EventEffect::default());
    assert_eq!(app.input_failures, failures + 1);
    app.selection = original_selection;
    assert_eq!(app.apply_selected_completion(), EventEffect::default());
    assert_eq!(app.handle_completion_key(0, false), None);
}

fn assert_completion_application_failures(
    app: &mut StudioApp,
    completion: &serde_json::value::RawValue,
) -> Result<(), Box<dyn std::error::Error>> {
    let invalid_range = serde_json::value::RawValue::from_string(
        r#"[{"label":"outside","textEdit":{"range":{"start":{"line":99,"character":0},"end":{"line":99,"character":1}},"newText":"outside"}}]"#.to_owned(),
    )?;
    app.runtime_document_revision -= 1;
    app.rust_diagnostics.install_completion_for_test(
        31,
        app.language_identity(),
        &invalid_range,
    )?;
    let failures = app.input_failures;
    assert!(app.apply_selected_completion().visual_changed);
    assert_eq!(app.input_failures, failures + 1);

    app.rust_diagnostics
        .install_completion_for_test(32, app.language_identity(), completion)?;
    assert!(
        app.handle_completion_key(KEY_ESCAPE, false)
            .is_some_and(|effect| effect.visual_changed)
    );
    Ok(())
}

#[test]
fn completion_scene_keyboard_focus_accessibility_and_atomic_edit_are_exact()
-> Result<(), Box<dyn std::error::Error>> {
    let root = TestWorkspace::new()?;
    root.write("main.rs", "fn broken( {\n")?;
    let path = root.path().join("main.rs");
    let mut app = StudioApp::open_file(TestTextSystem, &path)?;
    let input = app.active_rust_document().ok_or("Rust document")?;
    let diagnostics = rust_diagnostics::tests::diagnostics(&path, 1);
    app.rust_diagnostics.install_for_test(
        input,
        &diagnostics,
        rust_diagnostics::tests::mock_executable(),
    )?;
    assert_completion_trigger_guards(&mut app);

    let baseline = app.try_scene(SceneRevision::new(310), viewport()?)?;
    let completion = serde_json::value::RawValue::from_string(
        r#"[{"label":"println!","textEdit":{"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":2}},"newText":"println!"}},{"label":"print!","insertText":"print!"}]"#.to_owned(),
    )?;
    app.rust_diagnostics
        .install_completion_for_test(2, app.language_identity(), &completion)?;
    assert_eq!(
        app.handle_completion_key(KEY_UP, false),
        Some(EventEffect::default())
    );
    assert_eq!(app.handle_completion_key(KEY_DOWN, true), None);

    let scene = app.try_scene(SceneRevision::new(311), viewport()?)?;
    assert_eq!(scene.clips().len(), baseline.clips().len() + 1);
    assert_eq!(scene.quads().len(), baseline.quads().len() + 2);
    assert!(scene.glyphs().len() > baseline.glyphs().len());
    let first = app.accessibility_snapshot()?;
    assert_single_accessibility_focus(&first);
    assert!(first.nodes().iter().any(|node| {
        node.role() == AccessibilityRole::Dialog
            && node.name() == "Code completion: println!"
            && node.is_focused()
            && node.announces()
    }));

    assert!(
        app.handle_event(&key(KEY_DOWN, Modifiers::default()))
            .visual_changed
    );
    let second = app.accessibility_snapshot()?;
    assert!(
        second
            .nodes()
            .iter()
            .any(|node| { node.name() == "Code completion: print!" && node.is_focused() })
    );
    let original = app.buffer().snapshot().text();
    let applied = app.handle_event(&key(KEY_RETURN, Modifiers::default()));
    assert!(applied.document_changed);
    assert!(app.buffer().snapshot().text().starts_with("print!"));
    assert!(
        !app.rust_diagnostics
            .completion_is_open(app.language_identity())
    );
    let command = Modifiers::from_bits(Modifiers::COMMAND);
    assert!(app.handle_event(&key(KEY_Z, command)).document_changed);
    assert_eq!(app.buffer().snapshot().text(), original);

    app.rust_diagnostics
        .install_completion_for_test(3, app.language_identity(), &completion)?;
    app.runtime_document_revision += 1;
    let stale = app.accessibility_snapshot()?;
    assert!(
        !stale
            .nodes()
            .iter()
            .any(|node| { node.name().starts_with("Code completion:") })
    );
    assert!(
        stale
            .nodes()
            .iter()
            .any(|node| { node.role() == AccessibilityRole::CodeEditor && node.is_focused() })
    );

    assert_completion_application_failures(&mut app, &completion)?;

    app.rust_diagnostics
        .install_completion_for_test(4, app.language_identity(), &completion)?;
    assert!(
        app.handle_event(&SurfaceEvent::Focus {
            focused: false,
            timestamp: EventTimestamp::new(312),
            input_epoch: InputEpoch::INITIAL
                .checked_next()
                .ok_or("next input epoch")?,
        })
        .visual_changed
    );
    assert!(
        !app.rust_diagnostics
            .completion_is_open(app.language_identity())
    );
    assert_completion_shutdown_and_idle_are_bounded(&path, &diagnostics, &completion)?;
    Ok(())
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "the exact anchored, clamped, narrow, and fallback geometry controls stay in one scene journey"
)]
fn completion_overlay_geometry_is_exact_at_anchor_clamp_and_narrow_edges()
-> Result<(), Box<dyn std::error::Error>> {
    let root = TestWorkspace::new()?;
    root.write("main.rs", "zero\none\ntwo\n")?;
    let path = root.path().join("main.rs");
    let mut app = StudioApp::open_file(TestTextSystem, &path)?;
    let input = app.active_rust_document().ok_or("Rust document")?;
    app.rust_diagnostics.install_for_test(
        input,
        &rust_diagnostics::tests::diagnostics(&path, 1),
        rust_diagnostics::tests::mock_executable(),
    )?;
    app.selection = Selection::caret(ByteOffset::new(5));

    let three = serde_json::value::RawValue::from_string(
        r#"[{"label":"alpha"},{"label":"beta"},{"label":"gamma"}]"#.to_owned(),
    )?;
    app.rust_diagnostics
        .install_completion_for_test(40, app.language_identity(), &three)?;
    let wide = app.try_scene(SceneRevision::new(320), viewport()?)?;
    let completion_clip = wide.clips().len().checked_sub(1).ok_or("completion clip")?;
    let expected_wide = Rect::new(
        Point::new(48.0, 68.0).ok_or("wide completion origin")?,
        Size::new(420.0, 66.0).ok_or("wide completion size")?,
    );
    assert_eq!(wide.clips()[completion_clip].bounds(), expected_wide);
    assert!(wide.quads().iter().any(|quad| {
        quad.clip()
            .is_some_and(|clip| clip.index() == completion_clip)
            && quad.bounds()
                == Rect::new(
                    Point::new(48.0, 68.0).unwrap_or_else(|| unreachable!()),
                    Size::new(420.0, LINE_HEIGHT).unwrap_or_else(|| unreachable!()),
                )
    }));
    let completion_glyphs = wide
        .glyphs()
        .iter()
        .filter(|glyph| {
            glyph
                .clip()
                .is_some_and(|clip| clip.index() == completion_clip)
        })
        .collect::<Vec<_>>();
    assert!(!completion_glyphs.is_empty());
    assert_eq!(
        completion_glyphs[0].bounds().origin().x().to_bits(),
        56.0_f32.to_bits()
    );
    assert_eq!(
        completion_glyphs[0].bounds().origin().y().to_bits(),
        83.0_f32.to_bits()
    );
    let mut row_origins = completion_glyphs
        .iter()
        .map(|glyph| glyph.bounds().origin().y())
        .collect::<Vec<_>>();
    row_origins.sort_by(f32::total_cmp);
    row_origins.dedup_by(|left, right| left.to_bits() == right.to_bits());
    assert_eq!(row_origins, [83.0, 105.0, 127.0]);

    let many = (0..10)
        .map(|index| format!(r#"{{"label":"item-{index}"}}"#))
        .collect::<Vec<_>>()
        .join(",");
    let many = serde_json::value::RawValue::from_string(format!("[{many}]"))?;
    app.rust_diagnostics
        .install_completion_for_test(41, app.language_identity(), &many)?;
    let constrained_viewport = Size::new(260.0, 200.0).ok_or("constrained viewport")?;
    let constrained = app.try_scene(SceneRevision::new(321), constrained_viewport)?;
    assert_eq!(
        constrained
            .clips()
            .last()
            .ok_or("constrained clip")?
            .bounds(),
        Rect::new(
            Point::new(48.0, 24.0).ok_or("constrained origin")?,
            Size::new(164.0, 176.0).ok_or("constrained size")?,
        )
    );

    let narrow_viewport = Size::new(60.0, 200.0).ok_or("narrow viewport")?;
    let narrow = app.try_scene(SceneRevision::new(322), narrow_viewport)?;
    assert_eq!(
        narrow.clips().last().ok_or("narrow clip")?.bounds(),
        Rect::new(
            Point::new(35.0, 24.0).ok_or("narrow origin")?,
            Size::new(1.0, 176.0).ok_or("narrow size")?,
        )
    );

    root.write("deep.rs", "line\n".repeat(100))?;
    let deep_path = root.path().join("deep.rs");
    let mut fallback_app = StudioApp::open_file(TestTextSystem, &deep_path)?;
    let fallback_input = fallback_app
        .active_rust_document()
        .ok_or("deep Rust document")?;
    fallback_app.rust_diagnostics.install_for_test(
        fallback_input,
        &rust_diagnostics::tests::diagnostics(&deep_path, 1),
        rust_diagnostics::tests::mock_executable(),
    )?;
    fallback_app.selection = Selection::caret(ByteOffset::new(400));
    fallback_app.rust_diagnostics.install_completion_for_test(
        42,
        fallback_app.language_identity(),
        &many,
    )?;
    let fallback = fallback_app.try_scene(SceneRevision::new(323), viewport()?)?;
    assert_eq!(
        fallback.clips().last().ok_or("fallback clip")?.bounds(),
        Rect::new(
            Point::new(48.0, 48.0).ok_or("fallback origin")?,
            Size::new(420.0, 176.0).ok_or("fallback size")?,
        )
    );
    Ok(())
}

#[test]
fn completion_command_modifiers_and_context_axes_are_independent()
-> Result<(), Box<dyn std::error::Error>> {
    let root = TestWorkspace::new()?;
    root.write("main.rs", "fn main() {}\n")?;
    let path = root.path().join("main.rs");
    let mut app = StudioApp::open_file(TestTextSystem, &path)?;
    let input = app.active_rust_document().ok_or("Rust document")?;
    app.rust_diagnostics.install_for_test(
        input,
        &rust_diagnostics::tests::diagnostics(&path, 1),
        rust_diagnostics::tests::mock_executable(),
    )?;
    let completion = serde_json::value::RawValue::from_string(
        r#"[{"label":"first"},{"label":"second"}]"#.to_owned(),
    )?;
    app.rust_diagnostics
        .install_completion_for_test(42, app.language_identity(), &completion)?;
    for physical_key in [KEY_UP, KEY_DOWN, KEY_RETURN, KEY_TAB] {
        assert_eq!(app.handle_completion_key(physical_key, true), None);
        assert!(
            app.rust_diagnostics
                .completion_is_open(app.language_identity())
        );
    }

    assert!(app.command_context().can_complete);
    app.composition = Some(Composition {
        replacement: app.selection.range(),
        text: Box::default(),
        selected_start_utf16: 0,
        selected_length_utf16: 0,
    });
    assert!(!app.command_context().can_complete);

    let plain = StudioDocument::scratch("plain text");
    let plain = StudioApp::from_document(TestTextSystem, plain, Some(Path::new("notes.txt")))?;
    assert!(!plain.command_context().can_complete);
    Ok(())
}

#[test]
fn accessibility_non_identity_state_and_every_focus_owner_are_exact()
-> Result<(), Box<dyn std::error::Error>> {
    let mut changed =
        StudioApp::from_document(TestTextSystem, StudioDocument::scratch("a\nb"), None)?;
    changed.runtime_document_revision = 7;
    assert!(changed.replace_selection("z").document_changed);
    let changed_snapshot = changed.accessibility_snapshot()?;
    assert_eq!(changed_snapshot.revision().document(), 7);
    assert_eq!(changed_snapshot.revision().buffer(), 1);
    assert_eq!(changed_snapshot.line_count(), 2);
    assert!(changed_snapshot.is_dirty());

    let mut unfocused =
        StudioApp::from_document(TestTextSystem, StudioDocument::scratch("x"), None)?;
    unfocused.focused = false;
    assert_eq!(
        unfocused
            .accessibility_snapshot()?
            .nodes()
            .iter()
            .filter(|node| node.is_focused())
            .count(),
        0
    );

    let mut find = StudioApp::from_document(TestTextSystem, StudioDocument::scratch("x"), None)?;
    assert!(find.find.open(false));
    let find_snapshot = find.accessibility_snapshot()?;
    assert_single_accessibility_focus(&find_snapshot);
    assert!(
        find_snapshot
            .nodes()
            .iter()
            .any(|node| { node.name() == "Find in document" && node.is_focused() })
    );

    let mut quick = StudioApp::from_document(TestTextSystem, StudioDocument::scratch("x"), None)?;
    assert!(quick.quick_open.open(1)?);
    let quick_snapshot = quick.accessibility_snapshot()?;
    assert_single_accessibility_focus(&quick_snapshot);
    assert!(
        quick_snapshot
            .nodes()
            .iter()
            .any(|node| node.name() == "Quick open" && node.is_focused())
    );

    let mut project = StudioApp::from_document(TestTextSystem, StudioDocument::scratch("x"), None)?;
    assert!(project.project_search.open(1)?);
    let project_snapshot = project.accessibility_snapshot()?;
    assert_single_accessibility_focus(&project_snapshot);
    assert!(
        project_snapshot
            .nodes()
            .iter()
            .any(|node| node.name() == "Project search" && node.is_focused())
    );

    let mut tree = StudioApp::from_document(TestTextSystem, StudioDocument::scratch("x"), None)?;
    assert!(tree.file_tree.activate(1)?);
    let tree_snapshot = tree.accessibility_snapshot()?;
    assert_single_accessibility_focus(&tree_snapshot);
    assert!(
        tree_snapshot
            .nodes()
            .iter()
            .any(|node| node.name() == "Files" && node.is_focused())
    );
    Ok(())
}

#[test]
fn accessibility_text_requests_fail_before_unbounded_materialization()
-> Result<(), Box<dyn std::error::Error>> {
    let text = "x".repeat(accessibility::MAX_ACCESSIBILITY_TEXT_REQUEST_BYTES + 1);
    let app = StudioApp::from_document(TestTextSystem, StudioDocument::scratch(&text), None)?;
    let snapshot = app.accessibility_snapshot()?;
    assert_eq!(
        snapshot
            .text(AccessibilityTextRange::new(
                0,
                accessibility::MAX_ACCESSIBILITY_TEXT_REQUEST_BYTES,
            ))?
            .len(),
        accessibility::MAX_ACCESSIBILITY_TEXT_REQUEST_BYTES
    );
    assert!(matches!(
        snapshot.text(AccessibilityTextRange::new(0, text.len())),
        Err(AccessibilityError::TextRequestTooLarge { actual, limit })
            if actual == text.len()
                && limit == accessibility::MAX_ACCESSIBILITY_TEXT_REQUEST_BYTES
    ));
    assert!(matches!(
        snapshot.text(AccessibilityTextRange::new(usize::MAX, 1)),
        Err(AccessibilityError::ArithmeticOverflow)
    ));
    Ok(())
}

#[test]
fn folder_launch_primes_lazy_tree_without_stealing_editor_focus()
-> Result<(), Box<dyn std::error::Error>> {
    let root = TestWorkspace::new()?;
    root.write("main.rs", "fn main() {}\n")?;
    let workspace = Workspace::open_root(root.path())?;
    let mut app = StudioApp::from_workspace(TestTextSystem, workspace)?;
    app.prime_workspace_launch()?;

    assert!(app.file_tree.is_visible());
    assert!(app.file_tree.is_active());
    assert!(!app.file_tree.is_focused());
    let request = app
        .prepare_file_tree_request()?
        .ok_or("primed root request")?;
    assert!(app.prepare_file_tree_request()?.is_none());
    assert!(app.apply_file_tree_output(request.execute()).visual_changed);
    let rows = app.file_tree.visible_rows(0, 8, TREE_OVERSCAN_ROWS)?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].path.as_ref(), "main.rs");
    assert!(!app.file_tree.is_focused());
    Ok(())
}

fn assert_event_continuation_is_queued(
    mut app: StudioApp,
    wake: LanguageWake,
    viewport: Size,
    clear: LinearRgba,
) -> Result<(), Box<dyn std::error::Error>> {
    app.rust_diagnostics.force_continuation_once_for_test();
    app.language_wake_latch.publish(wake);
    let mut runtime = Application::new(app, viewport, clear, WorkerConfig::default())?;
    let before = runtime.snapshot().external();
    let _ = runtime.dispatch(&SurfaceEvent::Wake {
        timestamp: EventTimestamp::new(3_515),
    });
    let published = runtime.snapshot().external();
    assert_eq!(published.admitted(), before.admitted() + 1);
    assert_eq!(published.current_items(), 1);
    let _ = runtime.dispatch(&SurfaceEvent::Wake {
        timestamp: EventTimestamp::new(3_516),
    });
    assert_eq!(runtime.snapshot().external().current_items(), 0);
    Ok(())
}

fn assert_worker_continuation_is_drained(
    rust_path: &Path,
    viewport: Size,
    clear: LinearRgba,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut app = StudioApp::open_file(TestTextSystem, rust_path)?;
    let input = app.active_rust_document().ok_or("worker Rust document")?;
    let params = rust_diagnostics::tests::diagnostics(rust_path, 1);
    app.rust_diagnostics.install_for_test(
        input,
        &params,
        rust_diagnostics::tests::mock_executable(),
    )?;
    let wake = app
        .rust_diagnostics
        .current_wake_for_test()
        .ok_or("worker language wake")?;
    let latch = app.language_wake_latch.clone();
    app.rust_diagnostics.force_continuation_once_for_test();
    let mut runtime = Application::new(app, viewport, clear, WorkerConfig::default())?;
    let command = Modifiers::from_bits(Modifiers::COMMAND);
    runtime
        .dispatch(&key(KEY_F, command))
        .ok_or("worker find frame")?;
    runtime
        .dispatch(&ime(ImeEvent::Committed("fn".into())))
        .ok_or("worker query frame")?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    while runtime.snapshot().worker().queued_results() == 0 {
        if std::time::Instant::now() >= deadline {
            return Err("find worker did not publish a result".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    latch.publish(wake);
    let before = runtime.snapshot().external();
    let _ = runtime.dispatch(&SurfaceEvent::Wake {
        timestamp: EventTimestamp::new(3_517),
    });
    let after = runtime.snapshot().external();
    assert_eq!(after.admitted(), before.admitted() + 1);
    assert_eq!(after.drained(), before.drained() + 1);
    assert_eq!(after.current_items(), 0);
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "Miri cannot emulate child-process creation")]
fn runtime_rust_diagnostics_reach_the_rendered_scene_without_idle_work()
-> Result<(), Box<dyn std::error::Error>> {
    let root = TestWorkspace::new()?;
    root.write("main.rs", "fn broken( {\n")?;
    root.write("notes.txt", "plain text\n")?;
    let rust_path = root.path().join("main.rs");
    let text_path = root.path().join("notes.txt");
    let text_app = StudioApp::open_file(TestTextSystem, &text_path)?;
    assert!(text_app.active_rust_document().is_none());

    let mut app = StudioApp::open_file(TestTextSystem, &rust_path)?;
    assert!(app.active_rust_document().is_some());
    app.rust_diagnostics = RustDiagnostics::with_server(rust_diagnostics::tests::mock_executable());
    app.rust_diagnostics.force_continuation_once_for_test();
    let viewport = viewport()?;
    let clear = LinearRgba::new(0.02, 0.02, 0.02, 1.0).ok_or(SurfaceError::invariant(
        alpine_platform_macos::SurfaceOperation::Application,
    ))?;
    let mut runtime = Application::new(app, viewport, clear, WorkerConfig::default())?;
    let baseline_quads = runtime
        .frame_if_dirty()
        .ok_or("initial rust frame")?
        .scene()
        .quads()
        .len();

    let _ = runtime.dispatch(&SurfaceEvent::Wake {
        timestamp: EventTimestamp::new(3_000),
    });
    let mut rendered = false;
    for timestamp in 3_001..3_513 {
        std::thread::sleep(std::time::Duration::from_millis(1));
        if let Some(frame) = runtime.dispatch(&SurfaceEvent::Wake {
            timestamp: EventTimestamp::new(timestamp),
        }) && frame.scene().quads().len() >= baseline_quads + 2
        {
            rendered = true;
            break;
        }
    }
    assert!(rendered);
    assert!(
        runtime
            .dispatch(&SurfaceEvent::Wake {
                timestamp: EventTimestamp::new(3_514),
            })
            .is_none()
    );

    let mut latched_app = StudioApp::open_file(TestTextSystem, &rust_path)?;
    let input = latched_app
        .active_rust_document()
        .ok_or("latched Rust document")?;
    let params = rust_diagnostics::tests::diagnostics(&rust_path, 1);
    latched_app.rust_diagnostics.install_for_test(
        input,
        &params,
        rust_diagnostics::tests::mock_executable(),
    )?;
    let wake = latched_app
        .rust_diagnostics
        .current_wake_for_test()
        .ok_or("latched language wake")?;
    let stale_wake = wake.successor_for_test();
    latched_app.language_wake_latch.publish(stale_wake);
    assert_eq!(
        latched_app.poll_latched_language_wake(),
        LanguageEffect::default()
    );
    assert_eq!(latched_app.rust_diagnostics.snapshot().stale_wakes, 1);
    assert_event_continuation_is_queued(latched_app, wake, viewport, clear)?;
    assert_worker_continuation_is_drained(&rust_path, viewport, clear)?;
    assert!(!should_poll_latched_after_worker(true));
    assert!(should_poll_latched_after_worker(false));
    Ok(())
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one end-to-end semantic journey preserves action, frame, and no-mutation controls"
)]
fn accessibility_activation_routes_current_commands_and_rejects_stale_targets()
-> Result<(), Box<dyn Error>> {
    let root = TestWorkspace::new()?;
    root.write(
        "alpha.rs",
        "fn alpha() {}\nsecond line\nthird line\nfourth line\n",
    )?;
    root.write("beta.rs", "beta")?;
    let mut app = StudioApp::open_workspace(TestTextSystem, root.path())?;
    let workspace = app.workspace.as_ref().ok_or("workspace")?;
    let alpha = workspace.index_named("alpha.rs").ok_or("alpha")?;
    let beta = workspace.index_named("beta.rs").ok_or("beta")?;
    app.open_workspace_entry(alpha)?;
    app.open_workspace_entry(beta)?;
    let command_shift = Modifiers::from_bits(Modifiers::COMMAND | Modifiers::SHIFT);
    assert!(app.handle_event(&key(KEY_E, command_shift)).visual_changed);
    if let Some(tree_request) = app.prepare_file_tree_request()? {
        assert!(
            app.apply_file_tree_output(tree_request.execute())
                .visual_changed
        );
    }
    let tree_snapshot = app.accessibility_snapshot()?;
    let alpha_row = tree_snapshot
        .nodes()
        .iter()
        .find(|node| node.role() == AccessibilityRole::ListItem && node.name() == "alpha.rs")
        .ok_or("accessible alpha file row")?;
    let tree_effect = accessibility::apply_action(
        &mut app,
        AccessibilityAction::activate(tree_snapshot.revision(), alpha_row.id()),
    )?;
    assert!(tree_effect.document_changed);
    let active_path = app
        .tabs
        .path_at(app.tabs.active_index())
        .ok_or("active accessible file path")?;
    assert_eq!(
        active_path.file_name().and_then(|name| name.to_str()),
        Some("alpha.rs")
    );
    assert_eq!(
        app.buffer().snapshot().text(),
        "fn alpha() {}\nsecond line\nthird line\nfourth line\n"
    );

    let input = app.active_rust_document().ok_or("active Rust document")?;
    let alpha_path = fs::canonicalize(root.path().join("alpha.rs"))?;
    let base_diagnostics = rust_diagnostics::tests::diagnostics(&alpha_path, 1);
    let mut diagnostic_params: serde_json::Value = serde_json::from_str(base_diagnostics.get())?;
    let mut template = diagnostic_params["diagnostics"][0].clone();
    template["range"]["start"]["character"] = serde_json::Value::from(0);
    template["range"]["end"]["character"] = serde_json::Value::from(1);
    let mut diagnostic_values = Vec::with_capacity(MAX_VISIBLE_DIAGNOSTIC_MARKERS);
    for index in 0..MAX_VISIBLE_DIAGNOSTIC_MARKERS {
        let mut diagnostic = template.clone();
        if index >= MAX_VISIBLE_DIAGNOSTIC_MARKERS - 2 {
            let ordinal = index - (MAX_VISIBLE_DIAGNOSTIC_MARKERS - 2);
            diagnostic["range"]["start"]["line"] = serde_json::Value::from(2);
            diagnostic["range"]["end"]["line"] = serde_json::Value::from(2);
            diagnostic["range"]["start"]["character"] = serde_json::Value::from(ordinal);
            diagnostic["range"]["end"]["character"] = serde_json::Value::from(ordinal + 1);
        }
        if index % 2 == 0 {
            diagnostic
                .as_object_mut()
                .ok_or("diagnostic object")?
                .remove("severity");
        }
        diagnostic_values.push(diagnostic);
    }
    diagnostic_params["diagnostics"] = serde_json::Value::Array(diagnostic_values);
    let diagnostics =
        serde_json::value::RawValue::from_string(serde_json::to_string(&diagnostic_params)?)?;
    app.rust_diagnostics.install_for_test(
        input,
        &diagnostics,
        rust_diagnostics::tests::mock_executable(),
    )?;
    let _diagnostic_scene = app.try_scene(SceneRevision::new(700), viewport()?)?;
    let diagnostic_snapshot = app.accessibility_snapshot()?;
    let diagnostic = diagnostic_snapshot
        .nodes()
        .iter()
        .filter(|node| node.name().contains("diagnostic") && node.supports_activate())
        .max_by_key(|node| node.id().get())
        .ok_or("accessible diagnostic")?;
    let diagnostic_effect = accessibility::apply_action(
        &mut app,
        AccessibilityAction::activate(diagnostic_snapshot.revision(), diagnostic.id()),
    )?;
    assert!(diagnostic_effect.visual_changed);
    let line = app.buffer().snapshot().line_byte_range(2)?;
    assert_eq!(app.selection.anchor().get(), line.start + 1);
    assert_eq!(app.selection.head().get(), line.start + 2);

    let stale_revision = accessibility::revision(&app);
    assert!(app.replace_selection("dirty").document_changed);
    let context = app.command_context();
    assert!(app.command_palette.open(context)?);
    let snapshot = app.accessibility_snapshot()?;
    assert_eq!(
        snapshot
            .nodes()
            .iter()
            .filter(|node| node.is_focused())
            .count(),
        1
    );
    assert!(snapshot.nodes().iter().all(|node| {
        let bounds = node.bounds();
        [bounds.x(), bounds.y(), bounds.width(), bounds.height()]
            .into_iter()
            .all(f32::is_finite)
    }));
    let tab = snapshot
        .nodes()
        .iter()
        .find(|node| node.role() == AccessibilityRole::Tab && node.is_selected())
        .ok_or("tab node")?;
    assert!(tab.supports_activate());
    let tab_effect = accessibility::apply_action(
        &mut app,
        AccessibilityAction::activate(snapshot.revision(), tab.id()),
    )?;
    assert!(!tab_effect.visual_changed);
    let editor = snapshot
        .nodes()
        .iter()
        .find(|node| node.role() == AccessibilityRole::CodeEditor)
        .ok_or("editor node")?;
    let editor_error = accessibility::apply_action(
        &mut app,
        AccessibilityAction::activate(snapshot.revision(), editor.id()),
    );
    assert!(matches!(
        editor_error,
        Err(accessibility::AccessibilityError::Transport(
            alpine_platform_macos::AccessibilityError::ActionDisabled(id)
        )) if id == editor.id()
    ));
    let close = snapshot
        .nodes()
        .iter()
        .find(|node| node.name() == "File: Close Tab")
        .ok_or("close command node")?;
    assert!(close.supports_activate() && close.is_enabled());
    let revision = snapshot.revision();
    let close_id = close.id();
    let before = app.buffer().snapshot().text();
    let before_utf16 = before.encode_utf16().count();
    let clear = LinearRgba::new(0.02, 0.02, 0.02, 1.0).ok_or("clear color")?;
    let mut runtime = Application::new(app, viewport()?, clear, WorkerConfig::default())?;
    assert!(runtime.frame_if_dirty().is_some());
    assert!(runtime.frame_if_dirty().is_none());

    let close_request = AccessibilityRequest::action(
        AccessibilityRequestId::new(80),
        AccessibilityAction::activate(revision, close_id),
    )?;
    let close_response = runtime.dispatch_with_response(&SurfaceEvent::Accessibility {
        timestamp: EventTimestamp::new(80),
        request: close_request,
    });
    assert!(close_response.frame().is_some());
    assert!(runtime.frame_if_dirty().is_none());

    let snapshot_request = AccessibilityRequest::snapshot(AccessibilityRequestId::new(83))?;
    let current_response = runtime.dispatch_with_response(&SurfaceEvent::Accessibility {
        timestamp: EventTimestamp::new(83),
        request: snapshot_request,
    });
    assert!(current_response.frame().is_none());
    let current = match current_response
        .accessibility_response()
        .ok_or("current snapshot response")?
        .result()
    {
        Ok(AccessibilityPayload::Snapshot(snapshot)) => {
            assert!(snapshot.is_dirty());
            assert_eq!(snapshot.text_len_utf16(), before_utf16);
            snapshot.revision()
        }
        _ => return Err("current snapshot payload".into()),
    };

    let stale_request = AccessibilityRequest::action(
        AccessibilityRequestId::new(81),
        AccessibilityAction::activate(stale_revision, close_id),
    )?;
    let stale_response = runtime.dispatch_with_response(&SurfaceEvent::Accessibility {
        timestamp: EventTimestamp::new(81),
        request: stale_request,
    });
    assert!(stale_response.frame().is_none());
    assert!(matches!(
        stale_response
            .accessibility_response()
            .ok_or("stale action response")?
            .result(),
        Err(alpine_platform_macos::AccessibilityError::StaleRevision { .. })
    ));

    let missing_request = AccessibilityRequest::action(
        AccessibilityRequestId::new(82),
        AccessibilityAction::activate(
            current,
            alpine_platform_macos::AccessibilityNodeId::new(u64::MAX),
        ),
    )?;
    let missing_response = runtime.dispatch_with_response(&SurfaceEvent::Accessibility {
        timestamp: EventTimestamp::new(82),
        request: missing_request,
    });
    assert!(missing_response.frame().is_none());
    assert!(matches!(
        missing_response
            .accessibility_response()
            .ok_or("missing action response")?
            .result(),
        Err(alpine_platform_macos::AccessibilityError::ActionTargetMissing(id))
            if *id == alpine_platform_macos::AccessibilityNodeId::new(u64::MAX)
    ));
    Ok(())
}

#[test]
fn selection_revision_advances_only_for_real_selection_changes() -> Result<(), Box<dyn Error>> {
    let mut app = test_app()?;
    let original = app.selection;
    let revision = app.selection_revision;
    let failures = app.input_failures;
    app.advance_selection_revision(original);
    assert_eq!(app.selection_revision, revision);
    assert_eq!(app.input_failures, failures);

    app.selection = Selection::caret(ByteOffset::new(1));
    app.advance_selection_revision(original);
    assert_eq!(app.selection_revision, revision + 1);
    assert_eq!(app.input_failures, failures);

    let previous = app.selection;
    app.selection = Selection::caret(ByteOffset::new(2));
    app.selection_revision = u64::MAX;
    app.advance_selection_revision(previous);
    assert_eq!(app.selection_revision, u64::MAX);
    assert_eq!(app.input_failures, failures + 1);
    Ok(())
}
