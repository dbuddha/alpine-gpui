//! Real-client save/close ordering, not physical Studio or compiler acceptance.

use super::{
    Arc, AtomicUsize, Buffer, DOCUMENT_B, Duration, Error, Fixture, Instant, LanguageWake,
    MOCK_SERVER, Ordering, PathBuf, RecvTimeoutError, RustDiagnostics, RustDocumentInput, SAVED_A,
    SyncSender, UNSAVED_A, Value, WireDriver, fs, input, io, is_document_message, mpsc,
    wait_for_wire, wake_factory,
};
use std::os::unix::fs::PermissionsExt;

struct SaveCloseCase {
    fixture: Fixture,
    log: PathBuf,
    model: RustDiagnostics,
    sender: SyncSender<LanguageWake>,
    driver: WireDriver,
    starts: Arc<AtomicUsize>,
    a: RustDocumentInput,
    b: RustDocumentInput,
}

impl SaveCloseCase {
    fn new() -> Result<Self, Box<dyn Error>> {
        let fixture = Fixture::new()?;
        let log = fixture.root.join("wire.jsonl");
        let server = fixture.root.join("mock-rust-analyzer");
        fs::write(&log, "")?;
        fs::write(fixture.root.join("a.rs"), SAVED_A)?;
        fs::write(fixture.root.join("b.rs"), DOCUMENT_B)?;
        let log_path = serde_json::to_string(
            log.to_str()
                .ok_or_else(|| io::Error::other("fixture path is not UTF-8"))?,
        )?;
        fs::write(
            &server,
            format!("#!/usr/bin/env python3\nLOG_PATH = {log_path}\n{MOCK_SERVER}"),
        )?;
        fs::set_permissions(&server, fs::Permissions::from_mode(0o700))?;
        let (sender, receiver) = mpsc::sync_channel(32);
        let mut driver = WireDriver::new(receiver);
        let starts = Arc::new(AtomicUsize::new(0));
        let a = input(&fixture.root, "a.rs", 1, SAVED_A);
        let b = input(&fixture.root, "b.rs", 2, DOCUMENT_B);
        let mut model = RustDiagnostics {
            server_path: Some(server),
            ..RustDiagnostics::default()
        };
        let _ = model.sync_workspace(
            [a.clone(), b.clone()],
            Some(1),
            wake_factory(sender.clone(), Arc::clone(&starts)),
        );
        let _ = wait_for_wire(&mut model, &mut driver, &log, |records| {
            records
                .iter()
                .any(|record| is_document_message(record, "textDocument/didOpen", "/b.rs"))
        })?;
        settle_workspace(&mut model, &mut driver)?;
        Ok(Self {
            fixture,
            log,
            model,
            sender,
            driver,
            starts,
            a,
            b,
        })
    }

    fn queue_saved_change(&mut self, active: bool) -> Result<(), Box<dyn Error>> {
        let mut buffer = Buffer::new(SAVED_A);
        let mut transaction = alpine_text::Transaction::new(buffer.revision());
        transaction.replace(0..0, "// saved before close\n")?;
        let _ = buffer.apply(transaction)?;
        self.a.snapshot = buffer.snapshot();
        self.a.identity.buffer_revision = buffer.revision().get();
        fs::write(self.fixture.root.join("a.rs"), self.a.snapshot.text())?;
        let _ = self.model.sync_workspace(
            [self.a.clone(), self.b.clone()],
            Some(if active { 1 } else { 2 }),
            wake_factory(self.sender.clone(), Arc::clone(&self.starts)),
        );
        assert!(self.model.snapshot().overlay_write_pending);
        // Do not poll between these production calls. The earlier didChange's
        // writer acknowledgement cannot clear the accepted save's ownership.
        let _ = self.model.record_saved_document(self.a.identity);
        Ok(())
    }

