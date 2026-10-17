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
use crate::lsp_json::ServerError;

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

    fn frames(&self, method: &str) -> Result<Vec<Value>, Box<dyn Error>> {
        let needle = format!(r#""method":"{method}""#);
        let mut frames = Vec::new();
        for body in self.bodies.iter().filter(|body| body.contains(&needle)) {
            frames.push(serde_json::from_str(body)?);
        }
        Ok(frames)
    }

    /// The retry is a new request id carrying the first request's params.
    fn assert_resent_unchanged(&self, method: &str) -> TestResult {
        let frames = self.frames(method)?;
        assert_eq!(frames.len(), 2, "{method}");
        assert_ne!(frames[0]["id"], frames[1]["id"], "{method}");
        assert!(frames[0]["params"].is_object(), "{method}");
        assert_eq!(frames[0]["params"], frames[1]["params"], "{method}");
        Ok(())
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
    mock.finish()?.assert_resent_unchanged("textDocument/hover")
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
    let error = NavigationError::Server(ServerError::for_test(-32_801, true));
    assert_eq!(
        mock.status(),
        Some(RustDiagnosticsError::Navigation(error).to_string())
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
fn non_retryable_errors_report_the_server_code_without_a_retry() -> TestResult {
    for (code, data) in [
        ("-32603", None),
        ("-32802", None),
        ("-32802", Some(r#"{"retriggerRequest":false}"#)),
    ] {
        let mut arguments = vec!["--error", "textDocument/hover", code, "1"];
        arguments.extend(data.map(|data| ["--data", data]).into_iter().flatten());
        let mut mock = MockSession::start(&arguments)?;
        let _ = mock
            .model
            .request_navigation(NavigationRequestKind::Hover, LspPosition::new(0, 2)?);
        mock.drive_until("hover error", |model| model.status_message().is_some())?;
        assert_eq!(
            mock.status(),
            Some(format!("Language server reported error {code}.")),
            "{code} {data:?}"
        );
        let snapshot = mock.model.snapshot();
        assert_eq!(snapshot.request_retries, 0, "{code} {data:?}");
        assert!(!snapshot.navigation_pending(), "{code} {data:?}");
        assert_eq!(
            mock.finish()?.count("textDocument/hover"),
            1,
            "{code} {data:?}"
        );
    }
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
fn cancelling_a_retried_request_cancels_the_retry_id() -> TestResult {
    let mut mock = MockSession::start(&["--error", "textDocument/hover", "-32801", "1"])?;
    // The mock leaves a hover at character 99 unanswered, so the retry waits.
    let _ = mock
        .model
        .request_navigation(NavigationRequestKind::Hover, LspPosition::new(0, 99)?);
    mock.drive_until("retry", |model| model.snapshot().request_retries == 1)?;
    assert!(mock.model.snapshot().navigation_pending());
    let _ = mock.model.cancel_navigation();
    let snapshot = mock.model.snapshot();
    assert!(!snapshot.navigation_pending());
    assert_eq!(snapshot.navigation_cancellations, 1);
    mock.drive_until("late reply", |model| model.snapshot().stale_navigation == 1)?;
    assert_eq!(mock.model.snapshot().restarts, 0);
    let recorded = mock.finish()?;
    let hovers = recorded.frames("textDocument/hover")?;
    let cancels = recorded.frames("$/cancelRequest")?;
    assert_eq!(hovers.len(), 2);
    assert_eq!(cancels.len(), 1);
    assert_eq!(cancels[0]["params"]["id"], hovers[1]["id"]);
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
        "--data",
        r#"{"retriggerRequest":true}"#,
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
        recorded.assert_resent_unchanged(method)?;
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

/// A model on the inert test client; `running` lets requests be issued.
fn inert_model(running: bool) -> Result<(RustDiagnostics, PathBuf, PathBuf), Box<dyn Error>> {
    let (root, path, snapshot, identity) = fixture();
    let mut model = RustDiagnostics::default();
    let input = RustDocumentInput::new(&path, &root, identity, snapshot);
    model.install_for_test(input, &diagnostics(&path, 1), mock_executable())?;
    if running {
        let session = model.session.as_mut().ok_or("session")?;
        session.client.initialize_inert_for_test();
    }
    Ok((model, root, path))
}

fn modified() -> ResponseValue<'static> {
    ResponseValue::error_code_for_test(-32_801)
}

#[derive(Clone, Copy, Debug)]
enum Kind {
    Navigation,
    Completion,
    Symbols,
    WorkspaceEdit,
}

impl Kind {
    fn issue(self, model: &mut RustDiagnostics) -> Result<(u32, RequestStamp), Box<dyn Error>> {
        let position = LspPosition::new(0, 2)?;
        let _ = match self {
            Self::Navigation => model.request_navigation(NavigationRequestKind::Hover, position),
            Self::Completion => model.request_completion(position),
            Self::Symbols => model.open_symbols(SymbolRequestKind::Document),
            Self::WorkspaceEdit => model.request_rename(position, "renamed"),
        };
        let session = model.session.as_ref().ok_or("session")?;
        let issued = match self {
            Self::Navigation => session.pending_navigation.map(|p| (p.request_id, p.stamp)),
            Self::Completion => session.pending_completion.map(|p| (p.request_id, p.stamp)),
            Self::Symbols => session.pending_symbols.map(|p| (p.request_id, p.stamp)),
            Self::WorkspaceEdit => session
                .pending_workspace_edit
                .map(|p| (p.request_id, p.stamp)),
        };
        issued.ok_or_else(|| format!("{self:?} request").into())
    }

    fn admit_modified(self, model: &mut RustDiagnostics, (id, stamp): (u32, RequestStamp)) {
        let hover = NavigationRequestKind::Hover;
        let document = SymbolRequestKind::Document;
        let _ = match self {
            Self::Navigation => model.admit_navigation(
                id,
                stamp,
                hover,
                navigation_from_response(hover, modified()),
            ),
            Self::Completion => {
                model.admit_completion(id, stamp, completion_batch_from_response(modified()))
            }
            Self::Symbols => model.admit_symbols(
                id,
                stamp,
                document,
                symbols_from_response(document, modified(), "file:///a.rs"),
            ),
            Self::WorkspaceEdit => model.admit_workspace_edit(
                id,
                stamp,
                WorkspaceEditKind::Rename,
                workspace_edit_wire(modified()),
            ),
        };
    }

    fn pending(self, snapshot: &RustDiagnosticsSnapshot) -> bool {
        match self {
            Self::Navigation => snapshot.navigation_pending(),
            Self::Completion => snapshot.completion_pending(),
            Self::Symbols => snapshot.symbol_pending(),
            Self::WorkspaceEdit => snapshot.workspace_edit_pending,
        }
    }

    fn stale(self, snapshot: &RustDiagnosticsSnapshot) -> u64 {
        match self {
            Self::Navigation => snapshot.stale_navigation,
            Self::Completion => snapshot.stale_completions,
            Self::Symbols => snapshot.stale_symbols,
            Self::WorkspaceEdit => snapshot.stale_workspace_edits,
        }
    }
}

#[test]
fn retryable_replies_retry_only_while_every_staleness_axis_holds() -> TestResult {
    let kinds = [
        Kind::Navigation,
        Kind::Completion,
        Kind::Symbols,
        Kind::WorkspaceEdit,
    ];
    for kind in kinds {
        for axis in ["none", "identity", "process_epoch", "lsp_version"] {
            let (mut model, root, _) = inert_model(true)?;
            let issued = kind.issue(&mut model)?;
            let session = model.session.as_mut().ok_or("session")?;
            match axis {
                "identity" => session.identity.document_revision += 1,
                "process_epoch" => session.process_epoch += 1,
                "lsp_version" => session.lsp_version += 1,
                _ => {}
            }
            kind.admit_modified(&mut model, issued);
            let snapshot = model.snapshot();
            let current = axis == "none";
            assert_eq!(
                snapshot.request_retries,
                u64::from(current),
                "{kind:?} {axis}"
            );
            assert_eq!(kind.pending(&snapshot), current, "{kind:?} {axis}");
            assert_eq!(
                kind.stale(&snapshot),
                u64::from(!current),
                "{kind:?} {axis}"
            );
            let session = model.session.as_ref().ok_or("session")?;
            let params_kept = session.workspace_edit_params.is_some();
            assert_eq!(
                params_kept,
                current && matches!(kind, Kind::WorkspaceEdit),
                "{kind:?} {axis}"
            );
            assert!(!model.shutdown().active);
            fs::remove_dir_all(root)?;
        }
    }
    Ok(())
}

#[test]
fn retry_state_is_bounded_per_request_and_cancels_the_retry_id() -> TestResult {
    let (mut model, root, _) = inert_model(true)?;
    let hover = NavigationRequestKind::Hover;
    let pending = |model: &RustDiagnostics| {
        model
            .session
            .as_ref()
            .and_then(|session| session.pending_navigation)
    };
    let _ = model.request_navigation(hover, LspPosition::new(0, 2)?);
    let first = pending(&model).ok_or("first request")?;
    assert!(!first.retried);
    let reply = navigation_from_response(hover, modified());
    assert!(!model.admit_navigation(first.request_id, first.stamp, hover, reply));
    let second = pending(&model).ok_or("retried request")?;
    assert!(second.retried);
    assert_ne!(second.request_id, first.request_id);
    assert_eq!(second.position, first.position);
    assert_eq!(model.snapshot().request_retries, 1);
    assert_eq!(model.status_message(), None);
    let reply = navigation_from_response(hover, modified());
    assert!(model.admit_navigation(second.request_id, second.stamp, hover, reply));
    assert_eq!(pending(&model), None);
    assert_eq!(model.snapshot().request_retries, 1);
    assert_eq!(
        model.status_message().as_deref(),
        Some("Language server reported error -32801.")
    );

    let _ = model.request_navigation(hover, LspPosition::new(0, 2)?);
    let first = pending(&model).ok_or("next request")?;
    let reply = navigation_from_response(hover, modified());
    let _ = model.admit_navigation(first.request_id, first.stamp, hover, reply);
    let retry = pending(&model).ok_or("next retry")?;
    let cancellations = model.snapshot().navigation_cancellations;
    let _ = model.cancel_navigation();
    assert_eq!(pending(&model), None);
    assert_eq!(model.snapshot().navigation_cancellations, cancellations + 1);
    let client = &mut model.session.as_mut().ok_or("session")?.client;
    assert!(client.cancel(retry.request_id).is_err());

    let _ = model.request_completion(LspPosition::new(0, 2)?);
    for (attempt, code) in [(0, -32_801), (1, -32_800)] {
        let pending = model
            .session
            .as_ref()
            .and_then(|session| session.pending_completion)
            .ok_or("completion request")?;
        assert_eq!(pending.retried, attempt == 1);
        let reply = completion_batch_from_response(ResponseValue::error_code_for_test(code));
        let _ = model.admit_completion(pending.request_id, pending.stamp, reply);
    }
    assert!(!model.snapshot().completion_pending());
    assert_eq!(model.snapshot().request_retries, 3);
    assert_eq!(
        model.status_message().as_deref(),
        Some("Language server reported error -32800.")
    );
    assert!(!model.shutdown().active);
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn a_retry_that_cannot_be_sent_fails_like_a_first_request() -> TestResult {
    let (mut model, root, _) = inert_model(false)?;
    let hover = NavigationRequestKind::Hover;
    let session = model.session.as_mut().ok_or("session")?;
    let stamp = session.identity.request_stamp().ok_or("stamp")?;
    session.pending_navigation = Some(PendingNavigation {
        request_id: 7,
        stamp,
        kind: hover,
        identity: session.identity,
        process_epoch: session.process_epoch,
        lsp_version: session.lsp_version,
        position: LspPosition::new(0, 2)?,
        retried: false,
    });
    assert!(model.snapshot().diagnostic_items > 0);
    let reply = navigation_from_response(hover, modified());
    assert!(model.admit_navigation(7, stamp, hover, reply));
    let snapshot = model.snapshot();
    assert_eq!(snapshot.request_retries, 0);
    assert!(!snapshot.navigation_pending());
    assert_eq!(snapshot.diagnostic_items, 0);
    assert_eq!(
        model.status_message().as_deref(),
        Some("language diagnostics unavailable: Client(Protocol(InvalidLifecycle))")
    );

    // Without its params a workspace edit cannot be resent, so it reports.
    let session = model.session.as_mut().ok_or("session")?;
    session.pending_workspace_edit = Some(PendingWorkspaceEdit {
        request_id: 8,
        stamp,
        kind: WorkspaceEditKind::Rename,
        identity: session.identity,
        process_epoch: session.process_epoch,
        lsp_version: session.lsp_version,
        retried: false,
    });
    let reply = workspace_edit_wire(modified());
    assert!(model.admit_workspace_edit(8, stamp, WorkspaceEditKind::Rename, reply));
    assert_eq!(model.snapshot().request_retries, 0);
    assert_eq!(
        model.status_message().as_deref(),
        Some("Language server reported error -32801.")
    );
    assert!(!model.shutdown().active);
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn workspace_edit_params_leave_with_their_pending_request() -> TestResult {
    let scenarios = [
        "cancel",
        "identity change",
        "document switch",
        "failure",
        "restart limit",
        "restart",
    ];
    for scenario in scenarios {
        let (mut model, root, path) = inert_model(true)?;
        let held = |model: &RustDiagnostics| {
            model.session.as_ref().map(|session| {
                (
                    session.pending_workspace_edit.is_some(),
                    session.workspace_edit_params.is_some(),
                )
            })
        };
        let _ = model.request_rename(LspPosition::new(0, 4)?, "renamed");
        assert_eq!(held(&model), Some((true, true)), "{scenario}");
        let session = model.session.as_mut().ok_or("session")?;
        let mut identity = session.identity;
        match scenario {
            "cancel" => assert!(model.cancel_workspace_edit()),
            "identity change" => {
                identity.document_revision += 1;
                let snapshot = session.snapshot.clone();
                let input = RustDocumentInput::new(&path, &root, identity, snapshot);
                let _ = model.sync(Some(input), |_| Arc::new(|| {}));
            }
            "document switch" => {
                let other = root.join("other.rs");
                fs::write(&other, "fn other() {}\n")?;
                let snapshot = alpine_text::Buffer::new("fn other() {}\n").snapshot();
                identity.document_id += 1;
                identity.buffer_revision = snapshot.revision().get();
                let input = RustDocumentInput::new(&other, &root, identity, snapshot);
                let _ = model.sync(Some(input), |_| Arc::new(|| {}));
                assert_eq!(model.snapshot().document_switches, 1);
            }
            "failure" => {
                let _ = model.fail(RustDiagnosticsError::InvalidIdentity);
            }
            "restart limit" => {
                session.restart_count = MAX_RESTARTS_PER_DOCUMENT;
                let _ = model.restart_or_fail(RustDiagnosticsError::InvalidIdentity);
                assert_eq!(model.snapshot().restarts, 0);
            }
            _ => {
                let _ = model.restart_or_fail(RustDiagnosticsError::InvalidIdentity);
                assert_eq!(model.snapshot().restarts, 1);
            }
        }
        assert_eq!(held(&model), Some((false, false)), "{scenario}");
        assert!(!model.shutdown().active);
        fs::remove_dir_all(root)?;
    }
    Ok(())
}

#[test]
fn error_replies_carry_the_server_code_and_bad_results_stay_malformed() -> TestResult {
    let reply = modified();
    let retryable = ServerError::for_test(-32_801, true);
    assert!(matches!(
        navigation_from_response(NavigationRequestKind::Hover, reply),
        NavigationCandidate::Hover(Err(NavigationError::Server(error))) if error == retryable
    ));
    for kind in [
        NavigationRequestKind::Definition,
        NavigationRequestKind::References,
    ] {
        assert!(matches!(
            navigation_from_response(kind, reply),
            NavigationCandidate::Locations(Err(NavigationError::Server(error)))
                if error == retryable
        ));
    }
    assert_eq!(
        completion_batch_from_response(ResponseValue::error_code_for_test(-32_603)),
        Err(CompletionError::Server(ServerError::for_test(
            -32_603, false
        )))
    );
    assert_eq!(
        symbols_from_response(SymbolRequestKind::Document, reply, "file:///a.rs"),
        Err(SymbolError::Server(retryable))
    );
    assert!(matches!(
        workspace_edit_wire(reply),
        Err(WorkspaceEditError::Server(error)) if error == retryable
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

    let errors = [
        (
            NavigationError::Server(retryable).to_string(),
            RustDiagnosticsError::Navigation(NavigationError::Server(retryable)),
        ),
        (
            CompletionError::Server(retryable).to_string(),
            RustDiagnosticsError::Completion(CompletionError::Server(retryable)),
        ),
        (
            SymbolError::Server(retryable).to_string(),
            RustDiagnosticsError::Symbols(SymbolError::Server(retryable)),
        ),
        (
            WorkspaceEditError::Server(retryable).to_string(),
            RustDiagnosticsError::WorkspaceEdit(WorkspaceEditError::Server(retryable)),
        ),
    ];
    for (inner, error) in errors {
        assert_eq!(inner, "server error -32801");
        assert_eq!(error.to_string(), "Language server reported error -32801.");
    }
    assert_eq!(
        NavigationError::Malformed.to_string(),
        "Rust navigation rejected input: Malformed"
    );
    assert_eq!(
        RustDiagnosticsError::Navigation(NavigationError::Malformed).to_string(),
        "language diagnostics unavailable: Navigation(Malformed)"
    );
    Ok(())
}
