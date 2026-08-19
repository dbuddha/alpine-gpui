//! Bounded active-document Rust diagnostics above the local LSP client.

use std::{
    env,
    error::Error,
    fmt,
    ops::Range,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use alpine_text::BufferSnapshot;

use crate::{
    lsp_client::{LspClient, LspClientError, LspClientPoll},
    lsp_json::{PeerEvent, RequestStamp, ResponseValue},
    lsp_language::{
        DiagnosticBatch, LanguageProtocolError, LspDocument, LspPosition, initialize_params,
    },
    lsp_process::{ConfigError, ProcessIdentity, ProcessSpec, ProcessWake, StopReason},
    rust_completion::{CompletionBatch, CompletionError, CompletionItem},
};

const MAX_POLLS_PER_TURN: usize = 8;
const MAX_RESTARTS_PER_DOCUMENT: u8 = 2;
pub(crate) const MAX_VISIBLE_DIAGNOSTIC_MARKERS: usize = 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(
    clippy::struct_field_names,
    reason = "the revision suffix names four independent admission authorities"
)]
pub(crate) struct LanguageIdentity {
    pub(crate) workspace_id: u64,
    pub(crate) workspace_revision: u64,
    pub(crate) document_id: u64,
    pub(crate) document_revision: u64,
    pub(crate) buffer_revision: u64,
    pub(crate) selection_revision: u64,
}

impl LanguageIdentity {
    fn request_stamp(self) -> Option<RequestStamp> {
        RequestStamp::new(
            self.workspace_id,
            self.workspace_revision,
            self.document_id,
            self.document_revision,
            self.buffer_revision,
            self.selection_revision,
        )
    }
}

#[derive(Clone)]
pub(crate) struct RustDocumentInput {
    path: PathBuf,
    workspace_root: PathBuf,
    identity: LanguageIdentity,
    snapshot: BufferSnapshot,
}

