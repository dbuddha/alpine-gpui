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
#[cfg(test)]
use serde_json::value::RawValue;

use crate::{
    lsp_client::{LspClient, LspClientError, LspClientPoll},
    lsp_json::{PeerEvent, RequestStamp, ResponseValue},
    lsp_language::{
        DiagnosticBatch, LanguageProtocolError, LspDocument, LspPosition, initialize_params,
    },
    lsp_process::{ConfigError, ProcessIdentity, ProcessSpec, ProcessWake, StopReason},
    rust_completion::{CompletionBatch, CompletionError, CompletionItem},
    rust_navigation::{HoverContent, NavigationError, SourceLocation, SourceLocations},
    rust_symbols::{
        SymbolBatch, SymbolError, SymbolPicker, SymbolPickerReport, SymbolRequestKind, SymbolRow,
        workspace_symbol_params,
    },
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
    #[cfg(any(test, alpine_native_validation))]
    pub(crate) const fn generation(self) -> u64 {
        self.generation
    }

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

    #[cfg(any(test, alpine_native_validation))]
    pub(crate) fn pending_generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct NavigationRow<'a> {
    pub(crate) label: &'a str,
    pub(crate) selected: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NavigationRequestKind {
    Hover,
    Definition,
    References,
}

impl NavigationRequestKind {
    fn from_method(method: &str) -> Option<Self> {
        match method {
            "textDocument/hover" => Some(Self::Hover),
            "textDocument/definition" => Some(Self::Definition),
            "textDocument/references" => Some(Self::References),
            _ => None,
        }
    }

    const fn method(self) -> &'static str {
        match self {
            Self::Hover => "textDocument/hover",
            Self::Definition => "textDocument/definition",
            Self::References => "textDocument/references",
        }
    }

    const fn empty_status(self) -> &'static str {
        match self {
            Self::Hover => "No Rust hover information.",
            Self::Definition => "No Rust definition found.",
            Self::References => "No Rust references found.",
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Hover => "Rust hover",
            Self::Definition => "Rust definition",
            Self::References => "Rust references",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct RustDiagnosticsSnapshot {
    pub(crate) active: bool,
    pending: RustDiagnosticsPending,
    pub(crate) generation: u64,
    pub(crate) process_epoch: u64,
    pub(crate) lsp_version: i32,
    pub(crate) diagnostic_publications: u64,
    pub(crate) diagnostic_version: Option<i32>,
    pub(crate) diagnostic_items: usize,
    pub(crate) diagnostic_bytes: usize,
    pub(crate) peak_diagnostic_items: usize,
    pub(crate) peak_diagnostic_bytes: usize,
    pub(crate) completion_items: usize,
    pub(crate) completion_bytes: usize,
    pub(crate) peak_completion_items: usize,
    pub(crate) peak_completion_bytes: usize,
    pub(crate) completion_requests: u64,
    pub(crate) completion_cancellations: u64,
    pub(crate) stale_completions: u64,
    pub(crate) completion_truncations: u64,
    pub(crate) hover_bytes: usize,
    pub(crate) location_items: usize,
    pub(crate) location_bytes: usize,
    pub(crate) peak_hover_bytes: usize,
    pub(crate) peak_location_items: usize,
    pub(crate) peak_location_bytes: usize,
    pub(crate) navigation_requests: u64,
    pub(crate) navigation_cancellations: u64,
    pub(crate) stale_navigation: u64,
    pub(crate) navigation_truncations: u64,
    pub(crate) symbol_items: usize,
    pub(crate) symbol_matches: usize,
    pub(crate) symbol_bytes: usize,
    pub(crate) peak_symbol_items: usize,
    pub(crate) peak_symbol_bytes: usize,
    pub(crate) symbol_requests: u64,
    pub(crate) symbol_cancellations: u64,
    pub(crate) stale_symbols: u64,
    pub(crate) symbol_truncations: u64,
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

#[cfg(test)]
impl RustDiagnosticsSnapshot {
    pub(crate) const fn completion_pending(&self) -> bool {
        self.pending.completion
    }

    pub(crate) const fn navigation_pending(&self) -> bool {
        self.pending.navigation
    }

    pub(crate) const fn symbol_pending(&self) -> bool {
        self.pending.symbols
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct RustDiagnosticsPending {
    completion: bool,
    navigation: bool,
    symbols: bool,
}

#[derive(Default)]
struct CurrentNavigationSnapshot {
    pending: bool,
    hover_bytes: usize,
    location_items: usize,
    location_bytes: usize,
}

#[derive(Default)]
struct CurrentSymbolSnapshot {
    pending: bool,
    report: SymbolPickerReport,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PendingNavigation {
    request_id: u32,
    stamp: RequestStamp,
    kind: NavigationRequestKind,
    identity: LanguageIdentity,
    process_epoch: u64,
    lsp_version: i32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum NavigationResult {
    Hover(HoverContent),
    Locations {
        kind: NavigationRequestKind,
        batch: SourceLocations,
        selected: usize,
        first_visible: usize,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AdmittedNavigation {
    request_id: u32,
    identity: LanguageIdentity,
    process_epoch: u64,
    lsp_version: i32,
    result: NavigationResult,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PendingSymbols {
    request_id: u32,
    stamp: RequestStamp,
    kind: SymbolRequestKind,
    identity: LanguageIdentity,
    process_epoch: u64,
    lsp_version: i32,
    query_revision: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AdmittedSymbols {
    identity: LanguageIdentity,
    process_epoch: u64,
    lsp_version: i32,
    picker: SymbolPicker,
}

enum NavigationCandidate {
    Hover(Result<Option<HoverContent>, NavigationError>),
    Locations(Result<SourceLocations, NavigationError>),
}

#[derive(Default)]
struct PollCandidates {
    initialized: bool,
    diagnostics: Option<Result<DiagnosticBatch, LanguageProtocolError>>,
    completion: Option<(u32, RequestStamp, Result<CompletionBatch, CompletionError>)>,
    navigation: Option<(
        u32,
        RequestStamp,
        NavigationRequestKind,
        NavigationCandidate,
    )>,
    symbols: Option<(
        u32,
        RequestStamp,
        SymbolRequestKind,
        Result<SymbolBatch, SymbolError>,
    )>,
    stale_response: Option<u32>,
}

#[cfg(all(test, target_os = "macos", target_arch = "aarch64"))]
thread_local! {
    static LAST_COMPLETION_RESPONSE: std::cell::RefCell<Option<Box<str>>> = const {
        std::cell::RefCell::new(None)
    };
}

fn completion_batch_from_response(
    value: ResponseValue<'_>,
) -> Result<CompletionBatch, CompletionError> {
    #[cfg(all(test, target_os = "macos", target_arch = "aarch64"))]
    LAST_COMPLETION_RESPONSE.with(|response| {
        response.replace(Some(match value {
            ResponseValue::Result(result) => Box::from(result.get()),
            ResponseValue::Error(_) => Box::from("error"),
        }));
    });
    match value {
        ResponseValue::Result(result) => CompletionBatch::admit(result),
        ResponseValue::Error(_) => Err(CompletionError::Malformed),
    }
}

fn navigation_from_response(
    kind: NavigationRequestKind,
    value: ResponseValue<'_>,
) -> NavigationCandidate {
    match (kind, value) {
        (NavigationRequestKind::Hover, ResponseValue::Result(result)) => {
            NavigationCandidate::Hover(HoverContent::admit(result))
        }
        (
            NavigationRequestKind::Definition | NavigationRequestKind::References,
            ResponseValue::Result(result),
        ) => NavigationCandidate::Locations(SourceLocations::admit(result)),
        (NavigationRequestKind::Hover, ResponseValue::Error(_)) => {
            NavigationCandidate::Hover(Err(NavigationError::Malformed))
        }
        (
            NavigationRequestKind::Definition | NavigationRequestKind::References,
            ResponseValue::Error(_),
        ) => NavigationCandidate::Locations(Err(NavigationError::Malformed)),
    }
}

fn symbols_from_response(
    kind: SymbolRequestKind,
    value: ResponseValue<'_>,
    document_uri: &str,
) -> Result<SymbolBatch, SymbolError> {
    match value {
        ResponseValue::Result(result) => SymbolBatch::admit(kind, result, document_uri),
        ResponseValue::Error(_) => Err(SymbolError::Malformed),
    }
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
    pending_navigation: Option<PendingNavigation>,
    navigation: Option<AdmittedNavigation>,
    pending_symbols: Option<PendingSymbols>,
    symbols: Option<AdmittedSymbols>,
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
    Navigation(NavigationError),
    Symbols(SymbolError),
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
    peak_hover_bytes: usize,
    peak_location_items: usize,
    peak_location_bytes: usize,
    navigation_requests: u64,
    navigation_cancellations: u64,
    stale_navigation: u64,
    navigation_truncations: u64,
    peak_symbol_items: usize,
    peak_symbol_bytes: usize,
    symbol_requests: u64,
    symbol_cancellations: u64,
    stale_symbols: u64,
    symbol_truncations: u64,
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
            peak_hover_bytes: 0,
            peak_location_items: 0,
            peak_location_bytes: 0,
            navigation_requests: 0,
            navigation_cancellations: 0,
            stale_navigation: 0,
            navigation_truncations: 0,
            peak_symbol_items: 0,
            peak_symbol_bytes: 0,
            symbol_requests: 0,
            symbol_cancellations: 0,
            stale_symbols: 0,
            symbol_truncations: 0,
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
            merge_visual_changed(&mut visual_changed, session.navigation.take().is_some());
            merge_visual_changed(&mut visual_changed, session.symbols.take().is_some());
            if let Some(pending) = session.pending_completion.take() {
                let _ = session.client.cancel(pending.request_id);
                self.completion_cancellations = self.completion_cancellations.saturating_add(1);
            }
            if let Some(pending) = session.pending_navigation.take() {
                let _ = session.client.cancel(pending.request_id);
                self.navigation_cancellations = self.navigation_cancellations.saturating_add(1);
            }
            if let Some(pending) = session.pending_symbols.take() {
                let _ = session.client.cancel(pending.request_id);
                self.symbol_cancellations = self.symbol_cancellations.saturating_add(1);
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
            let (poll, candidates) = match self.collect_poll_candidates() {
                Ok(value) => value,
                Err(error) => {
                    let restarted = self.restart_or_fail(RustDiagnosticsError::Client(error));
                    merge_visual_changed(&mut visual_changed, restarted);
                    break;
                }
            };
            if candidates.initialized {
                let opened = self.open_document();
                merge_visual_changed(&mut visual_changed, opened);
            }
            if let Some(candidate) = candidates.diagnostics {
                let admitted = self.admit(candidate);
                merge_visual_changed(&mut visual_changed, admitted);
            }
            if let Some(id) = candidates.stale_response {
                merge_visual_changed(&mut visual_changed, self.reject_stale_completion(id));
                merge_visual_changed(&mut visual_changed, self.reject_stale_navigation(id));
                merge_visual_changed(&mut visual_changed, self.reject_stale_symbols(id));
            }
            if let Some((id, stamp, batch)) = candidates.completion {
                merge_visual_changed(&mut visual_changed, self.admit_completion(id, stamp, batch));
            }
            if let Some((id, stamp, kind, candidate)) = candidates.navigation {
                merge_visual_changed(
                    &mut visual_changed,
                    self.admit_navigation(id, stamp, kind, candidate),
                );
            }
            if let Some((id, stamp, kind, candidate)) = candidates.symbols {
                merge_visual_changed(
                    &mut visual_changed,
                    self.admit_symbols(id, stamp, kind, candidate),
                );
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

    fn collect_poll_candidates(
        &mut self,
    ) -> Result<(LspClientPoll, PollCandidates), LspClientError> {
        let session = self.session.as_mut().unwrap_or_else(|| unreachable!());
        let expected = &session.document;
        let current = session
            .pending_symbols
            .map(|pending| pending.stamp)
            .or_else(|| session.pending_navigation.map(|pending| pending.stamp))
            .or_else(|| session.pending_completion.map(|pending| pending.stamp));
        let mut candidates = PollCandidates::default();
        let poll = session.client.poll(current, |event| match event {
            PeerEvent::Initialized(_) => candidates.initialized = true,
            PeerEvent::InboundNotification {
                method: "textDocument/publishDiagnostics",
                params: Some(params),
            } => candidates.diagnostics = Some(DiagnosticBatch::admit(params, expected)),
            PeerEvent::Response {
                id,
                method,
                stamp,
                value,
            } if method.as_ref() == "textDocument/completion" => {
                candidates.completion = Some((id, stamp, completion_batch_from_response(value)));
            }
            PeerEvent::Response {
                id,
                method,
                stamp,
                value,
            } => {
                if let Some(kind) = SymbolRequestKind::from_method(method.as_ref()) {
                    candidates.symbols = Some((
                        id,
                        stamp,
                        kind,
                        symbols_from_response(kind, value, expected.uri()),
                    ));
                } else if let Some(kind) = NavigationRequestKind::from_method(method.as_ref()) {
                    candidates.navigation =
                        Some((id, stamp, kind, navigation_from_response(kind, value)));
                }
            }
            PeerEvent::StaleResponse { id } => candidates.stale_response = Some(id),
            _ => {}
        })?;
        Ok((poll, candidates))
    }

    pub(crate) fn status_message(&self) -> Option<Arc<str>> {
        self.status.clone()
    }

    pub(crate) fn record_symbol_error(&mut self, error: SymbolError) -> LanguageEffect {
        LanguageEffect {
            visual_changed: replace_status(
                &mut self.status,
                Some(Arc::from(RustDiagnosticsError::Symbols(error).to_string())),
            ),
            continuation: None,
        }
    }

    pub(crate) fn request_completion(&mut self, position: LspPosition) -> LanguageEffect {
        let mut visual_changed = self.cancel_symbols();
        visual_changed |= self.cancel_navigation();
        visual_changed |= self.cancel_completion();
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

    pub(crate) fn request_navigation(
        &mut self,
        kind: NavigationRequestKind,
        position: LspPosition,
    ) -> LanguageEffect {
        let mut visual_changed = self.cancel_symbols();
        visual_changed |= self.cancel_completion();
        visual_changed |= self.cancel_navigation();
        let Some(session) = self.session.as_mut() else {
            return LanguageEffect {
                visual_changed: replace_status(
                    &mut self.status,
                    Some(Arc::from("Rust analysis is not ready for navigation.")),
                ) || visual_changed,
                continuation: None,
            };
        };
        if session.state != SessionState::Open {
            return LanguageEffect {
                visual_changed: replace_status(
                    &mut self.status,
                    Some(Arc::from("Rust analysis is not ready for navigation.")),
                ) || visual_changed,
                continuation: None,
            };
        }
        let Some(stamp) = session.identity.request_stamp() else {
            return self.fail(RustDiagnosticsError::InvalidIdentity);
        };
        let params = match kind {
            NavigationRequestKind::References => session.document.references_params(position),
            NavigationRequestKind::Hover | NavigationRequestKind::Definition => {
                session.document.position_params(position)
            }
        };
        let result = params
            .map_err(RustDiagnosticsError::Language)
            .and_then(|params| {
                session
                    .client
                    .begin_request(kind.method(), Some(&params), stamp)
                    .map_err(RustDiagnosticsError::Client)
            });
        match result {
            Ok(request) => {
                session.pending_navigation = Some(PendingNavigation {
                    request_id: request.request_id,
                    stamp,
                    kind,
                    identity: session.identity,
                    process_epoch: session.process_epoch,
                    lsp_version: session.lsp_version,
                });
                self.navigation_requests = self.navigation_requests.saturating_add(1);
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

    pub(crate) fn cancel_navigation(&mut self) -> bool {
        let Some(session) = self.session.as_mut() else {
            return false;
        };
        let mut changed = session.navigation.take().is_some();
        if let Some(pending) = session.pending_navigation.take() {
            match session.client.cancel(pending.request_id) {
                Ok(_) => {
                    self.navigation_cancellations = self.navigation_cancellations.saturating_add(1);
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

    pub(crate) fn open_symbols(&mut self, kind: SymbolRequestKind) -> LanguageEffect {
        let mut visual_changed = self.cancel_completion();
        visual_changed |= self.cancel_navigation();
        visual_changed |= self.cancel_symbols();
        let Some(session) = self.session.as_mut() else {
            return LanguageEffect {
                visual_changed: replace_status(
                    &mut self.status,
                    Some(Arc::from("Rust analysis is not ready for symbols.")),
                ) || visual_changed,
                continuation: None,
            };
        };
        if session.state != SessionState::Open {
            return LanguageEffect {
                visual_changed: replace_status(
                    &mut self.status,
                    Some(Arc::from("Rust analysis is not ready for symbols.")),
                ) || visual_changed,
                continuation: None,
            };
        }
        session.symbols = Some(AdmittedSymbols {
            identity: session.identity,
            process_epoch: session.process_epoch,
            lsp_version: session.lsp_version,
            picker: SymbolPicker::new(kind),
        });
        visual_changed = true;
        let mut effect = self.issue_symbol_request();
        effect.visual_changed |= visual_changed;
        effect
    }

    fn issue_symbol_request(&mut self) -> LanguageEffect {
        let Some(session) = self.session.as_mut() else {
            return LanguageEffect::default();
        };
        if let Some(pending) = session.pending_symbols.take() {
            match session.client.cancel(pending.request_id) {
                Ok(_) => {
                    self.symbol_cancellations = self.symbol_cancellations.saturating_add(1);
                }
                Err(error) => return self.fail(RustDiagnosticsError::Client(error)),
            }
        }
        let Some(symbols) = session.symbols.as_ref() else {
            return LanguageEffect::default();
        };
        let Some(stamp) = session.identity.request_stamp() else {
            return self.fail(RustDiagnosticsError::InvalidIdentity);
        };
        let kind = symbols.picker.kind();
        let query_revision = symbols.picker.query_revision();
        let params = match kind {
            SymbolRequestKind::Document => session
                .document
                .text_document_params()
                .map_err(RustDiagnosticsError::Language),
            SymbolRequestKind::Workspace => workspace_symbol_params(symbols.picker.query())
                .map_err(RustDiagnosticsError::Symbols),
        };
        let result = params.and_then(|params| {
            session
                .client
                .begin_request(kind.method(), Some(&params), stamp)
                .map_err(RustDiagnosticsError::Client)
        });
        match result {
            Ok(request) => {
                session.pending_symbols = Some(PendingSymbols {
                    request_id: request.request_id,
                    stamp,
                    kind,
                    identity: session.identity,
                    process_epoch: session.process_epoch,
                    lsp_version: session.lsp_version,
                    query_revision,
                });
                self.symbol_requests = self.symbol_requests.saturating_add(1);
                LanguageEffect {
                    visual_changed: replace_status(&mut self.status, None),
                    continuation: None,
                }
            }
            Err(error) => self.fail(error),
        }
    }

    pub(crate) fn cancel_symbols(&mut self) -> bool {
        let Some(session) = self.session.as_mut() else {
            return false;
        };
        let mut changed = session.symbols.take().is_some();
        if let Some(pending) = session.pending_symbols.take() {
            match session.client.cancel(pending.request_id) {
                Ok(_) => {
                    self.symbol_cancellations = self.symbol_cancellations.saturating_add(1);
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

    pub(crate) fn symbols_are_open(&self, identity: LanguageIdentity) -> bool {
        self.symbols(identity).is_some()
    }

    pub(crate) fn commit_symbol_text(
        &mut self,
        identity: LanguageIdentity,
        text: &str,
    ) -> LanguageEffect {
        let changed = match self.symbols_mut(identity) {
            Some(symbols) => symbols.picker.commit_text(text),
            None => return LanguageEffect::default(),
        };
        self.finish_symbol_query_change(changed)
    }

    pub(crate) fn delete_symbol_backward(&mut self, identity: LanguageIdentity) -> LanguageEffect {
        let changed = match self.symbols_mut(identity) {
            Some(symbols) => symbols.picker.delete_backward(),
            None => return LanguageEffect::default(),
        };
        self.finish_symbol_query_change(changed)
    }

    fn finish_symbol_query_change(&mut self, changed: Result<bool, SymbolError>) -> LanguageEffect {
        match changed {
            Ok(false) => LanguageEffect::default(),
            Ok(true) => {
                if let Some(symbols) = self
                    .session
                    .as_mut()
                    .and_then(|session| session.symbols.as_mut())
                {
                    let _ = symbols.picker.clear_results();
                }
                let mut effect = self.issue_symbol_request();
                effect.visual_changed = true;
                effect
            }
            Err(error) => LanguageEffect {
                visual_changed: replace_status(
                    &mut self.status,
                    Some(Arc::from(RustDiagnosticsError::Symbols(error).to_string())),
                ),
                continuation: None,
            },
        }
    }

    pub(crate) fn begin_symbol_composition(&mut self, identity: LanguageIdentity) -> bool {
        self.symbols_mut(identity)
            .is_some_and(|symbols| symbols.picker.begin_composition())
    }

    pub(crate) fn update_symbol_composition(
        &mut self,
        identity: LanguageIdentity,
        text: &str,
        selected_start_utf16: u32,
        selected_length_utf16: u32,
    ) -> Result<bool, SymbolError> {
        self.symbols_mut(identity)
            .ok_or(SymbolError::InvalidComposition)?
            .picker
            .update_composition(text, selected_start_utf16, selected_length_utf16)
    }

    pub(crate) fn cancel_symbol_composition(&mut self, identity: LanguageIdentity) -> bool {
        self.symbols_mut(identity)
            .is_some_and(|symbols| symbols.picker.cancel_composition())
    }

    pub(crate) fn symbol_display_text(
        &self,
        identity: LanguageIdentity,
    ) -> Result<Option<String>, SymbolError> {
        self.symbols(identity)
            .map(|symbols| symbols.picker.display_text())
            .transpose()
    }

    pub(crate) fn symbol_visible_range(&self, identity: LanguageIdentity) -> Option<Range<usize>> {
        Some(self.symbols(identity)?.picker.visible_range())
    }

    pub(crate) fn symbol_row(
        &self,
        identity: LanguageIdentity,
        index: usize,
    ) -> Option<SymbolRow<'_>> {
        self.symbols(identity)?.picker.row(index)
    }

    pub(crate) fn navigate_symbols(&mut self, delta: isize) -> bool {
        self.session
            .as_mut()
            .and_then(|session| session.symbols.as_mut())
            .is_some_and(|symbols| symbols.picker.navigate(delta))
    }

    pub(crate) fn selected_symbol_location(
        &self,
        identity: LanguageIdentity,
    ) -> Option<SourceLocation> {
        self.symbols(identity)?.picker.selected_location()
    }

    pub(crate) fn symbol_accessibility_label(
        &self,
        identity: LanguageIdentity,
    ) -> Option<Arc<str>> {
        Some(self.symbols(identity)?.picker.accessibility_label())
    }

    #[cfg(test)]
    pub(crate) fn symbol_report(&self, identity: LanguageIdentity) -> Option<SymbolPickerReport> {
        Some(self.symbols(identity)?.picker.report())
    }

    pub(crate) fn navigation_is_open(&self, identity: LanguageIdentity) -> bool {
        self.navigation(identity).is_some()
    }

    pub(crate) fn hover_content(&self, identity: LanguageIdentity) -> Option<&HoverContent> {
        match &self.navigation(identity)?.result {
            NavigationResult::Hover(hover) => Some(hover),
            NavigationResult::Locations { .. } => None,
        }
    }

    pub(crate) fn hover_visible_line_count(&self, identity: LanguageIdentity) -> usize {
        self.hover_content(identity)
            .map_or(0, |hover| hover.visible_lines().count())
    }

    pub(crate) fn hover_line(&self, identity: LanguageIdentity, index: usize) -> Option<&str> {
        self.hover_content(identity)?.visible_lines().nth(index)
    }

    pub(crate) fn navigation_visible_range(
        &self,
        identity: LanguageIdentity,
    ) -> Option<Range<usize>> {
        match &self.navigation(identity)?.result {
            NavigationResult::Locations {
                batch,
                first_visible,
                ..
            } => Some(batch.visible_range(*first_visible)),
            NavigationResult::Hover(_) => None,
        }
    }

    pub(crate) fn navigation_row(
        &self,
        identity: LanguageIdentity,
        index: usize,
    ) -> Option<NavigationRow<'_>> {
        match &self.navigation(identity)?.result {
            NavigationResult::Locations {
                batch, selected, ..
            } => batch.locations().get(index).map(|location| NavigationRow {
                label: location.uri(),
                selected: index == *selected,
            }),
            NavigationResult::Hover(_) => None,
        }
    }

    pub(crate) fn navigate_navigation(&mut self, delta: isize) -> bool {
        let Some(NavigationResult::Locations {
            batch,
            selected,
            first_visible,
            ..
        }) = self
            .session
            .as_mut()
            .and_then(|session| session.navigation.as_mut())
            .map(|navigation| &mut navigation.result)
        else {
            return false;
        };
        let previous = *selected;
        *selected = selected
            .saturating_add_signed(delta)
            .min(batch.locations().len().saturating_sub(1));
        if (*selected).cmp(&*first_visible).is_lt() {
            *first_visible = *selected;
        } else if *selected
            >= first_visible.saturating_add(crate::rust_navigation::MAX_VISIBLE_SOURCE_LOCATIONS)
        {
            *first_visible = selected
                .saturating_add(1)
                .saturating_sub(crate::rust_navigation::MAX_VISIBLE_SOURCE_LOCATIONS);
        }
        previous != *selected
    }

    pub(crate) fn selected_source_location(
        &self,
        identity: LanguageIdentity,
    ) -> Option<SourceLocation> {
        match &self.navigation(identity)?.result {
            NavigationResult::Locations {
                batch, selected, ..
            } => batch.locations().get(*selected).cloned(),
            NavigationResult::Hover(_) => None,
        }
    }

    pub(crate) fn navigation_has_target(&self, identity: LanguageIdentity) -> bool {
        self.selected_source_location(identity).is_some()
    }

    pub(crate) fn navigation_accessibility_label(
        &self,
        identity: LanguageIdentity,
    ) -> Option<Arc<str>> {
        let navigation = self.navigation(identity)?;
        Some(match &navigation.result {
            NavigationResult::Hover(hover) => Arc::from(format!("Rust hover: {}", hover.text())),
            NavigationResult::Locations {
                kind,
                batch,
                selected,
                ..
            } => {
                let location = batch.locations().get(*selected)?;
                Arc::from(format!(
                    "{}: {} result(s), selected {}",
                    kind.label(),
                    batch.locations().len(),
                    location.uri()
                ))
            }
        })
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
        let previous = completion.selected;
        completion.selected = completion
            .selected
            .saturating_add_signed(delta)
            .min(count.saturating_sub(1));
        if matches!(
            completion.selected.cmp(&completion.first_visible),
            std::cmp::Ordering::Less
        ) {
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

    fn current_navigation_snapshot(&self) -> CurrentNavigationSnapshot {
        let Some(session) = self.session.as_ref() else {
            return CurrentNavigationSnapshot::default();
        };
        let pending = session.pending_navigation.is_some();
        match session.navigation.as_ref().map(|value| &value.result) {
            Some(NavigationResult::Hover(hover)) => CurrentNavigationSnapshot {
                pending,
                hover_bytes: hover.retained_bytes(),
                ..CurrentNavigationSnapshot::default()
            },
            Some(NavigationResult::Locations { batch, .. }) => CurrentNavigationSnapshot {
                pending,
                location_items: batch.locations().len(),
                location_bytes: batch.retained_bytes(),
                ..CurrentNavigationSnapshot::default()
            },
            None => CurrentNavigationSnapshot {
                pending,
                ..CurrentNavigationSnapshot::default()
            },
        }
    }

    fn current_symbol_snapshot(&self) -> CurrentSymbolSnapshot {
        let Some(session) = self.session.as_ref() else {
            return CurrentSymbolSnapshot::default();
        };
        CurrentSymbolSnapshot {
            pending: session.pending_symbols.is_some(),
            report: session
                .symbols
                .as_ref()
                .map_or(SymbolPickerReport::default(), |symbols| {
                    symbols.picker.report()
                }),
        }
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
        let navigation = self.current_navigation_snapshot();
        let symbols = self.current_symbol_snapshot();
        let process = self
            .session
            .as_ref()
            .map(|session| session.client.snapshot().process)
            .unwrap_or_default();
        RustDiagnosticsSnapshot {
            active: self.session.is_some(),
            pending: RustDiagnosticsPending {
                completion: completion_pending,
                navigation: navigation.pending,
                symbols: symbols.pending,
            },
            generation,
            process_epoch,
            lsp_version,
            diagnostic_publications: self.diagnostic_publications,
            diagnostic_version,
            diagnostic_items,
            diagnostic_bytes,
            peak_diagnostic_items: self.peak_diagnostic_items,
            peak_diagnostic_bytes: self.peak_diagnostic_bytes,
            completion_items,
            completion_bytes,
            peak_completion_items: self.peak_completion_items,
            peak_completion_bytes: self.peak_completion_bytes,
            completion_requests: self.completion_requests,
            completion_cancellations: self.completion_cancellations,
            stale_completions: self.stale_completions,
            completion_truncations: self.completion_truncations,
            hover_bytes: navigation.hover_bytes,
            location_items: navigation.location_items,
            location_bytes: navigation.location_bytes,
            peak_hover_bytes: self.peak_hover_bytes,
            peak_location_items: self.peak_location_items,
            peak_location_bytes: self.peak_location_bytes,
            navigation_requests: self.navigation_requests,
            navigation_cancellations: self.navigation_cancellations,
            stale_navigation: self.stale_navigation,
            navigation_truncations: self.navigation_truncations,
            symbol_items: symbols.report.items,
            symbol_matches: symbols.report.matches,
            symbol_bytes: symbols.report.retained_bytes,
            peak_symbol_items: self.peak_symbol_items,
            peak_symbol_bytes: self.peak_symbol_bytes,
            symbol_requests: self.symbol_requests,
            symbol_cancellations: self.symbol_cancellations,
            stale_symbols: self.stale_symbols,
            symbol_truncations: self.symbol_truncations,
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
            pending_navigation: None,
            navigation: None,
            pending_symbols: None,
            symbols: None,
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
            .saturating_add(u64::from(batch.was_truncated()));
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

    fn admit_navigation(
        &mut self,
        id: u32,
        stamp: RequestStamp,
        kind: NavigationRequestKind,
        candidate: NavigationCandidate,
    ) -> bool {
        let Some(session) = self.session.as_mut() else {
            return false;
        };
        let Some(pending) = session.pending_navigation.take() else {
            self.stale_navigation = self.stale_navigation.saturating_add(1);
            return false;
        };
        if pending.request_id != id
            || pending.stamp != stamp
            || pending.kind != kind
            || pending.identity != session.identity
            || pending.process_epoch != session.process_epoch
            || pending.lsp_version != session.lsp_version
        {
            self.stale_navigation = self.stale_navigation.saturating_add(1);
            return false;
        }
        let result = match candidate {
            NavigationCandidate::Hover(Ok(Some(hover))) => {
                self.peak_hover_bytes = self.peak_hover_bytes.max(hover.retained_bytes());
                NavigationResult::Hover(hover)
            }
            NavigationCandidate::Locations(Ok(batch)) if !batch.locations().is_empty() => {
                self.peak_location_items = self.peak_location_items.max(batch.locations().len());
                self.peak_location_bytes = self.peak_location_bytes.max(batch.retained_bytes());
                self.navigation_truncations = self
                    .navigation_truncations
                    .saturating_add(u64::from(batch.omitted() > 0));
                NavigationResult::Locations {
                    kind,
                    batch,
                    selected: 0,
                    first_visible: 0,
                }
            }
            NavigationCandidate::Hover(Ok(None)) | NavigationCandidate::Locations(Ok(_)) => {
                session.navigation = None;
                return replace_status(&mut self.status, Some(Arc::from(kind.empty_status())));
            }
            NavigationCandidate::Hover(Err(error)) | NavigationCandidate::Locations(Err(error)) => {
                session.navigation = None;
                return replace_status(
                    &mut self.status,
                    Some(Arc::from(
                        RustDiagnosticsError::Navigation(error).to_string(),
                    )),
                );
            }
        };
        session.navigation = Some(AdmittedNavigation {
            request_id: id,
            identity: pending.identity,
            process_epoch: pending.process_epoch,
            lsp_version: pending.lsp_version,
            result,
        });
        let _ = replace_status(&mut self.status, None);
        true
    }

    fn reject_stale_navigation(&mut self, id: u32) -> bool {
        let Some(session) = self.session.as_mut() else {
            return false;
        };
        let pending_matches = session
            .pending_navigation
            .is_some_and(|pending| pending.request_id == id);
        if pending_matches {
            session.pending_navigation = None;
        }
        let admitted_matches = session
            .navigation
            .as_ref()
            .is_some_and(|navigation| navigation.request_id == id);
        if admitted_matches {
            session.navigation = None;
        }
        self.stale_navigation = self.stale_navigation.saturating_add(1);
        admitted_matches
    }

    fn admit_symbols(
        &mut self,
        id: u32,
        stamp: RequestStamp,
        kind: SymbolRequestKind,
        candidate: Result<SymbolBatch, SymbolError>,
    ) -> bool {
        let Some(session) = self.session.as_mut() else {
            return false;
        };
        let Some(pending) = session.pending_symbols.take() else {
            self.stale_symbols = self.stale_symbols.saturating_add(1);
            return false;
        };
        let Some(symbols) = session.symbols.as_mut() else {
            self.stale_symbols = self.stale_symbols.saturating_add(1);
            return false;
        };
        if pending.request_id != id
            || pending.stamp != stamp
            || pending.kind != kind
            || pending.identity != session.identity
            || pending.process_epoch != session.process_epoch
            || pending.lsp_version != session.lsp_version
            || symbols.picker.query_revision() != pending.query_revision
        {
            self.stale_symbols = self.stale_symbols.saturating_add(1);
            return false;
        }
        let batch = match candidate {
            Ok(batch) => batch,
            Err(error) => {
                let _ = symbols.picker.clear_results();
                return replace_status(
                    &mut self.status,
                    Some(Arc::from(RustDiagnosticsError::Symbols(error).to_string())),
                );
            }
        };
        let omitted = batch.omitted();
        if let Err(error) = symbols.picker.admit(batch) {
            let _ = symbols.picker.clear_results();
            return replace_status(
                &mut self.status,
                Some(Arc::from(RustDiagnosticsError::Symbols(error).to_string())),
            );
        }
        let report = symbols.picker.report();
        self.peak_symbol_items = self.peak_symbol_items.max(report.items);
        self.peak_symbol_bytes = self.peak_symbol_bytes.max(report.retained_bytes);
        self.symbol_truncations = self
            .symbol_truncations
            .saturating_add(u64::from(omitted > 0));
        let status = if report.matches == 0 {
            Some(Arc::from(kind.empty_status()))
        } else if omitted > 0 {
            Some(Arc::from(format!(
                "Rust symbols truncated: {omitted} result(s) omitted."
            )))
        } else {
            None
        };
        let _ = replace_status(&mut self.status, status);
        true
    }

    fn reject_stale_symbols(&mut self, id: u32) -> bool {
        let Some(session) = self.session.as_mut() else {
            return false;
        };
        let pending_matches = session
            .pending_symbols
            .is_some_and(|pending| pending.request_id == id);
        if pending_matches {
            session.pending_symbols = None;
        }
        self.stale_symbols = self.stale_symbols.saturating_add(1);
        false
    }

    fn symbols(&self, identity: LanguageIdentity) -> Option<&AdmittedSymbols> {
        let session = self.session.as_ref()?;
        let symbols = session.symbols.as_ref()?;
        (symbols.identity == identity
            && symbols.process_epoch == session.process_epoch
            && symbols.lsp_version == session.lsp_version)
            .then_some(symbols)
    }

    fn symbols_mut(&mut self, identity: LanguageIdentity) -> Option<&mut AdmittedSymbols> {
        let session = self.session.as_mut()?;
        let symbols = session.symbols.as_mut()?;
        (symbols.identity == identity
            && symbols.process_epoch == session.process_epoch
            && symbols.lsp_version == session.lsp_version)
            .then_some(symbols)
    }

    fn navigation(&self, identity: LanguageIdentity) -> Option<&AdmittedNavigation> {
        let session = self.session.as_ref()?;
        let navigation = session.navigation.as_ref()?;
        (navigation.identity == identity
            && navigation.process_epoch == session.process_epoch
            && navigation.lsp_version == session.lsp_version)
            .then_some(navigation)
    }

    #[cfg(test)]
    pub(crate) fn install_navigation_for_test(
        &mut self,
        identity: LanguageIdentity,
        kind: NavigationRequestKind,
        result: &RawValue,
    ) -> Result<(), NavigationError> {
        let session = self.session.as_mut().ok_or(NavigationError::Malformed)?;
        if session.identity != identity {
            return Err(NavigationError::Malformed);
        }
        let admitted = match navigation_from_response(kind, ResponseValue::Result(result)) {
            NavigationCandidate::Hover(Ok(Some(hover))) => NavigationResult::Hover(hover),
            NavigationCandidate::Locations(Ok(batch)) if !batch.locations().is_empty() => {
                NavigationResult::Locations {
                    kind,
                    batch,
                    selected: 0,
                    first_visible: 0,
                }
            }
            NavigationCandidate::Hover(Ok(None) | Err(_))
            | NavigationCandidate::Locations(Ok(_) | Err(_)) => {
                return Err(NavigationError::Malformed);
            }
        };
        session.navigation = Some(AdmittedNavigation {
            request_id: 0,
            identity,
            process_epoch: session.process_epoch,
            lsp_version: session.lsp_version,
            result: admitted,
        });
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn install_symbols_for_test(
        &mut self,
        identity: LanguageIdentity,
        kind: SymbolRequestKind,
        result: &RawValue,
    ) -> Result<(), SymbolError> {
        let session = self.session.as_mut().ok_or(SymbolError::Malformed)?;
        if session.identity != identity {
            return Err(SymbolError::Malformed);
        }
        let batch = SymbolBatch::admit(kind, result, session.document.uri())?;
        let mut picker = SymbolPicker::new(kind);
        let _ = picker.admit(batch)?;
        session.symbols = Some(AdmittedSymbols {
            identity,
            process_epoch: session.process_epoch,
            lsp_version: session.lsp_version,
            picker,
        });
        Ok(())
    }

    fn restart_or_fail(&mut self, error: RustDiagnosticsError) -> bool {
        let Some(session) = self.session.as_mut() else {
            return replace_status(&mut self.status, Some(Arc::from(error.to_string())));
        };
        if session.restart_count == MAX_RESTARTS_PER_DOCUMENT {
            session.diagnostics = None;
            session.pending_completion = None;
            session.completion = None;
            session.pending_navigation = None;
            session.navigation = None;
            session.pending_symbols = None;
            session.symbols = None;
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
        session.pending_navigation = None;
        session.navigation = None;
        session.pending_symbols = None;
        session.symbols = None;
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
            session.pending_navigation = None;
            session.navigation = None;
            session.pending_symbols = None;
            session.symbols = None;
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
            .saturating_add(u64::from(batch.was_truncated()));
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
    _executable: &Path,
) -> RustSession {
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEST_SEQUENCE: AtomicUsize = AtomicUsize::new(1);
    let generation = u64::try_from(TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed))
        .unwrap_or_else(|_| unreachable!());
    let identity = ProcessIdentity::new(input.identity.workspace_revision, generation)
        .unwrap_or_else(|| unreachable!());
    let client = LspClient::inert_for_test(identity);
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
        pending_navigation: None,
        navigation: None,
        pending_symbols: None,
        symbols: None,
        client,
    }
}

#[cfg(test)]
#[path = "rust_diagnostics_tests.rs"]
pub(crate) mod tests;

#[cfg(test)]
#[path = "rust_diagnostics_coverage_tests.rs"]
mod coverage_tests;
