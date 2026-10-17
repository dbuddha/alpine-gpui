use std::{
    error::Error,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use serde_json::{Value, value::RawValue};

use super::{
    tests::{diagnostics, fixture, mock_executable},
    *,
};

const WAIT: Duration = Duration::from_secs(5);

type TestResult = Result<(), Box<dyn Error>>;

struct MockSession {
    model: RustDiagnostics,
    latch: LanguageWakeLatch,
    root: PathBuf,
    record: PathBuf,
    identity: LanguageIdentity,
}

struct Recorded {
    bodies: Vec<String>,
}

impl Recorded {
    fn count(&self, method: &str) -> usize {
        let needle = format!(r#""method":"{method}""#);
        self.bodies
            .iter()
            .filter(|body| body.contains(&needle))
            .count()
    }
}

impl MockSession {
    fn start(arguments: &[&str]) -> Result<Self, Box<dyn Error>> {
        let (root, path, snapshot, identity) = fixture();
        let record = root.join("stdin.record");
        let mut configured: Vec<Box<str>> =
            vec!["--record-stdin".into(), record.to_string_lossy().into()];
        configured.extend(arguments.iter().map(|argument| Box::from(*argument)));
        let latch = LanguageWakeLatch::default();
        let mut model = RustDiagnostics::with_server(mock_executable());
        model.configure_protocol("rust", &configured);
        let wake_latch = latch.clone();
        let input = RustDocumentInput::new(&path, &root, identity, snapshot);
        let _ = model.sync(Some(input), move |wake| {
            let wake_latch = wake_latch.clone();
            Arc::new(move || wake_latch.publish(wake))
        });
        let mut session = Self {
            model,
            latch,
            root,
            record,
            identity,
        };
        session.drive_until("diagnostics", |model| model.snapshot().diagnostic_items > 0)?;
        Ok(session)
    }

    fn drive_until(
        &mut self,
        what: &str,
        mut done: impl FnMut(&RustDiagnostics) -> bool,
    ) -> TestResult {
        let deadline = Instant::now() + WAIT;
        while !done(&self.model) {
            if let Some(wake) = self.latch.take() {
                if let Some(continuation) = self.model.poll(wake).continuation {
                    self.latch.publish(continuation);
                }
            } else {
                std::thread::sleep(Duration::from_millis(1));
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "{what} timed out: {:?}, status {:?}",
                    self.model.snapshot(),
                    self.model.status_message()
                )
                .into());
            }
        }
        Ok(())
    }

    fn take_workspace_edit(
        &mut self,
        what: &str,
    ) -> Result<WorkspaceEditPreparationRequest, Box<dyn Error>> {
        self.drive_until(what, |model| model.snapshot().workspace_edit_preparing)?;
        self.model
            .take_workspace_edit_preparation()
            .ok_or_else(|| format!("{what} preparation").into())
    }

    fn status(&self) -> Option<String> {
        self.model.status_message().map(|status| status.to_string())
    }

    fn finish(mut self) -> Result<Recorded, Box<dyn Error>> {
        assert!(!self.model.shutdown().active);
        let bodies = recorded_bodies(&self.record);
        fs::remove_dir_all(&self.root)?;
        Ok(Recorded { bodies })
    }
}

/// Complete frames the mock read from the client, in order.
fn recorded_bodies(record: &Path) -> Vec<String> {
    let bytes = fs::read(record).unwrap_or_default();
    let text = String::from_utf8_lossy(&bytes);
    let mut bodies = Vec::new();
    let mut rest = text.as_ref();
    while let Some((_, frame)) = rest.split_once("Content-Length: ") {
        let Some((length, body)) = frame.split_once("\r\n\r\n") else {
            break;
        };
        let Ok(length) = length.parse::<usize>() else {
            break;
        };
        let Some(content) = body.get(..length) else {
            break;
        };
        bodies.push(content.to_owned());
        rest = &body[length..];
    }
    bodies
}