    fn save_then_close(&mut self, active: bool) -> Result<Vec<Value>, Box<dyn Error>> {
        self.queue_saved_change(active)?;
        let _ = self.model.sync_workspace(
            [self.b.clone()],
            Some(2),
            wake_factory(self.sender.clone(), Arc::clone(&self.starts)),
        );
        wait_for_wire(&mut self.model, &mut self.driver, &self.log, |records| {
            records
                .iter()
                .any(|record| is_document_message(record, "textDocument/didClose", "/a.rs"))
        })
    }

    fn queue_unrelated_writer(&mut self, active: bool) -> Result<(), Box<dyn Error>> {
        let mut buffer = Buffer::new(DOCUMENT_B);
        let mut transaction = alpine_text::Transaction::new(buffer.revision());
        transaction.replace(0..0, "// hold the other document's writer\n")?;
        let _ = buffer.apply(transaction)?;
        self.b.snapshot = buffer.snapshot();
        self.b.identity.buffer_revision = buffer.revision().get();
        let _ = self.model.sync_workspace(
            [self.a.clone(), self.b.clone()],
            Some(if active { 1 } else { 2 }),
            wake_factory(self.sender.clone(), Arc::clone(&self.starts)),
        );
        assert!(self.model.snapshot().overlay_write_pending);
        Ok(())
    }

    fn replace_saved_owner(
        &mut self,
        active: bool,
        reincarnate: bool,
    ) -> Result<(Vec<Value>, RustDocumentInput), Box<dyn Error>> {
        self.queue_unrelated_writer(active)?;
        // No poll may acknowledge B before the old A text, save and replacement
        // are reconciled. This exercises unsent text, not just an in-flight save.
        self.queue_saved_change(active)?;
        let file = if reincarnate { "a.rs" } else { "renamed.rs" };
        let id = if reincarnate { 3 } else { 1 };
        let mut replacement = input(&self.fixture.root, file, id, UNSAVED_A);
        replacement.identity.document_revision = self.a.identity.document_revision + 1;
        if !reincarnate {
            fs::write(&replacement.path, self.a.snapshot.text())?;
        }
        let _ = self.model.sync_workspace(
            [replacement.clone(), self.b.clone()],
            Some(if active { id } else { 2 }),
            wake_factory(self.sender.clone(), Arc::clone(&self.starts)),
        );
        let suffix = if reincarnate { "/a.rs" } else { "/renamed.rs" };
        let count = if reincarnate { 2 } else { 1 };
        let records = wait_for_wire(&mut self.model, &mut self.driver, &self.log, |records| {
            records
                .iter()
                .filter(|record| is_document_message(record, "textDocument/didOpen", suffix))
                .count()
                == count
        })?;
        settle_workspace(&mut self.model, &mut self.driver)?;
        Ok((records, replacement))
    }

    fn reopen(&mut self) -> Result<Vec<Value>, Box<dyn Error>> {
        let reopened = input(&self.fixture.root, "a.rs", 3, &self.a.snapshot.text());
        let _ = self.model.sync_workspace(
            [reopened, self.b.clone()],
            Some(3),
            wake_factory(self.sender.clone(), Arc::clone(&self.starts)),
        );
        wait_for_wire(&mut self.model, &mut self.driver, &self.log, |records| {
            records
                .iter()
                .filter(|record| is_document_message(record, "textDocument/didOpen", "/a.rs"))
                .count()
                == 2
        })
    }
}

fn settle_workspace(
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
            return Err(io::Error::new(io::ErrorKind::TimedOut, "workspace did not settle").into());
        }
        let wake = match driver.next_wake(remaining.min(Duration::from_millis(10))) {
            Ok(wake) => wake,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(error) => return Err(error.into()),
        };
        driver.continuation = model.poll(wake).continuation;
    }
    Ok(())
}

