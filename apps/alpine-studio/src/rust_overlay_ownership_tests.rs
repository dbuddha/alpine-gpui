//! Bounded ownership transitions; physical LSP and native acceptance are separate.

use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
struct Owner {
    id: u64,
    path: PathBuf,
    version: i32,
    revision: u64,
    text: String,
    synced: String,
    opened: bool,
    changed: bool,
    save: Option<(u64, Option<InputSequence>)>,
}

#[derive(Debug, Eq, PartialEq)]
struct ClosingOwner {
    uri: String,
    opened: bool,
    save: Option<(u64, Option<InputSequence>)>,
    text: Option<(String, String)>,
}

#[derive(Debug, Eq, PartialEq)]
struct Ownership {
    active: Owner,
    parked: Vec<Owner>,
    closing: Vec<ClosingOwner>,
    active_view: bool,
}

fn save_state(save: Option<&PendingSave>) -> Option<(u64, Option<InputSequence>)> {
    save.map(|save| (save.buffer_revision, save.submitted))
}

fn ownership(session: &RustSession) -> Ownership {
    Ownership {
        active: Owner {
            id: session.identity.document_id,
            path: session.target.path.clone(),
            version: session.lsp_version,
            revision: session.identity.buffer_revision,
            text: session.snapshot.text(),
            synced: session.synced_snapshot.text(),
            opened: session.document_opened,
            changed: session.pending_change,
            save: save_state(session.pending_save.as_ref()),
        },
        parked: session
            .parked
            .iter()
            .map(|document| Owner {
                id: document.identity.document_id,
                path: document.target.path.clone(),
                version: document.lsp_version,
                revision: document.identity.buffer_revision,
                text: document.snapshot.text(),
                synced: document.synced_snapshot.text(),
                opened: document.opened,
                changed: document.pending_change,
                save: save_state(document.pending_save.as_ref()),
            })
            .collect(),
        closing: session
            .overlay_closes
            .iter()
            .map(|document| ClosingOwner {
                uri: document.document.uri().to_owned(),
                opened: document.opened,
                save: save_state(document.pending_save.as_ref()),
                text: document
                    .pending_text
                    .as_ref()
                    .map(|(current, synced)| (current.text(), synced.text())),
            })
            .collect(),
        active_view: session.active_view,
    }
}

fn input(id: u64) -> RustDocumentInput {
    let snapshot = alpine_text::Buffer::new("fn main() {}\n").snapshot();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("overlay-ownership-fixture");
    RustDocumentInput::new(
        &root.join(format!("alpine-owner-{id}.rs")),
        &root,
        LanguageIdentity {
            workspace_id: 1,
            workspace_revision: 1,
            document_id: id,
            document_revision: 1,
            buffer_revision: snapshot.revision().get(),
            selection_revision: 1,
        },
        snapshot,
    )
}

fn session() -> Result<RustSession, Box<dyn Error>> {
    let input = input(1);
    let document = LspDocument::from_file_path(&input.path, "rust", 1)?;
    let batch = crate::lsp_language::DiagnosticBatch::from_saved_items(document.uri(), &[])?;
    let mut session =
        super::super::super::test_session(input, document, batch, Path::new("/inert"));
    session.document_opened = false;
    session.pending_change = false;
    Ok(session)
}

fn owner_uri(id: u64) -> Result<String, Box<dyn Error>> {
    Ok(LspDocument::from_file_path(&input(id).path, "rust", 1)?
        .uri()
        .to_owned())
}

#[test]
fn ownership_fixture_paths_are_absolute_and_owner_uris_are_distinct() -> Result<(), Box<dyn Error>>
{
    assert!(input(1).path.is_absolute());
    assert!(input(2).path.is_absolute());
    assert_ne!(owner_uri(1)?, owner_uri(2)?);
    Ok(())
}

fn unsent_save(revision: u64) -> PendingSave {
    PendingSave {
        buffer_revision: revision,
        submitted: None,
    }
}

fn fill_closes(session: &mut RustSession, count: usize) -> Result<(), Box<dyn Error>> {
    for offset in 0..count {
        let mut document = ParkedDocument::new(input(100 + u64::try_from(offset)?))?;
        document.opened = true;
        session
            .overlay_closes
            .push(ClosingDocument::from_parked(document));
    }
    Ok(())
}

