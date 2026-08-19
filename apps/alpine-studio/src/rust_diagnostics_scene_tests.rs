use std::{
    fs,
    sync::atomic::{AtomicU64, Ordering},
};

use serde_json::value::RawValue;

use super::*;

static NEXT_SCENE: AtomicU64 = AtomicU64::new(1);

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