fn saved_close_replay(active: bool) -> Result<(), Box<dyn Error>> {
    let mut case = SaveCloseCase::new()?;
    let closed = case.save_then_close(active)?;
    let reopened = case.reopen()?;
    let snapshot = case.model.snapshot();
    assert!(!case.model.shutdown().active);
    assert_eq!(case.starts.load(Ordering::SeqCst), 1);
    assert_eq!(snapshot.restarts, 0);
    assert_eq!(case.model.snapshot().overlay_retained_text_bytes, 0);
    assert_eq!(case.model.snapshot().overlay_reserved_text_bytes, 0);
    for records in [&closed, &reopened] {
        assert!(
            !records
                .iter()
                .any(|record| { is_document_message(record, "textDocument/didClose", "/b.rs") })
        );
    }
    let methods = |records: &[Value]| -> Vec<String> {
        records
            .iter()
            .filter(|record| {
                record["message"]["method"].as_str().is_some_and(|method| {
                    matches!(
                        method,
                        "textDocument/didOpen"
                            | "textDocument/didChange"
                            | "textDocument/didSave"
                            | "textDocument/didClose"
                    )
                })
            })
            .filter(|record| {
                record["message"]["params"]["textDocument"]["uri"]
                    .as_str()
                    .is_some_and(|uri| uri.ends_with("/a.rs"))
            })
            .filter_map(|record| record["message"]["method"].as_str().map(str::to_owned))
            .collect()
    };
    assert_eq!(
        methods(&closed),
        [
            "textDocument/didOpen",
            "textDocument/didChange",
            "textDocument/didSave",
            "textDocument/didClose"
        ],
        "accepted save must reach the same workspace before its document closes",
    );
    assert_eq!(
        methods(&reopened),
        [
            "textDocument/didOpen",
            "textDocument/didChange",
            "textDocument/didSave",
            "textDocument/didClose",
            "textDocument/didOpen"
        ],
        "a new incarnation must not overtake its prior owner's save and close",
    );
    Ok(())
}

#[test]
fn saved_close_preserves_active_document_save_before_close() -> Result<(), Box<dyn Error>> {
    saved_close_replay(true)
}

#[test]
fn saved_close_preserves_parked_document_save_before_close() -> Result<(), Box<dyn Error>> {
    saved_close_replay(false)
}

// The fixtures are deliberately ASCII. This independent String oracle accepts
// both full and incremental LSP updates without reusing Alpine's text conversion.
fn wire_offset(text: &str, position: &Value) -> Result<usize, Box<dyn Error>> {
    if !text.is_ascii() {
        return Err("wire fixture oracle requires ASCII text".into());
    }
    let target_line = usize::try_from(position["line"].as_u64().ok_or("line missing")?)?;
    let column = usize::try_from(position["character"].as_u64().ok_or("column missing")?)?;
    let mut offset = 0;
    for (line_index, line) in text.split('\n').enumerate() {
        if line_index == target_line {
            return if column <= line.len() {
                Ok(offset + column)
            } else {
                Err("wire column exceeds line".into())
            };
        }
        offset += line.len() + 1;
    }
    Err("wire line exceeds document".into())
}

fn changed_wire_text(initial: &str, record: &Value) -> Result<String, Box<dyn Error>> {
    let mut text = initial.to_owned();
    let changes = record["message"]["params"]["contentChanges"]
        .as_array()
        .ok_or("changes missing")?;
    if changes.is_empty() {
        return Err("empty document change".into());
    }
    for change in changes {
        let next = change["text"].as_str().ok_or("changed text missing")?;
        if let Some(range) = change.get("range") {
            let start = wire_offset(&text, &range["start"])?;
            let end = wire_offset(&text, &range["end"])?;
            if start > end {
                return Err("reversed wire range".into());
            }
            text.replace_range(start..end, next);
        } else {
            next.clone_into(&mut text);
        }
    }
    Ok(text)
}

fn document_records<'a>(records: &'a [Value], suffixes: &[&str]) -> Vec<&'a Value> {
    records
        .iter()
        .filter(|record| {
            record["message"]["method"].as_str().is_some_and(|method| {
                matches!(
                    method,
                    "textDocument/didOpen"
                        | "textDocument/didChange"
                        | "textDocument/didSave"
                        | "textDocument/didClose"
                )
            }) && record["message"]["params"]["textDocument"]["uri"]
                .as_str()
                .is_some_and(|uri| suffixes.iter().any(|suffix| uri.ends_with(suffix)))
        })
        .collect()
}

