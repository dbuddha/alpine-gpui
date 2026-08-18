use std::{
    fs,
    sync::atomic::{AtomicU64, Ordering},
};

use serde_json::value::RawValue;

use super::*;

static NEXT_SCENE: AtomicU64 = AtomicU64::new(1);

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
    fs::write(&path, "fn broken() {}\n")?;
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
    let params: Box<RawValue> = serde_json::from_str(&format!(
        r#"{{"uri":"{uri}","version":1,"diagnostics":[{{"range":{{"start":{{"line":0,"character":3}},"end":{{"line":0,"character":9}}}},"severity":1,"message":"broken"}}]}}"#
    ))?;
    app.rust_diagnostics.install_for_test(input, &params)?;
    let diagnosed = app.scene(SceneRevision::new(2), viewport);
    assert_eq!(diagnosed.quads().len(), baseline.quads().len() + 2);
    let underline = diagnosed.quads().iter().find(|quad| {
        quad.bounds().size().height().to_bits() == 1.0_f32.to_bits() && quad.clip().is_some()
    });
    assert!(underline.is_some());
    assert!(underline.and_then(|quad| quad.clip()).is_some());
    let drained = app.rust_diagnostics.shutdown();
    assert!(!drained.active);
    let cleared = app.scene(SceneRevision::new(3), viewport);
    assert_eq!(cleared.quads().len(), baseline.quads().len());
    fs::remove_dir_all(root)?;
    Ok(())
}
