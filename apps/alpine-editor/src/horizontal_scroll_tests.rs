#![allow(
    clippy::float_cmp,
    reason = "test glyph advances are exact multiples of eight points"
)]

use std::{error::Error, fmt::Write as _, fs, path::PathBuf};

use alpine_platform_macos::{AccessibilityBounds, ScrollPhase};

use super::*;
use crate::tests::TestTextSystem;

// TestTextSystem gives every character an eight-point advance.
const ADVANCE: f32 = 8.0;

type TestResult = Result<(), Box<dyn Error>>;

fn viewport() -> Result<Size, Box<dyn Error>> {
    Ok(Size::new(WINDOW_WIDTH, WINDOW_HEIGHT).ok_or("viewport")?)
}

fn long_line_app(columns: usize, short_lines: usize) -> Result<EditorApp, Box<dyn Error>> {
    let mut text = "x".repeat(columns);
    for index in 0..short_lines {
        write!(text, "\nshort {index}")?;
    }
    let mut app = EditorApp::from_document(TestTextSystem, EditorDocument::scratch(&text), None)?;
    let _ = app.try_scene(SceneRevision::new(1), viewport()?)?;
    Ok(app)
}

fn text_area(app: &EditorApp) -> Result<(f32, f32), Box<dyn Error>> {
    let pane = app.active_pane_bounds()?;
    Ok((
        pane.origin().x() + GUTTER_WIDTH,
        editor_text_width(pane.size().width()),
    ))
}

fn caret_quad(app: &EditorApp, scene: &Scene) -> Result<Rect, Box<dyn Error>> {
    let caret = app.settings.active().theme.caret;
    Ok(scene
        .quads()
        .iter()
        .rev()
        .find(|quad| quad.color() == caret && quad.bounds().size().width() == CARET_WIDTH)
        .ok_or("caret quad")?
        .bounds())
}

fn assert_caret_visible(app: &EditorApp, scene: &Scene) -> TestResult {
    let (left, width) = text_area(app)?;
    let caret = caret_quad(app, scene)?;
    assert!(
        caret.origin().x() >= left && caret.origin().x() + CARET_WIDTH <= left + width,
        "caret at {} outside the text area {left}..{}",
        caret.origin().x(),
        left + width
    );
    Ok(())
}

fn scroll(app: &mut EditorApp, delta_x: f32, delta_y: f32, phase: ScrollPhase) -> EventEffect {
    app.handle_event(&SurfaceEvent::Scroll {
        timestamp: EventTimestamp::new(1),
        delta_x,
        delta_y,
        phase,
        precise: true,
        modifiers: Modifiers::default(),
    })
}

fn pointer(app: &mut EditorApp, action: PointerAction, x: f32, y: f32) -> TestResult {
    let button = if action == PointerAction::Moved {
        PointerButton::None
    } else {
        PointerButton::Primary
    };
    let _ = app.handle_event(&SurfaceEvent::Pointer {
        timestamp: EventTimestamp::new(1),
        action,
        position: Point::new(x, y).ok_or("pointer")?,
        button,
        modifiers: Modifiers::default(),
    });
    Ok(())
}

fn ime(event: ImeEvent) -> SurfaceEvent {
    SurfaceEvent::Ime {
        timestamp: EventTimestamp::new(1),
        input_epoch: InputEpoch::INITIAL,
        event,
    }
}

fn first_rect(
    app: &mut EditorApp,
    start: usize,
    length: usize,
    marked_text: bool,
) -> Result<AccessibilityBounds, Box<dyn Error>> {
    let request = AccessibilityRequest::first_rect_for_range(
        AccessibilityRequestId::new(7),
        accessibility::revision(app),
        AccessibilityTextRange::new(start, length),
        marked_text,
    )?;
    let (response, _) = accessibility::respond(app, &request);
    match response.result() {
        Ok(AccessibilityPayload::TextGeometry { bounds, .. }) => Ok(*bounds),
        other => Err(format!("text geometry: {other:?}").into()),
    }
}

