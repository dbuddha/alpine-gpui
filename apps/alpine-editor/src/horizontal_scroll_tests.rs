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

    assert!(scroll(&mut app, -96.0, 0.0, ScrollPhase::None).visual_changed);
    assert_eq!(app.scroll_x, 96.0, "precise deltas move by exact points");
    let _ = scroll(&mut app, -1.0e6, 0.0, ScrollPhase::None);
    let maximum = app.scroll_x;
    assert!(
        maximum + width >= line_end + CARET_WIDTH,
        "the end of the longest line and its caret must fit at maximum scroll"
    );
    assert!(
        maximum + width <= line_end + 4.0 * ADVANCE,
        "maximum scroll {maximum} leaves only a small right margin"
    );
    assert!(!scroll(&mut app, -10.0, 0.0, ScrollPhase::None).visual_changed);
    assert_eq!(app.scroll_x, maximum);

    assert!(scroll(&mut app, 0.0, -2.0 * LINE_HEIGHT, ScrollPhase::None).visual_changed);
    assert_eq!(app.scroll_y, 2.0 * LINE_HEIGHT);
    assert_eq!(
        app.scroll_x, maximum,
        "vertical deltas leave scroll_x alone"
    );

    // Trackpad x noise inside a mostly vertical delta does not drift sideways.
    let _ = scroll(&mut app, 2.0, -LINE_HEIGHT, ScrollPhase::Began);
    let _ = scroll(&mut app, 3.0, -LINE_HEIGHT, ScrollPhase::Changed);
    let _ = scroll(&mut app, 0.0, 0.0, ScrollPhase::Ended);
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

#[test]
fn diagonal_wheel_on_a_narrow_document_scrolls_vertically_and_frames() -> TestResult {
    let app = long_line_app(20, 80)?;
    let clear = LinearRgba::new(0.02, 0.02, 0.02, 1.0).ok_or("clear")?;
    let mut runtime = Application::new(app, viewport()?, clear, WorkerConfig::default())?;
    let _ = runtime.frame_if_dirty();
    // AppKit's line-based replay: three lines down, four sideways.
    let frame = runtime.dispatch(&SurfaceEvent::Scroll {
        timestamp: EventTimestamp::new(1),
        delta_x: 4.0,
        delta_y: -3.0,
        phase: ScrollPhase::None,
        precise: false,
        modifiers: Modifiers::default(),
    });
    assert!(frame.is_some(), "a vertical scroll must produce a frame");
    let app = runtime.delegate();
    assert_eq!((app.scroll_x, app.scroll_y), (0.0, 3.0));
    Ok(())
}

#[test]
fn a_gesture_keeps_its_first_axis_through_a_sideways_tail() -> TestResult {
    let mut app = long_line_app(500, 80)?;
    let _ = scroll(&mut app, 1.0, -20.0, ScrollPhase::Began);
    let _ = scroll(&mut app, 2.0, -10.0, ScrollPhase::Changed);
    assert!(scroll(&mut app, -3.0, -1.0, ScrollPhase::Changed).visual_changed);
    assert_eq!((app.scroll_x, app.scroll_y), (0.0, 31.0));
    let _ = scroll(&mut app, 0.0, 0.0, ScrollPhase::Ended);

    let _ = scroll(&mut app, -30.0, 2.0, ScrollPhase::Began);
    let _ = scroll(&mut app, -1.0, -5.0, ScrollPhase::Changed);
    assert_eq!((app.scroll_x, app.scroll_y), (31.0, 31.0));
    let _ = scroll(&mut app, 0.0, 0.0, ScrollPhase::Cancelled);
    Ok(())
}

#[test]
fn text_clip_starts_at_the_gutter_and_stops_at_the_scrollbar() -> TestResult {
    let mut app = long_line_app(500, 2)?;
    let scene = app.try_scene(SceneRevision::new(2), viewport()?)?;
    let pane = app.active_pane_bounds()?;
    let left = pane.origin().x() + GUTTER_WIDTH;
    let glyph = scene
        .glyphs()
        .iter()
        .find(|glyph| glyph.bounds().origin().x() == left)
        .ok_or("first text glyph")?;
    let clip = scene.clips()[glyph.clip().ok_or("text clip")?.index()].bounds();
    assert_eq!(clip.origin().x(), left);
    assert_eq!(
        clip.origin().x() + clip.size().width(),
        pane.origin().x() + pane.size().width() - SCROLLBAR_WIDTH,
        "scrolled text never draws under the vertical scrollbar"
    );
    Ok(())
}

