//! Capability negotiation through the production client, not just its peer.

use super::*;

const PROVIDER: &str =
    "'diagnosticProvider': {'interFileDependencies': True, 'workspaceDiagnostics': False}";

fn await_workspace_open(
    model: &mut RustDiagnostics,
    driver: &mut WireDriver,
) -> Result<(), Box<dyn Error>> {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !model
        .session
        .as_ref()
        .is_some_and(super::super::RustSession::workspace_ready)
    {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("capability fixture did not open: {:?}", model.snapshot()),
            )
            .into());
        }
        match driver.next_wake(remaining.min(Duration::from_millis(10))) {
            Ok(wake) => driver.continuation = model.poll(wake).continuation,
            Err(RecvTimeoutError::Timeout) => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn provider_replay(provider: &str, supported: bool) -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new()?;
    let log = fixture.root.join("capability-wire.jsonl");
    let server = fixture.root.join("capability-server");
    fs::write(&log, "")?;
    fs::write(fixture.root.join("a.rs"), SAVED_A)?;
    let log_path = serde_json::to_string(log.to_str().ok_or("fixture path")?)?;
    assert_eq!(MOCK_SERVER.matches(PROVIDER).count(), 1);
    let script = MOCK_SERVER.replace(PROVIDER, provider);
    fs::write(
        &server,
        format!("#!/usr/bin/env python3\nLOG_PATH = {log_path}\n{script}"),
    )?;
    fs::set_permissions(&server, fs::Permissions::from_mode(0o700))?;
    let (sender, receiver) = mpsc::sync_channel(32);
    let mut driver = WireDriver::new(receiver);
    let starts = Arc::new(AtomicUsize::new(0));
    let mut model = RustDiagnostics::with_server(&server);
    let _ = model.sync_workspace(
        [input(&fixture.root, "a.rs", 1, SAVED_A)],
        Some(1),
        wake_factory(sender, Arc::clone(&starts)),
    );
    let _ = wait_for_open(&mut model, &mut driver, &log, "/a.rs")?;
    let status_at_open = model.status_message();
    await_workspace_open(&mut model, &mut driver)?;
    if supported {
        let _ = wait_for_wire(&mut model, &mut driver, &log, |records| {
            records
                .iter()
                .any(|row| is_document_message(row, "textDocument/diagnostic", "/a.rs"))
        })?;
    }
    // The same FIFO writer supplies a server-observed barrier. Absence is not
    // inferred from an arbitrary sleep or a log sampled before prior writes.
    model
        .session
        .as_mut()
        .ok_or("workspace missing")?
        .client
        .notify("test/capability-barrier", None)?;
    let records = wait_for_wire(&mut model, &mut driver, &log, |records| {
        records
            .iter()
            .any(|row| row["message"]["method"] == "test/capability-barrier")
    })?;
    let requested = records
        .iter()
        .any(|row| is_document_message(row, "textDocument/diagnostic", "/a.rs"));
    eprintln!(
        "capability provider={provider:?} supported={supported} status_at_open={status_at_open:?} wire={}",
        serde_json::to_string(&records)?,
    );
    assert_eq!(
        requested, supported,
        "negotiation must gate actual requests"
    );
    if !supported {
        assert_eq!(
            status_at_open.as_deref(),
            Some("Rust diagnostics require inter-file pull diagnostic support.")
        );
    }
    assert_eq!(starts.load(Ordering::SeqCst), 1);
    assert_eq!(model.snapshot().overlay_documents, 1);
    assert!(
        model.snapshot().active,
        "unsupported pull must not close the editor workspace"
    );
    assert_eq!(fs::read_to_string(fixture.root.join("a.rs"))?, SAVED_A);
    assert!(!model.shutdown().active);
    Ok(())
}

#[test]
fn initialization_capabilities_gate_diagnostic_requests_on_the_real_wire()
-> Result<(), Box<dyn Error>> {
    for (provider, supported) in [
        ("", false),
        ("'diagnosticProvider': False", false),
        (
            "'diagnosticProvider': {'interFileDependencies': False, 'workspaceDiagnostics': False}",
            false,
        ),
        (PROVIDER, true),
    ] {
        provider_replay(provider, supported)?;
    }
    Ok(())
}