#[test]
fn caret_at_column_400_is_revealed_and_home_reveals_the_start() -> TestResult {
    let mut app = long_line_app(500, 4)?;
    let (left, width) = text_area(&app)?;
    let _ = app.set_selection(Selection::caret(ByteOffset::new(400)));
    let caret_x = 400.0 * ADVANCE;
    assert!(
        caret_x - app.scroll_x >= 0.0 && caret_x + CARET_WIDTH - app.scroll_x <= width,
        "caret x {caret_x} minus scroll {} must lie inside the {width} point text area",
        app.scroll_x
    );
    let scene = app.try_scene(SceneRevision::new(2), viewport()?)?;
    assert_eq!(
        caret_quad(&app, &scene)?.origin().x(),
        left + caret_x - app.scroll_x
    );
    assert_caret_visible(&app, &scene)?;

    let _ = app.handle_key(KEY_HOME, Modifiers::default());
    assert_eq!(app.scroll_x, 0.0, "moving to column 0 reveals the start");
    let scene = app.try_scene(SceneRevision::new(3), viewport()?)?;
    assert_eq!(caret_quad(&app, &scene)?.origin().x(), left);
    Ok(())
}

#[test]
fn typing_at_the_end_of_a_long_line_keeps_the_caret_visible() -> TestResult {
    let mut app = long_line_app(300, 2)?;
    let _ = app.handle_key(KEY_END, Modifiers::default());
    for (frame, text) in (2_u64..).zip(["a", "bc", "def"]) {
        let _ = app.handle_event(&ime(ImeEvent::Committed(text.into())));
        let scene = app.try_scene(SceneRevision::new(frame), viewport()?)?;
        assert_caret_visible(&app, &scene)?;
    }
    assert_eq!(app.buffer().snapshot().line_byte_range(0)?.len(), 307);
    Ok(())
}

#[test]
fn horizontal_scroll_moves_clamps_and_ignores_vertical_deltas() -> TestResult {
    let mut app = long_line_app(500, 80)?;
    let (_, width) = text_area(&app)?;
    let line_end = 500.0 * ADVANCE;

    assert!(scroll(&mut app, -96.0, 0.0, ScrollPhase::Changed).visual_changed);
    assert_eq!(app.scroll_x, 96.0, "precise deltas move by exact points");
    let _ = scroll(&mut app, -1.0e6, 0.0, ScrollPhase::Changed);
    let maximum = app.scroll_x;
    assert!(
        maximum + width >= line_end + CARET_WIDTH,
        "the end of the longest line and its caret must fit at maximum scroll"
    );
    assert!(
        maximum + width <= line_end + 4.0 * ADVANCE,
        "maximum scroll {maximum} leaves only a small right margin"
    );
    assert!(!scroll(&mut app, -10.0, 0.0, ScrollPhase::Changed).visual_changed);
    assert_eq!(app.scroll_x, maximum);

    assert!(scroll(&mut app, 0.0, -2.0 * LINE_HEIGHT, ScrollPhase::Changed).visual_changed);
    assert_eq!(app.scroll_y, 2.0 * LINE_HEIGHT);
    assert_eq!(
        app.scroll_x, maximum,
        "vertical deltas leave scroll_x alone"
    );

    // Trackpad x noise inside a mostly vertical delta does not drift sideways.
    let _ = scroll(&mut app, 2.0, -LINE_HEIGHT, ScrollPhase::Began);
    let _ = scroll(&mut app, 3.0, -LINE_HEIGHT, ScrollPhase::Changed);
    assert_eq!(app.scroll_x, maximum);
    assert_eq!(app.scroll_y, 4.0 * LINE_HEIGHT);

    assert!(scroll(&mut app, 1.0e6, 0.0, ScrollPhase::None).visual_changed);
    assert_eq!(app.scroll_x, 0.0);
    assert!(!scroll(&mut app, 10.0, 0.0, ScrollPhase::None).visual_changed);
    assert_eq!(app.scroll_y, 4.0 * LINE_HEIGHT);

    // Shift turns a line-based wheel sideways when AppKit has not swapped it.
    let _ = app.handle_event(&SurfaceEvent::Scroll {
        timestamp: EventTimestamp::new(1),
        delta_x: 0.0,
        delta_y: -40.0,
        phase: ScrollPhase::None,
        precise: false,
        modifiers: Modifiers::from_bits(Modifiers::SHIFT),
    });
    assert_eq!((app.scroll_x, app.scroll_y), (40.0, 4.0 * LINE_HEIGHT));
    Ok(())
}

