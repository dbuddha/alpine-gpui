//! Bounded ownership of one local language-server child process.

use std::{
    error::Error,
    ffi::{OsStr, OsString},
    fmt, fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, ExitStatus, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc::{Receiver, RecvTimeoutError, SyncSender, TryRecvError, TrySendError, sync_channel},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

const CONTROL_CAPACITY: usize = 8;
const EVENT_CAPACITY: usize = 16;
const INPUT_CAPACITY: usize = 4;
const OUTPUT_CAPACITY: usize = 8;
const WRITE_RESULT_CAPACITY: usize = 8;
const OUTPUT_CHUNK_BYTES: usize = 64 * 1_024;
const MAX_MESSAGE_BYTES: usize = 16 * 1_024 * 1_024;
const MAX_RETAINED_PAYLOAD_BYTES: usize = 16 * 1_024 * 1_024;
const MAX_PATH_BYTES: usize = 4 * 1_024;
const MAX_ARGUMENTS: usize = 64;
const MAX_ARGUMENT_BYTES: usize = 4 * 1_024;
const MAX_CONFIGURATION_BYTES: usize = 64 * 1_024;
const SUPERVISOR_POLL: Duration = Duration::from_millis(2);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ProcessIdentity {
    pub(crate) workspace_revision: u64,
    pub(crate) generation: u64,
}

impl ProcessIdentity {
    pub(crate) const fn new(workspace_revision: u64, generation: u64) -> Option<Self> {
        if workspace_revision == 0 || generation == 0 {
            return None;
        }
        Some(Self {
            workspace_revision,
            generation,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ProcessEpoch(u64);

impl ProcessEpoch {
    pub(crate) const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct InputSequence(u64);

impl InputSequence {
    pub(crate) const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProcessStream {
    Stdout,
    Stderr,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StopReason {
    Restart,
    OutputOverflow,
    EventOverflow,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProcessStage {
    SpawnChild,
    SpawnInput,
    SpawnStdout,
    SpawnStderr,
    Input,
    Output,
    Wait,
    ThreadJoin,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FailureKind {
    Io(io::ErrorKind),
    RetainedBudget,
    QueueSaturated,
    ThreadPanicked,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ProcessFailure {
    pub(crate) stage: ProcessStage,
    pub(crate) kind: FailureKind,
    pub(crate) raw_os_error: Option<i32>,
}

impl fmt::Display for ProcessFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "language-server process failed at {:?}: {:?}",
            self.stage, self.kind
        )
    }
}

impl Error for ProcessFailure {}

impl ProcessFailure {
    fn io(stage: ProcessStage, error: &io::Error) -> Self {
        Self {
            stage,
            kind: FailureKind::Io(error.kind()),
            raw_os_error: error.raw_os_error(),
        }
    }

    const fn retained(stage: ProcessStage) -> Self {
        Self {
            stage,
            kind: FailureKind::RetainedBudget,
            raw_os_error: None,
        }
    }

    const fn saturated(stage: ProcessStage) -> Self {
        Self {
            stage,
            kind: FailureKind::QueueSaturated,
            raw_os_error: None,
        }
    }

    const fn panicked() -> Self {
        Self {
            stage: ProcessStage::ThreadJoin,
            kind: FailureKind::ThreadPanicked,
            raw_os_error: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ConfigError {
    MissingExecutable,
    ExecutableNotFile,
    PathTooLong,
    WorkingDirectoryNotDirectory,
    TooManyArguments,
    ArgumentTooLong,
    ConfigurationTooLarge,
    ContainsNul,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingExecutable => formatter.write_str("language-server executable is missing"),
            Self::ExecutableNotFile => {
                formatter.write_str("language-server executable is not a regular file")
            }
            Self::PathTooLong => formatter.write_str("language-server path is too long"),
            Self::WorkingDirectoryNotDirectory => {
                formatter.write_str("language-server working directory is not a directory")
            }
            Self::TooManyArguments => formatter.write_str("language-server has too many arguments"),
            Self::ArgumentTooLong => formatter.write_str("language-server argument is too long"),
            Self::ConfigurationTooLarge => {
                formatter.write_str("language-server configuration is too large")
            }
            Self::ContainsNul => formatter.write_str("language-server configuration contains NUL"),
        }
    }
}

impl Error for ConfigError {}

#[derive(Clone, Debug)]
pub(crate) struct ProcessSpec {
    executable: PathBuf,
    arguments: Box<[OsString]>,
    working_directory: Option<PathBuf>,
    retained_bytes: usize,
}

impl ProcessSpec {
    pub(crate) fn new<I, S>(
        executable: impl AsRef<Path>,
        arguments: I,
        working_directory: Option<&Path>,
    ) -> Result<Self, ConfigError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let executable =
            fs::canonicalize(executable).map_err(|_| ConfigError::MissingExecutable)?;
        if !fs::metadata(&executable)
            .map_err(|_| ConfigError::MissingExecutable)?
            .is_file()
        {
            return Err(ConfigError::ExecutableNotFile);
        }
        validate_path(&executable)?;
        let working_directory = working_directory
            .map(|path| {
                let path = fs::canonicalize(path)
                    .map_err(|_| ConfigError::WorkingDirectoryNotDirectory)?;
                if !fs::metadata(&path)
                    .map_err(|_| ConfigError::WorkingDirectoryNotDirectory)?
                    .is_dir()
                {
                    return Err(ConfigError::WorkingDirectoryNotDirectory);
                }
                validate_path(&path)?;
                Ok(path)
            })
            .transpose()?;

        let mut owned = Vec::new();
        let mut retained_bytes = encoded_len(executable.as_os_str())
            + working_directory
                .as_ref()
                .map_or(0, |path| encoded_len(path.as_os_str()));
        for argument in arguments {
            if owned.len() == MAX_ARGUMENTS {
                return Err(ConfigError::TooManyArguments);
            }
            let argument = argument.as_ref();
            reject_nul(argument)?;
            let bytes = encoded_len(argument);
            if bytes > MAX_ARGUMENT_BYTES {
                return Err(ConfigError::ArgumentTooLong);
            }
            retained_bytes = retained_bytes
                .checked_add(bytes)
                .ok_or(ConfigError::ConfigurationTooLarge)?;
            owned.push(argument.to_os_string());
        }
        if retained_bytes > MAX_CONFIGURATION_BYTES {
            return Err(ConfigError::ConfigurationTooLarge);
        }
        Ok(Self {
            executable,
            arguments: owned.into_boxed_slice(),
            working_directory,
            retained_bytes,
        })
    }
}

fn validate_path(path: &Path) -> Result<(), ConfigError> {
    reject_nul(path.as_os_str())?;
    if encoded_len(path.as_os_str()) > MAX_PATH_BYTES {
        return Err(ConfigError::PathTooLong);
    }
    Ok(())
}

fn reject_nul(value: &OsStr) -> Result<(), ConfigError> {
    if value.as_encoded_bytes().contains(&0) {
        return Err(ConfigError::ContainsNul);
    }
    Ok(())
}

fn encoded_len(value: &OsStr) -> usize {
    value.as_encoded_bytes().len()
}

#[derive(Default)]
struct Counters {
    retained_bytes: AtomicUsize,
    peak_retained_bytes: AtomicUsize,
    queued_events: AtomicUsize,
    peak_queued_events: AtomicUsize,
    submitted_inputs: AtomicU64,
    written_inputs: AtomicU64,
    input_saturations: AtomicU64,
    output_saturations: AtomicU64,
    event_saturations: AtomicU64,
    stale_events: AtomicU64,
    starts: AtomicU64,
    restarts: AtomicU64,
    exits: AtomicU64,
}

pub(crate) struct Payload {
    bytes: Box<[u8]>,
    counters: Arc<Counters>,
}

impl Payload {
    fn copy(
        bytes: &[u8],
        counters: &Arc<Counters>,
        stage: ProcessStage,
    ) -> Result<Self, ProcessFailure> {
        let mut current = counters.retained_bytes.load(Ordering::Acquire);
        loop {
            let next = current
                .checked_add(bytes.len())
                .filter(|next| *next <= MAX_RETAINED_PAYLOAD_BYTES)
                .ok_or_else(|| ProcessFailure::retained(stage))?;
            match counters.retained_bytes.compare_exchange_weak(
                current,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    counters
                        .peak_retained_bytes
                        .fetch_max(next, Ordering::Relaxed);
                    return Ok(Self {
                        bytes: Box::from(bytes),
                        counters: Arc::clone(counters),
                    });
                }
                Err(observed) => current = observed,
            }
        }
    }
}

impl fmt::Debug for Payload {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("Payload")
            .field(&self.bytes.len())
            .finish()
    }
}

impl Drop for Payload {
    fn drop(&mut self) {
        self.counters
            .retained_bytes
            .fetch_sub(self.bytes.len(), Ordering::AcqRel);
    }
}

#[derive(Debug)]
pub(crate) enum ProcessEvent {
    Started {
        identity: ProcessIdentity,
        epoch: ProcessEpoch,
        process_id: u32,
    },
    Output {
        identity: ProcessIdentity,
        epoch: ProcessEpoch,
        stream: ProcessStream,
        payload: Payload,
    },
    InputWritten {
        identity: ProcessIdentity,
        epoch: ProcessEpoch,
        sequence: InputSequence,
        bytes: usize,
    },
    InputRejected {
        identity: ProcessIdentity,
        epoch: ProcessEpoch,
        sequence: InputSequence,
        failure: ProcessFailure,
    },
    Exited {
        identity: ProcessIdentity,
        epoch: ProcessEpoch,
        success: bool,
        code: Option<i32>,
    },
    Stopped {
        identity: ProcessIdentity,
        epoch: ProcessEpoch,
        reason: StopReason,
    },
    Failed {
        identity: ProcessIdentity,
        epoch: ProcessEpoch,
        failure: ProcessFailure,
    },
}

impl ProcessEvent {
    fn identity(&self) -> ProcessIdentity {
        match self {
            Self::Started { identity, .. }
            | Self::Output { identity, .. }
            | Self::InputWritten { identity, .. }
            | Self::InputRejected { identity, .. }
            | Self::Exited { identity, .. }
            | Self::Stopped { identity, .. }
            | Self::Failed { identity, .. } => *identity,
        }
    }

    fn epoch(&self) -> ProcessEpoch {
        match self {
            Self::Started { epoch, .. }
            | Self::Output { epoch, .. }
            | Self::InputWritten { epoch, .. }
            | Self::InputRejected { epoch, .. }
            | Self::Exited { epoch, .. }
            | Self::Stopped { epoch, .. }
            | Self::Failed { epoch, .. } => *epoch,
        }
    }

    pub(crate) fn output(&self) -> Option<(ProcessStream, &[u8])> {
        if let Self::Output {
            stream, payload, ..
        } = self
        {
            return Some((*stream, &payload.bytes));
        }
        None
    }

    pub(crate) fn rejection(&self) -> Option<(InputSequence, ProcessFailure)> {
        if let Self::InputRejected {
            sequence, failure, ..
        } = self
        {
            return Some((*sequence, *failure));
        }
        None
    }

    pub(crate) fn stop_reason(&self) -> Option<StopReason> {
        if let Self::Stopped { reason, .. } = self {
            return Some(*reason);
        }
        None
    }

    pub(crate) fn failure(&self) -> Option<ProcessFailure> {
        if let Self::Failed { failure, .. } = self {
            return Some(*failure);
        }
        None
    }
}

enum Control {
    Input {
        identity: ProcessIdentity,
        epoch: ProcessEpoch,
        sequence: InputSequence,
        payload: Payload,
    },
    Restart {
        identity: ProcessIdentity,
        epoch: ProcessEpoch,
    },
}

struct WriteRequest {
    sequence: InputSequence,
    payload: Payload,
}

struct WriteResult {
    sequence: InputSequence,
    bytes: usize,
    result: Result<(), ProcessFailure>,
}

struct OutputPacket {
    stream: ProcessStream,
    payload: Payload,
}

struct Running {
    child: Child,
    identity: ProcessIdentity,
    epoch: ProcessEpoch,
    input: Option<SyncSender<WriteRequest>>,
    output: Receiver<OutputPacket>,
    writes: Receiver<WriteResult>,
    overflowed: Arc<AtomicBool>,
    helpers: Vec<JoinHandle<()>>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ProcessSnapshot {
    pub(crate) configuration_bytes: usize,
    pub(crate) retained_bytes: usize,
    pub(crate) peak_retained_bytes: usize,
    pub(crate) queued_events: usize,
    pub(crate) peak_queued_events: usize,
    pub(crate) submitted_inputs: u64,
    pub(crate) written_inputs: u64,
    pub(crate) input_saturations: u64,
    pub(crate) output_saturations: u64,
    pub(crate) event_saturations: u64,
    pub(crate) stale_events: u64,
    pub(crate) starts: u64,
    pub(crate) restarts: u64,
    pub(crate) exits: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SubmitError {
    MessageTooLarge,
    RetainedBudget,
    Saturated,
    Closed,
    SequenceExhausted,
    EpochExhausted,
    StaleRestart,
}

impl fmt::Display for SubmitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "language-server submission failed: {self:?}")
    }
}

impl Error for SubmitError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SupervisorStopped;

impl fmt::Display for SupervisorStopped {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("language-server supervisor stopped")
    }
}

impl Error for SupervisorStopped {}

pub(crate) struct LanguageServerProcess {
    control: Option<SyncSender<Control>>,
    events: Receiver<ProcessEvent>,
    supervisor: Option<JoinHandle<()>>,
    shutdown: Arc<AtomicBool>,
    counters: Arc<Counters>,
    configuration_bytes: usize,
    identity: ProcessIdentity,
    epoch: ProcessEpoch,
    next_sequence: u64,
}

impl LanguageServerProcess {
    pub(crate) fn start(
        spec: ProcessSpec,
        identity: ProcessIdentity,
    ) -> Result<Self, ProcessFailure> {
        let configuration_bytes = spec.retained_bytes;
        let counters = Arc::new(Counters::default());
        let shutdown = Arc::new(AtomicBool::new(false));
        let (control_sender, control_receiver) = sync_channel(CONTROL_CAPACITY);
        let (event_sender, event_receiver) = sync_channel(EVENT_CAPACITY);
        let worker_counters = Arc::clone(&counters);
        let worker_shutdown = Arc::clone(&shutdown);
        let epoch = ProcessEpoch(1);
        let supervisor = thread::Builder::new()
            .name("alpine-lsp-supervisor".to_owned())
            .spawn(move || {
                supervise(
                    spec,
                    identity,
                    epoch,
                    control_receiver,
                    event_sender,
                    worker_shutdown,
                    worker_counters,
                );
            })
            .map_err(|error| ProcessFailure::io(ProcessStage::SpawnChild, &error))?;
        Ok(Self {
            control: Some(control_sender),
            events: event_receiver,
            supervisor: Some(supervisor),
            shutdown,
            counters,
            configuration_bytes,
            identity,
            epoch,
            next_sequence: 0,
        })
    }

    pub(crate) fn send(&mut self, bytes: &[u8]) -> Result<InputSequence, SubmitError> {
        if bytes.len() > MAX_MESSAGE_BYTES {
            return Err(SubmitError::MessageTooLarge);
        }
        let next = self
            .next_sequence
            .checked_add(1)
            .ok_or(SubmitError::SequenceExhausted)?;
        let payload = Payload::copy(bytes, &self.counters, ProcessStage::Input)
            .map_err(|_| SubmitError::RetainedBudget)?;
        let Some(sender) = &self.control else {
            return Err(SubmitError::Closed);
        };
        match sender.try_send(Control::Input {
            identity: self.identity,
            epoch: self.epoch,
            sequence: InputSequence(next),
            payload,
        }) {
            Ok(()) => {
                self.next_sequence = next;
                self.counters
                    .submitted_inputs
                    .fetch_add(1, Ordering::Relaxed);
                Ok(InputSequence(next))
            }
            Err(TrySendError::Full(_)) => {
                self.counters
                    .input_saturations
                    .fetch_add(1, Ordering::Relaxed);
                Err(SubmitError::Saturated)
            }
            Err(TrySendError::Disconnected(_)) => Err(SubmitError::Closed),
        }
    }

    pub(crate) fn restart(
        &mut self,
        identity: ProcessIdentity,
    ) -> Result<ProcessEpoch, SubmitError> {
        if identity.generation <= self.identity.generation {
            return Err(SubmitError::StaleRestart);
        }
        let epoch = ProcessEpoch(
            self.epoch
                .0
                .checked_add(1)
                .ok_or(SubmitError::EpochExhausted)?,
        );
        let Some(sender) = &self.control else {
            return Err(SubmitError::Closed);
        };
        match sender.try_send(Control::Restart { identity, epoch }) {
            Ok(()) => {
                self.identity = identity;
                self.epoch = epoch;
                self.counters.restarts.fetch_add(1, Ordering::Relaxed);
                Ok(epoch)
            }
            Err(TrySendError::Full(_)) => Err(SubmitError::Saturated),
            Err(TrySendError::Disconnected(_)) => Err(SubmitError::Closed),
        }
    }

    pub(crate) fn try_event(&mut self) -> Result<Option<ProcessEvent>, SupervisorStopped> {
        loop {
            match self.events.try_recv() {
                Ok(event) => {
                    self.counters.queued_events.fetch_sub(1, Ordering::AcqRel);
                    if event.identity() != self.identity || event.epoch() != self.epoch {
                        self.counters.stale_events.fetch_add(1, Ordering::Relaxed);
                        continue;
                    }
                    return Ok(Some(event));
                }
                Err(TryRecvError::Empty) => return Ok(None),
                Err(TryRecvError::Disconnected) => return Err(SupervisorStopped),
            }
        }
    }

    pub(crate) fn snapshot(&self) -> ProcessSnapshot {
        ProcessSnapshot {
            configuration_bytes: self.configuration_bytes,
            retained_bytes: self.counters.retained_bytes.load(Ordering::Acquire),
            peak_retained_bytes: self.counters.peak_retained_bytes.load(Ordering::Acquire),
            queued_events: self.counters.queued_events.load(Ordering::Acquire),
            peak_queued_events: self.counters.peak_queued_events.load(Ordering::Acquire),
            submitted_inputs: self.counters.submitted_inputs.load(Ordering::Acquire),
            written_inputs: self.counters.written_inputs.load(Ordering::Acquire),
            input_saturations: self.counters.input_saturations.load(Ordering::Acquire),
            output_saturations: self.counters.output_saturations.load(Ordering::Acquire),
            event_saturations: self.counters.event_saturations.load(Ordering::Acquire),
            stale_events: self.counters.stale_events.load(Ordering::Acquire),
            starts: self.counters.starts.load(Ordering::Acquire),
            restarts: self.counters.restarts.load(Ordering::Acquire),
            exits: self.counters.exits.load(Ordering::Acquire),
        }
    }

    pub(crate) fn shutdown(&mut self) -> ProcessSnapshot {
        self.control.take();
        self.shutdown.store(true, Ordering::Release);
        if let Some(supervisor) = self.supervisor.take() {
            let _ = supervisor.join();
        }
        while self.events.try_recv().is_ok() {
            self.counters.queued_events.fetch_sub(1, Ordering::AcqRel);
        }
        self.snapshot()
    }
}

impl Drop for LanguageServerProcess {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

#[expect(
    clippy::too_many_lines,
    clippy::needless_pass_by_value,
    reason = "the supervisor owns its specification, channels, shutdown signal, and counters for one complete child lifecycle"
)]
fn supervise(
    spec: ProcessSpec,
    first_identity: ProcessIdentity,
    first_epoch: ProcessEpoch,
    controls: Receiver<Control>,
    events: SyncSender<ProcessEvent>,
    shutdown: Arc<AtomicBool>,
    counters: Arc<Counters>,
) {
    let mut running = start_running(&spec, first_identity, first_epoch, &events, &counters);
    while !shutdown.load(Ordering::Acquire) {
        match controls.recv_timeout(SUPERVISOR_POLL) {
            Ok(Control::Input {
                identity,
                epoch,
                sequence,
                payload,
            }) => {
                let Some(process) = running
                    .as_mut()
                    .filter(|process| process.identity == identity && process.epoch == epoch)
                else {
                    let _ = emit(
                        &events,
                        ProcessEvent::InputRejected {
                            identity,
                            epoch,
                            sequence,
                            failure: broken_pipe(ProcessStage::Input),
                        },
                        &counters,
                    );
                    continue;
                };
                let request = WriteRequest { sequence, payload };
                let result = match process.input.as_ref() {
                    Some(sender) => sender.try_send(request),
                    None => Err(TrySendError::Disconnected(request)),
                };
                if let Err(error) = result {
                    counters.input_saturations.fetch_add(1, Ordering::Relaxed);
                    let (sequence, failure) = match error {
                        TrySendError::Full(request) => (
                            request.sequence,
                            ProcessFailure::saturated(ProcessStage::Input),
                        ),
                        TrySendError::Disconnected(request) => {
                            (request.sequence, broken_pipe(ProcessStage::Input))
                        }
                    };
                    let _ = emit(
                        &events,
                        ProcessEvent::InputRejected {
                            identity,
                            epoch,
                            sequence,
                            failure,
                        },
                        &counters,
                    );
                }
            }
            Ok(Control::Restart { identity, epoch }) => {
                if let Some(mut old) = running.take() {
                    let old_identity = old.identity;
                    let old_epoch = old.epoch;
                    let panicked = stop_running(&mut old, true);
                    let _ = emit(
                        &events,
                        ProcessEvent::Stopped {
                            identity: old_identity,
                            epoch: old_epoch,
                            reason: StopReason::Restart,
                        },
                        &counters,
                    );
                    if panicked {
                        let _ = emit(
                            &events,
                            ProcessEvent::Failed {
                                identity: old_identity,
                                epoch: old_epoch,
                                failure: ProcessFailure::panicked(),
                            },
                            &counters,
                        );
                    }
                }
                running = start_running(&spec, identity, epoch, &events, &counters);
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }

        let mut terminate = None;
        if let Some(process) = running.as_mut() {
            if process.overflowed.swap(false, Ordering::AcqRel) {
                counters.output_saturations.fetch_add(1, Ordering::Relaxed);
                terminate = Some(StopReason::OutputOverflow);
            }
            while let Ok(packet) = process.output.try_recv() {
                if !emit(
                    &events,
                    ProcessEvent::Output {
                        identity: process.identity,
                        epoch: process.epoch,
                        stream: packet.stream,
                        payload: packet.payload,
                    },
                    &counters,
                ) {
                    terminate = Some(StopReason::EventOverflow);
                    break;
                }
            }
            while let Ok(result) = process.writes.try_recv() {
                let event = match result.result {
                    Ok(()) => {
                        counters.written_inputs.fetch_add(1, Ordering::Relaxed);
                        ProcessEvent::InputWritten {
                            identity: process.identity,
                            epoch: process.epoch,
                            sequence: result.sequence,
                            bytes: result.bytes,
                        }
                    }
                    Err(failure) => ProcessEvent::InputRejected {
                        identity: process.identity,
                        epoch: process.epoch,
                        sequence: result.sequence,
                        failure,
                    },
                };
                if !emit(&events, event, &counters) {
                    terminate = Some(StopReason::EventOverflow);
                    break;
                }
            }
            match process.child.try_wait() {
                Ok(Some(status)) => {
                    counters.exits.fetch_add(1, Ordering::Relaxed);
                    let _ = emit(
                        &events,
                        exited(process.identity, process.epoch, status),
                        &counters,
                    );
                    let Some(mut stopped) = running.take() else {
                        continue;
                    };
                    let _ = stop_running(&mut stopped, false);
                }
                Ok(None) => {}
                Err(error) => {
                    let _ = emit(
                        &events,
                        ProcessEvent::Failed {
                            identity: process.identity,
                            epoch: process.epoch,
                            failure: ProcessFailure::io(ProcessStage::Wait, &error),
                        },
                        &counters,
                    );
                    terminate = Some(StopReason::OutputOverflow);
                }
            }
        }
        if let Some(reason) = terminate
            && let Some(mut process) = running.take()
        {
            let identity = process.identity;
            let epoch = process.epoch;
            let _ = stop_running(&mut process, true);
            let _ = emit(
                &events,
                ProcessEvent::Stopped {
                    identity,
                    epoch,
                    reason,
                },
                &counters,
            );
            if reason == StopReason::EventOverflow {
                break;
            }
        }
    }
    if let Some(mut process) = running {
        let _ = stop_running(&mut process, true);
    }
}

fn start_running(
    spec: &ProcessSpec,
    identity: ProcessIdentity,
    epoch: ProcessEpoch,
    events: &SyncSender<ProcessEvent>,
    counters: &Arc<Counters>,
) -> Option<Running> {
    match spawn_process(spec, identity, epoch, counters) {
        Ok(process) => {
            counters.starts.fetch_add(1, Ordering::Relaxed);
            if emit(
                events,
                ProcessEvent::Started {
                    identity,
                    epoch,
                    process_id: process.child.id(),
                },
                counters,
            ) {
                Some(process)
            } else {
                let mut process = process;
                let _ = stop_running(&mut process, true);
                None
            }
        }
        Err(failure) => {
            let _ = emit(
                events,
                ProcessEvent::Failed {
                    identity,
                    epoch,
                    failure,
                },
                counters,
            );
            None
        }
    }
}

fn emit(events: &SyncSender<ProcessEvent>, event: ProcessEvent, counters: &Counters) -> bool {
    let queued = counters.queued_events.fetch_add(1, Ordering::AcqRel) + 1;
    counters
        .peak_queued_events
        .fetch_max(queued, Ordering::Relaxed);
    match events.try_send(event) {
        Ok(()) => true,
        Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
            counters.queued_events.fetch_sub(1, Ordering::AcqRel);
            counters.event_saturations.fetch_add(1, Ordering::Relaxed);
            false
        }
    }
}

fn spawn_process(
    spec: &ProcessSpec,
    identity: ProcessIdentity,
    epoch: ProcessEpoch,
    counters: &Arc<Counters>,
) -> Result<Running, ProcessFailure> {
    let mut command = Command::new(&spec.executable);
    command
        .args(spec.arguments.iter())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(directory) = &spec.working_directory {
        command.current_dir(directory);
    }
    let mut child = command
        .spawn()
        .map_err(|error| ProcessFailure::io(ProcessStage::SpawnChild, &error))?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| broken_pipe(ProcessStage::SpawnInput))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| broken_pipe(ProcessStage::SpawnStdout))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| broken_pipe(ProcessStage::SpawnStderr))?;
    let (input_sender, input_receiver) = sync_channel(INPUT_CAPACITY);
    let (output_sender, output_receiver) = sync_channel(OUTPUT_CAPACITY);
    let (write_sender, write_receiver) = sync_channel(WRITE_RESULT_CAPACITY);
    let overflowed = Arc::new(AtomicBool::new(false));
    let mut helpers = Vec::with_capacity(3);

    match thread::Builder::new()
        .name("alpine-lsp-input".to_owned())
        .spawn(move || writer(stdin, input_receiver, &write_sender))
    {
        Ok(handle) => helpers.push(handle),
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ProcessFailure::io(ProcessStage::SpawnInput, &error));
        }
    }
    let stdout_overflow = Arc::clone(&overflowed);
    let stdout_counters = Arc::clone(counters);
    let stdout_sender = output_sender.clone();
    match thread::Builder::new()
        .name("alpine-lsp-stdout".to_owned())
        .spawn(move || {
            reader(
                stdout,
                ProcessStream::Stdout,
                &stdout_sender,
                &stdout_overflow,
                &stdout_counters,
            );
        }) {
        Ok(handle) => helpers.push(handle),
        Err(error) => {
            cleanup_failed_spawn(&mut child, input_sender, helpers);
            return Err(ProcessFailure::io(ProcessStage::SpawnStdout, &error));
        }
    }
    let stderr_overflow = Arc::clone(&overflowed);
    let stderr_counters = Arc::clone(counters);
    match thread::Builder::new()
        .name("alpine-lsp-stderr".to_owned())
        .spawn(move || {
            reader(
                stderr,
                ProcessStream::Stderr,
                &output_sender,
                &stderr_overflow,
                &stderr_counters,
            );
        }) {
        Ok(handle) => helpers.push(handle),
        Err(error) => {
            cleanup_failed_spawn(&mut child, input_sender, helpers);
            return Err(ProcessFailure::io(ProcessStage::SpawnStderr, &error));
        }
    }
    Ok(Running {
        child,
        identity,
        epoch,
        input: Some(input_sender),
        output: output_receiver,
        writes: write_receiver,
        overflowed,
        helpers,
    })
}