#[test]
fn restored_sessions_and_resizes_reveal_the_caret() -> TestResult {
    let path =
        std::env::temp_dir().join(format!("alpine-hscroll-{}-restore.txt", std::process::id()));
    let files = TempFiles(vec![path.clone()]);
    fs::write(&path, format!("{}\nshort", "x".repeat(500)))?;
    let view = DocumentViewState {
        selection: Selection::caret(ByteOffset::new(400)),
        ..DocumentViewState::default()
    };
    let mut nodes = [session::SessionNode::Empty; session::SESSION_NODE_CAPACITY];
    nodes[0] = session::SessionNode::Leaf { pane: 0 };
    let state = session::SessionState {
        workspace: None,
        tabs: vec![session::SessionTab {
            path: Some(path),
            view,
        }],
        active_tab: 0,
        panes: session::SessionPanes {
            nodes,
            panes: [
                Some(session::SessionPane { tab: 0, view }),
                None,
                None,
                None,
            ],
            active_pane: 0,
        },
        file_tree: session::SessionFileTree::default(),
    };
    let mut app =
        EditorApp::from_session(TestTextSystem, state).map_err(|error| error.to_string())?;
    let scene = app.try_scene(SceneRevision::new(1), viewport()?)?;
    assert_caret_visible(&app, &scene)?;

    let narrow = Size::new(600.0, WINDOW_HEIGHT).ok_or("narrow")?;
    let scene = app.try_scene(SceneRevision::new(2), narrow)?;
    assert_caret_visible(&app, &scene)?;
    drop(files);
    Ok(())
}

struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

// A 300-column Rust line with `needle` at 140 and markers at 3 and 150.
fn diagnosed_app(name: &str) -> Result<(EditorApp, TempDir), Box<dyn Error>> {
    let root =
        TempDir(std::env::temp_dir().join(format!("alpine-hscroll-{}-{name}", std::process::id())));
    fs::create_dir_all(&root.0)?;
    let path = root.0.join("main.rs");
    let line = format!("{}needle{}", "a".repeat(140), "b".repeat(154));
    fs::write(&path, format!("{line}\nsecond\n"))?;
    let path = fs::canonicalize(path)?;
    let mut app = EditorApp::open_file(TestTextSystem, &path)?;
    let _ = app.try_scene(SceneRevision::new(1), viewport()?)?;
    let open = lsp_language::LspDocument::from_file_path(&path, "rust", 1)?.did_open_params("")?;
    let value: serde_json::Value = serde_json::from_str(open.get())?;
    let uri = value["textDocument"]["uri"].as_str().ok_or("URI")?;
    let marker = |start: u32, end: u32| {
        format!(
            r#"{{"range":{{"start":{{"line":0,"character":{start}}},"end":{{"line":0,"character":{end}}}}},"severity":1,"message":"m"}}"#
        )
    };
    let params: Box<serde_json::value::RawValue> = serde_json::from_str(&format!(
        r#"{{"uri":"{uri}","version":1,"diagnostics":[{},{}]}}"#,
        marker(3, 9),
        marker(150, 156)
    ))?;
    let input = app.active_rust_document().ok_or("Rust document")?;
    app.rust_diagnostics.install_for_test(
        input,
        &params,
        rust_diagnostics::tests::mock_executable(),
    )?;
    Ok((app, root))
}