#[test]
fn clicks_and_drags_after_horizontal_scroll_land_on_the_drawn_column() -> TestResult {
    let mut app = long_line_app(500, 2)?;
    app.scroll_x = 800.0;
    let _ = app.try_scene(SceneRevision::new(2), viewport()?)?;
    let (left, _) = text_area(&app)?;
    let row = app.active_pane_bounds()?.origin().y() + 2.0;

    pointer(&mut app, PointerAction::Down, left + 400.0 + 2.0, row)?;
    assert_eq!(app.selection, Selection::caret(ByteOffset::new(150)));
    pointer(&mut app, PointerAction::Moved, left + 600.0 + 2.0, row)?;
    pointer(&mut app, PointerAction::Up, left + 600.0 + 2.0, row)?;
    assert_eq!(
        app.selection,
        Selection::new(ByteOffset::new(150), ByteOffset::new(175))
    );
    assert_eq!(app.scroll_x, 800.0, "columns inside the view do not scroll");
    Ok(())
}

#[test]
fn accessibility_frames_and_the_ime_rect_include_the_offset() -> TestResult {
    let mut app = long_line_app(500, 2)?;
    app.scroll_x = 800.0;
    let scene = app.try_scene(SceneRevision::new(2), viewport()?)?;
    let (left, _) = text_area(&app)?;

    let expected = left + 120.0 * ADVANCE - 800.0;
    let bounds = first_rect(&mut app, 120, 1, false)?;
    assert_eq!((bounds.x(), bounds.width()), (expected, ADVANCE));
    assert!(
        scene
            .glyphs()
            .iter()
            .any(|glyph| glyph.bounds().origin().x() == expected),
        "the accessibility frame must sit on a drawn glyph"
    );

    let _ = app.set_selection(Selection::caret(ByteOffset::new(300)));
    let _ = app.handle_event(&ime(ImeEvent::Started));
    let _ = app.handle_event(&ime(ImeEvent::Updated {
        text: "ab".into(),
        selected_start_utf16: 2,
        selected_length_utf16: 0,
    }));
    let scene = app.try_scene(SceneRevision::new(3), viewport()?)?;
    assert_caret_visible(&app, &scene)?;
    let caret = caret_quad(&app, &scene)?;
    let candidate = first_rect(&mut app, 302, 0, true)?;
    assert_eq!(
        (candidate.x(), candidate.y()),
        (caret.origin().x(), caret.origin().y()),
        "the IME candidate rect follows the drawn caret"
    );
    Ok(())
}

#[test]
fn panes_on_one_document_keep_separate_horizontal_offsets() -> TestResult {
    let mut app = long_line_app(500, 2)?;
    assert!(app.split_active_pane(SplitAxis::Columns).visual_changed);
    let _ = app.try_scene(SceneRevision::new(2), viewport()?)?;
    let _ = scroll(&mut app, -320.0, 0.0, ScrollPhase::None);
    assert_eq!(app.scroll_x, 320.0);

    let _ = app.focus_next_pane();
    assert_eq!(app.scroll_x, 0.0, "the first pane never scrolled");
    let _ = scroll(&mut app, -80.0, 0.0, ScrollPhase::None);
    assert_eq!(app.scroll_x, 80.0);
    let scene = app.try_scene(SceneRevision::new(3), viewport()?)?;

    let layout = app.panes.layout(app.editor_region(viewport()?)?)?;
    for (pane, offset) in layout.iter().zip([80.0, 320.0]) {
        let bounds = pane.bounds;
        let first_row = bounds.origin().y()..bounds.origin().y() + LINE_HEIGHT;
        let leftmost = scene
            .glyphs()
            .iter()
            .filter(|glyph| {
                let Some(clip) = glyph.clip().and_then(|id| scene.clips().get(id.index())) else {
                    return false;
                };
                let clip = clip.bounds();
                clip.size().width() > GUTTER_WIDTH
                    && rect_contains_bounds(bounds, clip)
                    && first_row.contains(&glyph.bounds().origin().y())
            })
            .map(|glyph| glyph.bounds().origin().x())
            .fold(f32::INFINITY, f32::min);
        assert_eq!(leftmost, bounds.origin().x() + GUTTER_WIDTH - offset);
    }

    let _ = app.focus_next_pane();
    assert_eq!(app.scroll_x, 320.0);
    let _ = app.focus_next_pane();
    assert_eq!(app.scroll_x, 80.0);
    Ok(())
}

