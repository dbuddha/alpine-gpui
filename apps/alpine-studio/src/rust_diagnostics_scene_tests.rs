use std::{
    fs,
    num::NonZeroU32,
    sync::atomic::{AtomicU64, Ordering},
};

use alpine_text_layout::{GlyphBitmap, RasterizedGlyph};
use serde_json::value::RawValue;

use super::*;

static NEXT_SCENE: AtomicU64 = AtomicU64::new(1);

struct NavigationFailingRasterTextSystem {
    glyph_id: u32,
}

impl TextShaper for NavigationFailingRasterTextSystem {
    fn shape(&mut self, text: &str, font: FontKey) -> Result<LineLayout, LayoutError> {
        tests::TestTextSystem.shape(text, font)
    }
}

impl GlyphRasterizer for NavigationFailingRasterTextSystem {
    fn rasterize(
        &mut self,
        font: FontKey,
        glyph_id: u32,
        subpixel_x: u8,
    ) -> Result<RasterizedGlyph, LayoutError> {
        if glyph_id == self.glyph_id {
            Err(LayoutError::NativeFailure(
                "injected navigation raster failure",
            ))
        } else {
            let width = NonZeroU32::new(2).ok_or(LayoutError::InvalidShaperOutput)?;
            let height = NonZeroU32::new(3).ok_or(LayoutError::InvalidShaperOutput)?;
            let bitmap = GlyphBitmap::new(width, height, vec![255; 6])?;
            let _ = (font, subpixel_x);
            RasterizedGlyph::new(Some(bitmap), 0.0, 3.0)
        }
    }
}

fn bounded_diagnostics(uri: &str) -> Result<Box<RawValue>, serde_json::Error> {
    let mut diagnostic_items = vec![format!(
        r#"{{"range":{{"start":{{"line":0,"character":3}},"end":{{"line":0,"character":9}}}},"severity":1,"message":"broken"}}"#
    )];
    diagnostic_items.extend((1..MAX_VISIBLE_DIAGNOSTIC_MARKERS).map(|index| {
        format!(
            r#"{{"range":{{"start":{{"line":0,"character":3}},"end":{{"line":1,"character":4}}}},"severity":2,"message":"span {index}"}}"#
        )
    }));
    serde_json::from_str(&format!(
        r#"{{"uri":"{uri}","version":1,"diagnostics":[{}]}}"#,
        diagnostic_items.join(",")
    ))
}

fn assert_invalid_diagnostic_origin(app: &mut StudioApp, viewport: Size) {
    app.diagnostic_origin_x_override = Some(f32::NAN);
    assert!(matches!(
        app.try_scene(SceneRevision::new(4), viewport),
        Err(StudioRenderError::Domain)
    ));
    app.diagnostic_origin_x_override = None;
}

#[test]
#[allow(
    clippy::float_cmp,
    reason = "exact boundary geometry distinguishes every overlay arithmetic mutation"
)]
fn language_overlay_geometry_is_exact_at_normal_and_constrained_bounds()
-> Result<(), StudioRenderError> {
    let normal = Rect::new(
        Point::new(20.0, 10.0).ok_or(StudioRenderError::Domain)?,
        Size::new(400.0, 360.0).ok_or(StudioRenderError::Domain)?,
    );
    assert_eq!(
        StudioApp::language_overlay_bounds(normal, 2)?,
        Rect::new(
            Point::new(44.0, 58.0).ok_or(StudioRenderError::Domain)?,
            Size::new(352.0, 44.0).ok_or(StudioRenderError::Domain)?,
        )
    );

    let bottom_limited = Rect::new(
        Point::new(20.0, 10.0).ok_or(StudioRenderError::Domain)?,
        Size::new(400.0, 70.0).ok_or(StudioRenderError::Domain)?,
    );
    assert_eq!(
        StudioApp::language_overlay_bounds(bottom_limited, 2)?.origin(),
        Point::new(44.0, 36.0).ok_or(StudioRenderError::Domain)?
    );

    let constrained = Rect::new(
        Point::new(5.0, 0.0).ok_or(StudioRenderError::Domain)?,
        Size::new(10.0, 20.0).ok_or(StudioRenderError::Domain)?,
    );
    assert_eq!(
        StudioApp::language_overlay_bounds(constrained, 1)?,
        Rect::new(
            Point::new(14.0, 0.0).ok_or(StudioRenderError::Domain)?,
            Size::new(1.0, LINE_HEIGHT).ok_or(StudioRenderError::Domain)?,
        )
    );
    Ok(())
}