#[test]
fn diagnostic_frames_scrolled_under_the_gutter_are_skipped() -> TestResult {
    let (mut app, _root) = diagnosed_app("diag-frames")?;
    app.scroll_x = 400.0;
    let scene = app.try_scene(SceneRevision::new(2), viewport()?)?;
    let (left, _) = text_area(&app)?;
    let visible = Rect::new(
        Point::new(left + 150.0 * ADVANCE - 400.0, 0.0).ok_or("marker")?,
        Size::new(6.0 * ADVANCE, 1.0).ok_or("marker size")?,
    );
    assert!(scene.quads().iter().any(|quad| {
        quad.bounds().origin().x() == visible.origin().x() && quad.bounds().size() == visible.size()
    }));
    let snapshot = accessibility::snapshot(&app)?;
    let frames: Vec<_> = snapshot
        .nodes()
        .iter()
        .filter(|node| node.name().contains("diagnostic"))
        .map(alpine_platform_macos::AccessibilityNode::bounds)
        .collect();
    assert_eq!(frames.len(), 1, "the marker under the gutter has no frame");
    assert_eq!(
        (frames[0].x(), frames[0].width()),
        (visible.origin().x(), visible.size().width())
    );
    Ok(())
}

// Every quad either stays put or moves by exactly -400; counts per kind.
fn shifted_quads(app: &mut EditorApp) -> Result<Vec<(LinearRgba, f32)>, Box<dyn Error>> {
    app.scroll_x = 0.0;
    let before = app.try_scene(SceneRevision::new(10), viewport()?)?;
    app.scroll_x = 400.0;
    let after = app.try_scene(SceneRevision::new(11), viewport()?)?;
    assert_eq!(before.quads().len(), after.quads().len());
    let mut shifted = Vec::new();
    for (old, new) in before.quads().iter().zip(after.quads()) {
        let delta = new.bounds().origin().x() - old.bounds().origin().x();
        assert!(delta == 0.0 || delta == -400.0, "quad moved by {delta}");
        assert_eq!(old.bounds().size(), new.bounds().size());
        if delta != 0.0 {
            shifted.push((old.color(), old.bounds().size().height()));
        }
    }
    Ok(shifted)
}

#[test]
fn text_anchored_quads_shift_by_exactly_scroll_x() -> TestResult {
    let (mut app, _root) = diagnosed_app("quads")?;
    let theme = app.settings.active().theme;
    let command = Modifiers::from_bits(Modifiers::COMMAND);
    let _ = app.handle_event(&SurfaceEvent::Keyboard {
        timestamp: EventTimestamp::new(1),
        state: KeyState::Down,
        physical_key: KEY_F,
        logical_key: Box::default(),
        modifiers: command,
        repeat: false,
    });
    let _ = app.handle_event(&ime(ImeEvent::Committed("needle".into())));
    let _ = app.complete_pending_find_for_test()?;
    assert_eq!(app.selection.range(), 140..146);
    let shifted = shifted_quads(&mut app)?;
    let count = |color: LinearRgba, height: f32| {
        shifted
            .iter()
            .filter(|(shifted, size)| *shifted == color && *size == height)
            .count()
    };
    assert_eq!(count(theme.selection, LINE_HEIGHT), 1);
    assert_eq!(count(theme.find_match, LINE_HEIGHT), 1);
    assert_eq!(count(theme.caret, 1.0), 2, "both diagnostic underlines");

    app.find.close();
    let _ = app.set_selection(Selection::caret(ByteOffset::new(200)));
    let _ = app.handle_event(&ime(ImeEvent::Started));
    let _ = app.handle_event(&ime(ImeEvent::Updated {
        text: "abc".into(),
        selected_start_utf16: 1,
        selected_length_utf16: 1,
    }));
    let shifted = shifted_quads(&mut app)?;
    let count = |color: LinearRgba, height: f32| {
        shifted
            .iter()
            .filter(|(shifted, size)| *shifted == color && *size == height)
            .count()
    };
    assert_eq!(count(theme.selection, LINE_HEIGHT), 1, "marked selection");
    assert_eq!(count(theme.caret, 1.0), 1, "marked-text underline");
    assert_eq!(count(theme.caret, LINE_HEIGHT), 1, "caret");
    Ok(())
}

#[test]
fn find_reveal_shows_the_match_start_first() -> TestResult {
    let mut app = long_line_app(500, 2)?;
    let (_, width) = text_area(&app)?;
    let visible = |app: &EditorApp, column: f32| {
        let x = column * ADVANCE - app.scroll_x;
        x >= 0.0 && x + CARET_WIDTH <= width
    };
    let _ = app.select_find_range(100..250);
    assert!(
        visible(&app, 100.0),
        "a match wider than the view shows its start"
    );
    let _ = app.select_find_range(300..310);
    assert!(visible(&app, 300.0) && visible(&app, 310.0));
    Ok(())
}