fn changed_input(id: u64) -> Result<RustDocumentInput, Box<dyn Error>> {
    let mut input = input(id);
    let mut buffer = alpine_text::Buffer::new(&input.snapshot.text());
    let mut transaction = alpine_text::Transaction::new(buffer.revision());
    transaction.replace(0..0, "// changed while inactive\n")?;
    let _ = buffer.apply(transaction)?;
    input.snapshot = buffer.snapshot();
    input.identity.buffer_revision = buffer.revision().get();
    Ok(input)
}

#[test]
fn parking_preserves_only_owned_transport_intents() -> Result<(), Box<dyn Error>> {
    for opened in [false, true] {
        for saved in [false, true] {
            for retain in [false, true] {
                let mut session = session()?;
                session.document_opened = opened;
                session.pending_save = saved.then(|| unsent_save(0));
                let uri = session.document.uri().to_owned();
                session.park_and_activate(input(2), retain)?;
                assert_eq!(session.identity.document_id, 2);
                assert!(session.pending_save.is_none());
                assert_eq!(session.parked.len(), usize::from(retain));
                assert_eq!(
                    session.overlay_closes.len(),
                    usize::from(!retain && (opened || saved))
                );
                if retain {
                    let previous = &session.parked[0];
                    assert_eq!(previous.identity.document_id, 1);
                    assert_eq!(previous.opened, opened);
                    assert_eq!(previous.pending_save.is_some(), saved);
                } else if opened || saved {
                    let previous = &session.overlay_closes[0];
                    assert_eq!(previous.document.uri(), uri);
                    assert_eq!(previous.opened, opened);
                    assert_eq!(previous.pending_save.is_some(), saved);
                    assert_eq!(previous.pending_text.is_some(), saved && !opened);
                }
            }
        }
    }
    Ok(())
}

#[test]
fn parking_close_capacity_is_checked_before_state_moves() -> Result<(), Box<dyn Error>> {
    for opened in [false, true] {
        for saved in [false, true] {
            for retain in [false, true] {
                let mut session = session()?;
                session.document_opened = opened;
                session.pending_save = saved.then(|| unsent_save(0));
                fill_closes(&mut session, MAX_OVERLAY_DOCUMENTS)?;
                let before = ownership(&session);
                let result = session.park_and_activate(input(2), retain);
                if !retain && (opened || saved) {
                    assert!(matches!(result, Err(RustDiagnosticsError::OverlayBudget)));
                    assert_eq!(ownership(&session), before);
                } else {
                    result?;
                    assert_eq!(session.identity.document_id, 2);
                    assert_eq!(session.parked.len(), usize::from(retain));
                    assert_eq!(ownership(&session).closing, before.closing);
                }
            }
        }
    }
    Ok(())
}

#[test]
fn parking_document_capacity_distinguishes_replacement_from_retention() -> Result<(), Box<dyn Error>>
{
    let mut session = session()?;
    for id in 2..=u64::try_from(MAX_OVERLAY_DOCUMENTS)? {
        assert!(session.update_parked(input(id))?);
    }
    let before = ownership(&session);
    assert!(matches!(
        session.park_and_activate(input(33), true),
        Err(RustDiagnosticsError::OverlayBudget)
    ));
    assert_eq!(ownership(&session), before);
    session.park_and_activate(input(33), false)?;
    assert_eq!(session.identity.document_id, 33);
    assert_eq!(session.parked.len() + 1, MAX_OVERLAY_DOCUMENTS);
    assert!(session.overlay_closes.is_empty());
    session.park_and_activate(input(2), true)?;
    assert_eq!(session.identity.document_id, 2);
    assert_eq!(session.parked.len() + 1, MAX_OVERLAY_DOCUMENTS);
    assert!(
        session
            .parked
            .iter()
            .any(|owner| owner.identity.document_id == 33)
    );
    assert!(
        session
            .parked
            .iter()
            .all(|owner| owner.identity.document_id != 2)
    );
    Ok(())
}

#[test]
fn parked_version_exhaustion_does_not_move_owners() -> Result<(), Box<dyn Error>> {
    let mut session = session()?;
    assert!(session.update_parked(input(2))?);
    session.parked[0].lsp_version = i32::MAX;
    session.parked[0].document.set_version(i32::MAX);
    let before = ownership(&session);
    assert!(matches!(
        session.park_and_activate(changed_input(2)?, true),
        Err(RustDiagnosticsError::VersionExhausted)
    ));
    assert_eq!(ownership(&session), before);
    session.park_and_activate(input(2), true)?;
    assert_eq!(session.identity.document_id, 2);
    assert_eq!(session.lsp_version, i32::MAX);
    assert_eq!(session.parked.len(), 1);
    assert_eq!(session.parked[0].identity.document_id, 1);
    Ok(())
}

