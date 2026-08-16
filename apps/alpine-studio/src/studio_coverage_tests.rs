use std::num::NonZeroU32;

use alpine_platform_macos::{EventTimestamp, ScrollPhase};
use alpine_text_layout::{GlyphBitmap, RasterizedGlyph, ShapedGlyph};

use super::*;

#[derive(Default)]
struct TestTextSystem;

impl TextShaper for TestTextSystem {
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
fn ime_preview_is_non_destructive_and_commit_is_atomic() -> Result<(), StudioRenderError> {
    let mut app = test_app().map_err(|_| StudioRenderError::Domain)?;
    let before = app.buffer.snapshot().text();
    assert!(app.handle_event(&ime(ImeEvent::Started)).visual_changed);
    assert!(
        app.handle_event(&ime(ImeEvent::Updated {
            text: "é".into(),
            selected_start_utf16: 1,
            selected_length_utf16: 0,
        }))
        .visual_changed
    );
    assert_eq!(app.buffer.snapshot().text(), before);
    let preview = app.try_scene(
        SceneRevision::new(1),
        viewport().map_err(|_| StudioRenderError::Domain)?,
    )?;
    assert!(preview.quads().len() >= 4);
    let effect = app.handle_event(&ime(ImeEvent::Committed("é".into())));
    assert!(effect.document_changed);
    assert!(app.buffer.snapshot().text().starts_with('é'));
    assert!(app.composition.is_none());
    Ok(())
}

#[test]
fn grapheme_delete_and_command_undo_restore_text() -> Result<(), SurfaceError> {
    let mut app = test_app()?;
    let original = app.buffer.snapshot().text();
    assert!(
        app.handle_event(&ime(ImeEvent::Committed("🦀".into())))
            .document_changed
    );
    let inserted = app.buffer.snapshot().text();
    assert!(inserted.starts_with('🦀'));
    assert!(
        app.handle_event(&key(KEY_DELETE_BACKWARD, Modifiers::default()))
            .document_changed
    );
    assert_eq!(app.buffer.snapshot().text(), original);
    assert!(
        app.handle_event(&key(KEY_Z, Modifiers::from_bits(Modifiers::COMMAND),))
            .document_changed
    );
    assert_eq!(app.buffer.snapshot().text(), inserted);
    Ok(())
}

#[test]
fn scroll_and_pointer_selection_use_rendered_visible_lines() -> Result<(), StudioRenderError> {
    let mut app = test_app().map_err(|_| StudioRenderError::Domain)?;
    app.buffer = Buffer::new(&"line\n".repeat(100));
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
        ByteOffset::new(app.buffer.snapshot().len_bytes()),
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
    assert_eq!(app.selection.range(), 0..app.buffer.snapshot().len_bytes());
    assert!(
        app.handle_event(&key(KEY_LEFT, Modifiers::default()))
            .visual_changed
    );
    assert_eq!(app.selection.head(), ByteOffset::new(0));

    app.selection = Selection::new(
        ByteOffset::new(0),
        ByteOffset::new(app.buffer.snapshot().len_bytes()),
    );
    assert!(
        app.handle_event(&key(KEY_RIGHT, Modifiers::default()))
            .visual_changed
    );
    assert_eq!(
        app.selection.head().get(),
        app.buffer.snapshot().len_bytes()
    );

    app.buffer = Buffer::new("ab\néx\nlast");
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

    app.buffer = Buffer::new("🦀x");
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
    app.selection = Selection::caret(ByteOffset::new(app.buffer.snapshot().len_bytes()));
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
    app.buffer = Buffer::new(&"line\n".repeat(100));
    app.selection = Selection::caret(ByteOffset::new(0));
    app.scroll_y = app.maximum_scroll();
    let snapshot = app.buffer.snapshot();
    assert!(app.caret_bounds(&snapshot, &[])?.is_none());
    app.selection = Selection::caret(ByteOffset::new(usize::MAX));
    assert!(app.caret_bounds(&snapshot, &[])?.is_none());
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
}