impl RustDocumentInput {
    pub(crate) fn new(
        path: &Path,
        workspace_root: &Path,
        identity: LanguageIdentity,
        snapshot: BufferSnapshot,
    ) -> Self {
        Self {
            path: path.to_path_buf(),
            workspace_root: workspace_root.to_path_buf(),
            identity,
            snapshot,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LanguageWake {
    generation: u64,
}

impl LanguageWake {
    #[cfg(test)]
    pub(crate) const fn successor_for_test(self) -> Self {
        Self {
            generation: self.generation.saturating_add(1),
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct LanguageWakeLatch {
    generation: Arc<AtomicU64>,
}

impl LanguageWakeLatch {
    pub(crate) fn publish(&self, wake: LanguageWake) {
        self.generation.store(wake.generation, Ordering::Release);
    }

    pub(crate) fn clear(&self, wake: LanguageWake) {
        let _ = self.generation.compare_exchange(
            wake.generation,
            0,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }

    pub(crate) fn take(&self) -> Option<LanguageWake> {
        let generation = self.generation.swap(0, Ordering::AcqRel);
        (generation != 0).then_some(LanguageWake { generation })
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct LanguageEffect {
    pub(crate) visual_changed: bool,
    pub(crate) continuation: Option<LanguageWake>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DiagnosticMarker {
    pub(crate) start_utf16: u32,
    pub(crate) end_utf16: Option<u32>,
    pub(crate) severity: Option<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CompletionApplication {
    pub(crate) range: Range<usize>,
    pub(crate) text: Box<str>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CompletionRow<'a> {
    pub(crate) label: &'a str,
    pub(crate) selected: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct RustDiagnosticsSnapshot {
    pub(crate) active: bool,
    pub(crate) generation: u64,
    pub(crate) process_epoch: u64,
    pub(crate) lsp_version: i32,
    pub(crate) diagnostic_publications: u64,
    pub(crate) diagnostic_version: Option<i32>,
    pub(crate) diagnostic_items: usize,
    pub(crate) diagnostic_bytes: usize,
    pub(crate) peak_diagnostic_items: usize,
    pub(crate) peak_diagnostic_bytes: usize,
    pub(crate) completion_pending: bool,
    pub(crate) completion_items: usize,
    pub(crate) completion_bytes: usize,
    pub(crate) peak_completion_items: usize,
    pub(crate) peak_completion_bytes: usize,
    pub(crate) completion_requests: u64,
    pub(crate) completion_cancellations: u64,
    pub(crate) stale_completions: u64,
    pub(crate) completion_truncations: u64,
    pub(crate) process_retained_bytes: usize,
    pub(crate) process_queued_events: usize,
    pub(crate) process_submitted_inputs: u64,
    pub(crate) process_written_inputs: u64,
    pub(crate) process_input_saturations: u64,
    pub(crate) polls: u64,
    pub(crate) stale_wakes: u64,
    pub(crate) stale_diagnostics: u64,
    pub(crate) restarts: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SessionState {
    Starting,
    Initializing,
    Open,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Target {
    path: PathBuf,
    workspace_root: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AdmittedDiagnostics {
    identity: LanguageIdentity,
    process_epoch: u64,
    batch: DiagnosticBatch,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PendingCompletion {
    request_id: u32,
    stamp: RequestStamp,
    identity: LanguageIdentity,
    process_epoch: u64,
    lsp_version: i32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AdmittedCompletion {
    request_id: u32,
    identity: LanguageIdentity,
    process_epoch: u64,
    lsp_version: i32,
    batch: CompletionBatch,
    selected: usize,
    first_visible: usize,
}

struct RustSession {
    target: Target,
    identity: LanguageIdentity,
    generation: u64,
    process_generation: u64,
    process_epoch: u64,
    lsp_version: i32,
    state: SessionState,
    snapshot: BufferSnapshot,
    synced_snapshot: BufferSnapshot,
    pending_change: bool,
    restart_count: u8,
    document: LspDocument,
    diagnostics: Option<AdmittedDiagnostics>,
    pending_completion: Option<PendingCompletion>,
    completion: Option<AdmittedCompletion>,
    client: LspClient,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RustDiagnosticsError {
    MissingServer,
    InvalidIdentity,
    GenerationExhausted,
    VersionExhausted,
    Configuration(ConfigError),
    Language(LanguageProtocolError),
    Client(LspClientError),
    Completion(CompletionError),
}

impl fmt::Display for RustDiagnosticsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Rust diagnostics unavailable: {self:?}")
    }
}

impl Error for RustDiagnosticsError {}

pub(crate) struct RustDiagnostics {
    server_path: Option<PathBuf>,
    target: Option<Target>,
    session: Option<RustSession>,
    next_generation: u64,
    status: Option<Arc<str>>,
    peak_diagnostic_items: usize,
    peak_diagnostic_bytes: usize,
    diagnostic_publications: u64,
    peak_completion_items: usize,
    peak_completion_bytes: usize,
    completion_requests: u64,
    completion_cancellations: u64,
    stale_completions: u64,
    completion_truncations: u64,
    polls: u64,
    stale_wakes: u64,
    stale_diagnostics: u64,
    restarts: u64,
    #[cfg(test)]
    force_continuation_once: bool,
}

impl Default for RustDiagnostics {
    fn default() -> Self {
        Self {
            server_path: env::var_os("ALPINE_RUST_ANALYZER").map(PathBuf::from),
            target: None,
            session: None,
            next_generation: 0,
            status: None,
            peak_diagnostic_items: 0,
            peak_diagnostic_bytes: 0,
            diagnostic_publications: 0,
            peak_completion_items: 0,
            peak_completion_bytes: 0,
            completion_requests: 0,
            completion_cancellations: 0,
            stale_completions: 0,
            completion_truncations: 0,
            polls: 0,
            stale_wakes: 0,
            stale_diagnostics: 0,
            restarts: 0,
            #[cfg(test)]
            force_continuation_once: false,
        }
    }
}

impl RustDiagnostics {
    pub(crate) fn sync<F>(
        &mut self,
        input: Option<RustDocumentInput>,
        wake_factory: F,
    ) -> LanguageEffect
    where
        F: FnOnce(LanguageWake) -> ProcessWake,
    {
        let Some(input) = input else {
            let changed = self.stop();
            return LanguageEffect {
                visual_changed: changed,
                continuation: None,
            };
        };
        let target = Target {
            path: input.path.clone(),
            workspace_root: input.workspace_root.clone(),
        };
        if self.target.as_ref() != Some(&target) {
            self.stop();
            self.target = Some(target.clone());
            let result = self.start(input, target, wake_factory);
            let status = result.err().map(|error| Arc::from(error.to_string()));
            return LanguageEffect {
                visual_changed: replace_status(&mut self.status, status) || self.session.is_some(),
                continuation: None,
            };
        }
        let Some(session) = self.session.as_mut() else {
            return LanguageEffect::default();
        };
        let mut visual_changed = false;
        if input.identity != session.identity {
            merge_visual_changed(&mut visual_changed, session.completion.take().is_some());
            if let Some(pending) = session.pending_completion.take() {
                let _ = session.client.cancel(pending.request_id);
                self.completion_cancellations = self.completion_cancellations.saturating_add(1);
            }
        }
        if input.identity.buffer_revision != session.identity.buffer_revision {
            let Some(version) = session.lsp_version.checked_add(1) else {
                return self.fail(RustDiagnosticsError::VersionExhausted);
            };
            session.document.set_version(version);
            session.lsp_version = version;
            session.snapshot = input.snapshot;
            session.pending_change = session.state == SessionState::Open;
            merge_visual_changed(&mut visual_changed, session.diagnostics.take().is_some());
        }
        if session.identity.selection_revision != input.identity.selection_revision
            && let Some(diagnostics) = session.diagnostics.as_mut()
        {
            diagnostics.identity.selection_revision = input.identity.selection_revision;
        }
        session.identity = input.identity;
        if session.pending_change {
            merge_visual_changed(&mut visual_changed, self.flush_change());
        }
        LanguageEffect {
            visual_changed,
            continuation: None,
        }
    }

    pub(crate) fn poll(&mut self, wake: LanguageWake) -> LanguageEffect {
        let Some(session) = self.session.as_ref() else {
            self.stale_wakes = self.stale_wakes.saturating_add(1);
            return LanguageEffect::default();
        };
        if wake.generation != session.generation {
            self.stale_wakes = self.stale_wakes.saturating_add(1);
            return LanguageEffect::default();
        }
        let mut visual_changed = false;
        for _ in 0..MAX_POLLS_PER_TURN {
            self.polls = self.polls.saturating_add(1);
            let mut initialized = false;
            let mut candidate = None;
            let mut completion_candidate = None;
            let mut stale_completion = None;
            let poll = {
                let session = self.session.as_mut().unwrap_or_else(|| unreachable!());
                let expected = &session.document;
                let current = session.pending_completion.map(|pending| pending.stamp);
                session.client.poll(current, |event| match event {
                    PeerEvent::Initialized(_) => initialized = true,
                    PeerEvent::InboundNotification {
                        method: "textDocument/publishDiagnostics",
                        params: Some(params),
                    } => candidate = Some(DiagnosticBatch::admit(params, expected)),
                    PeerEvent::Response {
                        id,
                        method,
                        stamp,
                        value,
                    } if method.as_ref() == "textDocument/completion" => {
                        let batch = match value {
                            ResponseValue::Result(result) => CompletionBatch::admit(result),
                            ResponseValue::Error(_) => Err(CompletionError::Malformed),
                        };
                        completion_candidate = Some((id, stamp, batch));
                    }
                    PeerEvent::StaleResponse { id } => stale_completion = Some(id),
                    _ => {}
                })
            };
            let poll = match poll {
                Ok(poll) => poll,
                Err(error) => {
                    let restarted = self.restart_or_fail(RustDiagnosticsError::Client(error));
                    merge_visual_changed(&mut visual_changed, restarted);
                    break;
                }
            };
            if initialized {
                let opened = self.open_document();
                merge_visual_changed(&mut visual_changed, opened);
            }
            if let Some(candidate) = candidate {
                let admitted = self.admit(candidate);
                merge_visual_changed(&mut visual_changed, admitted);
            }
            if let Some(id) = stale_completion {
                merge_visual_changed(&mut visual_changed, self.reject_stale_completion(id));
            }
            if let Some((id, stamp, batch)) = completion_candidate {
                merge_visual_changed(&mut visual_changed, self.admit_completion(id, stamp, batch));
            }
            if self.apply_poll(poll, &mut visual_changed) {
                break;
            }
        }
        let flushed = self.flush_change();
        merge_visual_changed(&mut visual_changed, flushed);
        #[allow(
            unused_mut,
            reason = "test pressure injection replaces an empty process queue"
        )]
        let mut continuation = self.session.as_ref().and_then(|session| {
            continuation_for_queued_events(
                session.client.snapshot().process.queued_events,
                session.generation,
            )
        });
        #[cfg(test)]
        if self.force_continuation_once {
            self.force_continuation_once = false;
            continuation = Some(wake);
        }
        LanguageEffect {
            visual_changed,
            continuation,
        }
    }

    pub(crate) fn status_message(&self) -> Option<Arc<str>> {
        self.status.clone()
    }

    pub(crate) fn request_completion(&mut self, position: LspPosition) -> LanguageEffect {
        let mut visual_changed = self.cancel_completion();
        let Some(session) = self.session.as_mut() else {
            return LanguageEffect {
                visual_changed: replace_status(
                    &mut self.status,
                    Some(Arc::from("Rust analysis is not ready for completion.")),
                ) || visual_changed,
                continuation: None,
            };
        };
        if session.state != SessionState::Open {
            return LanguageEffect {
                visual_changed: replace_status(
                    &mut self.status,
                    Some(Arc::from("Rust analysis is not ready for completion.")),
                ) || visual_changed,
                continuation: None,
            };
        }
        let Some(stamp) = session.identity.request_stamp() else {
            return self.fail(RustDiagnosticsError::InvalidIdentity);
        };
        let result = session
            .document
            .completion_params(position)
            .map_err(RustDiagnosticsError::Language)
            .and_then(|params| {
                session
                    .client
                    .begin_request("textDocument/completion", Some(&params), stamp)
                    .map_err(RustDiagnosticsError::Client)
            });
        match result {
            Ok(request) => {
                session.pending_completion = Some(PendingCompletion {
                    request_id: request.request_id,
                    stamp,
                    identity: session.identity,
                    process_epoch: session.process_epoch,
                    lsp_version: session.lsp_version,
                });
                self.completion_requests = self.completion_requests.saturating_add(1);
                visual_changed |= replace_status(&mut self.status, None);
                LanguageEffect {
                    visual_changed,
                    continuation: None,
                }
            }
            Err(error) => self.fail(error),
        }
    }

    pub(crate) fn completion_is_open(&self, identity: LanguageIdentity) -> bool {
        self.session.as_ref().is_some_and(|session| {
            session.completion.as_ref().is_some_and(|completion| {
                completion.identity == identity
                    && completion.process_epoch == session.process_epoch
                    && completion.lsp_version == session.lsp_version
            })
        })
    }

    pub(crate) fn cancel_completion(&mut self) -> bool {
        let Some(session) = self.session.as_mut() else {
            return false;
        };
        let mut changed = session.completion.take().is_some();
        if let Some(pending) = session.pending_completion.take() {
            match session.client.cancel(pending.request_id) {
                Ok(_) => {
                    self.completion_cancellations = self.completion_cancellations.saturating_add(1);
                }
                Err(error) => {
                    changed |= replace_status(
                        &mut self.status,
                        Some(Arc::from(RustDiagnosticsError::Client(error).to_string())),
                    );
                }
            }
        }
        changed
    }

    pub(crate) fn navigate_completion(&mut self, delta: isize) -> bool {
        let Some(completion) = self
            .session
            .as_mut()
            .and_then(|session| session.completion.as_mut())
        else {
            return false;
        };
        let count = completion.batch.items().len();
        if count == 0 {
            return false;
        }
        let previous = completion.selected;
        completion.selected = completion
            .selected
            .saturating_add_signed(delta)
            .min(count.saturating_sub(1));
        if completion.selected < completion.first_visible {
            completion.first_visible = completion.selected;
        } else if completion.selected
            >= completion
                .first_visible
                .saturating_add(crate::rust_completion::MAX_VISIBLE_COMPLETION_ROWS)
        {
            completion.first_visible = completion
                .selected
                .saturating_add(1)
                .saturating_sub(crate::rust_completion::MAX_VISIBLE_COMPLETION_ROWS);
        }
        previous != completion.selected
    }

    pub(crate) fn completion_visible_range(
        &self,
        identity: LanguageIdentity,
    ) -> Option<Range<usize>> {
        let session = self.session.as_ref()?;
        let completion = session.completion.as_ref()?;
        if completion.identity != identity
            || completion.process_epoch != session.process_epoch
            || completion.lsp_version != session.lsp_version
        {
            return None;
        }
        let end = completion
            .first_visible
            .saturating_add(crate::rust_completion::MAX_VISIBLE_COMPLETION_ROWS)
            .min(completion.batch.items().len());
        Some(completion.first_visible..end)
    }

    pub(crate) fn completion_row(
        &self,
        identity: LanguageIdentity,
        index: usize,
    ) -> Option<CompletionRow<'_>> {
        let session = self.session.as_ref()?;
        let completion = session.completion.as_ref()?;
        if completion.identity != identity
            || completion.process_epoch != session.process_epoch
            || completion.lsp_version != session.lsp_version
        {
            return None;
        }
        let item = completion.batch.items().get(index)?;
        Some(CompletionRow {
            label: item.label(),
            selected: completion.selected == index,
        })
    }

    pub(crate) fn completion_accessibility_label(
        &self,
        identity: LanguageIdentity,
    ) -> Option<Arc<str>> {
        let session = self.session.as_ref()?;
        let completion = session.completion.as_ref()?;
        if completion.identity != identity
            || completion.process_epoch != session.process_epoch
            || completion.lsp_version != session.lsp_version
        {
            return None;
        }
        let item = completion.batch.items().get(completion.selected)?;
        Some(Arc::from(format!("Code completion: {}", item.label())))
    }

    pub(crate) fn take_selected_completion(
        &mut self,
        identity: LanguageIdentity,
        snapshot: &BufferSnapshot,
        fallback: Range<usize>,
    ) -> Result<Option<CompletionApplication>, CompletionError> {
        let Some(session) = self.session.as_mut() else {
            return Ok(None);
        };
        let Some(completion) = session.completion.take() else {
            return Ok(None);
        };
        if completion.identity != identity
            || completion.process_epoch != session.process_epoch
            || completion.lsp_version != session.lsp_version
        {
            self.stale_completions = self.stale_completions.saturating_add(1);
            return Ok(None);
        }
        let item: &CompletionItem = completion
            .batch
            .items()
            .get(completion.selected)
            .ok_or(CompletionError::Malformed)?;
        let (range, text) = item.replacement(snapshot, fallback)?;
        let _ = replace_status(&mut self.status, None);
        Ok(Some(CompletionApplication { range, text }))
    }

    fn apply_poll(&mut self, poll: LspClientPoll, visual_changed: &mut bool) -> bool {
        match poll {
            LspClientPoll::Idle => true,
            LspClientPoll::Started { epoch, .. } => {
                let initialized = self.begin_initialize(epoch.get());
                merge_visual_changed(visual_changed, initialized);
                false
            }
            LspClientPoll::Stopped(StopReason::Restart)
            | LspClientPoll::Protocol { .. }
            | LspClientPoll::Stderr { .. }
            | LspClientPoll::InputWritten { .. } => false,
            LspClientPoll::Exited { .. } | LspClientPoll::Failed(_) | LspClientPoll::Stopped(_) => {
                let restarted = self.restart_or_fail(RustDiagnosticsError::Client(
                    LspClientError::ProcessNotStarted,
                ));
                merge_visual_changed(visual_changed, restarted);
                true
            }
            LspClientPoll::InputRejected { .. } => {
                if let Some(session) = self.session.as_mut() {
                    session.pending_change = session.state == SessionState::Open;
                }
                let status_changed = replace_status(
                    &mut self.status,
                    Some(Arc::from("Rust diagnostics input queue is saturated.")),
                );
                merge_visual_changed(visual_changed, status_changed);
                false
            }
        }
    }

    pub(crate) fn for_each_marker<E, F>(
        &self,
        identity: LanguageIdentity,
        line: usize,
        limit: usize,
        mut visitor: F,
    ) -> Result<usize, E>
    where
        F: FnMut(DiagnosticMarker) -> Result<(), E>,
    {
        let Some(session) = &self.session else {
            return Ok(0);
        };
        let Some(admitted) = &session.diagnostics else {
            return Ok(0);
        };
        if admitted.identity != identity || admitted.process_epoch != session.process_epoch {
            return Ok(0);
        }
        let Ok(line) = u32::try_from(line) else {
            return Ok(0);
        };
        let mut count = 0;
        for diagnostic in admitted.batch.diagnostics() {
            if count == limit {
                break;
            }
            let start = diagnostic.start();
            let end = diagnostic.end();
            if line < start.line() || line > end.line() {
                continue;
            }
            visitor(DiagnosticMarker {
                start_utf16: if line == start.line() {
                    start.utf16_character()
                } else {
                    0
                },
                end_utf16: (line == end.line()).then_some(end.utf16_character()),
                severity: diagnostic.severity(),
            })?;
            count += 1;
        }
        Ok(count)
    }

    pub(crate) fn snapshot(&self) -> RustDiagnosticsSnapshot {
        let (
            generation,
            process_epoch,
            lsp_version,
            diagnostic_version,
            diagnostic_items,
            diagnostic_bytes,
            completion_pending,
            completion_items,
            completion_bytes,
        ) = self
            .session
            .as_ref()
            .map_or((0, 0, 0, None, 0, 0, false, 0, 0), |session| {
                let diagnostics = session.diagnostics.as_ref();
                let completion = session.completion.as_ref();
                (
                    session.generation,
                    session.process_epoch,
                    session.lsp_version,
                    diagnostics.and_then(|value| value.batch.document_version()),
                    diagnostics.map_or(0, |value| value.batch.diagnostics().len()),
                    diagnostics.map_or(0, |value| value.batch.retained_bytes()),
                    session.pending_completion.is_some(),
                    completion.map_or(0, |value| value.batch.items().len()),
                    completion.map_or(0, |value| value.batch.retained_bytes()),
                )
            });
        let process = self
            .session
            .as_ref()
            .map(|session| session.client.snapshot().process)
            .unwrap_or_default();
        RustDiagnosticsSnapshot {
            active: self.session.is_some(),
            generation,
            process_epoch,
            lsp_version,
            diagnostic_publications: self.diagnostic_publications,
            diagnostic_version,
            diagnostic_items,
            diagnostic_bytes,
            peak_diagnostic_items: self.peak_diagnostic_items,
            peak_diagnostic_bytes: self.peak_diagnostic_bytes,
            completion_pending,
            completion_items,
            completion_bytes,
            peak_completion_items: self.peak_completion_items,
            peak_completion_bytes: self.peak_completion_bytes,
            completion_requests: self.completion_requests,
            completion_cancellations: self.completion_cancellations,
            stale_completions: self.stale_completions,
            completion_truncations: self.completion_truncations,
            process_retained_bytes: process.retained_bytes,
            process_queued_events: process.queued_events,
            process_submitted_inputs: process.submitted_inputs,
            process_written_inputs: process.written_inputs,
            process_input_saturations: process.input_saturations,
            polls: self.polls,
            stale_wakes: self.stale_wakes,
            stale_diagnostics: self.stale_diagnostics,
            restarts: self.restarts,
        }
    }

    pub(crate) fn shutdown(&mut self) -> RustDiagnosticsSnapshot {
        if let Some(session) = self.session.as_mut() {
            let _ = session.client.shutdown();
        }
        self.session = None;
        self.target = None;
        self.status = None;
        self.snapshot()
    }

    fn start<F>(
        &mut self,
        input: RustDocumentInput,
        target: Target,
        wake_factory: F,
    ) -> Result<(), RustDiagnosticsError>
    where
        F: FnOnce(LanguageWake) -> ProcessWake,
    {
        let executable = self
            .server_path
            .clone()
            .ok_or(RustDiagnosticsError::MissingServer)?;
        let generation = self
            .next_generation
            .checked_add(1)
            .ok_or(RustDiagnosticsError::GenerationExhausted)?;
        let process_identity = ProcessIdentity::new(input.identity.workspace_revision, generation)
            .ok_or(RustDiagnosticsError::InvalidIdentity)?;
        let spec = ProcessSpec::new(
            executable,
            std::iter::empty::<&str>(),
            Some(&input.workspace_root),
        )
        .map_err(RustDiagnosticsError::Configuration)?;
        let wake = wake_factory(LanguageWake { generation });
        let client = LspClient::start_with_waker(spec, process_identity, wake)
            .map_err(RustDiagnosticsError::Client)?;
        let document = LspDocument::from_file_path(&input.path, "rust", 1)
            .map_err(RustDiagnosticsError::Language)?;
        self.next_generation = generation;
        let synced_snapshot = input.snapshot.clone();
        self.session = Some(RustSession {
            target,
            identity: input.identity,
            generation,
            process_generation: generation,
            process_epoch: 1,
            lsp_version: 1,
            state: SessionState::Starting,
            snapshot: input.snapshot,
            synced_snapshot,
            pending_change: false,
            restart_count: 0,
            document,
            diagnostics: None,
            pending_completion: None,
            completion: None,
            client,
        });
        Ok(())
    }

    fn begin_initialize(&mut self, epoch: u64) -> bool {
        let Some(session) = self.session.as_mut() else {
            return false;
        };
        session.process_epoch = epoch;
        let params = initialize_params(&session.target.workspace_root);
        let result = params
            .map_err(RustDiagnosticsError::Language)
            .and_then(|params| {
                session
                    .client
                    .begin_initialize_with(&params)
                    .map_err(RustDiagnosticsError::Client)
            });
        if let Err(error) = result {
            return self.restart_or_fail(error);
        }
        session.state = SessionState::Initializing;
        replace_status(
            &mut self.status,
            Some(Arc::from("Rust analysis is initializing.")),
        )
    }

    fn open_document(&mut self) -> bool {
        let Some(session) = self.session.as_mut() else {
            return false;
        };
        let text = session.snapshot.text();
        let result = session
            .document
            .did_open_params(&text)
            .map_err(RustDiagnosticsError::Language)
            .and_then(|params| {
                session
                    .client
                    .notify("textDocument/didOpen", Some(&params))
                    .map_err(RustDiagnosticsError::Client)
            });
        if let Err(error) = result {
            return self.restart_or_fail(error);
        }
        session.state = SessionState::Open;
        session.synced_snapshot = session.snapshot.clone();
        session.pending_change = false;
        replace_status(&mut self.status, None)
    }

    fn flush_change(&mut self) -> bool {
        let Some(session) = self.session.as_mut() else {
            return false;
        };
        if !session.pending_change || session.state != SessionState::Open {
            return false;
        }
        let text = session.snapshot.text();
        let result = document_end(&session.synced_snapshot)
            .and_then(|previous_end| session.document.did_change_params(&text, previous_end))
            .map_err(RustDiagnosticsError::Language)
            .and_then(|params| {
                session
                    .client
                    .notify("textDocument/didChange", Some(&params))
                    .map_err(RustDiagnosticsError::Client)
            });
        match result {
            Ok(_) => {
                session.synced_snapshot = session.snapshot.clone();
                session.pending_change = false;
                replace_status(&mut self.status, None)
            }
            Err(error) => replace_status(&mut self.status, Some(Arc::from(error.to_string()))),
        }
    }

    fn admit(&mut self, candidate: Result<DiagnosticBatch, LanguageProtocolError>) -> bool {
        let Some(session) = self.session.as_mut() else {
            return false;
        };
        let batch = match candidate {
            Ok(batch) => batch,
            Err(error) => {
                self.stale_diagnostics = self.stale_diagnostics.saturating_add(1);
                return replace_status(&mut self.status, Some(Arc::from(error.to_string())));
            }
        };
        let admitted = AdmittedDiagnostics {
            identity: session.identity,
            process_epoch: session.process_epoch,
            batch,
        };
        if session.diagnostics.as_ref() == Some(&admitted) {
            return false;
        }
        self.diagnostic_publications = self.diagnostic_publications.saturating_add(1);
        self.peak_diagnostic_items = self
            .peak_diagnostic_items
            .max(admitted.batch.diagnostics().len());
        self.peak_diagnostic_bytes = self
            .peak_diagnostic_bytes
            .max(admitted.batch.retained_bytes());
        let status = admitted
            .batch
            .primary_message()
            .map(|message| Arc::from(format!("Rust: {message}")));
        session.diagnostics = Some(admitted);
        let _ = replace_status(&mut self.status, status);
        true
    }

    fn admit_completion(
        &mut self,
        id: u32,
        stamp: RequestStamp,
        candidate: Result<CompletionBatch, CompletionError>,
    ) -> bool {
        let Some(session) = self.session.as_mut() else {
            return false;
        };
        let Some(pending) = session.pending_completion.take() else {
            self.stale_completions = self.stale_completions.saturating_add(1);
            return false;
        };
        if pending.request_id != id
            || pending.stamp != stamp
            || pending.identity != session.identity
            || pending.process_epoch != session.process_epoch
            || pending.lsp_version != session.lsp_version
        {
            self.stale_completions = self.stale_completions.saturating_add(1);
            return false;
        }
        let batch = match candidate {
            Ok(batch) => batch,
            Err(error) => {
                return replace_status(
                    &mut self.status,
                    Some(Arc::from(
                        RustDiagnosticsError::Completion(error).to_string(),
                    )),
                );
            }
        };
        if batch.items().is_empty() {
            session.completion = None;
            return replace_status(&mut self.status, Some(Arc::from("No Rust completions.")));
        }
        self.peak_completion_items = self.peak_completion_items.max(batch.items().len());
        self.peak_completion_bytes = self.peak_completion_bytes.max(batch.retained_bytes());
        self.completion_truncations = self
            .completion_truncations
            .saturating_add(u64::from(batch.omitted_items() > 0));
        session.completion = Some(AdmittedCompletion {
            request_id: id,
            identity: pending.identity,
            process_epoch: pending.process_epoch,
            lsp_version: pending.lsp_version,
            batch,
            selected: 0,
            first_visible: 0,
        });
        let _ = replace_status(&mut self.status, None);
        true
    }

    fn reject_stale_completion(&mut self, id: u32) -> bool {
        let Some(session) = self.session.as_mut() else {
            return false;
        };
        let pending_matches = session
            .pending_completion
            .is_some_and(|pending| pending.request_id == id);
        if pending_matches {
            session.pending_completion = None;
        }
        let admitted_matches = session
            .completion
            .as_ref()
            .is_some_and(|completion| completion.request_id == id);
        if admitted_matches {
            session.completion = None;
        }
        self.stale_completions = self.stale_completions.saturating_add(1);
        admitted_matches
    }

    fn restart_or_fail(&mut self, error: RustDiagnosticsError) -> bool {
        let Some(session) = self.session.as_mut() else {
            return replace_status(&mut self.status, Some(Arc::from(error.to_string())));
        };
        if session.restart_count == MAX_RESTARTS_PER_DOCUMENT {
            session.diagnostics = None;
            session.pending_completion = None;
            session.completion = None;
            return replace_status(&mut self.status, Some(Arc::from(error.to_string())));
        }
        let Some(generation) = session.process_generation.checked_add(1) else {
            return self
                .fail(RustDiagnosticsError::GenerationExhausted)
                .visual_changed;
        };
        let Some(identity) = ProcessIdentity::new(session.identity.workspace_revision, generation)
        else {
            return self
                .fail(RustDiagnosticsError::InvalidIdentity)
                .visual_changed;
        };
        if let Err(restart_error) = session.client.restart(identity) {
            return replace_status(
                &mut self.status,
                Some(Arc::from(
                    RustDiagnosticsError::Client(restart_error).to_string(),
                )),
            );
        }
        session.process_generation = generation;
        session.restart_count += 1;
        session.state = SessionState::Starting;
        session.pending_change = false;
        session.diagnostics = None;
        session.pending_completion = None;
        session.completion = None;
        self.restarts = self.restarts.saturating_add(1);
        replace_status(
            &mut self.status,
            Some(Arc::from("Rust analysis is restarting.")),
        )
    }

    fn fail(&mut self, error: RustDiagnosticsError) -> LanguageEffect {
        if let Some(session) = self.session.as_mut() {
            session.diagnostics = None;
            session.pending_completion = None;
            session.completion = None;
        }
        LanguageEffect {
            visual_changed: replace_status(&mut self.status, Some(Arc::from(error.to_string()))),
            continuation: None,
        }
    }

    fn stop(&mut self) -> bool {
        let changed = self.session.is_some() || self.status.is_some();
        if let Some(session) = self.session.as_mut() {
            let _ = session.client.shutdown();
        }
        self.session = None;
        self.target = None;
        self.status = None;
        changed
    }

    #[cfg(test)]
    pub(crate) fn with_server(server_path: &Path) -> Self {
        Self {
            server_path: Some(server_path.to_path_buf()),
            ..Self::default()
        }
    }

    #[cfg(test)]
    pub(crate) fn force_continuation_once_for_test(&mut self) {
        self.force_continuation_once = true;
    }

    #[cfg(test)]
    pub(crate) fn current_wake_for_test(&self) -> Option<LanguageWake> {
        self.session.as_ref().map(|session| LanguageWake {
            generation: session.generation,
        })
    }

    #[cfg(test)]
    pub(crate) fn install_for_test(
        &mut self,
        input: RustDocumentInput,
        params: &serde_json::value::RawValue,
        executable: &Path,
    ) -> Result<(), RustDiagnosticsError> {
        let document = LspDocument::from_file_path(&input.path, "rust", 1)
            .map_err(RustDiagnosticsError::Language)?;
        let batch =
            DiagnosticBatch::admit(params, &document).map_err(RustDiagnosticsError::Language)?;
        self.target = Some(Target {
            path: input.path.clone(),
            workspace_root: input.workspace_root.clone(),
        });
        self.status = batch.primary_message().map(Arc::<str>::from);
        self.peak_diagnostic_items = batch.diagnostics().len();
        self.peak_diagnostic_bytes = batch.retained_bytes();
        self.session = Some(test_session(input, document, batch, executable));
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn install_completion_for_test(
        &mut self,
        request_id: u32,
        identity: LanguageIdentity,
        result: &serde_json::value::RawValue,
    ) -> Result<(), RustDiagnosticsError> {
        let batch = CompletionBatch::admit(result).map_err(RustDiagnosticsError::Completion)?;
        if batch.items().is_empty() {
            return Err(RustDiagnosticsError::Completion(CompletionError::Malformed));
        }
        let session = self
            .session
            .as_mut()
            .ok_or(RustDiagnosticsError::InvalidIdentity)?;
        self.peak_completion_items = self.peak_completion_items.max(batch.items().len());
        self.peak_completion_bytes = self.peak_completion_bytes.max(batch.retained_bytes());
        self.completion_truncations = self
            .completion_truncations
            .saturating_add(u64::from(batch.omitted_items() > 0));
        session.completion = Some(AdmittedCompletion {
            request_id,
            identity,
            process_epoch: session.process_epoch,
            lsp_version: session.lsp_version,
            batch,
            selected: 0,
            first_visible: 0,
        });
        Ok(())
    }
}

fn merge_visual_changed(current: &mut bool, observed: bool) {
    *current = *current || observed;
}

fn continuation_for_queued_events(queued_events: usize, generation: u64) -> Option<LanguageWake> {
    (queued_events != 0).then_some(LanguageWake { generation })
}

fn replace_status(status: &mut Option<Arc<str>>, next: Option<Arc<str>>) -> bool {
    if *status == next {
        false
    } else {
        *status = next;
        true
    }
}

fn document_end(snapshot: &BufferSnapshot) -> Result<LspPosition, LanguageProtocolError> {
    let last_line = snapshot
        .line_count()
        .checked_sub(1)
        .ok_or(LanguageProtocolError::InvalidRange)?;
    let range = snapshot
        .line_byte_range(last_line)
        .map_err(|_| LanguageProtocolError::InvalidRange)?;
    let text = snapshot
        .slice(range)
        .map_err(|_| LanguageProtocolError::InvalidRange)?;
    let utf16_character = text.chars().try_fold(0_usize, |units, character| {
        units
            .checked_add(character.len_utf16())
            .ok_or(LanguageProtocolError::InvalidPosition)
    })?;
    LspPosition::new(
        u32::try_from(last_line).map_err(|_| LanguageProtocolError::InvalidPosition)?,
        u32::try_from(utf16_character).map_err(|_| LanguageProtocolError::InvalidPosition)?,
    )
}

#[cfg(test)]
fn test_session(
    input: RustDocumentInput,
    document: LspDocument,
    batch: DiagnosticBatch,
    executable: &Path,
) -> RustSession {
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEST_SEQUENCE: AtomicUsize = AtomicUsize::new(1);
    let spec = ProcessSpec::new(
        executable,
        std::iter::empty::<&str>(),
        Some(&input.workspace_root),
    )
    .unwrap_or_else(|_| unreachable!());
    let generation = u64::try_from(TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed))
        .unwrap_or_else(|_| unreachable!());
    let identity = ProcessIdentity::new(input.identity.workspace_revision, generation)
        .unwrap_or_else(|| unreachable!());
    let client = LspClient::start(spec, identity).unwrap_or_else(|_| unreachable!());
    let synced_snapshot = input.snapshot.clone();
    RustSession {
        target: Target {
            path: input.path,
            workspace_root: input.workspace_root,
        },
        identity: input.identity,
        generation,
        process_generation: generation,
        process_epoch: 1,
        lsp_version: 1,
        state: SessionState::Open,
        snapshot: input.snapshot,
        synced_snapshot,
        pending_change: false,
        restart_count: 0,
        document,
        diagnostics: Some(AdmittedDiagnostics {
            identity: input.identity,
            process_epoch: 1,
            batch,
        }),
        pending_completion: None,
        completion: None,
        client,
    }
}

#[cfg(test)]
#[path = "rust_diagnostics_tests.rs"]
pub(crate) mod tests;

#[cfg(test)]
#[path = "rust_diagnostics_coverage_tests.rs"]
mod coverage_tests;
