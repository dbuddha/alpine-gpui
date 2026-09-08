//! #576 compiler coverage discriminator. Requires the externally hash-pinned server.

use std::{
    error::Error,
    fs,
    path::PathBuf,
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};

use super::{LanguageIdentity, RustDiagnostics, RustDocumentInput, tests};

const COMPILER_ONLY_ERROR: &str = concat!(
    "pub fn borrow_error() {\n",
    "    let mut values = vec![1];\n",
    "    let first = &values[0];\n",
    "    values.push(2);\n",
    "    println!(\"{first}\");\n",
    "}\n",
);

#[test]
#[ignore = "requires ALPINE_RUST_ANALYZER with the qualification executable digest"]
fn pinned_rust_analyzer_retains_compiler_only_diagnostics() -> Result<(), Box<dyn Error>> {
    let server = PathBuf::from(std::env::var_os("ALPINE_RUST_ANALYZER").ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "missing pinned rust-analyzer")
    })?);
    let (root, path, _, _) = tests::fixture();
    fs::write(
        root.join("Cargo.toml"),
        concat!(
            "[package]\nname=\"alpine_compiler_coverage\"\n",
            "version=\"0.1.0\"\nedition=\"2021\"\n",
            "[lib]\npath=\"main.rs\"\n",
        ),
    )?;
    fs::write(&path, COMPILER_ONLY_ERROR)?;
    let snapshot = alpine_text::Buffer::new(COMPILER_ONLY_ERROR).snapshot();
    let identity = LanguageIdentity {
        workspace_id: 1,
        workspace_revision: 1,
        document_id: 1,
        document_revision: 1,
        buffer_revision: snapshot.revision().get(),
        selection_revision: 1,
    };
    let input = RustDocumentInput::new(&path, &root, identity, snapshot);
    let (sender, receiver) = mpsc::sync_channel(16);
    let mut model = RustDiagnostics::with_server(&server);
    let effect = model.sync_workspace([input], Some(1), move |wake| {
        Arc::new(move || {
            let _ = sender.try_send(wake);
        })
    });
    let mut continuation = effect.continuation;
    let started = Instant::now();
    let mut native_response_seen = false;
    let mut compiler_report_seen = false;
    while started.elapsed() < tests::PRODUCT_DIAGNOSTIC_READINESS {
        let wake = continuation
            .take()
            .or_else(|| receiver.recv_timeout(Duration::from_millis(20)).ok());
        if let Some(wake) = wake {
            continuation = model.poll(wake).continuation;
        }
        native_response_seen |= model
            .session
            .as_ref()
            .and_then(|session| session.diagnostics.as_ref())
            .is_some();
        compiler_report_seen = model.status_message().is_some_and(|message| {
            message.contains("Saved compiler (not current buffer)")
                && message.contains("E0502")
                && message.contains("cannot borrow `values` as mutable")
        });
        if compiler_report_seen && native_response_seen {
            break;
        }
    }
    let evidence = format!("{:?}", model.snapshot());
    let closed = model.shutdown();
    eprintln!(
        "compiler coverage root={} native_ready={native_response_seen} compiler_ready={compiler_report_seen}; before={evidence}; closed={closed:?}",
        root.display()
    );
    assert!(
        native_response_seen,
        "native analysis never became ready; compiler coverage is inconclusive: {evidence}"
    );
    assert!(
        compiler_report_seen,
        "native analysis is ready but compiler-only E0502 is absent: {evidence}"
    );
    assert_eq!(closed.saved_compiler_items, 0);
    assert_eq!(closed.saved_compiler_bytes, 0);
    let shutdown = closed
        .last_shutdown
        .ok_or("missing final shutdown evidence")?;
    assert_eq!(
        shutdown.protocol,
        crate::lsp_client::LspShutdownProtocol::AcknowledgedAndExited
    );
    assert_eq!(shutdown.transport.shutdown_timeouts, 0);
    assert_eq!(shutdown.transport.retained_bytes, 0);
    assert_eq!(shutdown.transport.queued_events, 0);
    fs::remove_dir_all(&root).map_err(|error| {
        std::io::Error::new(
            error.kind(),
            format!(
                "compiler fixture cleanup failed at {} after {closed:?}: {error}",
                root.display()
            ),
        )
    })?;
    Ok(())
}