fn cleanup_failed_spawn(
    child: &mut Child,
    input: SyncSender<WriteRequest>,
    helpers: Vec<JoinHandle<()>>,
) {
    drop(input);
    let _ = child.kill();
    let _ = child.wait();
    for helper in helpers {
        let _ = helper.join();
    }
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "the dedicated writer thread owns its request receiver"
)]
fn writer(
    mut stdin: ChildStdin,
    requests: Receiver<WriteRequest>,
    results: &SyncSender<WriteResult>,
) {
    while let Ok(request) = requests.recv() {
        let bytes = request.payload.bytes.len();
        let result = stdin
            .write_all(&request.payload.bytes)
            .and_then(|()| stdin.flush())
            .map_err(|error| ProcessFailure::io(ProcessStage::Input, &error));
        let failed = result.is_err();
        drop(request.payload);
        if results
            .try_send(WriteResult {
                sequence: request.sequence,
                bytes,
                result,
            })
            .is_err()
            || failed
        {
            return;
        }
    }
}

fn reader<R: Read>(
    mut source: R,
    stream: ProcessStream,
    outputs: &SyncSender<OutputPacket>,
    overflowed: &AtomicBool,
    counters: &Arc<Counters>,
) {
    let mut buffer = vec![0u8; OUTPUT_CHUNK_BYTES].into_boxed_slice();
    loop {
        let read = match source.read(&mut buffer) {
            Ok(0) | Err(_) => return,
            Ok(read) => read,
        };
        let Ok(payload) = Payload::copy(&buffer[..read], counters, ProcessStage::Output) else {
            overflowed.store(true, Ordering::Release);
            return;
        };
        if outputs.try_send(OutputPacket { stream, payload }).is_err() {
            overflowed.store(true, Ordering::Release);
            return;
        }
    }
}