#[test]
#[cfg_attr(miri, ignore = "Miri cannot emulate child-process creation")]
fn content_modified_once_retries_hover_once_and_admits_the_result() -> TestResult {
    let mut mock = MockSession::start(&["--error", "textDocument/hover", "-32801", "1"])?;
    let _ = mock
        .model
        .request_navigation(NavigationRequestKind::Hover, LspPosition::new(0, 2)?);
    mock.drive_until("retried hover", |model| model.snapshot().hover_bytes > 0)?;
    assert_eq!(
        mock.model
            .hover_content(mock.identity)
            .map(HoverContent::text),
        Some("`fn main()`\n\nMock hover")
    );
    assert_eq!(mock.status(), None);
    let snapshot = mock.model.snapshot();
    assert_eq!(snapshot.request_retries, 1);
    assert_eq!(snapshot.navigation_requests, 1);
    assert_eq!(snapshot.restarts, 0);
    assert_eq!(mock.finish()?.count("textDocument/hover"), 2);
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "Miri cannot emulate child-process creation")]
fn content_modified_twice_reports_the_server_code_after_one_retry() -> TestResult {
    let mut mock = MockSession::start(&["--error", "textDocument/hover", "-32801", "2"])?;
    let _ = mock
        .model
        .request_navigation(NavigationRequestKind::Hover, LspPosition::new(0, 2)?);
    mock.drive_until("hover error", |model| model.status_message().is_some())?;
    assert_eq!(
        mock.status().as_deref(),
        Some("Language server reported error -32801.")
    );
    assert_eq!(
        mock.status(),
        Some(RustDiagnosticsError::Navigation(NavigationError::Server(-32_801)).to_string())
    );
    let snapshot = mock.model.snapshot();
    assert_eq!(snapshot.request_retries, 1);
    assert!(!snapshot.navigation_pending());
    assert_eq!(snapshot.hover_bytes, 0);
    assert_eq!(mock.finish()?.count("textDocument/hover"), 2);
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "Miri cannot emulate child-process creation")]
fn internal_error_reports_the_server_code_without_a_retry() -> TestResult {
    let mut mock = MockSession::start(&["--error", "textDocument/hover", "-32603", "1"])?;
    let _ = mock
        .model
        .request_navigation(NavigationRequestKind::Hover, LspPosition::new(0, 2)?);
    mock.drive_until("hover error", |model| model.status_message().is_some())?;
    assert_eq!(
        mock.status().as_deref(),
        Some("Language server reported error -32603.")
    );
    let snapshot = mock.model.snapshot();
    assert_eq!(snapshot.request_retries, 0);
    assert!(!snapshot.navigation_pending());
    assert_eq!(mock.finish()?.count("textDocument/hover"), 1);
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "Miri cannot emulate child-process creation")]
fn superseded_request_is_not_retried_but_its_successor_is() -> TestResult {
    let mut mock = MockSession::start(&["--error", "textDocument/hover", "-32801", "2"])?;
    let _ = mock
        .model
        .request_navigation(NavigationRequestKind::Hover, LspPosition::new(0, 2)?);
    let _ = mock
        .model
        .request_navigation(NavigationRequestKind::Hover, LspPosition::new(0, 3)?);
    assert_eq!(mock.model.snapshot().navigation_cancellations, 1);
    mock.drive_until("successor hover", |model| model.snapshot().hover_bytes > 0)?;
    assert_eq!(mock.status(), None);
    let snapshot = mock.model.snapshot();
    assert_eq!(snapshot.request_retries, 1);
    assert_eq!(snapshot.stale_navigation, 1);
    assert_eq!(snapshot.restarts, 0);
    assert_eq!(mock.finish()?.count("textDocument/hover"), 3);
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "Miri cannot emulate child-process creation")]
fn completion_symbols_and_workspace_edits_each_retry_once() -> TestResult {
    let mut mock = MockSession::start(&[
        "--error",
        "textDocument/completion",
        "-32801",
        "1",
        "--error",
        "textDocument/documentSymbol",
        "-32800",
        "1",
        "--error",
        "textDocument/rename",
        "-32802",
        "1",
        "--error",
        "textDocument/formatting",
        "-32801",
        "1",
    ])?;
    let _ = mock.model.request_completion(LspPosition::new(0, 2)?);
    mock.drive_until("completion", |model| model.snapshot().completion_items > 0)?;
    let _ = mock.model.open_symbols(SymbolRequestKind::Document);
    mock.drive_until("symbols", |model| model.snapshot().symbol_items > 0)?;
    let _ = mock
        .model
        .request_rename(LspPosition::new(0, 4)?, "renamed");
    let rename = mock.take_workspace_edit("rename")?;
    assert_eq!(rename.identity().kind(), WorkspaceEditKind::Rename);
    assert!(rename.execute().result.is_ok());
    let _ = mock.model.request_formatting(4, true);
    let formatting = mock.take_workspace_edit("formatting")?;
    assert_eq!(formatting.identity().kind(), WorkspaceEditKind::Formatting);
    assert!(formatting.execute().result.is_ok());
    let snapshot = mock.model.snapshot();
    assert_eq!(snapshot.request_retries, 4);
    assert_eq!(snapshot.restarts, 0);
    let recorded = mock.finish()?;
    for method in [
        "textDocument/completion",
        "textDocument/documentSymbol",
        "textDocument/rename",
        "textDocument/formatting",
    ] {
        assert_eq!(recorded.count(method), 2, "{method}");
    }
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "Miri cannot emulate child-process creation")]
fn apply_edit_is_declined_with_the_request_id_and_not_advertised() -> TestResult {
    let mock = MockSession::start(&["--apply-edit"])?;
    // The reply left during an earlier poll; wait for the mock to record it.
    let deadline = Instant::now() + WAIT;
    let reply = loop {
        let reply = recorded_bodies(&mock.record)
            .into_iter()
            .find(|body| body.contains(r#""id":41"#) && !body.contains(r#""method""#));
        if let Some(reply) = reply {
            break reply;
        }
        if Instant::now() >= deadline {
            return Err("the applyEdit reply never reached the mock".into());
        }
        std::thread::sleep(Duration::from_millis(1));
    };
    let reply: Value = serde_json::from_str(&reply)?;
    assert_eq!(reply["jsonrpc"], "2.0");
    assert_eq!(reply["id"], 41);
    assert!(reply.get("error").is_none(), "{reply}");
    assert_eq!(reply["result"]["applied"], false);
    assert!(
        reply["result"]["failureReason"]
            .as_str()
            .is_some_and(|reason| !reason.is_empty())
    );
    assert_eq!(mock.model.snapshot().restarts, 0);
    let recorded = mock.finish()?;
    let initialize = recorded
        .bodies
        .iter()
        .find(|body| body.contains(r#""method":"initialize""#))
        .ok_or("initialize request")?;
    let initialize: Value = serde_json::from_str(initialize)?;
    let workspace = &initialize["params"]["capabilities"]["workspace"];
    assert!(workspace.is_object());
    assert!(workspace.get("applyEdit").is_none(), "{workspace}");
    Ok(())
}

#[test]
fn retry_state_is_bounded_per_request_and_ignores_superseded_replies() -> TestResult {
    let (root, path, snapshot, identity) = fixture();
    let mut model = RustDiagnostics::default();
    let input = RustDocumentInput::new(&path, &root, identity, snapshot);
    model.install_for_test(input, &diagnostics(&path, 1), mock_executable())?;
    let session = model.session.as_mut().ok_or("session")?;
    session.client.initialize_inert_for_test();
    let modified = || NavigationCandidate::Hover(Err(NavigationError::Server(-32_801)));
    let pending = |model: &RustDiagnostics| {
        model
            .session
            .as_ref()
            .and_then(|session| session.pending_navigation)
    };

    let _ = model.request_navigation(NavigationRequestKind::Hover, LspPosition::new(0, 2)?);
    let first = pending(&model).ok_or("first request")?;
    assert!(!first.retried);
    assert!(!model.admit_navigation(first.request_id, first.stamp, first.kind, modified()));
    let second = pending(&model).ok_or("retried request")?;
    assert!(second.retried);
    assert_ne!(second.request_id, first.request_id);
    assert_eq!(second.position, first.position);
    assert_eq!(model.snapshot().request_retries, 1);
    assert_eq!(model.status_message(), None);
    assert!(model.admit_navigation(second.request_id, second.stamp, second.kind, modified()));
    assert_eq!(pending(&model), None);
    assert_eq!(model.snapshot().request_retries, 1);
    assert_eq!(
        model.status_message().as_deref(),
        Some("Language server reported error -32801.")
    );

    let _ = model.request_navigation(NavigationRequestKind::Hover, LspPosition::new(0, 2)?);
    let replaced = pending(&model).ok_or("replaced request")?;
    let _ = model.request_navigation(NavigationRequestKind::Hover, LspPosition::new(0, 3)?);
    let stale_before = model.snapshot().stale_navigation;
    assert!(!model.admit_navigation(
        replaced.request_id,
        replaced.stamp,
        replaced.kind,
        modified()
    ));
    assert_eq!(model.snapshot().stale_navigation, stale_before + 1);
    assert_eq!(model.snapshot().request_retries, 1);

    let _ = model.request_completion(LspPosition::new(0, 2)?);
    for (attempt, code) in [(0, -32_802), (1, -32_800)] {
        let pending = model
            .session
            .as_ref()
            .and_then(|session| session.pending_completion)
            .ok_or("completion request")?;
        assert_eq!(pending.retried, attempt == 1);
        let _ = model.admit_completion(
            pending.request_id,
            pending.stamp,
            Err(CompletionError::Server(code)),
        );
    }
    assert!(!model.snapshot().completion_pending());
    assert_eq!(model.snapshot().request_retries, 2);
    assert_eq!(
        model.status_message().as_deref(),
        Some("Language server reported error -32800.")
    );
    assert!(!model.shutdown().active);
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn only_dropped_or_outdated_replies_are_retryable() {
    for code in [-32_800, -32_801, -32_802] {
        assert!(is_retryable_server_error(code), "{code}");
    }
    for code in [-32_803, -32_799, -32_603, -32_601, -32_000, 0, i32::MIN] {
        assert!(!is_retryable_server_error(code), "{code}");
    }
}

#[test]
fn error_replies_carry_the_server_code_and_bad_results_stay_malformed() -> TestResult {
    let reply = ResponseValue::error_code_for_test(-32_801);
    assert!(matches!(
        navigation_from_response(NavigationRequestKind::Hover, reply),
        NavigationCandidate::Hover(Err(NavigationError::Server(-32_801)))
    ));
    for kind in [
        NavigationRequestKind::Definition,
        NavigationRequestKind::References,
    ] {
        assert!(matches!(
            navigation_from_response(kind, reply),
            NavigationCandidate::Locations(Err(NavigationError::Server(-32_801)))
        ));
    }
    assert_eq!(
        completion_batch_from_response(ResponseValue::error_code_for_test(-32_603)),
        Err(CompletionError::Server(-32_603))
    );
    assert_eq!(
        symbols_from_response(SymbolRequestKind::Document, reply, "file:///a.rs"),
        Err(SymbolError::Server(-32_801))
    );
    assert!(matches!(
        workspace_edit_wire(reply),
        Err(WorkspaceEditError::Server(-32_801))
    ));

    let bad = RawValue::from_string(String::from("42"))?;
    assert!(matches!(
        navigation_from_response(NavigationRequestKind::Hover, ResponseValue::Result(&bad)),
        NavigationCandidate::Hover(Err(NavigationError::Malformed))
    ));
    assert_eq!(
        completion_batch_from_response(ResponseValue::Result(&bad)),
        Err(CompletionError::Malformed)
    );

    for error in [
        RustDiagnosticsError::Navigation(NavigationError::Server(-32_801)),
        RustDiagnosticsError::Completion(CompletionError::Server(-32_801)),
        RustDiagnosticsError::Symbols(SymbolError::Server(-32_801)),
        RustDiagnosticsError::WorkspaceEdit(WorkspaceEditError::Server(-32_801)),
    ] {
        assert_eq!(error.to_string(), "Language server reported error -32801.");
    }
    assert_eq!(
        RustDiagnosticsError::Navigation(NavigationError::Malformed).to_string(),
        "language diagnostics unavailable: Navigation(Malformed)"
    );
    Ok(())
}
