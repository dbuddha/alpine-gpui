use super::*;

#[test]
fn saved_notifications_follow_real_open_and_change_writes() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new()?;
    let log = fixture.root.join("save-wire.jsonl");
    let server = fixture.root.join("save-server");
    fs::write(&log, "")?;
    fs::write(fixture.root.join("a.rs"), SAVED_A)?;
    fs::write(fixture.root.join("b.rs"), DOCUMENT_B)?;
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
    let mut a = input(&fixture.root, "a.rs", 1, SAVED_A);
    let b = input(&fixture.root, "b.rs", 2, DOCUMENT_B);
    let _ = model.sync_workspace(
        [a.clone(), b.clone()],
        Some(2),
        wake_factory(sender, Arc::clone(&starts)),
    );
    for _ in 0..100 {
        let _ = model.record_saved_document(a.identity);
    }
    let records = wait_for_wire(&mut model, &mut driver, &log, |records| {
        records
            .iter()
            .any(|row| is_document_message(row, "textDocument/didSave", "/a.rs"))
    })?;
    let saved = records
        .iter()
        .position(|row| is_document_message(row, "textDocument/didSave", "/a.rs"))
        .ok_or("save missing")?;
    for suffix in ["/a.rs", "/b.rs"] {
        assert!(records[..saved].iter().any(|row| is_document_message(
            row,
            "textDocument/didOpen",
            suffix
        )));
    }
    let mut buffer = Buffer::new(SAVED_A);
    let mut transaction = alpine_text::Transaction::new(buffer.revision());
    transaction.replace(0..SAVED_A.len(), UNSAVED_A)?;
    let _ = buffer.apply(transaction)?;
    a.snapshot = buffer.snapshot();
    a.identity.buffer_revision = buffer.revision().get();
    fs::write(fixture.root.join("a.rs"), UNSAVED_A)?;
    let _ = model.sync_workspace([a.clone(), b], Some(2), |_| Arc::new(|| {}));
    let _ = model.record_saved_document(a.identity);
    let records = wait_for_wire(&mut model, &mut driver, &log, |records| {
        records
            .iter()
            .filter(|row| is_document_message(row, "textDocument/didSave", "/a.rs"))
            .count()
            == 2
    })?;
    let changed = records
        .iter()
        .position(|row| is_document_message(row, "textDocument/didChange", "/a.rs"))
        .ok_or("change missing")?;
    let saved = records
        .iter()
        .rposition(|row| is_document_message(row, "textDocument/didSave", "/a.rs"))
        .ok_or("second save missing")?;
    assert!(changed < saved);
    assert_eq!(
        records[changed]["message"]["params"]["contentChanges"][0]["text"],
        UNSAVED_A
    );
    for row in records
        .iter()
        .filter(|row| is_document_message(row, "textDocument/didSave", "/a.rs"))
    {
        let document = &row["message"]["params"]["textDocument"];
        assert!(document.get("version").is_none());
        assert!(row["message"]["params"].get("text").is_none());
    }
    assert!(
        !records
            .iter()
            .any(|row| row["message"]["method"] == "textDocument/didClose")
    );
    assert_eq!(starts.load(Ordering::SeqCst), 1);
    assert!(!model.shutdown().active);
    Ok(())
}