fn assert_replacement_wire(
    records: &[Value],
    replacement_suffix: &str,
    saved_text: &str,
    other_text: &str,
) -> Result<(), Box<dyn Error>> {
    let old_and_new = document_records(records, &["/a.rs", "/renamed.rs"]);
    let methods: Vec<_> = old_and_new
        .iter()
        .map(|record| record["message"]["method"].as_str().unwrap_or(""))
        .collect();
    assert_eq!(
        methods,
        [
            "textDocument/didOpen",
            "textDocument/didChange",
            "textDocument/didSave",
            "textDocument/didClose",
            "textDocument/didOpen"
        ],
        "replacement must not drop or overtake the prior owner's text/save/close"
    );
    assert!(old_and_new[..4].iter().all(|record| {
        record["message"]["params"]["textDocument"]["uri"]
            .as_str()
            .is_some_and(|uri| uri.ends_with("/a.rs"))
    }));
    assert!(is_document_message(
        old_and_new[4],
        "textDocument/didOpen",
        replacement_suffix
    ));
    assert_eq!(
        old_and_new[0]["message"]["params"]["textDocument"]["text"],
        SAVED_A
    );
    assert_eq!(changed_wire_text(SAVED_A, old_and_new[1])?, saved_text);
    assert_eq!(
        old_and_new[1]["message"]["params"]["textDocument"]["version"],
        2
    );
    assert!(
        old_and_new[2]["message"]["params"]["textDocument"]
            .get("version")
            .is_none()
    );
    assert_eq!(
        old_and_new[4]["message"]["params"]["textDocument"]["version"],
        1
    );
    assert_eq!(
        old_and_new[4]["message"]["params"]["textDocument"]["text"],
        UNSAVED_A
    );
    let other = document_records(records, &["/b.rs"]);
    let other_methods: Vec<_> = other
        .iter()
        .map(|record| record["message"]["method"].as_str().unwrap_or(""))
        .collect();
    assert_eq!(
        other_methods,
        ["textDocument/didOpen", "textDocument/didChange"]
    );
    assert_eq!(changed_wire_text(DOCUMENT_B, other[1])?, other_text);
    let held = records
        .iter()
        .position(|record| is_document_message(record, "textDocument/didChange", "/b.rs"))
        .ok_or("writer holder missing")?;
    let changed = records
        .iter()
        .position(|record| is_document_message(record, "textDocument/didChange", "/a.rs"))
        .ok_or("old change missing")?;
    assert!(
        held < changed,
        "the other document must occupy the writer first"
    );
    Ok(())
}

fn saved_replacement_replay(active: bool, reincarnate: bool) -> Result<(), Box<dyn Error>> {
    let mut case = SaveCloseCase::new()?;
    let initial = case.model.snapshot();
    let (records, replacement) = case.replace_saved_owner(active, reincarnate)?;
    let live = case.model.snapshot();
    let active_id = case
        .model
        .session
        .as_ref()
        .ok_or("session lost")?
        .identity
        .document_id;
    let terminal = case.model.shutdown();
    let terminal_records = fs::read_to_string(&case.log)?
        .lines()
        .map(serde_json::from_str)
        .collect::<Result<Vec<Value>, _>>()?;
    let suffix = if reincarnate { "/a.rs" } else { "/renamed.rs" };
    assert_replacement_wire(
        &records,
        suffix,
        &case.a.snapshot.text(),
        &case.b.snapshot.text(),
    )?;
    let pid = records.first().ok_or("wire empty")?["pid"]
        .as_u64()
        .ok_or("PID missing")?;
    assert!(
        terminal_records
            .iter()
            .all(|record| record["pid"].as_u64() == Some(pid))
    );
    assert_eq!(case.starts.load(Ordering::SeqCst), 1);
    assert_eq!(live.process_starts, 1);
    assert_eq!(live.restarts, 0);
    assert_eq!(live.generation, initial.generation);
    assert_eq!(live.overlay_documents, 2);
    assert_eq!(
        active_id,
        if active {
            replacement.identity.document_id
        } else {
            2
        }
    );
    assert!(!terminal.active);
    assert_eq!(terminal.overlay_retained_text_bytes, 0);
    assert_eq!(terminal.overlay_reserved_text_bytes, 0);
    let teardown: Vec<_> = terminal_records
        .iter()
        .filter_map(|record| record["message"]["method"].as_str())
        .filter(|method| matches!(*method, "shutdown" | "exit"))
        .collect();
    assert_eq!(teardown, ["shutdown", "exit"]);
    assert_eq!(fs::read_to_string(&case.a.path)?, case.a.snapshot.text());
    assert_eq!(
        fs::read_to_string(&replacement.path)?,
        case.a.snapshot.text()
    );
    assert_eq!(fs::read_to_string(&case.b.path)?, DOCUMENT_B);
    let outcome = serde_json::json!({
        "active": active, "same_uri_new_id": reincarnate,
        "old_text_save_close_before_replacement_open": true,
        "unrelated_writer_preceded_old_text": true,
        "replacement_wire_version": 1, "process_starts": 1,
        "pid": pid, "graceful_shutdown_and_exit": true,
        "physical_qualification": false, "full_workspace_acceptance": false
    });
    fs::write(
        case.fixture.root.join("replacement-outcome.json"),
        serde_json::to_vec_pretty(&outcome)?,
    )?;
    eprintln!("{outcome}");
    Ok(())
}