#[test]
#[cfg(unix)]
#[allow(
    clippy::too_many_lines,
    reason = "one production-path journey keeps overlay rendering, keyboard, accessibility, command, and path rejection behavior connected"
)]
fn navigation_overlay_keyboard_and_accessibility_use_validated_product_paths()
-> Result<(), Box<dyn Error>> {
    let root = std::env::temp_dir().join(format!(
        "alpine-navigation-scene-{}-{}",
        std::process::id(),
        NEXT_SCENE.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root)?;
    let path = root.join("main.rs");
    fs::write(&path, "fn target() {}\n")?;
    let path = fs::canonicalize(path)?;
    let mut app = StudioApp::open_file(tests::TestTextSystem, &path)?;
    let viewport = Size::new(640.0, 360.0).ok_or("viewport")?;
    let input = app.active_rust_document().ok_or("Rust document")?;
    let identity = app.language_identity();
    let document = lsp_language::LspDocument::from_file_path(&path, "rust", 1)?;
    let open = document.did_open_params("")?;
    let value: serde_json::Value = serde_json::from_str(open.get())?;
    let uri = value["textDocument"]["uri"].as_str().ok_or("URI")?;
    let diagnostics: Box<RawValue> = serde_json::from_str(&format!(
        r#"{{"uri":"{uri}","version":1,"diagnostics":[]}}"#
    ))?;
    app.rust_diagnostics.install_for_test(
        input,
        &diagnostics,
        rust_diagnostics::tests::mock_executable(),
    )?;
    let baseline = app.scene(SceneRevision::new(1), viewport);

    let hover: Box<RawValue> =
        serde_json::from_str(r#"{"contents":["fn target()","Local hover"]}"#)?;
    app.rust_diagnostics.install_navigation_for_test(
        identity,
        NavigationRequestKind::Hover,
        &hover,
    )?;
    let hover_scene = app.scene(SceneRevision::new(2), viewport);
    assert_eq!(hover_scene.quads().len(), baseline.quads().len() + 1);
    let pane = app
        .panes
        .layout(app.editor_region(viewport)?)?
        .active()
        .ok_or("active pane")?
        .bounds;
    let hover_bounds = StudioApp::language_overlay_bounds(pane, 2)?;
    let hover_clip = hover_scene
        .clips()
        .iter()
        .position(|clip| clip.bounds() == hover_bounds)
        .ok_or("hover clip")?;
    assert!(hover_scene.quads().iter().any(|quad| {
        quad.bounds() == hover_bounds && quad.clip().is_some_and(|clip| clip.index() == hover_clip)
    }));
    for row in 0..2 {
        let expected = Point::new(
            hover_bounds.origin().x() + FIND_BAR_INSET,
            hover_bounds.origin().y() + usize_as_f32(row) * LINE_HEIGHT + 15.0,
        )
        .ok_or("hover glyph origin")?;
        let first = hover_scene
            .glyphs()
            .iter()
            .filter(|glyph| {
                glyph.clip().is_some_and(|clip| clip.index() == hover_clip)
                    && glyph.bounds().origin().y().to_bits() == expected.y().to_bits()
            })
            .min_by(|left, right| {
                left.bounds()
                    .origin()
                    .x()
                    .total_cmp(&right.bounds().origin().x())
            })
            .ok_or("hover row glyph")?;
        assert_eq!(first.bounds().origin(), expected);
    }
    let hover_snapshot = app.accessibility_snapshot()?;
    assert!(hover_snapshot.nodes().iter().any(|node| {
        node.role() == AccessibilityRole::Dialog
            && node.name().starts_with("Rust hover:")
            && node.is_focused()
            && !node.supports_activate()
    }));

    let locations: Box<RawValue> = serde_json::from_str(&format!(
        r#"[{{"uri":"{uri}","range":{{"start":{{"line":0,"character":0}},"end":{{"line":0,"character":2}}}}}},{{"uri":"{uri}","range":{{"start":{{"line":0,"character":3}},"end":{{"line":0,"character":9}}}}}}]"#
    ))?;
    app.rust_diagnostics.install_navigation_for_test(
        identity,
        NavigationRequestKind::References,
        &locations,
    )?;
    let location_scene = app.scene(SceneRevision::new(3), viewport);
    assert_eq!(location_scene.quads().len(), baseline.quads().len() + 2);
    let location_bounds = StudioApp::language_overlay_bounds(pane, 2)?;
    let location_clip = location_scene
        .clips()
        .iter()
        .position(|clip| clip.bounds() == location_bounds)
        .ok_or("location clip")?;
    let selected_bounds = Rect::new(
        location_bounds.origin(),
        Size::new(location_bounds.size().width(), LINE_HEIGHT).ok_or("selected size")?,
    );
    assert!(location_scene.quads().iter().any(|quad| {
        quad.bounds() == selected_bounds
            && quad
                .clip()
                .is_some_and(|clip| clip.index() == location_clip)
    }));
    for row in 0..2 {
        let expected = Point::new(
            location_bounds.origin().x() + FIND_BAR_INSET,
            location_bounds.origin().y() + usize_as_f32(row) * LINE_HEIGHT + 15.0,
        )
        .ok_or("location glyph origin")?;
        let first = location_scene
            .glyphs()
            .iter()
            .filter(|glyph| {
                glyph
                    .clip()
                    .is_some_and(|clip| clip.index() == location_clip)
                    && glyph.bounds().origin().y().to_bits() == expected.y().to_bits()
            })
            .min_by(|left, right| {
                left.bounds()
                    .origin()
                    .x()
                    .total_cmp(&right.bounds().origin().x())
            })
            .ok_or("location row glyph")?;
        assert_eq!(first.bounds().origin(), expected);
    }
    let location_snapshot = app.accessibility_snapshot()?;
    let navigation = location_snapshot
        .nodes()
        .iter()
        .find(|node| node.name().starts_with("Rust references:"))
        .ok_or("navigation accessibility node")?;
    assert!(navigation.is_focused() && navigation.supports_activate());
    let action = AccessibilityAction::Activate {
        revision: location_snapshot.revision(),
        node: navigation.id(),
    };
    assert!(app.handle_accessibility_action(action)?.visual_changed);
    assert_eq!(app.selection.range(), 0..2);

    app.rust_diagnostics.install_navigation_for_test(
        app.language_identity(),
        NavigationRequestKind::References,
        &locations,
    )?;
    assert_eq!(
        app.handle_navigation_key(KEY_UP, false),
        Some(EventEffect::default())
    );
    for key in [KEY_UP, KEY_DOWN, KEY_RETURN] {
        assert_eq!(app.handle_navigation_key(key, true), None);
    }
    assert_eq!(app.handle_navigation_key(KEY_TAB, false), None);
    assert!(
        app.handle_key(KEY_DOWN, Modifiers::from_bits(0))
            .visual_changed
    );
    assert!(
        app.handle_key(KEY_RETURN, Modifiers::from_bits(0))
            .visual_changed
    );
    assert_eq!(app.selection.range(), 3..9);

    app.rust_diagnostics.install_navigation_for_test(
        app.language_identity(),
        NavigationRequestKind::Hover,
        &hover,
    )?;
    assert!(
        app.handle_navigation_key(KEY_ESCAPE, false)
            .is_some_and(|effect| effect.visual_changed)
    );
    app.rust_diagnostics.install_navigation_for_test(
        app.language_identity(),
        NavigationRequestKind::References,
        &locations,
    )?;
    let next_epoch = InputEpoch::INITIAL.checked_next().ok_or("input epoch")?;
    assert!(app.handle_focus(next_epoch, false).visual_changed);
    assert!(
        !app.rust_diagnostics
            .navigation_is_open(app.language_identity())
    );
    assert!(app.handle_focus(next_epoch, true).visual_changed);
    assert_eq!(app.apply_selected_navigation(), EventEffect::default());
    for command in [
        StudioCommand::ShowRustHover,
        StudioCommand::GoToRustDefinition,
        StudioCommand::FindRustReferences,
    ] {
        let _ = app.dispatch_command(command);
    }
    app.composition = Some(Composition {
        replacement: app.selection.range(),
        text: Box::default(),
        selected_start_utf16: 0,
        selected_length_utf16: 0,
    });
    for kind in [
        NavigationRequestKind::Hover,
        NavigationRequestKind::Definition,
        NavigationRequestKind::References,
    ] {
        assert_eq!(app.trigger_rust_navigation(kind), EventEffect::default());
    }
    app.composition = None;
    let original = app.selection;
    app.selection = Selection::caret(ByteOffset::new(app.buffer().snapshot().len_bytes() + 1));
    let failures = app.input_failures;
    assert_eq!(
        app.trigger_rust_navigation(NavigationRequestKind::Hover),
        EventEffect::default()
    );
    assert_eq!(app.input_failures, failures + 1);
    app.selection = original;

    let outside: Box<RawValue> = serde_json::from_str(
        r#"[{"uri":"file:///etc/passwd","range":{"start":{"line":0,"character":0},"end":{"line":0,"character":1}}}]"#,
    )?;
    app.rust_diagnostics.install_navigation_for_test(
        app.language_identity(),
        NavigationRequestKind::Definition,
        &outside,
    )?;
    assert!(app.apply_selected_navigation().visual_changed);
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
#[cfg(unix)]
fn navigation_overlay_raster_failures_preserve_scene_atomicity() -> Result<(), Box<dyn Error>> {
    let root = std::env::temp_dir().join(format!(
        "alpine-navigation-raster-{}-{}",
        std::process::id(),
        NEXT_SCENE.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root)?;
    let path = root.join("main.rs");
    fs::write(&path, "fn main() {}\n")?;
    let path = fs::canonicalize(path)?;
    let install = |app: &mut StudioApp| {
        let input = app.active_rust_document().ok_or("Rust document")?;
        app.rust_diagnostics.install_for_test(
            input,
            &rust_diagnostics::tests::diagnostics(&path, 1),
            rust_diagnostics::tests::mock_executable(),
        )?;
        Ok::<_, Box<dyn Error>>(())
    };

    let mut hover_app = StudioApp::open_file(
        NavigationFailingRasterTextSystem {
            glyph_id: u32::from('Ω'),
        },
        &path,
    )?;
    install(&mut hover_app)?;
    let hover: Box<RawValue> = serde_json::from_str(r#"{"contents":"Ω"}"#)?;
    hover_app.rust_diagnostics.install_navigation_for_test(
        hover_app.language_identity(),
        NavigationRequestKind::Hover,
        &hover,
    )?;
    assert!(matches!(
        hover_app.try_scene(
            SceneRevision::new(20),
            Size::new(640.0, 360.0).ok_or("viewport")?
        ),
        Err(StudioRenderError::Layout(LayoutError::NativeFailure(
            "injected navigation raster failure"
        )))
    ));

    let mut location_app = StudioApp::open_file(
        NavigationFailingRasterTextSystem {
            glyph_id: u32::from('§'),
        },
        &path,
    )?;
    install(&mut location_app)?;
    let location: Box<RawValue> = serde_json::from_str(
        r#"[{"uri":"file:///tmp/§.rs","range":{"start":{"line":0,"character":0},"end":{"line":0,"character":1}}}]"#,
    )?;
    location_app.rust_diagnostics.install_navigation_for_test(
        location_app.language_identity(),
        NavigationRequestKind::Definition,
        &location,
    )?;
    assert!(matches!(
        location_app.try_scene(
            SceneRevision::new(21),
            Size::new(640.0, 360.0).ok_or("viewport")?
        ),
        Err(StudioRenderError::Layout(LayoutError::NativeFailure(
            "injected navigation raster failure"
        )))
    ));
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn diagnostic_scene_adds_clipped_marker_and_bounded_message_then_clears()
-> Result<(), Box<dyn Error>> {
    let root = std::env::temp_dir().join(format!(
        "alpine-diagnostic-scene-{}-{}",
        std::process::id(),
        NEXT_SCENE.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root)?;
    let path = root.join("main.rs");
    fs::write(&path, "fn broken() {}\nsecond line\n")?;
    let path = fs::canonicalize(path)?;
    let root = fs::canonicalize(root)?;
    let mut app = StudioApp::open_file(tests::TestTextSystem, &path)?;
    let viewport = Size::new(320.0, 180.0).ok_or("viewport")?;
    let baseline = app.scene(SceneRevision::new(1), viewport);
    let input = app.active_rust_document().ok_or("Rust document")?;
    let document = lsp_language::LspDocument::from_file_path(&path, "rust", 1)?;
    let open = document.did_open_params("")?;
    let value: serde_json::Value = serde_json::from_str(open.get())?;
    let uri = value["textDocument"]["uri"].as_str().ok_or("URI")?;
    let params = bounded_diagnostics(uri)?;
    app.rust_diagnostics.install_for_test(
        input,
        &params,
        rust_diagnostics::tests::mock_executable(),
    )?;
    let diagnosed = app.scene(SceneRevision::new(2), viewport);
    assert_eq!(
        diagnosed.quads().len(),
        baseline.quads().len() + MAX_VISIBLE_DIAGNOSTIC_MARKERS + 1
    );
    let underlines = diagnosed
        .quads()
        .iter()
        .filter(|quad| {
            quad.bounds().size().height().to_bits() == 1.0_f32.to_bits() && quad.clip().is_some()
        })
        .collect::<Vec<_>>();
    assert_eq!(underlines.len(), MAX_VISIBLE_DIAGNOSTIC_MARKERS);
    let pane = app
        .panes
        .layout(app.editor_region(viewport)?)?
        .active()
        .ok_or("active pane")?
        .bounds;
    let exact = Rect::new(
        Point::new(pane.origin().x() + 24.0, pane.origin().y() + 20.0).ok_or("underline origin")?,
        Size::new(48.0, 1.0).ok_or("underline size")?,
    );
    let to_line_end = Rect::new(
        Point::new(pane.origin().x() + 24.0, pane.origin().y() + 20.0).ok_or("span origin")?,
        Size::new(88.0, 1.0).ok_or("span size")?,
    );
    assert_eq!(
        underlines
            .iter()
            .filter(|quad| quad.bounds() == exact)
            .count(),
        1
    );
    assert_eq!(
        underlines
            .iter()
            .filter(|quad| quad.bounds() == to_line_end)
            .count(),
        MAX_VISIBLE_DIAGNOSTIC_MARKERS - 1
    );
    assert_eq!(
        remaining_diagnostic_markers(0),
        Some(MAX_VISIBLE_DIAGNOSTIC_MARKERS)
    );
    assert_eq!(
        remaining_diagnostic_markers(MAX_VISIBLE_DIAGNOSTIC_MARKERS - 1),
        Some(1)
    );
    assert_eq!(
        remaining_diagnostic_markers(MAX_VISIBLE_DIAGNOSTIC_MARKERS),
        None
    );
    assert_eq!(
        remaining_diagnostic_markers(MAX_VISIBLE_DIAGNOSTIC_MARKERS + 1),
        None
    );

    let clip_bounds = Rect::new(Point::new(0.0, 0.0).ok_or("clip origin")?, viewport);
    let mut foreign_builder = SceneBuilder::new(SceneRevision::new(99), viewport);
    let mut foreign_clip = foreign_builder.push_clip(Clip::new(clip_bounds));
    for _ in 0..1_024 {
        foreign_clip = foreign_builder.push_clip(Clip::new(clip_bounds));
    }
    app.diagnostic_clip_override = Some(foreign_clip);
    assert!(matches!(
        app.try_scene(SceneRevision::new(3), viewport),
        Err(StudioRenderError::Scene(SceneError::InvalidClip { .. }))
    ));
    app.diagnostic_clip_override = None;

    assert_invalid_diagnostic_origin(&mut app, viewport);

    let drained = app.rust_diagnostics.shutdown();
    assert!(!drained.active);
    let cleared = app.scene(SceneRevision::new(5), viewport);
    assert_eq!(cleared.quads().len(), baseline.quads().len());
    fs::remove_dir_all(root)?;
    Ok(())
}
