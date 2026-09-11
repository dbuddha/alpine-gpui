//! Quiet active-view changes must schedule their own diagnostic work.

use super::*;

#[test]
fn quiet_tab_switch_requests_diagnostics_without_an_unrelated_wake() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new()?;
    let log = fixture.root.join("diagnostic-wakes.jsonl");
    let server = fixture.root.join("diagnostic-server");
    fs::write(&log, "")?;
    let log_path = serde_json::to_string(log.to_str().ok_or("fixture path")?)?;
    fs::write(
        &server,
        format!("#!/usr/bin/env python3\nLOG_PATH = {log_path}\n{MOCK_SERVER}"),
    )?;
    fs::set_permissions(&server, fs::Permissions::from_mode(0o700))?;
    let (sender, receiver) = mpsc::sync_channel(32);
    let mut driver = WireDriver::new(receiver);
    let starts = Arc::new(AtomicUsize::new(0));
    let mut model = RustDiagnostics::with_server(&server);
    let a = input(&fixture.root, "a.rs", 1, SAVED_A);
    let b = input(&fixture.root, "b.rs", 2, DOCUMENT_B);
    let _ = model.sync_workspace(
        [a.clone()],
        Some(1),
        wake_factory(sender, Arc::clone(&starts)),
    );
    settle_view(&mut model, &mut driver)?;
    let _ = model.sync_workspace([a.clone(), b.clone()], Some(2), |_| Arc::new(|| {}));
    settle_view(&mut model, &mut driver)?;
    let session = model.session.as_ref().ok_or("workspace")?;
    assert_eq!(session.identity.document_id, 2);
    assert!(session.diagnostic_pull.pending.is_none());
    let before = model.snapshot().process_submitted_inputs;
    // No poll, timer, input edit or background wake occurs between these two
    // observations. Reconciliation itself must admit the uncached A request.
    let _ = model.sync_workspace([a.clone(), b.clone()], Some(1), |_| Arc::new(|| {}));
    assert_eq!(model.snapshot().process_submitted_inputs, before + 1);
    let session = model.session.as_ref().ok_or("workspace")?;
    assert_eq!(session.identity.document_id, 1);
    assert!(session.diagnostic_pull.pending.is_some());
    settle_view(&mut model, &mut driver)?;
    let settled = model.snapshot().process_submitted_inputs;
    for _ in 0..100 {
        let _ = model.sync_workspace([a.clone(), b.clone()], Some(1), |_| Arc::new(|| {}));
    }
    assert_eq!(model.snapshot().process_submitted_inputs, settled);
    let records = wait_for_wire(&mut model, &mut driver, &log, |records| {
        records
            .iter()
            .filter(|row| is_document_message(row, "textDocument/diagnostic", "/a.rs"))
            .count()
            == 2
    })?;
    assert!(
        !records
            .iter()
            .any(|row| row["message"]["method"] == "textDocument/didClose")
    );
    assert_eq!(starts.load(Ordering::SeqCst), 1);
    assert!(!model.shutdown().active);
    Ok(())
}

fn settle_view(model: &mut RustDiagnostics, driver: &mut WireDriver) -> Result<(), Box<dyn Error>> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        // Drain actual wake delivery before declaring the fixture quiescent.
        match driver.next_wake(Duration::ZERO) {
            Ok(wake) => driver.continuation = model.poll(wake).continuation,
            Err(RecvTimeoutError::Disconnected) => {
                return Err(RecvTimeoutError::Disconnected.into());
            }
            Err(RecvTimeoutError::Timeout) => {
                let snapshot = model.snapshot();
                let ready = model.session.as_ref().is_some_and(|session| {
                    session.diagnostics.is_some() && session.diagnostic_pull.pending.is_none()
                });
                if ready
                    && !snapshot.overlay_write_pending
                    && snapshot.process_queued_events == 0
                    && snapshot.process_submitted_inputs == snapshot.process_written_inputs
                {
                    return Ok(());
                }
                match driver.next_wake(Duration::from_millis(10)) {
                    Ok(wake) => driver.continuation = model.poll(wake).continuation,
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(error) => return Err(error.into()),
                }
            }
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("diagnostic view did not settle: {:?}", model.snapshot()),
            )
            .into());
        }
    }
}
