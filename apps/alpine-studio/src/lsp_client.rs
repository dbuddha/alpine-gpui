//! Bounded composition of one local process, LSP framer, and JSON-RPC peer.

use std::{
    collections::VecDeque,
    error::Error,
    fmt,
    mem::size_of,
    num::NonZeroUsize,
    thread,
    time::{Duration, Instant},
};

use serde_json::value::RawValue;

use crate::{
    lsp_framing::{LspFrameError, LspFrameLimits, LspFramer, LspFramerSnapshot},
    lsp_json::{
        LspPeer, OutboundMessage, PeerEvent, PeerLifecycle, PeerSnapshot, ProtocolError,
        RequestStamp,
    },
    lsp_process::{
        InputSequence, LanguageServerProcess, ProcessBinding, ProcessEpoch, ProcessEvent,
        ProcessFailure, ProcessIdentity, ProcessSnapshot, ProcessSpec, ProcessStream, ProcessWake,
        SUPERVISOR_SHUTDOWN_TIMEOUT, StopReason, SubmitError, SupervisorStopped,
    },
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LspClientError {
    ProcessNotStarted,
    StaleCancellation,
    ProtocolWriteBudget,
    Process(ProcessFailure),
    Submit(SubmitError),
    SupervisorStopped,
    Frame(LspFrameError),
    Protocol(ProtocolError),
}

impl fmt::Display for LspClientError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "local LSP client failed: {self:?}")
    }
}

impl Error for LspClientError {}

impl From<SubmitError> for LspClientError {
    fn from(error: SubmitError) -> Self {
        Self::Submit(error)
    }
}

impl From<SupervisorStopped> for LspClientError {
    fn from(_: SupervisorStopped) -> Self {
        Self::SupervisorStopped
    }
}

impl From<LspFrameError> for LspClientError {
    fn from(error: LspFrameError) -> Self {
        Self::Frame(error)
    }
}