fn stop_running(process: &mut Running, kill: bool) -> bool {
    process.input.take();
    if kill {
        let _ = process.child.kill();
    }
    let _ = process.child.wait();
    let mut panicked = false;
    for helper in process.helpers.drain(..) {
        panicked |= helper.join().is_err();
    }
    panicked
}

fn exited(identity: ProcessIdentity, epoch: ProcessEpoch, status: ExitStatus) -> ProcessEvent {
    ProcessEvent::Exited {
        identity,
        epoch,
        success: status.success(),
        code: status.code(),
    }
}

fn broken_pipe(stage: ProcessStage) -> ProcessFailure {
    ProcessFailure::io(
        stage,
        &io::Error::new(io::ErrorKind::BrokenPipe, "language-server pipe is closed"),
    )
}

#[cfg(test)]
mod tests {
    use std::{env, thread, time::Instant};

    use super::*;

    const TIMEOUT: Duration = Duration::from_secs(3);

    fn identity(generation: u64) -> ProcessIdentity {
        ProcessIdentity {
            workspace_revision: 1,
            generation,
        }
    }

    #[cfg(unix)]
    fn shell(script: &str) -> Result<ProcessSpec, ConfigError> {
        ProcessSpec::new("/bin/sh", ["-c", script], None)
    }