#[test]
fn saved_replacement_preserves_active_path_wire_order() -> Result<(), Box<dyn Error>> {
    saved_replacement_replay(true, false)
}

#[test]
fn saved_replacement_preserves_parked_path_wire_order() -> Result<(), Box<dyn Error>> {
    saved_replacement_replay(false, false)
}

#[test]
fn saved_replacement_preserves_active_reincarnation_wire_order() -> Result<(), Box<dyn Error>> {
    saved_replacement_replay(true, true)
}

#[test]
fn saved_replacement_preserves_parked_reincarnation_wire_order() -> Result<(), Box<dyn Error>> {
    saved_replacement_replay(false, true)
}

#[test]
fn replacement_wire_oracle_accepts_full_incremental_and_terminal_edits()
-> Result<(), Box<dyn Error>> {
    for (changes, expected) in [
        (
            serde_json::json!([{"text": "replacement\n"}]),
            "replacement\n",
        ),
        (
            serde_json::json!([{
                "range": {"start": {"line": 0, "character": 1}, "end": {"line": 1, "character": 1}},
                "text": "-\n"
            }]),
            "a-\nd\n",
        ),
        (
            serde_json::json!([{
                "range": {"start": {"line": 2, "character": 0}, "end": {"line": 2, "character": 0}},
                "text": "ef"
            }]),
            "ab\ncd\nef",
        ),
    ] {
        let record = serde_json::json!({"message": {"params": {"contentChanges": changes}}});
        assert_eq!(changed_wire_text("ab\ncd\n", &record)?, expected);
    }
    Ok(())
}

#[test]
fn replacement_wire_oracle_rejects_malformed_and_out_of_range_edits() {
    for changes in [
        serde_json::json!([]),
        serde_json::json!({}),
        serde_json::json!([{}]),
        serde_json::json!([{"range": {"start": {"line": 0, "character": 3}, "end": {"line": 0, "character": 3}}, "text": ""}]),
        serde_json::json!([{"range": {"start": {"line": 1, "character": 1}, "end": {"line": 0, "character": 1}}, "text": ""}]),
        serde_json::json!([{"range": {"start": {"line": 3, "character": 0}, "end": {"line": 3, "character": 0}}, "text": ""}]),
        serde_json::json!([{"range": {"start": {"line": 0}, "end": {"line": 0, "character": 0}}, "text": ""}]),
    ] {
        let record = serde_json::json!({"message": {"params": {"contentChanges": changes}}});
        assert!(changed_wire_text("ab\ncd\n", &record).is_err());
    }
    let ranged = serde_json::json!({"message": {"params": {"contentChanges": [{
        "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 0}},
        "text": ""
    }]}}});
    assert!(changed_wire_text("\u{03bb}\n", &ranged).is_err());
}