impl From<ProtocolError> for LspClientError {
    fn from(error: ProtocolError) -> Self {
        Self::Protocol(error)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SubmittedRequest {
    pub(crate) request_id: u32,
    pub(crate) input_sequence: InputSequence,
}

/// One locally revoked request's process-bound delivery obligation.
///
/// The fixed cancellation method and u32 ID bound the retained frame below
/// 128 bytes, outside the process payload counters until successful enqueue.
/// This owner is deliberately not Clone. Successful enqueue releases its bytes.
pub(crate) struct PreparedCancellation {
    outbound: Option<OutboundMessage>,
    binding: ProcessBinding,
}

#[cfg(test)]
impl PreparedCancellation {
    fn retained_bytes(&self) -> usize {
        self.outbound
            .as_ref()
            .map_or(0, |outbound| outbound.bytes().len())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LspClientPoll {
    Idle,
    Started {
        epoch: ProcessEpoch,
        process_id: u32,
    },
    Protocol {
        frames: usize,
        body_bytes: usize,
    },
    Stderr {
        bytes: usize,
    },
    InputWritten {
        sequence: InputSequence,
        bytes: usize,
    },
    InputRejected {
        sequence: InputSequence,
        failure: ProcessFailure,
    },
    Exited {
        success: bool,
        code: Option<i32>,
    },
    Stopped(StopReason),
    Failed(ProcessFailure),
}

const MAX_PROTOCOL_WRITES: usize = 256;
const MAX_PROTOCOL_PAYLOAD_BYTES: usize = 16_384;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
/// Owned boxed payloads and deque storage, excluding allocator overhead,
/// temporary encoding allocations, and unrelated client/process storage.
pub(crate) struct ProtocolWriteSnapshot {
    pub(crate) queued: usize,
    pub(crate) payload_bytes: usize,
    pub(crate) capacity_bytes: usize,
    pub(crate) retained_bytes: usize,
    pub(crate) peak_retained_bytes: usize,
    pub(crate) failed: bool,
}

#[derive(Clone, Copy)]
enum ProtocolWriteKind {
    Response,
    Initialized,
    Exit,
}

struct ProtocolWrite {
    outbound: OutboundMessage,
    kind: ProtocolWriteKind,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct LspClientSnapshot {
    pub(crate) started: bool,
    pub(crate) process: ProcessSnapshot,
    pub(crate) framing: LspFramerSnapshot,
    pub(crate) peer: PeerSnapshot,
    pub(crate) protocol_writes: ProtocolWriteSnapshot,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LspShutdownProtocol {
    AcknowledgedAndExited,
    NotReady(PeerLifecycle),
    Deadline,
    UnexpectedExit { success: bool, code: Option<i32> },
    RejectedInput(ProcessFailure),
    Stopped(StopReason),
    Failed(LspClientError),
}

/// Protocol and direct-child drain evidence, never a descendant-residency claim.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LspShutdownReport {
    pub(crate) protocol: LspShutdownProtocol,
    pub(crate) transport: ProcessSnapshot,
}

pub(crate) struct LspClient {
    process: LanguageServerProcess,
    framer: LspFramer,
    peer: LspPeer,
    started: bool,
    protocol_writes: VecDeque<ProtocolWrite>,
    protocol_payload_bytes: usize,
    protocol_peak_bytes: usize,
    protocol_failure: Option<LspClientError>,
}

impl LspClient {
    pub(crate) fn start(
        spec: ProcessSpec,
        identity: ProcessIdentity,
    ) -> Result<Self, LspClientError> {
        let process =
            LanguageServerProcess::start(spec, identity).map_err(LspClientError::Process)?;
        Ok(Self {
            process,
            framer: LspFramer::new(LspFrameLimits::default()),
            peer: LspPeer::new(),
            protocol_writes: VecDeque::new(),
            protocol_payload_bytes: 0,
            protocol_peak_bytes: 0,
            protocol_failure: None,
            started: false,
        })
    }

    pub(crate) fn start_with_waker(
        spec: ProcessSpec,
        identity: ProcessIdentity,
        wake: ProcessWake,
    ) -> Result<Self, LspClientError> {
        let process = LanguageServerProcess::start_with_waker(spec, identity, wake)
            .map_err(LspClientError::Process)?;
        Ok(Self {
            process,
            framer: LspFramer::new(LspFrameLimits::default()),
            peer: LspPeer::new(),
            protocol_writes: VecDeque::new(),
            protocol_payload_bytes: 0,
            protocol_peak_bytes: 0,
            protocol_failure: None,
            started: false,
        })
    }

    #[cfg(test)]
    pub(crate) fn inert_for_test(identity: ProcessIdentity) -> Self {
        Self {
            process: LanguageServerProcess::inert_for_test(identity),
            framer: LspFramer::new(LspFrameLimits::default()),
            peer: LspPeer::new(),
            protocol_writes: VecDeque::new(),
            protocol_payload_bytes: 0,
            protocol_peak_bytes: 0,
            protocol_failure: None,
            started: true,
        }
    }

    #[cfg(test)]
    pub(crate) fn initialize_inert_for_test(&mut self) {
        assert!(self.begin_initialize().is_ok());
        assert!(
            self.peer
                .receive(
                    br#"{"jsonrpc":"2.0","id":1,"result":{"capabilities":{}}}"#,
                    None,
                )
                .is_ok()
        );
    }

    pub(crate) fn begin_initialize(&mut self) -> Result<SubmittedRequest, LspClientError> {
        self.require_write_order()?;
        let outbound = self.peer.begin_initialize()?;
        self.submit_pending(&outbound)
    }

    pub(crate) fn begin_initialize_with(
        &mut self,
        params: &RawValue,
    ) -> Result<SubmittedRequest, LspClientError> {
        self.require_write_order()?;
        let outbound = self.peer.begin_initialize_with(Some(params))?;
        self.submit_pending(&outbound)
    }

    pub(crate) fn begin_request(
        &mut self,
        method: &str,
        params: Option<&RawValue>,
        stamp: RequestStamp,
    ) -> Result<SubmittedRequest, LspClientError> {
        self.require_write_order()?;
        let outbound = self.peer.begin_request(method, params, stamp)?;
        self.submit_pending(&outbound)
    }

    pub(crate) fn cancel(&mut self, request_id: u32) -> Result<InputSequence, LspClientError> {
        self.require_write_order()?;
        let outbound = self.peer.cancel(request_id)?;
        self.process.send(outbound.bytes()).map_err(Into::into)
    }

    pub(crate) fn prepare_cancel(
        &mut self,
        request_id: u32,
    ) -> Result<PreparedCancellation, LspClientError> {
        self.require_started()?;
        let outbound = self.peer.cancel(request_id)?;
        Ok(PreparedCancellation {
            outbound: Some(outbound),
            binding: self.process.binding(),
        })
    }

    pub(crate) fn send_cancel(
        &mut self,
        cancellation: &mut PreparedCancellation,
    ) -> Result<InputSequence, LspClientError> {
        // Check before require_started: a replacement may still be starting,
        // but an obsolete token is retirement, not its transport failing.
        if !self.process.owns_binding(&cancellation.binding) {
            return Err(LspClientError::StaleCancellation);
        }
        self.require_write_order()?;
        if self.peer.snapshot().lifecycle() != PeerLifecycle::Running {
            return Err(ProtocolError::InvalidLifecycle.into());
        }
        let outbound = cancellation
            .outbound
            .as_ref()
            .ok_or(ProtocolError::InvalidLifecycle)?;
        let sequence = self.process.send(outbound.bytes())?;
        cancellation.outbound = None;
        Ok(sequence)
    }

    pub(crate) fn notify(
        &mut self,
        method: &str,
        params: Option<&RawValue>,
    ) -> Result<InputSequence, LspClientError> {
        self.require_write_order()?;
        let outbound = self.peer.notification(method, params)?;
        self.process.send(outbound.bytes()).map_err(Into::into)
    }

    pub(crate) fn begin_shutdown(&mut self) -> Result<SubmittedRequest, LspClientError> {
        self.require_write_order()?;
        let outbound = self.peer.begin_shutdown()?;
        self.submit_pending(&outbound)
    }

    pub(crate) fn restart(
        &mut self,
        identity: ProcessIdentity,
    ) -> Result<ProcessEpoch, LspClientError> {
        let epoch = self.process.restart(identity)?;
        self.retire_protocol_writes();
        self.framer = LspFramer::new(LspFrameLimits::default());
        self.peer = LspPeer::new();
        self.started = false;
        Ok(epoch)
    }

    pub(crate) fn poll<F>(
        &mut self,
        current: Option<RequestStamp>,
        mut visitor: F,
    ) -> Result<LspClientPoll, LspClientError>
    where
        F: FnMut(PeerEvent<'_>),
    {
        if let Some(error) = self.protocol_failure {
            // Failed ingress never publishes again, but still releases one
            // bounded event per poll so it cannot starve transport teardown.
            if let Ok(Some(event)) = self.process.try_event()
                && matches!(
                    event,
                    ProcessEvent::Exited { .. }
                        | ProcessEvent::Stopped { .. }
                        | ProcessEvent::Failed { .. }
                )
            {
                self.started = false;
                self.retire_protocol_writes();
                self.protocol_failure = Some(error);
            }
            return Err(error);
        }
        let sent_before = self.flush_protocol_writes(&mut visitor)?;
        let Some(event) = self.process.try_event()? else {
            return Ok(if sent_before == 0 {
                LspClientPoll::Idle
            } else {
                LspClientPoll::Protocol {
                    frames: 0,
                    body_bytes: 0,
                }
            });
        };
        // Complete each admitted output event. A deferred response is not an
        // excuse to drop later frames or suspend writer/terminal event intake.
        let result = match event {
            ProcessEvent::Started {
                epoch, process_id, ..
            } => {
                self.started = true;
                Ok(LspClientPoll::Started { epoch, process_id })
            }
            event @ ProcessEvent::Output { .. } => {
                let (stream, bytes) = event
                    .output()
                    .ok_or(LspClientError::Frame(LspFrameError::InvalidState))?;
                match stream {
                    ProcessStream::Stdout => self.ingest_stdout(bytes, current, &mut visitor),
                    ProcessStream::Stderr => Ok(LspClientPoll::Stderr { bytes: bytes.len() }),
                }
            }
            ProcessEvent::InputWritten {
                sequence, bytes, ..
            } => Ok(LspClientPoll::InputWritten { sequence, bytes }),
            ProcessEvent::InputRejected {
                sequence, failure, ..
            } => Ok(LspClientPoll::InputRejected { sequence, failure }),
            ProcessEvent::Exited { success, code, .. } => {
                self.started = false;
                self.retire_protocol_writes();
                self.framer.finish()?;
                Ok(LspClientPoll::Exited { success, code })
            }
            ProcessEvent::Stopped { reason, .. } => {
                self.started = false;
                self.retire_protocol_writes();
                Ok(LspClientPoll::Stopped(reason))
            }
            ProcessEvent::Failed { failure, .. } => {
                self.started = false;
                self.retire_protocol_writes();
                Ok(LspClientPoll::Failed(failure))
            }
        };
        let poll = match result {
            Ok(poll) => poll,
            Err(error) => return self.fail_protocol(error),
        };
        // The output payload has now dropped. Its released budget may admit
        // the response without another external event or an idle retry loop.
        if self.started {
            let _ = self.flush_protocol_writes(&mut visitor)?;
        }
        Ok(poll)
    }

    pub(crate) const fn diagnostic_pull_supported(&self) -> bool {
        self.peer.diagnostic_pull_supported()
    }

    pub(crate) fn diagnostic_provider_identifier(&self) -> Option<&str> {
        self.peer.diagnostic_provider_identifier()
    }

    #[cfg(test)]
    pub(crate) fn fill_control_for_test(&self) -> Result<usize, LspClientError> {
        self.process.fill_control_for_test().map_err(Into::into)
    }

    #[cfg(test)]
    pub(crate) fn take_input_observer_for_test(
        &mut self,
    ) -> Result<crate::lsp_process::ProcessInputObserver, LspClientError> {
        self.process
            .take_input_observer_for_test()
            .map_err(LspClientError::Process)
    }

    #[cfg(test)]
    pub(crate) fn take_input_for_test(&mut self) -> Result<Option<Vec<u8>>, LspClientError> {
        self.process
            .take_input_for_test()
            .map_err(LspClientError::Process)
    }

    #[cfg(test)]
    pub(crate) fn inject_stdout_for_test(&mut self, bytes: &[u8]) -> Result<(), LspClientError> {
        self.process
            .inject_stdout_for_test(bytes)
            .map_err(LspClientError::Process)
    }

    pub(crate) fn snapshot(&self) -> LspClientSnapshot {
        LspClientSnapshot {
            started: self.started,
            process: self.process.snapshot(),
            framing: self.framer.snapshot(),
            peer: self.peer.snapshot(),
            protocol_writes: self.protocol_write_snapshot(),
        }
    }

    pub(crate) fn shutdown(&mut self) -> LspClientSnapshot {
        self.started = false;
        let process = self.process.shutdown();
        self.retire_protocol_writes();
        LspClientSnapshot {
            started: false,
            process,
            framing: self.framer.snapshot(),
            peer: self.peer.snapshot(),
            protocol_writes: self.protocol_write_snapshot(),
        }
    }

    /// Final application teardown only, not a foreground workspace-switch wait.
    pub(crate) fn shutdown_gracefully(&mut self) -> LspShutdownReport {
        self.shutdown_gracefully_until(Instant::now() + SUPERVISOR_SHUTDOWN_TIMEOUT)
    }

    fn shutdown_gracefully_until(&mut self, deadline: Instant) -> LspShutdownReport {
        let protocol = self.drain_shutdown_protocol(deadline);
        self.started = false;
        // Protocol work and forced transport cleanup share the original budget.
        // Exhaustion remains visible as an incomplete join, not another five seconds.
        let transport = self
            .process
            .shutdown_with_budget(deadline.saturating_duration_since(Instant::now()));
        self.retire_protocol_writes();
        LspShutdownReport {
            protocol,
            transport,
        }
    }

    fn drain_shutdown_protocol(&mut self, deadline: Instant) -> LspShutdownProtocol {
        let lifecycle = self.peer.snapshot().lifecycle();
        if !self.started
            || matches!(
                lifecycle,
                PeerLifecycle::Created | PeerLifecycle::Initializing | PeerLifecycle::Failed
            )
        {
            return LspShutdownProtocol::NotReady(lifecycle);
        }
        let mut acknowledged = lifecycle == PeerLifecycle::Exited
            && self.protocol_writes.is_empty()
            && self.protocol_failure.is_none();
        loop {
            if Instant::now() >= deadline {
                return LspShutdownProtocol::Deadline;
            }
            let peer = self.peer.snapshot();
            if peer.lifecycle() == PeerLifecycle::Running
                && peer.pending_requests() == 0
                && let Err(error) = self.begin_shutdown()
                && !matches!(
                    error,
                    LspClientError::Submit(SubmitError::Saturated | SubmitError::RetainedBudget)
                )
            {
                return LspShutdownProtocol::Failed(error);
            }
            // Pending replies drain without publishing back into editor state.
            // A stuck request cannot prolong the shared teardown deadline.
            match self.poll(None, |event| {
                acknowledged |= matches!(event, PeerEvent::ShutdownAcknowledged);
            }) {
                Ok(LspClientPoll::Exited { success, code }) => {
                    return if acknowledged && success {
                        LspShutdownProtocol::AcknowledgedAndExited
                    } else {
                        LspShutdownProtocol::UnexpectedExit { success, code }
                    };
                }
                Ok(LspClientPoll::InputRejected { failure, .. }) => {
                    return LspShutdownProtocol::RejectedInput(failure);
                }
                Ok(LspClientPoll::Stopped(reason)) => {
                    return LspShutdownProtocol::Stopped(reason);
                }
                Ok(LspClientPoll::Failed(failure)) => {
                    return LspShutdownProtocol::Failed(LspClientError::Process(failure));
                }
                Err(error) => return LspShutdownProtocol::Failed(error),
                Ok(LspClientPoll::Idle) => thread::sleep(
                    Duration::from_millis(1)
                        .min(deadline.saturating_duration_since(Instant::now())),
                ),
                Ok(_) => {}
            }
        }
    }

    fn require_write_order(&self) -> Result<(), LspClientError> {
        self.require_started()?;
        if !self.protocol_writes.is_empty() {
            return Err(SubmitError::Saturated.into());
        }
        Ok(())
    }

    fn protocol_write_snapshot(&self) -> ProtocolWriteSnapshot {
        let capacity_bytes = self.protocol_writes.capacity() * size_of::<ProtocolWrite>();
        ProtocolWriteSnapshot {
            queued: self.protocol_writes.len(),
            payload_bytes: self.protocol_payload_bytes,
            capacity_bytes,
            retained_bytes: self.protocol_payload_bytes + capacity_bytes,
            peak_retained_bytes: self.protocol_peak_bytes,
            failed: self.protocol_failure.is_some(),
        }
    }

    fn retire_protocol_writes(&mut self) {
        self.protocol_writes = VecDeque::new();
        self.protocol_payload_bytes = 0;
        self.protocol_failure = None;
    }

    fn fail_protocol<T>(&mut self, error: LspClientError) -> Result<T, LspClientError> {
        self.protocol_failure = Some(error);
        Err(error)
    }

    fn complete_protocol_write<F>(write: ProtocolWrite, visitor: &mut F)
    where
        F: FnMut(PeerEvent<'_>),
    {
        match write.kind {
            ProtocolWriteKind::Response => {}
            ProtocolWriteKind::Initialized => visitor(PeerEvent::Initialized(write.outbound)),
            ProtocolWriteKind::Exit => visitor(PeerEvent::ShutdownAcknowledged),
        }
    }

    fn retain_protocol_write<F>(
        &mut self,
        outbound: OutboundMessage,
        kind: ProtocolWriteKind,
        visitor: &mut F,
    ) -> Result<(), LspClientError>
    where
        F: FnMut(PeerEvent<'_>),
    {
        if let Some(error) = self.protocol_failure {
            return Err(error);
        }
        let write = ProtocolWrite { outbound, kind };
        if self.protocol_writes.is_empty() {
            match self.process.send(write.outbound.bytes()) {
                Ok(_) => {
                    Self::complete_protocol_write(write, visitor);
                    return Ok(());
                }
                Err(SubmitError::Saturated | SubmitError::RetainedBudget) => {}
                Err(error) => return self.fail_protocol(error.into()),
            }
        }
        let Some(bytes) = self
            .protocol_payload_bytes
            .checked_add(write.outbound.bytes().len())
            .filter(|bytes| *bytes <= MAX_PROTOCOL_PAYLOAD_BYTES)
        else {
            return self.fail_protocol(LspClientError::ProtocolWriteBudget);
        };
        if self.protocol_writes.len() == MAX_PROTOCOL_WRITES {
            return self.fail_protocol(LspClientError::ProtocolWriteBudget);
        }
        if self.protocol_writes.try_reserve(1).is_err() {
            return self.fail_protocol(ProtocolError::AllocationFailed.into());
        }
        self.protocol_writes.push_back(write);
        self.protocol_payload_bytes = bytes;
        self.protocol_peak_bytes = self.protocol_peak_bytes.max(
            self.protocol_payload_bytes
                + self.protocol_writes.capacity() * size_of::<ProtocolWrite>(),
        );
        Ok(())
    }

    fn flush_protocol_writes<F>(&mut self, visitor: &mut F) -> Result<usize, LspClientError>
    where
        F: FnMut(PeerEvent<'_>),
    {
        if let Some(error) = self.protocol_failure {
            return Err(error);
        }
        let mut sent = 0;
        while let Some(write) = self.protocol_writes.pop_front() {
            match self.process.send(write.outbound.bytes()) {
                Ok(_) => {}
                Err(error) => {
                    // Admission failed, so this owner retains the same FIFO
                    // entry and byte reservation. Popping preserved capacity.
                    self.protocol_writes.push_front(write);
                    match error {
                        SubmitError::Saturated | SubmitError::RetainedBudget => break,
                        error => return self.fail_protocol(error.into()),
                    }
                }
            }
            self.protocol_payload_bytes -= write.outbound.bytes().len();
            sent += 1;
            Self::complete_protocol_write(write, visitor);
        }
        if self.protocol_writes.is_empty() {
            self.protocol_writes = VecDeque::new();
        }
        Ok(sent)
    }

    fn require_started(&self) -> Result<(), LspClientError> {
        if let Some(error) = self.protocol_failure {
            return Err(error);
        }
        if !self.started {
            return Err(LspClientError::ProcessNotStarted);
        }
        Ok(())
    }

    fn submit_pending(
        &mut self,
        outbound: &OutboundMessage,
    ) -> Result<SubmittedRequest, LspClientError> {
        let request_id = outbound
            .request_id()
            .ok_or(LspClientError::Protocol(ProtocolError::InvalidEnvelope))?;
        match self.process.send(outbound.bytes()) {
            Ok(input_sequence) => Ok(SubmittedRequest {
                request_id,
                input_sequence,
            }),
            Err(error) => {
                self.peer.rollback_unsent(request_id)?;
                Err(LspClientError::Submit(error))
            }
        }
    }

    fn ingest_stdout<F>(
        &mut self,
        bytes: &[u8],
        current: Option<RequestStamp>,
        visitor: &mut F,
    ) -> Result<LspClientPoll, LspClientError>
    where
        F: FnMut(PeerEvent<'_>),
    {
        let mut consumed = 0;
        let mut frames = 0_usize;
        let mut body_bytes = 0_usize;
        while consumed < bytes.len() {
            let batch = self.framer.ingest(&bytes[consumed..])?;
            let batch_consumed = NonZeroUsize::new(batch.consumed())
                .ok_or(LspClientError::Frame(LspFrameError::InvalidState))?
                .get();
            consumed += batch_consumed;
            frames = frames
                .checked_add(batch.frames().len())
                .ok_or(LspClientError::Frame(LspFrameError::CounterOverflow))?;
            body_bytes = body_bytes
                .checked_add(batch.body_bytes())
                .ok_or(LspClientError::Frame(LspFrameError::CounterOverflow))?;
            for frame in batch.frames() {
                let event = self.peer.receive(frame.body(), current)?;
                self.dispatch_peer_event(event, visitor)?;
            }
        }
        Ok(LspClientPoll::Protocol { frames, body_bytes })
    }

    fn dispatch_peer_event<F>(
        &mut self,
        event: PeerEvent<'_>,
        visitor: &mut F,
    ) -> Result<(), LspClientError>
    where
        F: FnMut(PeerEvent<'_>),
    {
        match event {
            PeerEvent::Initialized(outbound) => {
                self.retain_protocol_write(outbound, ProtocolWriteKind::Initialized, visitor)?;
            }
            PeerEvent::ShutdownAcknowledged => {
                let exit = self.peer.exit()?;
                self.retain_protocol_write(exit, ProtocolWriteKind::Exit, visitor)?;
            }
            event @ PeerEvent::InboundRequest { id, method, .. } => {
                let lifecycle = self.peer.snapshot().lifecycle();
                // Exit is already owned. A late server request must not cause
                // a forced kill before that notification reaches the server.
                if lifecycle == PeerLifecycle::Exited {
                    return Ok(());
                }
                let response = self.peer.respond_to_server_request(id, method)?;
                self.retain_protocol_write(response, ProtocolWriteKind::Response, visitor)?;
                if lifecycle == PeerLifecycle::Running {
                    visitor(event);
                }
            }
            event => visitor(event),
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "lsp_cancellation_tests.rs"]
mod cancellation_tests;

#[cfg(test)]
#[path = "lsp_protocol_write_tests.rs"]
mod protocol_write_tests;

#[cfg(test)]
mod tests {
    use std::{
        env, fs,
        path::{Path, PathBuf},
        process::{self, Command},
        sync::OnceLock,
        thread,
        time::{Duration, Instant},
    };

    use super::*;
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    use crate::lsp_language::{
        DiagnosticBatch, LspDocument, LspPosition, initialize_params, pinned_server_version,
    };
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    use crate::rust_workspace_edit::WorkspaceEditProposal;
    use crate::{
        lsp_json::{PeerLifecycle, ResponseValue},
        lsp_process::{ConfigError, FailureKind, ProcessStage},
    };

    const WAIT: Duration = Duration::from_secs(5);
    const MOCK_STEM: &str = "alpine-lsp-mock";
    static MOCK_EXECUTABLE: OnceLock<MockExecutable> = OnceLock::new();

    struct MockExecutable {
        directory: PathBuf,
        path: PathBuf,
    }

    fn identity(generation: u64) -> ProcessIdentity {
        ProcessIdentity::new(generation, generation).unwrap_or_else(|| unreachable!())
    }

    fn stamp(revision: u64) -> RequestStamp {
        RequestStamp::new(revision, revision, revision, revision, revision, revision)
            .unwrap_or_else(|| unreachable!())
    }

    fn mock_executable() -> &'static MockExecutable {
        MOCK_EXECUTABLE.get_or_init(|| {
            compile_mock_executable()
                .unwrap_or_else(|error| unreachable!("failed to prepare mock server: {error}"))
        })
    }

    fn compile_mock_executable() -> Result<MockExecutable, Box<dyn Error>> {
        let current = env::current_exe()?;
        let directory = current
            .parent()
            .ok_or("test executable has no parent directory")?
            .to_path_buf();
        let suffix = env::consts::EXE_SUFFIX;
        let path = directory.join(format!("{MOCK_STEM}-{}{suffix}", process::id()));
        let source =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/lsp_mock_server.rs");
        let rustc = env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
        let output = Command::new(rustc)
            .args(["--edition=2024", "-o"])
            .arg(&path)
            .arg(source)
            .output()?;
        let compiler_stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success(),
            "failed to compile mock language server: {compiler_stderr}"
        );
        Ok(MockExecutable { directory, path })
    }

    fn mock_spec(executable: &Path) -> Result<ProcessSpec, ConfigError> {
        ProcessSpec::new(executable, std::iter::empty::<&str>(), None)
    }

    fn wait_poll<F>(
        client: &mut LspClient,
        current: Option<RequestStamp>,
        mut predicate: F,
    ) -> Result<LspClientPoll, Box<dyn Error>>
    where
        F: FnMut(&LspClientPoll) -> bool,
    {
        let deadline = Instant::now() + WAIT;
        loop {
            assert!(
                Instant::now() < deadline,
                "timed out waiting for local LSP event"
            );
            let poll = client.poll(current, |_| {})?;
            if predicate(&poll) {
                return Ok(poll);
            }
            thread::sleep(Duration::from_millis(2));
        }
    }

    fn wait_peer_event<F>(
        client: &mut LspClient,
        current: Option<RequestStamp>,
        timeout: Duration,
        message: &'static str,
        mut predicate: F,
    ) -> Result<(), Box<dyn Error>>
    where
        F: FnMut(PeerEvent<'_>) -> bool,
    {
        let deadline = Instant::now() + timeout;
        loop {
            assert!(Instant::now() < deadline, "{message}");
            let mut matched = false;
            let _ = client.poll(current, |event| matched |= predicate(event))?;
            if matched {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(2));
        }
    }

    fn initialize_client(
        client: &mut LspClient,
        params: Option<&RawValue>,
        message: &'static str,
    ) -> Result<(), Box<dyn Error>> {
        let request = match params {
            Some(params) => client.begin_initialize_with(params)?,
            None => client.begin_initialize()?,
        };
        assert_eq!(request.request_id, 1);
        wait_peer_event(client, None, WAIT, message, |event| {
            matches!(event, PeerEvent::Initialized(_))
        })
    }

    fn start_initialized(
        executable: &MockExecutable,
        generation: u64,
    ) -> Result<LspClient, Box<dyn Error>> {
        let mut client = LspClient::start(mock_spec(&executable.path)?, identity(generation))?;
        let _ = wait_poll(&mut client, None, |poll| {
            matches!(poll, LspClientPoll::Started { .. })
        })?;
        initialize_client(&mut client, None, "initialize response timed out")?;
        Ok(client)
    }

    fn mock_save_notify_written(
        client: &mut LspClient,
        method: &str,
        params: &str,
    ) -> Result<(), Box<dyn Error>> {
        let params = serde_json::from_str::<Box<RawValue>>(params)?;
        let submitted = client.notify(method, Some(&params))?;
        let _ = wait_poll(
            client,
            Some(stamp(1)),
            |poll| matches!(poll, LspClientPoll::InputWritten { sequence, .. } if *sequence == submitted),
        )?;
        Ok(())
    }

    const MOCK_SAVE_OPEN: &str = r#"{"textDocument":{"uri":"file:///workspace/save.rs","languageId":"rust","version":1,"text":"fn main() {}\n"}}"#;
    const MOCK_SAVE_CHANGE: &str = r#"{"textDocument":{"uri":"file:///workspace/save.rs","version":2},"contentChanges":[{"text":"let ok = 1;\n"}]}"#;
    const MOCK_SAVE_DOCUMENT: &str = r#"{"textDocument":{"uri":"file:///workspace/save.rs"}}"#;
    const MOCK_SAVE_DIRTY_CHANGE: &str = r#"{"textDocument":{"uri":"file:///workspace/save.rs","version":2},"contentChanges":[{"text":"broken();\n"}]}"#;
    const MOCK_SAVE_NEXT_CHANGE: &str = r#"{"textDocument":{"uri":"file:///workspace/save.rs","version":3},"contentChanges":[{"text":"let ok = 1;\n"}]}"#;

    #[test]
    #[cfg_attr(miri, ignore = "Miri cannot emulate child-process creation")]
    fn mock_save_notification_preserves_the_live_overlay_and_process() -> Result<(), Box<dyn Error>>
    {
        let mut client = start_initialized(mock_executable(), 1)?;
        mock_save_notify_written(&mut client, "textDocument/didOpen", MOCK_SAVE_OPEN)?;
        mock_save_notify_written(
            &mut client,
            "textDocument/didChange",
            MOCK_SAVE_DIRTY_CHANGE,
        )?;
        mock_save_notify_written(&mut client, "textDocument/didSave", MOCK_SAVE_DOCUMENT)?;

        // A fresh response after the save proves server receipt and continued
        // overlay authority, not just admission to the parent writer queue.
        let params = serde_json::from_str::<Box<RawValue>>(MOCK_SAVE_DOCUMENT)?;
        let request = client.begin_request("textDocument/diagnostic", Some(&params), stamp(1))?;
        wait_peer_event(
            &mut client,
            Some(stamp(1)),
            WAIT,
            "save incorrectly cleared the live overlay diagnostics",
            |event| {
                matches!(
                    event,
                    PeerEvent::Response {
                        id,
                        value: ResponseValue::Result(value),
                        ..
                    } if id == request.request_id
                        && serde_json::from_str::<serde_json::Value>(value.get())
                            .is_ok_and(|value| value.get("items")
                                .and_then(serde_json::Value::as_array)
                                .is_some_and(|items| !items.is_empty()))
                )
            },
        )?;
        // Saving version 2 must not consume version 3. Require a new response
        // for the immediately following edit, not only duplicate rejection.
        mock_save_notify_written(&mut client, "textDocument/didChange", MOCK_SAVE_NEXT_CHANGE)?;
        let request = client.begin_request("textDocument/diagnostic", Some(&params), stamp(1))?;
        wait_peer_event(
            &mut client,
            Some(stamp(1)),
            WAIT,
            "save prevented the next version from replacing the overlay",
            |event| {
                matches!(
                    event,
                    PeerEvent::Response {
                        id,
                        value: ResponseValue::Result(value),
                        ..
                    } if id == request.request_id
                        && serde_json::from_str::<serde_json::Value>(value.get())
                            .is_ok_and(|value| value.get("items")
                                .and_then(serde_json::Value::as_array)
                                .is_some_and(Vec::is_empty))
                )
            },
        )?;
        mock_save_notify_written(&mut client, "textDocument/didClose", MOCK_SAVE_DOCUMENT)?;
        let report = client.shutdown_gracefully();
        assert_eq!(report.protocol, LspShutdownProtocol::AcknowledgedAndExited);
        assert_eq!(report.transport.starts, 1);
        assert_eq!(report.transport.restarts, 0);
        assert_eq!(report.transport.exits, 1);
        assert_eq!(report.transport.input_saturations, 0);
        assert_eq!(report.transport.retained_bytes, 0);
        assert_eq!(report.transport.shutdown_timeouts, 0);
        Ok(())
    }

    #[test]
    #[cfg_attr(miri, ignore = "Miri cannot emulate child-process creation")]
    fn mock_save_notification_keeps_invalid_lifecycle_controls_discriminating()
    -> Result<(), Box<dyn Error>> {
        for scenario in ["missing-uri", "unopened", "closed", "version", "unknown"] {
            let mut client = start_initialized(mock_executable(), 1)?;
            mock_save_notify_written(&mut client, "textDocument/didOpen", MOCK_SAVE_OPEN)?;
            if scenario == "closed" {
                mock_save_notify_written(&mut client, "textDocument/didClose", MOCK_SAVE_DOCUMENT)?;
            }
            if scenario == "version" {
                mock_save_notify_written(&mut client, "textDocument/didChange", MOCK_SAVE_CHANGE)?;
                mock_save_notify_written(&mut client, "textDocument/didSave", MOCK_SAVE_DOCUMENT)?;
            }
            let (method, params) = match scenario {
                "missing-uri" => ("textDocument/didSave", "{}"),
                "unopened" => (
                    "textDocument/didSave",
                    r#"{"textDocument":{"uri":"file:///workspace/unopened.rs"}}"#,
                ),
                "version" => ("textDocument/didChange", MOCK_SAVE_CHANGE),
                "unknown" => ("test/unknown-save-method", MOCK_SAVE_DOCUMENT),
                _ => ("textDocument/didSave", MOCK_SAVE_DOCUMENT),
            };
            let params = serde_json::from_str::<Box<RawValue>>(params)?;
            client.notify(method, Some(&params))?;
            let exited = wait_poll(&mut client, Some(stamp(1)), |poll| {
                matches!(poll, LspClientPoll::Exited { .. })
            })?;
            assert_eq!(
                exited,
                LspClientPoll::Exited {
                    success: false,
                    code: Some(2),
                },
                "invalid lifecycle was accepted: {scenario}"
            );
            let snapshot = client.shutdown();
            assert_eq!(snapshot.process.starts, 1);
            assert_eq!(snapshot.process.restarts, 0);
            assert_eq!(snapshot.process.retained_bytes, 0);
            assert_eq!(snapshot.process.shutdown_timeouts, 0);
        }
        Ok(())
    }

    fn qualify_mock_requests(
        client: &mut LspClient,
        current: RequestStamp,
    ) -> Result<(), Box<dyn Error>> {
        let echo_params = serde_json::from_str::<Box<RawValue>>(r#"{"value":1}"#)?;
        let echo = client.begin_request("test/echo", Some(&echo_params), current)?;
        let echo_matches = |event: PeerEvent<'_>| {
            matches!(
                event,
                PeerEvent::Response {
                    id,
                    value: ResponseValue::Result(value),
                    ..
                } if id == echo.request_id && value.get() == r#"{"ok":true}"#
            )
        };
        wait_peer_event(client, Some(current), WAIT, "echo timed out", echo_matches)?;

        client.notify("test/notification", None)?;
        let request = client.begin_request("test/server-request", None, current)?;
        let deadline = Instant::now() + WAIT;
        let mut completed = false;
        let mut refresh = false;
        let mut acknowledged = false;
        while Instant::now() < deadline && (!completed || !acknowledged) {
            let _ = client.poll(Some(current), |event| match event {
                PeerEvent::Response { id, .. } if id == request.request_id => completed = true,
                PeerEvent::InboundRequest {
                    id: 0,
                    method: "workspace/diagnostic/refresh",
                    ..
                } => refresh = true,
                PeerEvent::InboundNotification {
                    method: "test/server-request-acknowledged",
                    ..
                } => acknowledged = true,
                _ => {}
            })?;
            thread::sleep(Duration::from_millis(2));
        }
        assert!(completed && refresh && acknowledged);
        Ok(())
    }

    fn qualify_mock_cancellation(
        client: &mut LspClient,
        current: RequestStamp,
    ) -> Result<u32, Box<dyn Error>> {
        let slow = client.begin_request("test/slow", None, current)?;
        client.cancel(slow.request_id)?;
        let deadline = Instant::now() + WAIT;
        loop {
            assert!(
                Instant::now() < deadline,
                "mock late response was not rejected"
            );
            let mut rejected = false;
            let poll = client.poll(Some(current), |event| {
                rejected |=
                    matches!(event, PeerEvent::StaleResponse { id } if id == slow.request_id);
            });
            if rejected {
                return Ok(slow.request_id);
            }
            assert!(poll.is_ok(), "unexpected cancellation poll: {poll:?}");
            thread::sleep(Duration::from_millis(2));
        }
    }

    fn restart_and_shutdown_mock(
        client: &mut LspClient,
        current: RequestStamp,
        prior_request_id: u32,
    ) -> Result<(), Box<dyn Error>> {
        let crash = client.begin_request("test/crash", None, current)?;
        assert!(crash.request_id > prior_request_id);
        let crash_poll = wait_poll(client, Some(current), |poll| {
            matches!(
                poll,
                LspClientPoll::Exited {
                    success: false,
                    code: Some(7)
                }
            )
        })?;
        assert_eq!(
            crash_poll,
            LspClientPoll::Exited {
                success: false,
                code: Some(7),
            }
        );
        assert_eq!(client.restart(identity(2))?.get(), 2);
        let epoch_two = |poll: &LspClientPoll| matches!(poll, LspClientPoll::Started { epoch, .. } if epoch.get() == 2);
        let started = wait_poll(client, None, epoch_two)?;
        assert!(matches!(started, LspClientPoll::Started { epoch, .. } if epoch.get() == 2));
        initialize_client(client, None, "mock restart did not initialize")?;
        client.begin_shutdown()?;
        wait_peer_event(client, None, WAIT, "mock shutdown timed out", |event| {
            matches!(event, PeerEvent::ShutdownAcknowledged)
        })?;
        let _ = wait_poll(client, None, |poll| {
            matches!(poll, LspClientPoll::Exited { success: true, .. })
        })?;
        Ok(())
    }

    #[test]
    #[cfg_attr(miri, ignore = "Miri cannot emulate child-process creation")]
    fn parameterized_initialize_uses_the_production_submission_path() -> Result<(), Box<dyn Error>>
    {
        let executable = mock_executable();
        let mut client = LspClient::start(mock_spec(&executable.path)?, identity(1))?;
        let _ = wait_poll(&mut client, None, |poll| {
            matches!(poll, LspClientPoll::Started { .. })
        })?;
        let params = serde_json::from_str::<Box<RawValue>>(r#"{"processId":null}"#)?;
        initialize_client(&mut client, Some(&params), "initialize timed out")?;
        client.begin_shutdown()?;
        wait_peer_event(&mut client, None, WAIT, "shutdown timed out", |event| {
            matches!(event, PeerEvent::ShutdownAcknowledged)
        })?;
        let _ = wait_poll(&mut client, None, |poll| {
            matches!(poll, LspClientPoll::Exited { success: true, .. })
        })?;
        let snapshot = client.shutdown();
        assert_eq!(snapshot.process.retained_bytes, 0);
        assert_eq!(snapshot.peer.pending_requests(), 0);
        Ok(())
    }

    #[test]
    #[cfg_attr(miri, ignore = "Miri cannot emulate child-process creation")]
    fn graceful_shutdown_drains_a_pending_reply_and_observes_real_exit()
    -> Result<(), Box<dyn Error>> {
        let executable = mock_executable();
        let mut client = start_initialized(executable, 1)?;
        let params = serde_json::from_str::<Box<RawValue>>("{}")?;
        client.begin_request("textDocument/hover", Some(&params), stamp(1))?;
        assert_eq!(client.snapshot().peer.pending_requests(), 1);
        let report = client.shutdown_gracefully();
        assert_eq!(report.protocol, LspShutdownProtocol::AcknowledgedAndExited);
        assert_eq!(report.transport.exits, 1);
        assert_eq!(report.transport.retained_bytes, 0);
        assert_eq!(report.transport.queued_events, 0);
        assert_eq!(report.transport.shutdown_timeouts, 0);
        assert_eq!(client.snapshot().peer.pending_requests(), 0);
        assert!(!client.snapshot().started);
        Ok(())
    }

    #[test]
    fn shutdown_drains_racing_requests_without_reopening_editor_work() -> Result<(), Box<dyn Error>>
    {
        let mut client = LspClient::inert_for_test(identity(1));
        client.initialize_inert_for_test();
        assert!(client.take_input_for_test()?.is_some());
        let shutdown = client.begin_shutdown()?;
        assert_eq!(shutdown.request_id, 2);
        assert!(client.take_input_for_test()?.is_some());

        let frame = |body: &str| format!("Content-Length: {}\r\n\r\n{body}", body.len());
        let input = [
            r#"{"jsonrpc":"2.0","id":41,"method":"workspace/diagnostic/refresh"}"#,
            r#"{"jsonrpc":"2.0","id":2,"result":null}"#,
            r#"{"jsonrpc":"2.0","id":42,"method":"workspace/diagnostic/refresh"}"#,
            r#"{"jsonrpc":"2.0","id":43,"method":"rust-analyzer/extension"}"#,
        ]
        .map(frame)
        .concat();
        let mut acknowledged = 0;
        let result = client.ingest_stdout(input.as_bytes(), None, &mut |event| {
            assert!(
                matches!(event, PeerEvent::ShutdownAcknowledged),
                "shutdown must not publish server requests into editor state"
            );
            acknowledged += 1;
        })?;
        assert!(matches!(result, LspClientPoll::Protocol { frames: 4, .. }));
        assert_eq!(acknowledged, 1);
        assert_eq!(client.snapshot().peer.lifecycle(), PeerLifecycle::Exited);
        assert_eq!(client.snapshot().peer.pending_requests(), 0);
        let cancelled = frame(
            r#"{"jsonrpc":"2.0","id":41,"error":{"code":-32800,"message":"Client is shutting down"}}"#,
        );
        assert_eq!(client.take_input_for_test()?, Some(cancelled.into_bytes()));
        let exit = frame(r#"{"jsonrpc":"2.0","method":"exit"}"#);
        assert_eq!(client.take_input_for_test()?, Some(exit.into_bytes()));
        assert!(client.take_input_for_test()?.is_none());

        // Closing changes admission, not JSON/framing validation.
        assert!(matches!(
            client.ingest_stdout(frame("{").as_bytes(), None, &mut |_| {}),
            Err(LspClientError::Protocol(ProtocolError::MalformedJson))
        ));
        Ok(())
    }

    #[test]
    #[cfg_attr(miri, ignore = "Miri cannot emulate child-process creation")]
    fn graceful_shutdown_rejects_false_success() -> Result<(), Box<dyn Error>> {
        let executable = mock_executable();
        for (method, lifecycle, success, code, pending) in [
            (
                "test/shutdown-without-ack",
                PeerLifecycle::ShuttingDown,
                true,
                0,
                1,
            ),
            (
                "test/shutdown-exit-error",
                PeerLifecycle::Exited,
                false,
                7,
                0,
            ),
        ] {
            let mut client = start_initialized(executable, 1)?;
            client.notify(method, None)?;
            let report = client.shutdown_gracefully();
            assert_eq!(
                report.protocol,
                LspShutdownProtocol::UnexpectedExit {
                    success,
                    code: Some(code),
                },
                "{method}"
            );
            assert_eq!(client.snapshot().peer.lifecycle(), lifecycle, "{method}");
            assert_eq!(client.snapshot().peer.pending_requests(), pending);
            assert!(!client.snapshot().started);
            assert_eq!(report.transport.exits, 1);
            assert_eq!(report.transport.shutdown_timeouts, 0);
            assert_eq!(report.transport.retained_bytes, 0);
            assert_eq!(report.transport.queued_events, 0);
            assert_eq!(
                report.transport.written_inputs,
                report.transport.submitted_inputs
            );
        }
        Ok(())
    }

    #[test]
    fn graceful_shutdown_rejects_unready_and_malformed_protocols() -> Result<(), Box<dyn Error>> {
        let mut unready = LspClient::inert_for_test(identity(1));
        let report = unready.shutdown_gracefully();
        assert_eq!(
            report.protocol,
            LspShutdownProtocol::NotReady(PeerLifecycle::Created)
        );
        assert_eq!(report.transport.submitted_inputs, 0);
        assert_eq!(report.transport.shutdown_timeouts, 0);

        let mut malformed = LspClient::inert_for_test(identity(2));
        malformed.initialize_inert_for_test();
        malformed.inject_stdout_for_test(b"Content-Length: 1\r\n\r\n{")?;
        let report = malformed.shutdown_gracefully();
        assert!(matches!(
            report.protocol,
            LspShutdownProtocol::Failed(LspClientError::Protocol(_))
        ));
        assert!(!malformed.snapshot().started);
        Ok(())
    }

    #[test]
    #[cfg_attr(miri, ignore = "Miri cannot emulate child-process creation")]
    fn graceful_shutdown_unanswered_request_cannot_claim_protocol_success()
    -> Result<(), Box<dyn Error>> {
        let executable = mock_executable();
        let mut client = start_initialized(executable, 1)?;
        let params = serde_json::from_str::<Box<RawValue>>(r#"{"character":99}"#)?;
        client.begin_request("textDocument/hover", Some(&params), stamp(1))?;
        let report = client.shutdown_gracefully_until(Instant::now() + Duration::from_millis(20));
        assert_eq!(report.protocol, LspShutdownProtocol::Deadline);
        assert_eq!(client.snapshot().peer.pending_requests(), 1);
        assert!(!client.snapshot().started);
        Ok(())
    }

    #[test]
    #[cfg_attr(miri, ignore = "Miri cannot emulate child-process creation")]
    fn requests_fail_closed_until_the_started_event_is_observed() -> Result<(), Box<dyn Error>> {
        let executable = mock_executable();
        let mut client = LspClient::start(mock_spec(&executable.path)?, identity(1))?;
        assert_eq!(
            client.begin_initialize(),
            Err(LspClientError::ProcessNotStarted)
        );
        assert_eq!(
            client.begin_request("test/echo", None, stamp(1)),
            Err(LspClientError::ProcessNotStarted)
        );
        assert_eq!(client.cancel(1), Err(LspClientError::ProcessNotStarted));
        assert_eq!(
            client.begin_shutdown(),
            Err(LspClientError::ProcessNotStarted)
        );
        let snapshot = client.shutdown();
        assert_eq!(snapshot.peer.pending_requests(), 0);
        assert_eq!(snapshot.peer.lifecycle(), PeerLifecycle::Created);
        Ok(())
    }

    #[test]
    #[cfg_attr(miri, ignore = "Miri cannot emulate child-process creation")]
    fn poll_classifies_diagnostics_rejection_stop_and_spawn_failure() -> Result<(), Box<dyn Error>>
    {
        let executable = mock_executable();
        let current = stamp(1);
        let mut client = start_initialized(executable, 1)?;

        let diagnostic = client.begin_request("test/stderr", None, current)?;
        let deadline = Instant::now() + WAIT;
        let mut saw_stderr = false;
        let mut saw_response = false;
        while !saw_stderr || !saw_response {
            assert!(Instant::now() < deadline, "timed out waiting for stderr");
            let poll = client.poll(Some(current), |event| {
                saw_response |= matches!(
                    event,
                    PeerEvent::Response { id, .. } if id == diagnostic.request_id
                );
            })?;
            saw_stderr |= matches!(poll, LspClientPoll::Stderr { bytes } if bytes > 0);
            thread::sleep(Duration::from_millis(2));
        }

        let _ = client.begin_request("test/crash", None, current)?;
        let exited = wait_poll(&mut client, Some(current), |poll| {
            matches!(poll, LspClientPoll::Exited { .. })
        })?;
        assert!(matches!(
            exited,
            LspClientPoll::Exited {
                success: false,
                code: Some(7)
            }
        ));
        // Ordinary pressure is refused before ownership transfer. Exercise a
        // genuine asynchronous failure through the still-owned supervisor
        // after observing its child's exit, not through an overloaded queue.
        let written = client.snapshot().process.written_inputs;
        let sequence = client
            .process
            .send(b"unwritable after observed child exit")?;
        let rejected = wait_poll(&mut client, Some(current), |poll| {
            matches!(poll, LspClientPoll::InputRejected { .. })
        })?;
        assert!(
            matches!(
                rejected,
                LspClientPoll::InputRejected {
                    sequence: rejected_sequence,
                    failure: ProcessFailure {
                        stage: ProcessStage::Input,
                        kind: FailureKind::Io(std::io::ErrorKind::BrokenPipe),
                        raw_os_error: None,
                    },
                } if rejected_sequence == sequence
            ),
            "unexpected rejection: {rejected:?}"
        );
        assert_eq!(client.snapshot().process.written_inputs, written);
        client.shutdown();

        let mut overflowing = start_initialized(executable, 2)?;
        overflowing.begin_request("test/flood-stderr", None, current)?;
        let stopped = wait_poll(&mut overflowing, Some(current), |poll| {
            matches!(
                poll,
                LspClientPoll::Stopped(StopReason::OutputOverflow | StopReason::EventOverflow)
            )
        })?;
        assert!(matches!(
            stopped,
            LspClientPoll::Stopped(StopReason::OutputOverflow | StopReason::EventOverflow)
        ));
        overflowing.shutdown();

        let invalid = executable.directory.join(format!(
            "invalid-language-server{}",
            env::consts::EXE_SUFFIX
        ));
        fs::write(&invalid, b"not an executable")?;
        let mut failed = LspClient::start(mock_spec(&invalid)?, identity(3))?;
        let failure = wait_poll(&mut failed, None, |poll| {
            matches!(poll, LspClientPoll::Failed(_))
        })?;
        assert!(matches!(
            failure,
            LspClientPoll::Failed(ProcessFailure {
                stage: ProcessStage::SpawnChild,
                ..
            })
        ));
        failed.shutdown();
        Ok(())
    }

    #[test]
    #[cfg_attr(miri, ignore = "Miri cannot emulate child-process creation")]
    fn production_process_framer_and_peer_complete_lifecycle_and_restart()
    -> Result<(), Box<dyn Error>> {
        let executable = mock_executable();
        let current = stamp(1);
        let mut client = start_initialized(executable, 1)?;
        qualify_mock_requests(&mut client, current)?;
        let cancelled = qualify_mock_cancellation(&mut client, current)?;
        restart_and_shutdown_mock(&mut client, current, cancelled)?;
        let snapshot = client.shutdown();
        assert!(!snapshot.started);
        assert_eq!(snapshot.process.retained_bytes, 0);
        assert_eq!(snapshot.process.starts, 2);
        assert_eq!(snapshot.process.restarts, 1);
        assert_eq!(snapshot.peer.pending_requests(), 0);
        assert_eq!(snapshot.peer.lifecycle(), PeerLifecycle::Exited);
        assert!(!snapshot.framing.poisoned());
        Ok(())
    }

    #[test]
    #[cfg_attr(miri, ignore = "Miri cannot emulate child-process creation")]
    fn saturated_submission_rolls_back_peer_admission_and_releases_bounds()
    -> Result<(), Box<dyn Error>> {
        let executable = mock_executable();
        let current = stamp(1);
        let mut client = start_initialized(executable, 1)?;
        client.begin_request("test/block", None, current)?;
        thread::sleep(Duration::from_millis(20));

        let chunk = "x".repeat(1_000_000);
        let mut params = String::from(r#"{"values":["#);
        for index in 0..12 {
            if index > 0 {
                params.push(',');
            }
            params.push('"');
            params.push_str(&chunk);
            params.push('"');
        }
        params.push_str("]}");
        let params = serde_json::from_str::<Box<RawValue>>(&params)?;
        let mut rejection = None;
        for _ in 0..4 {
            let before = client.snapshot().peer.pending_requests();
            let submission = client.begin_request("test/large", Some(&params), current);
            if let Err(LspClientError::Submit(
                error @ (SubmitError::Saturated | SubmitError::RetainedBudget),
            )) = &submission
            {
                assert_eq!(client.snapshot().peer.pending_requests(), before);
                rejection = Some(*error);
                break;
            }
            assert!(submission.is_ok(), "unexpected submission: {submission:?}");
        }
        assert!(rejection.is_some());
        let snapshot = client.shutdown();
        assert_eq!(snapshot.process.retained_bytes, 0);
        assert!(snapshot.process.peak_retained_bytes <= 16_777_216);
        assert!(
            snapshot.process.input_saturations > 0
                || snapshot.process.peak_retained_bytes >= 12_000_000
        );
        Ok(())
    }

    #[test]
    fn client_errors_preserve_structured_process_and_protocol_boundaries() {
        assert_eq!(
            LspClientError::from(SubmitError::Closed),
            LspClientError::Submit(SubmitError::Closed)
        );
        assert_eq!(
            LspClientError::from(SupervisorStopped),
            LspClientError::SupervisorStopped
        );
        assert_eq!(
            LspClientError::from(LspFrameError::Poisoned),
            LspClientError::Frame(LspFrameError::Poisoned)
        );
        assert_eq!(
            LspClientError::from(ProtocolError::InvalidEnvelope),
            LspClientError::Protocol(ProtocolError::InvalidEnvelope)
        );
        let errors = [
            LspClientError::ProcessNotStarted,
            LspClientError::Process(ProcessFailure {
                stage: ProcessStage::Input,
                kind: FailureKind::Io(std::io::ErrorKind::BrokenPipe),
                raw_os_error: None,
            }),
            LspClientError::Submit(SubmitError::Closed),
            LspClientError::SupervisorStopped,
            LspClientError::Frame(LspFrameError::Poisoned),
            LspClientError::Protocol(ProtocolError::InvalidEnvelope),
        ];
        for error in errors {
            assert!(!error.to_string().is_empty());
            assert!(error.source().is_none());
        }
    }

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    fn wait_for_real_diagnostics(
        client: &mut LspClient,
        document: &LspDocument,
    ) -> Result<DiagnosticBatch, Box<dyn Error>> {
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            let mut parsed = None;
            let poll = client.poll(None, |event| {
                if let PeerEvent::InboundNotification {
                    method: "textDocument/publishDiagnostics",
                    params: Some(params),
                } = event
                {
                    parsed = Some(DiagnosticBatch::admit(params, document));
                }
            });
            if let Some(batch) = parsed {
                let batch = batch?;
                if !batch.is_empty() {
                    return Ok(batch);
                }
            }
            assert!(poll.is_ok(), "diagnostic poll failed: {poll:?}");
            thread::sleep(Duration::from_millis(2));
        }
        Err("real rust-analyzer published no diagnostics".into())
    }

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    fn wait_for_real_result(
        client: &mut LspClient,
        request_id: u32,
        current: RequestStamp,
        message: &'static str,
    ) -> Result<Box<RawValue>, Box<dyn Error>> {
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            let mut response = None;
            let poll = client.poll(Some(current), |event| {
                if let PeerEvent::Response { id, value, .. } = event
                    && id == request_id
                {
                    response = Some(match value {
                        ResponseValue::Result(value) => Ok(value.get().to_owned()),
                        ResponseValue::Error(_) => Err(message),
                    });
                }
            });
            assert!(poll.is_ok(), "workspace-edit poll failed: {poll:?}");
            if let Some(response) = response {
                return Ok(RawValue::from_string(response?)?);
            }
            thread::sleep(Duration::from_millis(2));
        }
        Err(message.into())
    }

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    fn qualify_real_workspace_edits(
        client: &mut LspClient,
        document: &LspDocument,
        workspace: &Path,
        document_path: &Path,
        document_text: &str,
    ) -> Result<(), Box<dyn Error>> {
        let current = stamp(1);
        let new_name = "renamed_for_qualification";
        let rename_params = document.rename_params(LspPosition::new(0, 7)?, new_name)?;
        let rename = client.begin_request("textDocument/rename", Some(&rename_params), current)?;
        let rename_response = wait_for_real_result(
            client,
            rename.request_id,
            current,
            "real rust-analyzer returned no rename workspace edit",
        )?;
        let rename = WorkspaceEditProposal::admit_rename(&rename_response, workspace)?.prepare()?;
        assert_eq!(rename.file_count(), 1);
        assert!(rename.edit_count() > 0);
        let rename_file = &rename.files()[0];
        assert_eq!(rename_file.path(), document_path);
        assert_eq!(rename_file.original(), document_text);
        assert!(rename_file.replacement().contains(new_name));

        let formatting_params = document.formatting_params(4, true)?;
        let formatting =
            client.begin_request("textDocument/formatting", Some(&formatting_params), current)?;
        let formatting_response = wait_for_real_result(
            client,
            formatting.request_id,
            current,
            "real rust-analyzer returned no formatting edits",
        )?;
        let formatting = WorkspaceEditProposal::admit_formatting(
            &formatting_response,
            workspace,
            document.uri(),
            document.version(),
        )?
        .prepare()?;
        assert_eq!(formatting.file_count(), 1);
        assert!(formatting.edit_count() > 0);
        let formatting_file = &formatting.files()[0];
        assert_eq!(formatting_file.path(), document_path);
        assert_eq!(formatting_file.original(), document_text);
        assert_ne!(formatting_file.replacement(), document_text);
        assert!(
            formatting_file
                .replacement()
                .contains("pub fn deliberately_invalid() -> u32 {")
        );
        Ok(())
    }

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    fn qualify_real_request_admission(
        client: &mut LspClient,
        document: &LspDocument,
    ) -> Result<(), Box<dyn Error>> {
        let before = client.snapshot().peer;
        let hover_params = document.position_params(LspPosition::new(0, 7)?)?;
        let cancelled =
            client.begin_request("textDocument/hover", Some(&hover_params), stamp(1))?;
        client.cancel(cancelled.request_id)?;
        let symbols_params = document.text_document_params()?;
        let stale = client.begin_request(
            "textDocument/documentSymbol",
            Some(&symbols_params),
            stamp(1),
        )?;
        let deadline = Instant::now() + WAIT;
        let mut cancelled_rejected = false;
        let mut stale_rejected = false;
        while Instant::now() < deadline && !(cancelled_rejected && stale_rejected) {
            match client.poll(Some(stamp(2)), |event| {
                if let PeerEvent::StaleResponse { id } = event {
                    cancelled_rejected |= id == cancelled.request_id;
                    stale_rejected |= id == stale.request_id;
                }
            }) {
                Ok(_) | Err(LspClientError::Protocol(ProtocolError::UnknownResponseId)) => {}
                Err(error) => return Err(error.into()),
            }
            thread::sleep(Duration::from_millis(2));
        }
        assert!(
            cancelled_rejected,
            "real cancelled result was not rejected as stale"
        );
        assert!(stale_rejected, "real result was not rejected as stale");
        let after = client.snapshot().peer;
        let Some(expected_cancelled) = before.cancelled_requests().checked_add(1) else {
            return Err("real cancellation counter overflowed".into());
        };
        let Some(expected_stale) = before.stale_responses().checked_add(2) else {
            return Err("real stale-response counter overflowed".into());
        };
        assert_eq!(after.cancelled_requests(), expected_cancelled);
        assert_eq!(after.stale_responses(), expected_stale);
        Ok(())
    }

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    fn restart_and_shutdown_real(
        client: &mut LspClient,
        process_id: u32,
        initialize: &RawValue,
    ) -> Result<(), Box<dyn Error>> {
        let process_id = process_id.to_string();
        assert!(
            Command::new("/bin/kill")
                .args(["-KILL", process_id.as_str()])
                .status()?
                .success()
        );
        let _ = wait_poll(client, Some(stamp(2)), |poll| {
            matches!(poll, LspClientPoll::Exited { success: false, .. })
        })?;
        assert_eq!(client.restart(identity(2))?.get(), 2);
        let _ = wait_poll(
            client,
            None,
            |poll| matches!(poll, LspClientPoll::Started { epoch, .. } if epoch.get() == 2),
        )?;
        initialize_client(client, Some(initialize), "real server did not reinitialize")?;
        client.begin_shutdown()?;
        let _ = wait_poll(client, None, |poll| {
            matches!(poll, LspClientPoll::Exited { success: true, .. })
        })?;
        Ok(())
    }

    #[test]
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    #[ignore = "requires the checksum-verified Task #208 rust-analyzer binary"]
    fn pinned_rust_analyzer_qualifies_real_document_lifecycle() -> Result<(), Box<dyn Error>> {
        let executable = PathBuf::from(
            env::var_os("ALPINE_RUST_ANALYZER")
                .ok_or("ALPINE_RUST_ANALYZER must name the checksum-verified Task #208 binary")?,
        );
        let version = Command::new(&executable).arg("--version").output()?;
        assert!(version.status.success());
        let version = String::from_utf8(version.stdout)?;
        assert_eq!(version.trim(), pinned_server_version());

        let workspace = fs::canonicalize(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rust-analyzer-workspace"),
        )?;
        let document_path = fs::canonicalize(workspace.join("src/lib.rs"))?;
        let document_text = fs::read_to_string(&document_path)?;
        let document = LspDocument::from_file_path(&document_path, "rust", 1)?;
        let initialize = initialize_params(&workspace)?;
        let spec = ProcessSpec::new(&executable, std::iter::empty::<&str>(), Some(&workspace))?;
        let mut client = LspClient::start(spec, identity(1))?;
        let started = wait_poll(&mut client, None, |poll| {
            matches!(poll, LspClientPoll::Started { .. })
        })?;
        let LspClientPoll::Started { process_id, .. } = started else {
            unreachable!()
        };
        initialize_client(
            &mut client,
            Some(&initialize),
            "real rust-analyzer did not initialize",
        )?;

        let did_open = document.did_open_params(&document_text)?;
        client.notify("textDocument/didOpen", Some(&did_open))?;
        let diagnostics = wait_for_real_diagnostics(&mut client, &document)?;
        assert!(!diagnostics.is_empty());
        assert_eq!(diagnostics.document_version(), Some(1));
        assert!(diagnostics.retained_bytes() <= 262_144);
        qualify_real_workspace_edits(
            &mut client,
            &document,
            &workspace,
            &document_path,
            &document_text,
        )?;
        qualify_real_request_admission(&mut client, &document)?;
        restart_and_shutdown_real(&mut client, process_id, &initialize)?;
        let snapshot = client.shutdown();
        assert_eq!(snapshot.process.retained_bytes, 0);
        assert_eq!(snapshot.process.restarts, 1);
        assert_eq!(snapshot.peer.pending_requests(), 0);
        assert_eq!(snapshot.peer.retained_bytes(), 0);
        assert!(snapshot.peer.peak_retained_bytes() > 0);
        assert_eq!(snapshot.peer.lifecycle(), PeerLifecycle::Exited);
        Ok(())
    }
}
