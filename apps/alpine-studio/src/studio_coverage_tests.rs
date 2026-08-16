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
struct TestTextSystem;

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
    Size::new(WINDOW_WIDTH, WINDOW_HEIGHT).ok_or(SurfaceError::DriverUnavailable)
}

fn ime(event: ImeEvent) -> SurfaceEvent {
    SurfaceEvent::Ime {
        timestamp: EventTimestamp::new(1),
        event,
    }
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
    assert_eq!(scene.clips().len(), 1);
    assert!(!scene.glyphs().is_empty());
    assert!(scene.glyph_atlas().is_some());
    assert!(scene.quads().len() >= 3);
    assert_eq!(app.render_failures, 0);
    Ok(())
}

#[test]
fn runtime_builds_only_after_an_accepted_editor_change() -> Result<(), RuntimeError> {
    let viewport = viewport()?;
    let clear = LinearRgba::new(0.02, 0.02, 0.02, 1.0).ok_or(SurfaceError::DriverUnavailable)?;
    let mut application = Application::new(test_app()?, viewport, clear, WorkerConfig::default())?;
    let first = application
        .frame_if_dirty()
        .ok_or(SurfaceError::DriverUnavailable)?;
    assert!(
        application
            .dispatch(&SurfaceEvent::Wake {
                timestamp: alpine_platform_macos::EventTimestamp::new(1),
            })
            .is_none()
    );
    let changed = application
        .dispatch(&ime(ImeEvent::Committed("x".into())))
        .ok_or(SurfaceError::DriverUnavailable)?;
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
    Ok(())
}

#[test]
fn studio_runtime_returns_clipboard_and_dirty_close_responses()
-> Result<(), Box<dyn std::error::Error>> {
    let viewport = viewport()?;
    let clear = LinearRgba::new(0.02, 0.02, 0.02, 1.0).ok_or(SurfaceError::DriverUnavailable)?;
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

    #[cfg(target_os = "linux")]
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
        .workspace
        .as_ref()
        .ok_or("missing rendered workspace")?
        .visible_range(0, visible_rows, TREE_OVERSCAN_ROWS)
        .len();
    let editor_rows = app.buffer().snapshot().line_count();
    TEST_SHAPE_CALLS.with(|calls| calls.set(0));
    let _scene = app.try_scene(SceneRevision::new(1), viewport)?;
    let expected_shapes = u64::try_from(projected_tree_rows.saturating_add(editor_rows))?;
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
    assert_eq!(pointer_app.active_workspace_entry, Some(1));

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
    assert_eq!(app.active_workspace_entry, Some(alpha));
    let accepted_revision = app.runtime_document_revision;

    let failed = app.handle_event(&click(invalid, 2)?);
    assert!(failed.visual_changed);
    assert!(!failed.document_changed);
    assert_eq!(app.buffer().snapshot().text(), "alpha");
    assert_eq!(app.runtime_document_revision, accepted_revision);
    assert_eq!(app.workspace_failures, 1);
    assert!(app.last_workspace_error.is_some());

    let directory_failed = app.handle_event(&click(directory, 3)?);
    assert!(directory_failed.visual_changed);
    assert!(matches!(
        app.workspace
            .as_ref()
            .ok_or("missing workspace")?
            .path_for_file(directory),
        Err(WorkspaceError::NotRegularFile(_))
    ));
    assert_eq!(app.buffer().snapshot().text(), "alpha");
    assert_eq!(app.workspace_failures, 2);

    fs::remove_file(root.path().join("replace.rs"))?;
    let missing_failed = app.handle_event(&click(replacement, 4)?);
    assert!(missing_failed.visual_changed);
    assert_eq!(app.buffer().snapshot().text(), "alpha");
    assert_eq!(app.runtime_document_revision, accepted_revision);
    assert_eq!(app.workspace_failures, 3);

    #[cfg(unix)]
    {
        let outside = TestFile::new("outside")?;
        std::os::unix::fs::symlink(outside.path(), root.path().join("replace.rs"))?;
        let symlink_failed = app.handle_event(&click(replacement, 5)?);
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
        assert_eq!(app.workspace_failures, 4);
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
    let layout_error: StudioRenderError = LayoutError::InvalidScroll.into();
    let scene_error: StudioRenderError = SceneError::MissingGlyphAtlas.into();
    assert!(
        text_error
            .to_string()
            .starts_with("text layout input failed")
    );
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
    assert!(
        app.handle_event(&SurfaceEvent::Focus {
            timestamp: EventTimestamp::new(1),
            focused: false,
        })
        .visual_changed
    );
    assert!(
        !app.handle_event(&SurfaceEvent::Focus {
            timestamp: EventTimestamp::new(2),
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
    assert!(app.handle_event(&ime(ImeEvent::Started)).visual_changed);
    assert!(app.handle_event(&ime(ImeEvent::Cancelled)).visual_changed);
    assert!(!app.handle_event(&ime(ImeEvent::Cancelled)).visual_changed);

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
