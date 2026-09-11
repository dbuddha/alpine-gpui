//! Real save boundary controls; native presentation is qualified separately.

use super::*;
use crate::rust_diagnostics::tests as diagnostic_tests;

#[test]
fn only_successful_studio_saves_notify_the_owned_rust_document()
-> Result<(), Box<dyn std::error::Error>> {
    let root = TestWorkspace::new()?;
    root.write("main.rs", "fn main() {}\n")?;
    let path = root.path().join("main.rs");
    let mut app = StudioApp::open_file(TestTextSystem, &path)?;
    let _ = app.handle_event(&ime(ImeEvent::Committed("// saved\n".into())));
    let input = app.active_rust_document().ok_or("Rust document")?;
    app.rust_diagnostics.install_for_test(
        input,
        &diagnostic_tests::diagnostics(&path, 1),
        diagnostic_tests::mock_executable(),
    )?;
    app.rust_diagnostics
        .initialize_installed_transport_for_test()?;
    let before = app.rust_diagnostics.snapshot().process_submitted_inputs;
    let _ = app.save_document();
    assert_eq!(
        app.rust_diagnostics.snapshot().process_submitted_inputs,
        before + 1
    );
    assert_eq!(fs::read_to_string(&path)?, "// saved\nfn main() {}\n");
    assert!(!app.document.is_dirty());
    assert_eq!(
        app.last_save.ok_or("save receipt")?.revision(),
        app.buffer().revision()
    );
    let _ = app.handle_event(&ime(ImeEvent::Committed("// unsaved\n".into())));
    fs::write(&path, "// external\n")?;
    let _ = app.save_document();
    assert_eq!(app.save_failures, 1);
    assert!(app.document.is_dirty());
    assert_eq!(fs::read_to_string(&path)?, "// external\n");
    assert_eq!(
        app.rust_diagnostics.snapshot().process_submitted_inputs,
        before + 1
    );
    Ok(())
}

#[test]
#[ignore = "requires ALPINE_RUST_ANALYZER with the qualification executable digest"]
fn pinned_rust_analyzer_refreshes_compiler_errors_after_a_real_studio_save()
-> Result<(), Box<dyn std::error::Error>> {
    let server = PathBuf::from(std::env::var_os("ALPINE_RUST_ANALYZER").ok_or("pinned server")?);
    let root = TestWorkspace::new()?;
    root.write(
        "Cargo.toml",
        concat!(
            "[package]\nname=\"alpine_save_notification\"\nversion=\"0.1.0\"\nedition=\"2021\"\n",
            "[lib]\npath=\"main.rs\"\n",
        ),
    )?;
    root.write(
        "main.rs",
        concat!(
            "pub fn borrow_error() {\nlet mut values = vec![1];\nlet first = &values[0];\n",
            "values.push(2);\nprintln!(\"{first}\");\n}\n",
        ),
    )?;
    let path = root.path().join("main.rs");
    let mut app = StudioApp::open_file(TestTextSystem, &path)?;
    app.rust_diagnostics = RustDiagnostics::with_server(&server);
    let input = app.active_rust_document().ok_or("Rust document")?;
    let (sender, receiver) = std::sync::mpsc::sync_channel(16);
    let _ = app
        .rust_diagnostics
        .sync_workspace([input], Some(1), move |wake| {
            Arc::new(move || {
                let _ = sender.try_send(wake);
            })
        });
    wait_for_compiler(&mut app, &receiver, true)?;
    let _ = app.replace_range(
        0..app.buffer().snapshot().len_bytes(),
        "pub fn valid() {}\n",
    );
    let input = app.active_rust_document().ok_or("changed Rust document")?;
    let _ = app
        .rust_diagnostics
        .sync_workspace([input], Some(1), |_| Arc::new(|| {}));
    assert!(app.rust_diagnostics.snapshot().saved_compiler_items > 0);
    let _ = app.save_document();
    assert_eq!(app.save_failures, 0);
    assert!(!app.document.is_dirty());
    assert_eq!(fs::read_to_string(&path)?, "pub fn valid() {}\n");
    wait_for_compiler(&mut app, &receiver, false)?;
    let evidence = app.rust_diagnostics.snapshot();
    eprintln!("successful-save compiler refresh: {evidence:?}");
    assert_eq!(evidence.process_starts, 1);
    assert_eq!(evidence.restarts, 0);
    let closed = app.rust_diagnostics.shutdown();
    assert_eq!(closed.saved_compiler_items, 0);
    assert_eq!(closed.saved_compiler_bytes, 0);
    Ok(())
}

fn wait_for_compiler(
    app: &mut StudioApp,
    receiver: &std::sync::mpsc::Receiver<LanguageWake>,
    expected_error: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let deadline = std::time::Instant::now() + diagnostic_tests::PRODUCT_DIAGNOSTIC_READINESS;
    let mut continuation = None;
    while std::time::Instant::now() < deadline {
        let wake = continuation.take().or_else(|| {
            receiver
                .recv_timeout(std::time::Duration::from_millis(20))
                .ok()
        });
        if let Some(wake) = wake {
            continuation = app.rust_diagnostics.poll(wake).continuation;
        }
        let snapshot = app.rust_diagnostics.snapshot();
        let compiler_error = app
            .rust_diagnostics
            .status_message()
            .is_some_and(|status| status.contains("E0502"));
        if snapshot.diagnostic_publications > 0
            && compiler_error == expected_error
            && (snapshot.saved_compiler_items > 0) == expected_error
        {
            return Ok(());
        }
    }
    Err(format!(
        "compiler save refresh timed out: {:?}",
        app.rust_diagnostics.snapshot()
    )
    .into())
}