#[test]
fn updating_parked_version_exhaustion_preserves_text() -> Result<(), Box<dyn Error>> {
    let mut session = session()?;
    assert!(session.update_parked(input(2))?);
    session.parked[0].lsp_version = i32::MAX;
    session.parked[0].document.set_version(i32::MAX);
    let before = ownership(&session);
    assert!(matches!(
        session.update_parked(changed_input(2)?),
        Err(RustDiagnosticsError::VersionExhausted)
    ));
    assert_eq!(ownership(&session), before);
    assert!(!session.update_parked(input(2))?);
    assert_eq!(ownership(&session), before);

    // Exercise the production roster boundary, not just the parked helper.
    // The successful case proves the observer sees real framed submissions;
    // the rejected case must not publish A before B's version check fails.
    for exhausted in [false, true] {
        let (mut model, mut first, root) = installed_workspace()?;
        model.initialize_installed_transport_for_test()?;
        let mut second = first.clone();
        second.path = root.join("second.rs");
        second.identity.document_id += 1;
        let session = model.session.as_mut().ok_or("installed workspace")?;
        assert!(session.update_parked(second.clone())?);
        session.parked[0].opened = true;
        if exhausted {
            session.parked[0].lsp_version = i32::MAX;
            session.parked[0].document.set_version(i32::MAX);
        }
        assert!(session.document_opened);
        assert!(!session.pending_change);
        assert!(session.overlay_write.is_none());
        while session.client.take_input_for_test()?.is_some() {}
        let active_uri = session.document.uri().to_owned();
        let mut observer = session.client.take_input_observer_for_test()?;
        for document in [&mut first, &mut second] {
            let mut buffer = alpine_text::Buffer::new(&document.snapshot.text());
            let mut edit = alpine_text::Transaction::new(buffer.revision());
            edit.replace(0..0, "// newly admitted workspace edit\n")?;
            let _ = buffer.apply(edit)?;
            document.snapshot = buffer.snapshot();
            document.identity.buffer_revision = buffer.revision().get();
        }
        let effect = model.sync_workspace(
            [first.clone(), second],
            Some(first.identity.document_id),
            |_| Arc::new(|| {}),
        );
        let retained_session = model.session.is_some();
        let _ = model.stop();
        let mut messages = Vec::new();
        while let Some(bytes) = observer.take_input()? {
            assert!(messages.len() < 4, "bounded fixture output exceeded");
            let mut framer =
                crate::lsp_framing::LspFramer::new(crate::lsp_framing::LspFrameLimits::default());
            let batch = framer.ingest(&bytes)?;
            assert_eq!(batch.consumed(), bytes.len());
            assert_eq!(batch.frames().len(), 1);
            messages.push(serde_json::from_slice::<serde_json::Value>(
                batch.frames()[0].body(),
            )?);
            framer.finish()?;
        }
        let active_changes: Vec<_> = messages
            .iter()
            .filter(|message| {
                message["method"] == "textDocument/didChange"
                    && message["params"]["textDocument"]["uri"] == active_uri
            })
            .collect();
        let retained_bytes = observer.retained_bytes();
        std::fs::remove_dir_all(root)?;
        assert_eq!(retained_session, !exhausted);
        assert_eq!(retained_bytes, 0);
        if exhausted {
            assert!(effect.visual_changed);
            assert!(
                active_changes.is_empty(),
                "a rejected roster submitted active-document edits: {messages:?}"
            );
        } else {
            assert_eq!(active_changes.len(), 1, "observer missed accepted payload");
            let params = &active_changes[0]["params"];
            assert_eq!(params["textDocument"]["version"], 2);
            assert_eq!(params["contentChanges"][0]["text"], first.snapshot.text());
        }
    }
    Ok(())
}