#[test]
fn find_navigation_dispatch_keeps_a_wide_match_start_visible() -> TestResult {
    let mut app = long_line_app(500, 2)?;
    let _ = app.handle_key(KEY_F, Modifiers::from_bits(Modifiers::COMMAND));
    let _ = app.handle_event(&ime(ImeEvent::Committed("x".repeat(150).into())));
    let _ = app.complete_pending_find_for_test()?;
    let clear = LinearRgba::new(0.02, 0.02, 0.02, 1.0).ok_or("clear")?;
    let mut runtime = Application::new(app, viewport()?, clear, WorkerConfig::default())?;
    let _ = runtime.frame_if_dirty();
    let frame = runtime.dispatch(&SurfaceEvent::Keyboard {
        timestamp: EventTimestamp::new(1),
        state: KeyState::Down,
        physical_key: KEY_RETURN,
        logical_key: Box::default(),
        modifiers: Modifiers::default(),
        repeat: false,
    });
    assert!(frame.is_some());
    let app = runtime.delegate();
    assert_eq!(app.selection.range(), 150..300);
    let (_, width) = text_area(app)?;
    let start = 150.0 * ADVANCE - app.scroll_x;
    assert!(
        start >= 0.0 && start + CARET_WIDTH <= width,
        "wide match start at {start} must remain visible after dispatch"
    );
    Ok(())
}

#[test]
fn cancelling_a_wide_composition_reveals_the_source_caret() -> TestResult {
    let app = long_line_app(500, 2)?;
    let clear = LinearRgba::new(0.02, 0.02, 0.02, 1.0).ok_or("clear")?;
    let mut runtime = Application::new(app, viewport()?, clear, WorkerConfig::default())?;
    let _ = runtime.frame_if_dirty();
    let _ = runtime.dispatch(&ime(ImeEvent::Started));
    let _ = runtime.dispatch(&ime(ImeEvent::Updated {
        text: "x".repeat(300).into(),
        selected_start_utf16: 300,
        selected_length_utf16: 0,
    }));
    assert!(runtime.delegate().scroll_x > 0.0);
    let _ = runtime.dispatch(&ime(ImeEvent::Cancelled));
    let app = runtime.delegate();
    assert!(app.composition.is_none());
    assert_eq!(app.selection, Selection::caret(ByteOffset::new(0)));
    assert_eq!(app.scroll_x, 0.0, "the restored caret must be visible");
    Ok(())
}

#[test]
fn showing_the_sidebar_reveals_the_caret_in_the_narrower_pane() -> TestResult {
    let root = TempDir(std::env::temp_dir().join(format!(
        "alpine-hscroll-{}-sidebar-reveal",
        std::process::id()
    )));
    fs::create_dir_all(&root.0)?;
    let path = root.0.join("long.txt");
    fs::write(&path, "x".repeat(500))?;
    for palette in [false, true] {
        let mut app = EditorApp::open_workspace_lazy(TestTextSystem, &root.0)?;
        let _ = app.open_workspace_path(&path, None)?;
        app.file_tree.hide();
        let _ = app.try_scene(SceneRevision::new(1), viewport()?)?;
        let _ = app.set_selection(Selection::caret(ByteOffset::new(400)));
        let scene = app.try_scene(SceneRevision::new(2), viewport()?)?;
        assert_caret_visible(&app, &scene)?;
        if palette {
            let _ = app.dispatch_command(EditorCommand::ToggleFileTree);
        } else {
            let _ = app.toggle_file_tree();
        }
        let _ = app.try_scene(SceneRevision::new(3), viewport()?)?;
        let _ = app.handle_key(KEY_ESCAPE, Modifiers::default());
        let scene = app.try_scene(SceneRevision::new(4), viewport()?)?;
        assert_eq!(app.selection, Selection::caret(ByteOffset::new(400)));
        assert!(app.file_tree.is_visible());
        assert!(!app.file_tree.is_focused());
        assert_caret_visible(&app, &scene)?;
    }
    Ok(())
}

fn dispatch_key(code: u16, modifiers: Modifiers) -> SurfaceEvent {
    SurfaceEvent::Keyboard {
        timestamp: EventTimestamp::new(1),
        state: KeyState::Down,
        physical_key: code,
        logical_key: Box::default(),
        modifiers,
        repeat: false,
    }
}

