//! Request-correlated diagnostics for the complete admitted workspace overlay.
//!
//! A server result ID is deliberately not used as a freshness certificate.
//! At most one request and three attempts per unchanged target are retained.

use super::{
    Arc, DiagnosticBatch, LanguageIdentity, LanguageProtocolError, PollCandidates, RequestStamp,
    RustDiagnostics, RustDiagnosticsError, RustSession, replace_status,
};
use crate::{
    lsp_client::{LspClientError, PreparedCancellation},
    lsp_process::SubmitError,
};
use serde_json::value::RawValue;

const MAX_ATTEMPTS: u8 = 3;

pub(super) fn is_input_pressure(error: RustDiagnosticsError) -> bool {
    matches!(
        error,
        RustDiagnosticsError::Client(LspClientError::Submit(
            SubmitError::Saturated | SubmitError::RetainedBudget
        ))
    )
}

pub(super) fn batch_from_response(
    value: super::ResponseValue<'_>,
    expected: &super::LspDocument,
) -> Result<DiagnosticBatch, LanguageProtocolError> {
    match value {
        super::ResponseValue::Result(result) => DiagnosticBatch::admit_pull(result, expected),
        super::ResponseValue::Error(error) => {
            Err(LanguageProtocolError::from_diagnostic_error(error))
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct DiagnosticKey {
    workspace: u64,
    workspace_revision: u64,
    document: u64,
    buffer_revision: u64,
    overlay_epoch: u64,
    process_epoch: u64,
    lsp_version: i32,
}

impl DiagnosticKey {
    fn new(
        identity: LanguageIdentity,
        overlay_epoch: u64,
        process_epoch: u64,
        lsp_version: i32,
    ) -> Self {
        Self {
            workspace: identity.workspace_id,
            workspace_revision: identity.workspace_revision,
            document: identity.document_id,
            buffer_revision: identity.buffer_revision,
            overlay_epoch,
            process_epoch,
            lsp_version,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct PendingDiagnostic {
    pub(super) request_id: u32,
    pub(super) stamp: RequestStamp,
    key: DiagnosticKey,
}

#[derive(Default)]
pub(super) struct PullState {
    pub(super) enabled: bool,
    pub(super) pending: Option<PendingDiagnostic>,
    // Local authority is already revoked. Only delivery remains, in this
    // process epoch; reset_overlay_transport retires this bounded obligation.
    deferred_cancel: Option<PreparedCancellation>,
    epoch: u64,
    attempted: Option<DiagnosticKey>,
    settled: Option<DiagnosticKey>,
    attempts: u8,
}

impl PullState {
    fn invalidate(&mut self) -> Result<Option<u32>, RustDiagnosticsError> {
        self.epoch = self
            .epoch
            .checked_add(1)
            .ok_or(RustDiagnosticsError::GenerationExhausted)?;
        self.attempted = None;
        self.settled = None;
        self.attempts = 0;
        Ok(self.pending.take().map(|pending| pending.request_id))
    }

    fn admit_attempt(&mut self, key: DiagnosticKey) -> bool {
        if self.attempted != Some(key) {
            self.attempted = Some(key);
            self.attempts = 0;
        }
        if self.settled == Some(key) || self.attempts == MAX_ATTEMPTS {
            return false;
        }
        self.attempts += 1;
        true
    }

    fn take_matching(&mut self, id: u32) -> Option<PendingDiagnostic> {
        if self.pending.is_some_and(|pending| pending.request_id == id) {
            self.pending.take()
        } else {
            None
        }
    }

    fn refund_interrupted_attempt(&mut self, pending: PendingDiagnostic) {
        // View changes and stale publication do not constitute server failures.
        // Do not refund an attempt belonging to a different current target.
        self.refund_attempt(pending.key);
    }

    fn refund_attempt(&mut self, key: DiagnosticKey) {
        if self.attempted == Some(key) {
            self.attempts = self.attempts.saturating_sub(1);
        }
    }
}

impl RustSession {
    fn diagnostic_key(&self) -> DiagnosticKey {
        DiagnosticKey::new(
            self.identity,
            self.diagnostic_pull.epoch,
            self.process_epoch,
            self.lsp_version,
        )
    }

    pub(super) fn invalidate_workspace_diagnostics(
        &mut self,
    ) -> Result<bool, RustDiagnosticsError> {
        // Revoke authority before attempting a fallible cancellation write.
        let pending = self.diagnostic_pull.invalidate()?;
        let changed = self.clear_workspace_diagnostics();
        if let Some(id) = pending {
            self.cancel_diagnostic(id)?;
        }
        Ok(changed)
    }

    fn cancel_diagnostic(&mut self, id: u32) -> Result<(), RustDiagnosticsError> {
        let prepared = self
            .client
            .prepare_cancel(id)
            .map_err(RustDiagnosticsError::Client)?;
        // Store ownership before a fallible send; repeated invalidation cannot
        // add a replacement request while this one cancellation is deferred.
        self.diagnostic_pull.deferred_cancel = Some(prepared);
        self.flush_diagnostic_cancel().map(|_| ())
    }

    fn flush_diagnostic_cancel(&mut self) -> Result<bool, RustDiagnosticsError> {
        let Some(prepared) = self.diagnostic_pull.deferred_cancel.as_mut() else {
            return Ok(true);
        };
        // Send the original prepared bytes, never generic notify or a second
        // peer cancellation. A late response may have consumed its tombstone.
        match self
            .client
            .send_cancel(prepared)
            .map_err(RustDiagnosticsError::Client)
        {
            Ok(_) | Err(RustDiagnosticsError::Client(LspClientError::StaleCancellation)) => {
                // A retired token cannot restart a healthy replacement.
                self.diagnostic_pull.deferred_cancel = None;
                Ok(true)
            }
            Err(error) if is_input_pressure(error) => Ok(false),
            Err(error) => Err(error),
        }
    }
}

fn request_params(
    uri: &str,
    identifier: Option<&str>,
) -> Result<Box<RawValue>, LanguageProtocolError> {
    let mut params = serde_json::json!({ "textDocument": { "uri": uri } });
    if let Some(identifier) = identifier {
        params["identifier"] = serde_json::Value::String(identifier.into());
    }
    RawValue::from_string(params.to_string()).map_err(|_| LanguageProtocolError::AllocationFailed)
}

impl RustDiagnostics {
    pub(super) fn apply_diagnostic_candidates(&mut self, candidates: &mut PollCandidates) -> bool {
        let mut changed = false;
        if let Some(supported) = candidates.initialized {
            if let Some(session) = self.session.as_mut() {
                session.diagnostic_pull.enabled = supported;
            }
            changed |= self.open_document();
            if !supported {
                changed |= replace_status(
                    &mut self.status,
                    Some(Arc::from(
                        "Rust diagnostics require inter-file pull diagnostic support.",
                    )),
                );
            }
        }
        if candidates.diagnostic_refresh {
            // The peer has already retired responses in this input batch.
            // Invalidate their publication authority without trying to cancel
            // a request which the transport no longer owns, in either wire order.
            if let Some((id, _, _)) = candidates.diagnostic_response.take() {
                self.reject_stale_diagnostic(id);
            }
            if let Some(id) = candidates.stale_diagnostic.take() {
                self.reject_stale_diagnostic(id);
            }
            changed |= self.refresh_diagnostics();
        }
        if candidates.ignored_diagnostics {
            self.stale_diagnostics = self.stale_diagnostics.saturating_add(1);
        }
        if let Some(id) = candidates.stale_diagnostic {
            self.reject_stale_diagnostic(id);
        }
        if let Some((id, stamp, batch)) = candidates.diagnostic_response.take() {
            changed |= self.admit_pull_diagnostics(id, stamp, batch);
        }
        changed
    }

    pub(super) fn refresh_diagnostics(&mut self) -> bool {
        // A server diagnostic refresh does not change document or view identity.
        // Real workspace changes revoke language views at their own boundary.
        let Some(session) = self.session.as_mut() else {
            return false;
        };
        match session.invalidate_workspace_diagnostics() {
            Ok(invalidated) => replace_status(&mut self.status, None) || invalidated,
            Err(error) => self.restart_or_fail(error),
        }
    }

    pub(super) fn pump_diagnostics(&mut self) -> bool {
        let Some(session) = self.session.as_mut() else {
            return false;
        };
        // Existing writer/event progress drives retries, not an idle loop.
        // Cancellation delivery does not require an active or ready view.
        match session.flush_diagnostic_cancel() {
            Ok(true) => {}
            Ok(false) => return false,
            Err(error) => return self.restart_or_fail(error),
        }
        if !session.diagnostic_pull.enabled {
            return false;
        }
        let key = session.diagnostic_key();
        if session
            .diagnostic_pull
            .pending
            .is_some_and(|pending| pending.key != key || !session.workspace_ready())
        {
            let pending = session
                .diagnostic_pull
                .pending
                .take()
                .unwrap_or_else(|| unreachable!());
            session.diagnostic_pull.refund_interrupted_attempt(pending);
            if let Err(error) = session.cancel_diagnostic(pending.request_id) {
                return self.restart_or_fail(error);
            }
        }
        if !session.workspace_ready()
            || session.diagnostic_pull.deferred_cancel.is_some()
            || session.diagnostic_pull.pending.is_some()
            || session.diagnostics.is_some()
            || !session.diagnostic_pull.admit_attempt(key)
        {
            return false;
        }
        let Some(stamp) = session.identity.request_stamp() else {
            return replace_status(
                &mut self.status,
                Some(Arc::from("Invalid Rust diagnostic identity.")),
            );
        };
        let result = request_params(
            session.document.uri(),
            session.client.diagnostic_provider_identifier(),
        )
        .map_err(RustDiagnosticsError::Language)
        .and_then(|params| {
            session
                .client
                .begin_request("textDocument/diagnostic", Some(&params), stamp)
                .map_err(RustDiagnosticsError::Client)
        });
        match result {
            Ok(request) => {
                session.diagnostic_pull.pending = Some(PendingDiagnostic {
                    request_id: request.request_id,
                    stamp,
                    key,
                });
                false
            }
            Err(error) if is_input_pressure(error) => {
                // submit_pending already rolled back the unsent peer request.
                // Preserve previous server failures; this was not an attempt
                // admitted to the server and must not consume the retry budget.
                session.diagnostic_pull.refund_attempt(key);
                false
            }
            Err(error) => self.restart_or_fail(error),
        }
    }

    pub(super) fn reject_stale_diagnostic(&mut self, id: u32) {
        if let Some(session) = self.session.as_mut()
            && let Some(pending) = session.diagnostic_pull.take_matching(id)
        {
            session.diagnostic_pull.refund_interrupted_attempt(pending);
        }
        self.stale_diagnostics = self.stale_diagnostics.saturating_add(1);
    }

    pub(super) fn admit_pull_diagnostics(
        &mut self,
        id: u32,
        stamp: RequestStamp,
        candidate: Result<DiagnosticBatch, LanguageProtocolError>,
    ) -> bool {
        let Some(session) = self.session.as_mut() else {
            return false;
        };
        let Some(pending) = session.diagnostic_pull.take_matching(id) else {
            self.stale_diagnostics = self.stale_diagnostics.saturating_add(1);
            return false;
        };
        if pending.stamp != stamp
            || pending.key != session.diagnostic_key()
            || !session.workspace_ready()
        {
            session.diagnostic_pull.refund_interrupted_attempt(pending);
            self.stale_diagnostics = self.stale_diagnostics.saturating_add(1);
            return false;
        }
        if let Err(LanguageProtocolError::DiagnosticServerCancelled { retrigger_request }) =
            candidate
        {
            if !retrigger_request {
                session.diagnostic_pull.settled = Some(pending.key);
            }
            // Server cancellation is neither a stale publication nor a report.
            // Retriable cancellations still consume the existing bounded budget.
            return replace_status(
                &mut self.status,
                Some(Arc::from(if !retrigger_request {
                    "Rust diagnostics deferred by the server until the next invalidation."
                } else if session.diagnostic_pull.attempts == MAX_ATTEMPTS {
                    "Rust diagnostic retry budget exhausted; waiting for the next invalidation."
                } else {
                    "Rust diagnostics canceled by the server; bounded retry pending."
                })),
            );
        }
        if candidate.is_ok() {
            session.diagnostic_pull.settled = Some(pending.key);
        }
        self.admit(candidate)
    }
}

#[cfg(test)]
#[path = "rust_diagnostic_pull_tests.rs"]
mod tests;