// Execute retention cases in an owned process. A broken loop is a bounded
// regression failure, not an unbounded thread or whole-suite timeout.
fn owned_case(name: &str, body: fn() -> Result<(), Box<dyn Error>>) -> Result<(), Box<dyn Error>> {
    use std::io::Read;

    const CHILD: &str = "ALPINE_OVERLAY_OWNERSHIP_CHILD";
    let test = format!("rust_diagnostics::workspace::tests::overlay_ownership::{name}");
    let receipt = format!("alpine-owned-complete:{test}");
    if std::env::var(CHILD).ok().as_deref() == Some(test.as_str()) {
        body()?;
        println!("\n{receipt}");
        return Ok(());
    }
    let executable = std::env::current_exe()?;
    let started = std::time::Instant::now();
    let mut child = std::process::Command::new(&executable)
        .args(["--exact", &test, "--nocapture"])
        .env(CHILD, &test)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .spawn()?;
    eprintln!("owned regression started test={test} pid={}", child.id());
    let mut timed_out = false;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() >= std::time::Duration::from_secs(5) {
            timed_out = true;
            let _ = child.kill();
            break child.wait()?;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    // These exact child tests emit only libtest's bounded summary and this
    // receipt on stdout. Diagnostic observations and panics use inherited stderr.
    let mut output = String::new();
    child
        .stdout
        .take()
        .ok_or("missing owned regression stdout")?
        .take(4_097)
        .read_to_string(&mut output)?;
    print!("{output}");
    if timed_out {
        return Err(format!("owned regression {test} exceeded five seconds: {status}").into());
    }
    if !status.success() {
        return Err(format!("owned regression {test} failed: {status}").into());
    }
    if output.len() > 4_096 || !output.lines().any(|line| line == receipt) {
        return Err(format!("missing bounded completion receipt for {test}").into());
    }
    Ok(())
}

#[test]
fn owned_child_rejects_zero_selected_success() {
    let result = owned_case("absent_owned_case_negative_control", || Ok(()));
    assert!(result.err().is_some_and(|error| {
        error
            .to_string()
            .contains("missing bounded completion receipt")
    }));
}

#[test]
fn retention_preserves_order_and_only_closes_transport_owners() -> Result<(), Box<dyn Error>> {
    owned_case(
        "retention_preserves_order_and_only_closes_transport_owners",
        retention_membership,
    )
}

fn retention_membership() -> Result<(), Box<dyn Error>> {
    let mut session = session()?;
    for (id, opened, saved) in [
        (2, false, false),
        (3, true, false),
        (4, false, true),
        (5, false, false),
        (6, true, true),
    ] {
        let mut document = ParkedDocument::new(input(id))?;
        document.opened = opened;
        document.pending_save = saved.then(|| unsent_save(0));
        session.parked.push(document);
    }
    let before = ownership(&session);
    session.retain_overlays(&[1, 2, 6])?;
    let after = ownership(&session);
    assert_eq!(after.active, before.active);
    assert_eq!(
        after.parked,
        vec![before.parked[0].clone(), before.parked[4].clone()]
    );
    assert_eq!(session.overlay_closes.len(), 2);
    assert_eq!(session.overlay_closes[0].document.uri(), owner_uri(3)?);
    assert!(session.overlay_closes[0].pending_save.is_none());
    assert_eq!(session.overlay_closes[1].document.uri(), owner_uri(4)?);
    assert!(session.overlay_closes[1].pending_save.is_some());
    assert!(session.overlay_closes[1].pending_text.is_some());
    session.retain_overlays(&[1, 2, 6])?;
    assert_eq!(ownership(&session), after);
    Ok(())
}

#[test]
fn retention_checks_exact_close_capacity_before_removal() -> Result<(), Box<dyn Error>> {
    owned_case(
        "retention_checks_exact_close_capacity_before_removal",
        retention_capacity,
    )
}

fn retention_capacity() -> Result<(), Box<dyn Error>> {
    for opened in [false, true] {
        for saved in [false, true] {
            for retain in [false, true] {
                for count in [MAX_OVERLAY_DOCUMENTS - 1, MAX_OVERLAY_DOCUMENTS] {
                    let mut session = session()?;
                    fill_closes(&mut session, count)?;
                    let mut document = ParkedDocument::new(input(2))?;
                    document.opened = opened;
                    document.pending_save = saved.then(|| unsent_save(0));
                    session.parked.push(document);
                    let before = ownership(&session);
                    let retained = if retain { &[1, 2][..] } else { &[1][..] };
                    let closing = !retain && (opened || saved);
                    let result = session.retain_overlays(retained);
                    if closing && count == MAX_OVERLAY_DOCUMENTS {
                        assert!(matches!(result, Err(RustDiagnosticsError::OverlayBudget)));
                        assert_eq!(ownership(&session), before);
                    } else {
                        result?;
                        assert_eq!(session.parked.len(), usize::from(retain));
                        assert_eq!(session.overlay_closes.len(), count + usize::from(closing));
                        assert_eq!(ownership(&session).active, before.active);
                    }
                }
            }
        }
    }
    Ok(())
}

#[test]
fn inactive_roster_path_replacement_retires_the_previous_uri() -> Result<(), Box<dyn Error>> {
    owned_case(
        "inactive_roster_path_replacement_retires_the_previous_uri",
        || roster_path_replacement(1),
    )
}

#[test]
fn active_roster_path_replacement_retires_the_previous_uri() -> Result<(), Box<dyn Error>> {
    owned_case(
        "active_roster_path_replacement_retires_the_previous_uri",
        || roster_path_replacement(2),
    )
}

fn roster_path_replacement(active: u64) -> Result<(), Box<dyn Error>> {
    let (mut model, first, root) = installed_workspace()?;
    model.initialize_installed_transport_for_test()?;
    eprintln!(
        "path replacement phase=installed active={active} root={}",
        root.display()
    );
    model.session.as_mut().ok_or("workspace")?.overlay_write = Some(InputSequence::for_test(7));
    let mut second = first.clone();
    second.path = root.join("second.rs");
    second.identity.document_id = 2;
    let _ = model.sync_workspace([first.clone(), second.clone()], Some(1), |_| {
        Arc::new(|| {})
    });
    model.session.as_mut().ok_or("workspace")?.parked[0].opened = true;
    let _ = model.sync_workspace([first.clone(), second.clone()], Some(active), |_| {
        Arc::new(|| {})
    });
    let old_uri = LspDocument::from_file_path(&second.path, "rust", 1)?
        .uri()
        .to_owned();
    second.path = root.join("renamed.rs");
    second.identity.document_revision += 1;
    let expected = vec![
        (first.identity.document_id, first.path.clone()),
        (2, second.path.clone()),
    ];
    eprintln!("path replacement phase=reconcile active={active}");
    let _ = model.sync_workspace([first, second], Some(active), |_| Arc::new(|| {}));
    let observed = model.session.as_ref().map(|session| {
        let mut owners = vec![(session.identity.document_id, session.target.path.clone())];
        owners.extend(
            session
                .parked
                .iter()
                .map(|document| (document.identity.document_id, document.target.path.clone())),
        );
        owners.sort();
        let closing: Vec<_> = session
            .overlay_closes
            .iter()
            .map(|document| document.document.uri().to_owned())
            .collect();
        (owners, closing)
    });
    eprintln!("path replacement phase=observed active={active} owners={observed:?}");
    // This is an inert roster model, not a server shutdown qualification.
    // Retire its transport without awaiting replies that this fixture cannot send.
    let stopped = model.stop();
    eprintln!("path replacement phase=retired active={active} stopped={stopped}");
    std::fs::remove_dir_all(root)?;
    assert_eq!(
        observed,
        Some((expected, vec![old_uri])),
        "active document {active}"
    );
    Ok(())
}

#[test]
fn full_roster_path_replacement_preserves_save_and_capacity() -> Result<(), Box<dyn Error>> {
    owned_case(
        "full_roster_path_replacement_preserves_save_and_capacity",
        || full_roster_replacements(false),
    )
}

#[test]
fn full_roster_reincarnation_preserves_save_and_capacity() -> Result<(), Box<dyn Error>> {
    owned_case(
        "full_roster_reincarnation_preserves_save_and_capacity",
        || full_roster_replacements(true),
    )
}

fn full_roster_replacements(reincarnate: bool) -> Result<(), Box<dyn Error>> {
    for active in [1, 2] {
        full_roster_replacement(active, reincarnate)?;
    }
    Ok(())
}

fn installed_full_roster(
    active: u64,
) -> Result<(RustDiagnostics, Vec<RustDocumentInput>, PathBuf), Box<dyn Error>> {
    let (mut model, first, root) = installed_workspace()?;
    model.initialize_installed_transport_for_test()?;
    model.session.as_mut().ok_or("workspace")?.overlay_write = Some(InputSequence::for_test(7));
    let mut roster = vec![first.clone()];
    for id in 2..=u64::try_from(MAX_OVERLAY_DOCUMENTS)? {
        let mut next = first.clone();
        next.path = root.join(format!("document-{id}.rs"));
        next.identity.document_id = id;
        next.snapshot = alpine_text::Buffer::new(&format!("fn doc_{id}() {{}}\n")).snapshot();
        next.identity.buffer_revision = next.snapshot.revision().get();
        roster.push(next);
    }
    let _ = model.sync_workspace(roster.clone(), Some(1), |_| Arc::new(|| {}));
    for document in &mut model.session.as_mut().ok_or("workspace")?.parked {
        document.opened = true;
    }
    let _ = model.sync_workspace(roster.clone(), Some(active), |_| Arc::new(|| {}));
    let session = model.session.as_mut().ok_or("workspace")?;
    if active == 2 {
        session.pending_change = true;
        session.pending_save = Some(unsent_save(session.identity.buffer_revision));
    } else {
        let document = session
            .parked
            .iter_mut()
            .find(|document| document.identity.document_id == 2)
            .ok_or("missing saved owner")?;
        document.pending_change = true;
        document.pending_save = Some(unsent_save(document.identity.buffer_revision));
    }
    assert_eq!(session.parked.len() + 1, MAX_OVERLAY_DOCUMENTS);
    Ok((model, roster, root))
}

fn full_roster_replacement(active: u64, reincarnate: bool) -> Result<(), Box<dyn Error>> {
    let (mut model, mut roster, root) = installed_full_roster(active)?;
    let before = model.snapshot();
    let old_uri = LspDocument::from_file_path(&roster[1].path, "rust", 1)?
        .uri()
        .to_owned();
    if reincarnate {
        roster[1].identity.document_id = 33;
    } else {
        roster[1].path = root.join("renamed.rs");
    }
    roster[1].identity.document_revision += 1;
    let new_active = if active == 2 {
        roster[1].identity.document_id
    } else {
        active
    };
    let mut expected: Vec<_> = roster
        .iter()
        .map(|input| (input.identity.document_id, input.path.clone()))
        .collect();
    expected.sort();
    let _ = model.sync_workspace(roster, Some(new_active), |_| Arc::new(|| {}));
    let after = model.snapshot();
    let observed = model.session.as_ref().map(|session| {
        let mut owners = vec![(session.identity.document_id, session.target.path.clone())];
        owners.extend(
            session
                .parked
                .iter()
                .map(|document| (document.identity.document_id, document.target.path.clone())),
        );
        owners.sort();
        let closes: Vec<_> = session
            .overlay_closes
            .iter()
            .map(|document| {
                (
                    document.document.uri().to_owned(),
                    document.pending_save.is_some(),
                    document
                        .pending_text
                        .as_ref()
                        .map(|(current, _)| current.text()),
                )
            })
            .collect();
        (owners, closes)
    });
    eprintln!(
        "full roster active={active} reincarnate={reincarnate} owners={} starts={}",
        after.overlay_documents, after.process_starts
    );
    let _ = model.stop();
    std::fs::remove_dir_all(root)?;
    assert_eq!(
        observed,
        Some((
            expected,
            vec![(old_uri, true, Some("fn doc_2() {}\n".into()))]
        ))
    );
    assert_eq!(after.overlay_documents, MAX_OVERLAY_DOCUMENTS);
    assert_eq!(after.generation, before.generation);
    assert_eq!(after.process_starts, before.process_starts);
    assert_eq!(after.restarts, before.restarts);
    Ok(())
}

#[test]
fn document_owner_key_requires_both_id_and_path() {
    let owner = input(1);
    let other = input(2);
    for (id, path, expected) in [
        (owner.identity.document_id, owner.path.as_path(), true),
        (owner.identity.document_id, other.path.as_path(), false),
        (other.identity.document_id, owner.path.as_path(), false),
        (other.identity.document_id, other.path.as_path(), false),
    ] {
        assert_eq!(
            matches_document_owner(id, path, &owner),
            expected,
            "owner lookup id={id} path={path:?}"
        );
    }
}

#[test]
fn document_owner_key_survives_edit_and_view_revision_changes() {
    let owner = input(1);
    let mut changed = owner.clone();
    changed.identity.document_revision += 1;
    changed.identity.buffer_revision += 1;
    changed.identity.selection_revision += 1;
    assert!(matches_document_owner(
        owner.identity.document_id,
        &owner.path,
        &changed
    ));
    // Revision equality alone cannot join a replacement to the old owner.
    changed.identity = owner.identity;
    changed.path = input(2).path;
    assert!(!matches_document_owner(
        owner.identity.document_id,
        &owner.path,
        &changed
    ));
}