#[test]
fn integrated_reveal_tab_switch_preserves_stored_view_with_a_different_caret() -> TestResult {
    let files = TempFiles(
        ["alpha", "beta"]
            .map(|name| {
                std::env::temp_dir().join(format!(
                    "alpine-hscroll-switch-{}-{name}.txt",
                    std::process::id()
                ))
            })
            .to_vec(),
    );
    for path in &files.0 {
        fs::write(path, "abcdefgh".repeat(72))?;
    }
    let mut app = EditorApp::open_file(TestTextSystem, &files.0[0])?;
    let _ = app.try_scene(SceneRevision::new(1), viewport()?)?;
    let _ = scroll(&mut app, -240.0, 0.0, ScrollPhase::None);
    let _ = app.open_workspace_path(&files.0[1], None)?;
    let _ = app.set_selection(Selection::caret(ByteOffset::new(320)));
    let clear = LinearRgba::new(0.02, 0.02, 0.02, 1.0).ok_or("clear color")?;
    let mut runtime = Application::new(app, viewport()?, clear, WorkerConfig::default())?;
    let _ = runtime.frame_if_dirty();
    let _ = runtime.dispatch(&dispatch_key(
        KEY_LEFT,
        Modifiers::from_bits(Modifiers::COMMAND | Modifiers::OPTION),
    ));
    assert_eq!(runtime.delegate().tabs.active_index(), 0);
    assert_eq!(runtime.delegate().selection.head().get(), 0);
    assert_eq!(runtime.delegate().scroll_x, 240.0);
    Ok(())
}