struct TempFiles(Vec<PathBuf>);

impl Drop for TempFiles {
    fn drop(&mut self) {
        for path in &self.0 {
            let _ = fs::remove_file(path);
        }
    }
}

#[test]
fn switching_tabs_restores_the_horizontal_offset_like_scroll_y() -> TestResult {
    let directory = std::env::temp_dir();
    let id = std::process::id();
    let files = TempFiles(vec![
        directory.join(format!("alpine-hscroll-{id}-a.txt")),
        directory.join(format!("alpine-hscroll-{id}-b.txt")),
    ]);
    for path in &files.0 {
        fs::write(path, "y".repeat(400))?;
    }
    let mut app = EditorApp::open_file(TestTextSystem, &files.0[0])?;
    let _ = app.try_scene(SceneRevision::new(1), viewport()?)?;
    let _ = scroll(&mut app, -240.0, 0.0, ScrollPhase::None);
    assert_eq!(app.scroll_x, 240.0);

    let _ = app.open_workspace_path(&files.0[1], None)?;
    assert_eq!(
        app.scroll_x, 0.0,
        "a newly opened tab starts at the left edge"
    );
    let _ = app.activate_document_tab(0)?;
    assert_eq!(
        app.scroll_x, 240.0,
        "returning to a tab restores its offset"
    );
    Ok(())
}

#[test]
fn horizontal_scroll_does_no_language_shaping_or_raster_work() -> TestResult {
    let mut text = String::new();
    for index in 0..40 {
        writeln!(text, "let value_{index} = {};", "1 + ".repeat(60))?;
    }
    let mut app = EditorApp::from_document(
        TestTextSystem,
        EditorDocument::scratch(&text),
        Some(Path::new("main.rs")),
    )?;
    let before = {
        let _ = app.try_scene(SceneRevision::new(1), viewport()?)?;
        app.try_scene(SceneRevision::new(2), viewport()?)?
    };
    let lexed = app.syntax_cache.snapshot().misses();
    let shaped = app.layout_cache.snapshot().shaped_lines();
    let rasterized = app.glyph_atlas.snapshot().misses();

    assert!(scroll(&mut app, -200.0, 0.0, ScrollPhase::Changed).visual_changed);
    let after = app.try_scene(SceneRevision::new(3), viewport()?)?;
    assert_eq!(app.syntax_cache.snapshot().misses(), lexed, "no lexing");
    assert_eq!(app.layout_cache.snapshot().shaped_lines(), shaped);
    assert_eq!(
        app.glyph_atlas.snapshot().misses(),
        rasterized,
        "laid-out lines were rasterized whole, so no glyph is new"
    );

    assert_eq!(before.glyphs().len(), after.glyphs().len());
    let mut shifted = 0_usize;
    for (old, new) in before.glyphs().iter().zip(after.glyphs()) {
        let delta = new.bounds().origin().x() - old.bounds().origin().x();
        assert!(delta == 0.0 || delta == -200.0, "glyph moved by {delta}");
        shifted += usize::from(delta != 0.0);
    }
    let gutter = app
        .settings
        .active()
        .theme
        .syntax
        .color(SyntaxClass::Comment);
    assert!(shifted > 1_000, "document text follows the scroll");
    assert!(
        before
            .glyphs()
            .iter()
            .zip(after.glyphs())
            .filter(|(old, _)| old.color() == gutter && old.bounds().size().width() > 0.0)
            .any(|(old, new)| old.bounds() == new.bounds()),
        "line numbers do not scroll"
    );
    Ok(())
}
