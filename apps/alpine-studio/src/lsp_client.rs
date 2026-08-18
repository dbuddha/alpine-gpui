//! Bounded composition of one local process, LSP framer, and JSON-RPC peer.

use std::{error::Error, fmt};

use serde_json::value::RawValue;

use crate::{
    lsp_framing::{LspFrameError, LspFrameLimits, LspFramer, LspFramerSnapshot},
    lsp_json::{LspPeer, OutboundMessage, PeerEvent, PeerSnapshot, ProtocolError, RequestStamp},
    lsp_process::{
        InputSequence, LanguageServerProcess, ProcessEpoch, ProcessEvent, ProcessFailure,
        ProcessIdentity, ProcessSnapshot, ProcessSpec, ProcessStream, StopReason, SubmitError,
        SupervisorStopped,
    },
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LspClientError {
    ProcessNotStarted,
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

#[derive(Clone, Copy, Debug)]
pub(crate) struct LspClientSnapshot {
    pub(crate) started: bool,
    pub(crate) process: ProcessSnapshot,
    pub(crate) framing: LspFramerSnapshot,
    pub(crate) peer: PeerSnapshot,
}

pub(crate) struct LspClient {
    process: LanguageServerProcess,
    framer: LspFramer,
    peer: LspPeer,
    started: bool,
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
            started: false,
        })
    }

    pub(crate) fn begin_initialize(&mut self) -> Result<SubmittedRequest, LspClientError> {
        self.require_started()?;
        let outbound = self.peer.begin_initialize()?;
        self.submit_pending(&outbound)
    }

    pub(crate) fn begin_request(
        &mut self,
        method: &str,
        params: Option<&RawValue>,
        stamp: RequestStamp,
    ) -> Result<SubmittedRequest, LspClientError> {
        self.require_started()?;
        let outbound = self.peer.begin_request(method, params, stamp)?;
        self.submit_pending(&outbound)
    }

    pub(crate) fn cancel(&mut self, request_id: u32) -> Result<InputSequence, LspClientError> {
        self.require_started()?;
        let outbound = self.peer.cancel(request_id)?;
        self.process.send(outbound.bytes()).map_err(Into::into)
    }

    pub(crate) fn begin_shutdown(&mut self) -> Result<SubmittedRequest, LspClientError> {
        self.require_started()?;
        let outbound = self.peer.begin_shutdown()?;
        self.submit_pending(&outbound)
    }

    pub(crate) fn restart(
        &mut self,
        identity: ProcessIdentity,
    ) -> Result<ProcessEpoch, LspClientError> {
        let epoch = self.process.restart(identity)?;
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
        let Some(event) = self.process.try_event()? else {
            return Ok(LspClientPoll::Idle);
        };
        match event {
            ProcessEvent::Started {
                epoch, process_id, ..
            } => {
                self.started = true;
                Ok(LspClientPoll::Started { epoch, process_id })
            }
            event @ ProcessEvent::Output { .. } => match event.output() {
                Some((ProcessStream::Stdout, bytes)) => {
                    self.ingest_stdout(bytes, current, &mut visitor)
                }
                Some((ProcessStream::Stderr, bytes)) => {
                    Ok(LspClientPoll::Stderr { bytes: bytes.len() })
                }
                None => Ok(LspClientPoll::Idle),
            },
            ProcessEvent::InputWritten {
                sequence, bytes, ..
            } => Ok(LspClientPoll::InputWritten { sequence, bytes }),
            ProcessEvent::InputRejected {
                sequence, failure, ..
            } => Ok(LspClientPoll::InputRejected { sequence, failure }),
            ProcessEvent::Exited { success, code, .. } => {
                self.started = false;
                self.framer.finish()?;
                Ok(LspClientPoll::Exited { success, code })
            }
            ProcessEvent::Stopped { reason, .. } => {
                self.started = false;
                Ok(LspClientPoll::Stopped(reason))
            }
            ProcessEvent::Failed { failure, .. } => {
                self.started = false;
                Ok(LspClientPoll::Failed(failure))
            }
        }
    }

    pub(crate) fn snapshot(&self) -> LspClientSnapshot {
        LspClientSnapshot {
            started: self.started,
            process: self.process.snapshot(),
            framing: self.framer.snapshot(),
            peer: self.peer.snapshot(),
        }
    }

    pub(crate) fn shutdown(&mut self) -> LspClientSnapshot {
        self.started = false;
        let process = self.process.shutdown();
        LspClientSnapshot {
            started: false,
            process,
            framing: self.framer.snapshot(),
            peer: self.peer.snapshot(),
        }
    }

    fn require_started(&self) -> Result<(), LspClientError> {
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
            if batch.consumed() == 0 {
                return Err(LspClientError::Frame(LspFrameError::InvalidState));
            }
            consumed += batch.consumed();
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
                self.process.send(outbound.bytes())?;
                visitor(PeerEvent::Initialized(outbound));
            }
            PeerEvent::ShutdownAcknowledged => {
                let exit = self.peer.exit()?;
                self.process.send(exit.bytes())?;
                visitor(PeerEvent::ShutdownAcknowledged);
            }
            event => visitor(event),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{
        env, fs,
        path::{Path, PathBuf},
        process::{self, Command},
        sync::atomic::{AtomicU64, Ordering},
        thread,
        time::{Duration, Instant},
    };

    use super::*;
    use crate::{
        lsp_json::{PeerLifecycle, ResponseValue},
        lsp_process::{ConfigError, FailureKind, ProcessStage},
    };

    const WAIT: Duration = Duration::from_secs(5);
    const MOCK_STEM: &str = "alpine-lsp-mock";
    static MOCK_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct MockExecutable {
        directory: PathBuf,
        path: PathBuf,
    }

    impl Drop for MockExecutable {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.directory);
        }
    }

    fn identity(generation: u64) -> ProcessIdentity {
        ProcessIdentity::new(generation, generation).unwrap_or_else(|| unreachable!())
    }

    fn stamp(revision: u64) -> RequestStamp {
        RequestStamp::new(revision, revision, revision, revision).unwrap_or_else(|| unreachable!())
    }

    fn mock_executable() -> Result<MockExecutable, Box<dyn Error>> {
        let sequence = MOCK_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let directory = env::temp_dir().join(format!("{MOCK_STEM}-{}-{sequence}", process::id()));
        fs::create_dir(&directory)?;
        let suffix = env::consts::EXE_SUFFIX;
        let path = directory.join(format!("{MOCK_STEM}{suffix}"));
        let source =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/lsp_mock_server.rs");
        let rustc = env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
        let output = Command::new(rustc)
            .args(["--edition=2024", "-o"])
            .arg(&path)
            .arg(source)
            .output()?;
        if !output.status.success() {
            return Err(format!(
                "failed to compile mock language server: {}",
                String::from_utf8_lossy(&output.stderr)
            )
            .into());
        }
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
        while Instant::now() < deadline {
            let poll = client.poll(current, |_| {})?;
            if predicate(&poll) {
                return Ok(poll);
            }
            thread::sleep(Duration::from_millis(2));
        }
        Err("timed out waiting for local LSP event".into())
    }

    fn start_initialized(
        executable: &MockExecutable,
        generation: u64,
    ) -> Result<LspClient, Box<dyn Error>> {
        let mut client = LspClient::start(mock_spec(&executable.path)?, identity(generation))?;
        let _ = wait_poll(&mut client, None, |poll| {
            matches!(poll, LspClientPoll::Started { .. })
        })?;
        assert_eq!(client.begin_initialize()?.request_id, 1);
        let deadline = Instant::now() + WAIT;
        let mut initialized = false;
        while Instant::now() < deadline && !initialized {
            let _ = client.poll(None, |event| {
                if matches!(event, PeerEvent::Initialized(_)) {
                    initialized = true;
                }
            })?;
            thread::sleep(Duration::from_millis(2));
        }
        if !initialized {
            return Err("timed out waiting for initialize response".into());
        }
        Ok(client)
    }

    #[test]
    fn production_process_framer_and_peer_complete_lifecycle_and_restart()
    -> Result<(), Box<dyn Error>> {
        let executable = mock_executable()?;
        let current = stamp(1);
        let mut client = start_initialized(&executable, 1)?;

        let echo_params = serde_json::from_str::<Box<RawValue>>(r#"{"value":1}"#)?;
        let echo = client.begin_request("test/echo", Some(&echo_params), current)?;
        let deadline = Instant::now() + WAIT;
        let mut echoed = false;
        while Instant::now() < deadline && !echoed {
            let _ = client.poll(Some(current), |event| {
                if matches!(
                    event,
                    PeerEvent::Response {
                        id,
                        value: ResponseValue::Result(value),
                        ..
                    } if id == echo.request_id && value.get() == r#"{"ok":true}"#
                ) {
                    echoed = true;
                }
            })?;
            thread::sleep(Duration::from_millis(2));
        }
        assert!(echoed);

        let slow = client.begin_request("test/slow", None, current)?;
        client.cancel(slow.request_id)?;
        let deadline = Instant::now() + WAIT;
        let mut rejected_late = false;
        while Instant::now() < deadline && !rejected_late {
            match client.poll(Some(current), |_| {}) {
                Err(LspClientError::Protocol(ProtocolError::UnknownResponseId)) => {
                    rejected_late = true;
                }
                Ok(_) => thread::sleep(Duration::from_millis(2)),
                Err(error) => return Err(error.into()),
            }
        }
        assert!(rejected_late);

        let crash = client.begin_request("test/crash", None, current)?;
        assert!(crash.request_id > slow.request_id);
        let crash_poll = wait_poll(&mut client, Some(current), |poll| {
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
                code: Some(7)
            }
        );

        assert_eq!(client.restart(identity(2))?.get(), 2);
        let _ = wait_poll(
            &mut client,
            None,
            |poll| matches!(poll, LspClientPoll::Started { epoch, .. } if epoch.get() == 2),
        )?;
        assert_eq!(client.begin_initialize()?.request_id, 1);
        let deadline = Instant::now() + WAIT;
        let mut initialized = false;
        while Instant::now() < deadline && !initialized {
            let _ = client.poll(None, |event| {
                initialized |= matches!(event, PeerEvent::Initialized(_));
            })?;
            thread::sleep(Duration::from_millis(2));
        }
        assert!(initialized);

        client.begin_shutdown()?;
        let deadline = Instant::now() + WAIT;
        let mut acknowledged = false;
        while Instant::now() < deadline && !acknowledged {
            let _ = client.poll(None, |event| {
                acknowledged |= matches!(event, PeerEvent::ShutdownAcknowledged);
            })?;
            thread::sleep(Duration::from_millis(2));
        }
        assert!(acknowledged);
        let _ = wait_poll(&mut client, None, |poll| {
            matches!(poll, LspClientPoll::Exited { success: true, .. })
        })?;
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
    fn saturated_submission_rolls_back_peer_admission_without_blocking()
    -> Result<(), Box<dyn Error>> {
        let executable = mock_executable()?;
        let current = stamp(1);
        let mut client = start_initialized(&executable, 1)?;
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
        let started = Instant::now();
        let mut rejection = None;
        for _ in 0..4 {
            let before = client.snapshot().peer.pending_requests();
            match client.begin_request("test/large", Some(&params), current) {
                Ok(_) => {}
                Err(LspClientError::Submit(
                    error @ (SubmitError::Saturated | SubmitError::RetainedBudget),
                )) => {
                    assert_eq!(client.snapshot().peer.pending_requests(), before);
                    rejection = Some(error);
                    break;
                }
                Err(error) => return Err(error.into()),
            }
        }
        assert!(rejection.is_some());
        assert!(started.elapsed() < Duration::from_secs(1));
        let snapshot = client.shutdown();
        assert_eq!(snapshot.process.retained_bytes, 0);
        assert!(snapshot.process.peak_retained_bytes <= 16_777_216);
        match rejection {
            Some(SubmitError::Saturated) => assert!(snapshot.process.input_saturations > 0),
            Some(SubmitError::RetainedBudget) => {
                assert!(snapshot.process.peak_retained_bytes >= 12_000_000);
            }
            _ => unreachable!(),
        }
        Ok(())
    }

    #[test]
    fn client_errors_preserve_structured_process_and_protocol_boundaries() {
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
}