#[test]
fn integrated_reveal_find_shows_matches_inside_a_row_split() -> TestResult {
    let mut lines = vec!["plain"; 48];
    lines[0] = "target";
    lines[16] = "target";
    let mut app = EditorApp::from_document(
        TestTextSystem,
        EditorDocument::scratch(&lines.join("\n")),
        None,
    )?;
    let _ = app.try_scene(SceneRevision::new(1), viewport()?)?;
    let _ = app.split_active_pane(SplitAxis::Rows);
    let _ = app.try_scene(SceneRevision::new(2), viewport()?)?;
    let _ = app.handle_key(KEY_F, Modifiers::from_bits(Modifiers::COMMAND));
    let _ = app.handle_event(&ime(ImeEvent::Committed("target".into())));
    let _ = app.complete_pending_find_for_test()?;
    let clear = LinearRgba::new(0.02, 0.02, 0.02, 1.0).ok_or("clear color")?;
    let mut runtime = Application::new(app, viewport()?, clear, WorkerConfig::default())?;
    let _ = runtime.frame_if_dirty();
    let _ = runtime.dispatch(&dispatch_key(KEY_RETURN, Modifiers::default()));
    let app = runtime.delegate();
    assert_eq!(
        app.selection.range().start,
        lines[..16].join("\n").len() + 1
    );
    let y = 16.0 * LINE_HEIGHT - app.scroll_y;
    assert!(
        y >= 0.0 && y + LINE_HEIGHT <= app.active_pane_bounds()?.size().height(),
        "match top {y} outside active pane"
    );
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn integrated_reveal_rtl_marked_text_agrees_with_the_native_caret() -> TestResult {
    let mut system = alpine_text_layout::CoreTextSystem::new();
    system.register_font(FONT_FAMILY, "Menlo-Regular")?;
    let app = EditorApp::from_document(system, EditorDocument::scratch(""), None)?;
    let clear = LinearRgba::new(0.02, 0.02, 0.02, 1.0).ok_or("clear color")?;
    let mut runtime = Application::new(app, viewport()?, clear, WorkerConfig::default())?;
    let _ = runtime.frame_if_dirty();
    let _ = runtime.dispatch(&ime(ImeEvent::Started));
    let mark = "אבגדה".repeat(80);
    let count = mark.encode_utf16().count();
    let frame = runtime
        .dispatch(&ime(ImeEvent::Updated {
            text: mark.into(),
            selected_start_utf16: u32::try_from(count)?,
            selected_length_utf16: 0,
        }))
        .ok_or("missing composition frame")?;
    let app = runtime.delegate();
    assert!(app.composition.is_some());
    assert_caret_visible(app, frame.scene())?;
    let caret = caret_quad(app, frame.scene())?;
    let request = AccessibilityRequest::first_rect_for_range(
        AccessibilityRequestId::new(12),
        accessibility::revision(app),
        AccessibilityTextRange::new(count, 0),
        true,
    )?;
    let reply = runtime.dispatch_with_response(&SurfaceEvent::Accessibility {
        timestamp: EventTimestamp::new(12),
        request,
    });
    let Ok(AccessibilityPayload::TextGeometry { bounds, .. }) = reply
        .accessibility_response()
        .ok_or("missing marked-text geometry")?
        .result()
    else {
        return Err("missing IME rectangle".into());
    };
    assert_eq!(
        (bounds.x(), bounds.y()),
        (caret.origin().x(), caret.origin().y())
    );
    Ok(())
}

#[test]
fn integrated_reveal_rejected_line_never_copies_its_prefix() -> TestResult {
    let bytes = alpine_text_layout::DEFAULT_MAX_LINE_BYTES + 1;
    let mut app = EditorApp::from_document(
        TestTextSystem,
        EditorDocument::scratch(&"x".repeat(bytes)),
        None,
    )?;
    LOCAL_UTF16_PREFIX_BYTES.with(|n| n.set(0));
    assert_eq!(app.line_x_for_offset(bytes, 400.0), None);
    assert_eq!(LOCAL_UTF16_PREFIX_BYTES.with(std::cell::Cell::get), 0);
    Ok(())
}

#[test]
fn integrated_reveal_focus_next_pane_restores_the_independent_view() -> TestResult {
    let mut app = long_line_app(500, 2)?;
    let _ = app.split_active_pane(SplitAxis::Columns);
    let _ = app.try_scene(SceneRevision::new(2), viewport()?)?;
    let _ = scroll(&mut app, -320.0, 0.0, ScrollPhase::None);
    let _ = app.focus_next_pane();
    let files = TempFiles(vec![
        std::env::temp_dir().join(format!("alpine-hscroll-pane-{}.txt", std::process::id())),
    ]);
    fs::write(&files.0[0], "other".repeat(100))?;
    let _ = app.open_workspace_path(&files.0[0], None)?;
    let _ = app.set_selection(Selection::caret(ByteOffset::new(350)));
    let clear = LinearRgba::new(0.02, 0.02, 0.02, 1.0).ok_or("clear color")?;
    prepare_focus_next(&mut app)?;
    let mut runtime = Application::new(app, viewport()?, clear, WorkerConfig::default())?;
    let _ = runtime.frame_if_dirty();
    let _ = runtime.dispatch(&dispatch_key(KEY_RETURN, Modifiers::default()));
    assert_eq!(runtime.delegate().selection.head().get(), 0);
    assert_eq!(runtime.delegate().scroll_x, 320.0);
    Ok(())
}

fn prepare_focus_next(app: &mut EditorApp) -> TestResult {
    let _ = app.open_command_palette();
    let _ = app.handle_command_palette_ime(&ImeEvent::Committed("Pane: Focus Next".into()));
    assert!(
        app.command_palette
            .visible_commands()?
            .iter()
            .any(|row| row.selected && row.command == EditorCommand::FocusNextPane)
    );
    Ok(())
}

#[test]
fn integrated_view_width_does_not_treat_focus_as_a_resize() -> TestResult {
    let mut app = long_line_app(500, 2)?;
    let _ = app.split_active_pane(SplitAxis::Columns);
    let _ = app.focus_next_pane();
    let _ = app.try_scene(SceneRevision::new(2), viewport()?)?;
    let _ = scroll(&mut app, -240.0, 0.0, ScrollPhase::None);
    let wide = app.active_pane_bounds()?.size().width();
    let _ = app.focus_next_pane();
    let _ = app.split_active_pane(SplitAxis::Columns);
    let _ = app.try_scene(SceneRevision::new(3), viewport()?)?;
    assert!(app.active_pane_bounds()?.size().width() < wide);
    prepare_focus_next(&mut app)?;
    let clear = LinearRgba::new(0.02, 0.02, 0.02, 1.0).ok_or("clear color")?;
    let mut runtime = Application::new(app, viewport()?, clear, WorkerConfig::default())?;
    let _ = runtime.frame_if_dirty();
    let _ = runtime.dispatch(&dispatch_key(KEY_RETURN, Modifiers::default()));
    assert_eq!(
        runtime.delegate().active_pane_bounds()?.size().width(),
        wide
    );
    assert_eq!(runtime.delegate().scroll_x, 240.0);
    Ok(())
}

#[test]
fn integrated_view_legacy_inactive_tab_reveals_on_first_activation() -> TestResult {
    let root =
        TempDir(std::env::temp_dir().join(format!("alpine-hscroll-legacy-{}", std::process::id())));
    fs::create_dir_all(&root.0)?;
    let a = root.0.join("a.txt");
    let b = root.0.join("b.txt");
    fs::write(&a, "a".repeat(500))?;
    fs::write(&b, "b")?;
    let mut app = EditorApp::open_file(TestTextSystem, &a)?;
    let _ = app.try_scene(SceneRevision::new(1), viewport()?)?;
    let _ = app.set_selection(Selection::caret(ByteOffset::new(400)));
    let _ = app.open_workspace_path(&b, None)?;
    let state = app.capture_session().map_err(|e| format!("{e:?}"))?;
    let file = root.0.join("session.bin");
    session::save(&file, &state)?;
    let restored = EditorApp::from_session(TestTextSystem, session::load(&file)?)
        .map_err(|e| e.to_string())?;
    let clear = LinearRgba::new(0.02, 0.02, 0.02, 1.0).ok_or("clear color")?;
    let mut runtime = Application::new(restored, viewport()?, clear, WorkerConfig::default())?;
    let _ = runtime.frame_if_dirty();
    let frame = runtime
        .dispatch(&dispatch_key(
            KEY_LEFT,
            Modifiers::from_bits(Modifiers::COMMAND | Modifiers::OPTION),
        ))
        .ok_or("missing activation frame")?;
    assert_eq!(runtime.delegate().selection.head().get(), 400);
    assert_caret_visible(runtime.delegate(), frame.scene())?;
    let _ = runtime.dispatch(&SurfaceEvent::Scroll {
        timestamp: EventTimestamp::new(2),
        delta_x: 10_000.0,
        delta_y: 0.0,
        phase: ScrollPhase::None,
        precise: true,
        modifiers: Modifiers::default(),
    });
    let cycle = Modifiers::from_bits(Modifiers::COMMAND | Modifiers::OPTION);
    let _ = runtime.dispatch(&dispatch_key(KEY_RIGHT, cycle));
    let _ = runtime.dispatch(&dispatch_key(KEY_LEFT, cycle));
    assert_eq!(runtime.delegate().selection.head().get(), 400);
    assert_eq!(
        runtime.delegate().scroll_x,
        0.0,
        "keep the live view's manual pan"
    );
    Ok(())
}

#[test]
fn integrated_view_explicit_navigation_reveals_the_same_selected_range() -> TestResult {
    let root = TempDir(
        std::env::temp_dir().join(format!("alpine-hscroll-location-{}", std::process::id())),
    );
    fs::create_dir_all(&root.0)?;
    let path = root.0.join("main.rs");
    fs::write(&path, "a".repeat(500))?;
    let path = fs::canonicalize(path)?;
    let mut app = EditorApp::open_file(TestTextSystem, &path)?;
    let _ = app.try_scene(SceneRevision::new(1), viewport()?)?;
    app.selection = Selection::new(ByteOffset::new(400), ByteOffset::new(404));
    let identity = app.language_identity();
    let input = app.active_rust_document().ok_or("rust input")?;
    let uri = format!("file://{}", path.display());
    let diagnostics: Box<serde_json::value::RawValue> = serde_json::from_str(&format!(
        r#"{{"uri":"{uri}","version":1,"diagnostics":[]}}"#
    ))?;
    app.rust_diagnostics.install_for_test(
        input,
        &diagnostics,
        rust_diagnostics::tests::mock_executable(),
    )?;
    let locations: Box<serde_json::value::RawValue> = serde_json::from_str(&format!(
        r#"[{{"uri":"{uri}","range":{{"start":{{"line":0,"character":400}},"end":{{"line":0,"character":404}}}}}}]"#
    ))?;
    app.rust_diagnostics.install_navigation_for_test(
        identity,
        NavigationRequestKind::References,
        &locations,
    )?;
    let clear = LinearRgba::new(0.02, 0.02, 0.02, 1.0).ok_or("clear color")?;
    let mut runtime = Application::new(app, viewport()?, clear, WorkerConfig::default())?;
    let _ = runtime.frame_if_dirty();
    let frame = runtime
        .dispatch(&dispatch_key(KEY_RETURN, Modifiers::default()))
        .ok_or("missing jump frame")?;
    assert_eq!(runtime.delegate().selection.range(), 400..404);
    assert_caret_visible(runtime.delegate(), frame.scene())
}

#[test]
fn integrated_view_project_search_reveals_an_already_selected_match() -> TestResult {
    let root =
        TempDir(std::env::temp_dir().join(format!("alpine-hscroll-search-{}", std::process::id())));
    fs::create_dir_all(&root.0)?;
    let path = root.0.join("match.txt");
    fs::write(&path, format!("{}needle", "a".repeat(400)))?;
    let mut app = EditorApp::open_workspace(TestTextSystem, &root.0)?;
    let _ = app.open_workspace_path(&path, None)?;
    let _ = app.try_scene(SceneRevision::new(1), viewport()?)?;
    app.selection = Selection::new(ByteOffset::new(400), ByteOffset::new(406));
    let _ = app.handle_key(
        KEY_F,
        Modifiers::from_bits(Modifiers::COMMAND | Modifiers::SHIFT),
    );
    let _ = app.handle_event(&ime(ImeEvent::Committed("needle".into())));
    for _ in 0..64 {
        let Some(request) = app.prepare_project_search_request()? else {
            break;
        };
        let _ = app.apply_project_search_output(request.execute());
    }
    let found = app.project_search.selected_match()?;
    assert_eq!(found.start..found.end, 400..406);
    let clear = LinearRgba::new(0.02, 0.02, 0.02, 1.0).ok_or("clear color")?;
    let mut runtime = Application::new(app, viewport()?, clear, WorkerConfig::default())?;
    let _ = runtime.frame_if_dirty();
    let frame = runtime
        .dispatch(&dispatch_key(KEY_RETURN, Modifiers::default()))
        .ok_or("missing project match frame")?;
    assert!(!runtime.delegate().project_search.is_open());
    assert_caret_visible(runtime.delegate(), frame.scene())
}

#[test]
fn integrated_view_decoded_inactive_pane_reveals_on_focus() -> TestResult {
    let root = TempDir(std::env::temp_dir().join(format!(
        "alpine-hscroll-inactive-pane-{}",
        std::process::id()
    )));
    fs::create_dir_all(&root.0)?;
    let a = root.0.join("a.txt");
    let b = root.0.join("b.txt");
    fs::write(&a, "a".repeat(500))?;
    fs::write(&b, "b")?;
    let mut app = EditorApp::open_file(TestTextSystem, &a)?;
    let _ = app.try_scene(SceneRevision::new(1), viewport()?)?;
    let _ = app.set_selection(Selection::caret(ByteOffset::new(400)));
    let _ = app.split_active_pane(SplitAxis::Columns);
    let _ = app.open_workspace_path(&b, None)?;
    let state = app.capture_session().map_err(|e| format!("{e:?}"))?;
    let file = root.0.join("session.bin");
    session::save(&file, &state)?;
    let mut restored = EditorApp::from_session(TestTextSystem, session::load(&file)?)
        .map_err(|e| e.to_string())?;
    prepare_focus_next(&mut restored)?;
    let clear = LinearRgba::new(0.02, 0.02, 0.02, 1.0).ok_or("clear color")?;
    let mut runtime = Application::new(restored, viewport()?, clear, WorkerConfig::default())?;
    let _ = runtime.frame_if_dirty();
    let frame = runtime
        .dispatch(&dispatch_key(KEY_RETURN, Modifiers::default()))
        .ok_or("missing focus frame")?;
    assert_eq!(runtime.delegate().selection.head().get(), 400);
    assert_caret_visible(runtime.delegate(), frame.scene())
}