    fn wait_for(
        process: &mut LanguageServerProcess,
        predicate: impl Fn(&ProcessEvent) -> bool,
    ) -> Result<ProcessEvent, Box<dyn Error>> {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if let Some(event) = process.try_event()?
                && predicate(&event)
            {
                return Ok(event);
            }
            if Instant::now() >= deadline {
                return Err("timed out waiting for language-server event".into());
            }
            thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn identities_and_configuration_fail_before_process_ownership() -> Result<(), Box<dyn Error>> {
        assert_eq!(ProcessIdentity::new(0, 1), None);
        assert_eq!(ProcessIdentity::new(1, 0), None);
        assert!(matches!(
            ProcessSpec::new("missing-alpine-language-server", ["x"], None),
            Err(ConfigError::MissingExecutable)
        ));
        let executable = env::current_exe()?;
        assert!(matches!(
            ProcessSpec::new(&executable, vec!["x"; MAX_ARGUMENTS + 1], None),
            Err(ConfigError::TooManyArguments)
        ));
        assert!(matches!(
            ProcessSpec::new(&executable, ["x".repeat(MAX_ARGUMENT_BYTES + 1)], None),
            Err(ConfigError::ArgumentTooLong)
        ));
        assert!(matches!(
            ProcessSpec::new(&executable, [OsString::from("a\0b")], None),
            Err(ConfigError::ContainsNul)
        ));
        assert!(matches!(
            ProcessSpec::new(&executable, ["x"], Some(&executable)),
            Err(ConfigError::WorkingDirectoryNotDirectory)
        ));
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn echo_restart_and_stale_events_are_bounded() -> Result<(), Box<dyn Error>> {
        let mut process = LanguageServerProcess::start(shell("cat")?, identity(1))?;
        let started = wait_for(&mut process, |event| {
            matches!(event, ProcessEvent::Started { .. })
        })?;
        assert!(matches!(
            started,
            ProcessEvent::Started {
                process_id: 1..,
                ..
            }
        ));
        let sequence = process.send(b"Content-Length: 2\r\n\r\n{}")?;
        assert_eq!(sequence.get(), 1);
        let mut written = false;
        let mut echoed = false;
        while !written || !echoed {
            let event = wait_for(&mut process, |_| true)?;
            match event {
                ProcessEvent::InputWritten {
                    sequence: InputSequence(1),
                    bytes: 23,
                    ..
                } => written = true,
                output @ ProcessEvent::Output { .. } => {
                    assert_eq!(
                        output.output(),
                        Some((ProcessStream::Stdout, &b"Content-Length: 2\r\n\r\n{}"[..]))
                    );
                    echoed = true;
                }
                unexpected => return Err(format!("unexpected echo event: {unexpected:?}").into()),
            }
        }
        assert_eq!(process.restart(identity(2))?.get(), 2);
        assert_eq!(process.restart(identity(2)), Err(SubmitError::StaleRestart));
        let _ = wait_for(
            &mut process,
            |event| matches!(event, ProcessEvent::Started { epoch, .. } if epoch.get() == 2),
        )?;
        assert!(process.snapshot().stale_events >= 1);
        let snapshot = process.shutdown();
        assert_eq!(snapshot.starts, 2);
        assert_eq!(snapshot.restarts, 1);
        assert_eq!(snapshot.written_inputs, 1);
        assert_eq!(snapshot.retained_bytes, 0);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn crash_is_structured_and_restart_recovers() -> Result<(), Box<dyn Error>> {
        let mut process = LanguageServerProcess::start(shell("exit 7")?, identity(1))?;
        let exited = wait_for(&mut process, |event| {
            matches!(event, ProcessEvent::Exited { .. })
        })?;
        assert!(matches!(
            exited,
            ProcessEvent::Exited {
                success: false,
                code: Some(7),
                ..
            }
        ));
        process.restart(identity(2))?;
        let _ = wait_for(
            &mut process,
            |event| matches!(event, ProcessEvent::Started { epoch, .. } if epoch.get() == 2),
        )?;
        let snapshot = process.shutdown();
        assert_eq!(snapshot.exits, 1);
        assert_eq!(snapshot.starts, 2);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn output_flood_terminates_without_unbounded_retention() -> Result<(), Box<dyn Error>> {
        let mut process = LanguageServerProcess::start(
            shell("head -c 4194304 /dev/zero; exec sleep 30")?,
            identity(1),
        )?;
        let deadline = Instant::now() + TIMEOUT;
        while Instant::now() < deadline {
            if process.try_event().is_err() {
                break;
            }
            if process.snapshot().output_saturations > 0 || process.snapshot().event_saturations > 0
            {
                break;
            }
            thread::sleep(Duration::from_millis(2));
        }
        let snapshot = process.shutdown();
        assert!(snapshot.output_saturations > 0 || snapshot.event_saturations > 0);
        assert!(snapshot.peak_retained_bytes <= MAX_RETAINED_PAYLOAD_BYTES);
        assert!(snapshot.peak_queued_events <= EVENT_CAPACITY + 1);
        assert_eq!(snapshot.retained_bytes, 0);
        Ok(())
    }

    #[test]
    fn event_observers_preserve_rejection_stop_and_failure_details() {
        let identity = identity(1);
        let epoch = ProcessEpoch(1);
        let rejection = ProcessFailure::saturated(ProcessStage::Input);
        let rejected = ProcessEvent::InputRejected {
            identity,
            epoch,
            sequence: InputSequence(7),
            failure: rejection,
        };
        assert_eq!(rejected.rejection(), Some((InputSequence(7), rejection)));
        let stopped = ProcessEvent::Stopped {
            identity,
            epoch,
            reason: StopReason::Restart,
        };
        assert_eq!(stopped.stop_reason(), Some(StopReason::Restart));
        let failure = ProcessFailure::panicked();
        let failed = ProcessEvent::Failed {
            identity,
            epoch,
            failure,
        };
        assert_eq!(failed.failure(), Some(failure));
    }

    #[cfg(unix)]
    #[test]
    fn blocked_input_is_nonblocking_and_shutdown_releases_payloads() -> Result<(), Box<dyn Error>> {
        let mut process = LanguageServerProcess::start(shell("sleep 30")?, identity(1))?;
        let _ = wait_for(&mut process, |event| {
            matches!(event, ProcessEvent::Started { .. })
        })?;
        let payload = vec![b'x'; MAX_MESSAGE_BYTES / 2];
        let start = Instant::now();
        let mut admitted = 0;
        let mut rejected = 0;
        for _ in 0..16 {
            match process.send(&payload) {
                Ok(_) => admitted += 1,
                Err(SubmitError::RetainedBudget | SubmitError::Saturated) => rejected += 1,
                Err(error) => return Err(error.into()),
            }
        }
        assert!(start.elapsed() < Duration::from_secs(1));
        assert!(admitted > 0 && rejected > 0);
        let snapshot = process.shutdown();
        assert!(snapshot.peak_retained_bytes <= MAX_RETAINED_PAYLOAD_BYTES);
        assert_eq!(snapshot.retained_bytes, 0);
        Ok(())
    }
}
