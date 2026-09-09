//! Bounded ownership of one local language-server child process.

use std::{
    error::Error,
    ffi::{OsStr, OsString},
    fmt, fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        Arc, Weak,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc::{Receiver, RecvTimeoutError, SyncSender, TryRecvError, TrySendError, sync_channel},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

const CONTROL_CAPACITY: usize = 8;
const EVENT_CAPACITY: usize = 16;
const TERMINAL_EVENT_RESERVE: usize = 2;
pub(crate) const INPUT_CAPACITY: usize = 4;
const OUTPUT_CAPACITY: usize = 8;
const WRITE_RESULT_CAPACITY: usize = 8;
const OUTPUT_CHUNK_BYTES: usize = 65_536;
const MAX_MESSAGE_BYTES: usize = 16_777_216;
const MAX_RETAINED_PAYLOAD_BYTES: usize = 16_777_216;
const MAX_PATH_BYTES: usize = 4_096;
const MAX_ARGUMENTS: usize = 64;
const MAX_ARGUMENT_BYTES: usize = 4_096;
const MAX_CONFIGURATION_BYTES: usize = 65_536;
const SUPERVISOR_POLL: Duration = Duration::from_millis(2);
pub(crate) const SUPERVISOR_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) type ProcessWake = Arc<dyn Fn() + Send + Sync + 'static>;

#[cfg(test)]
fn record_lsp_startup_phase(phase: &str, detail: u64) {
    static RECORDS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    const MAX_RECORDS: usize = 64;
    let Some(path) = std::env::var_os("ALPINE_STUDIO_LSP_STARTUP_TIMING") else {
        return;
    };
    let Ok(index) = RECORDS.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
        value.checked_add(1).filter(|next| *next <= MAX_RECORDS)
    }) else {
        return;
    };
    let phase = if index == MAX_RECORDS - 1 {
        "trace-budget-exhausted"
    } else {
        phase
    };
    let result = (|| -> io::Result<()> {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_nanos();
        let record = format!(
            "{timestamp}\ttransport\t{}\t{phase}\t{detail}\n",
            std::process::id()
        );
        let mut file = std::fs::OpenOptions::new().append(true).open(path)?;
        file.write_all(record.as_bytes())
    })();
    if let Err(error) = result {
        eprintln!("invalid LSP startup capture: {error}");
    }
}

trait ThreadSpawner {
    fn spawn<F>(
        &self,
        stage: ProcessStage,
        name: &'static str,
        job: F,
    ) -> io::Result<JoinHandle<()>>
    where
        F: FnOnce() + Send + 'static;
}

struct SystemThreadSpawner;

impl ThreadSpawner for SystemThreadSpawner {
    fn spawn<F>(
        &self,
        _stage: ProcessStage,
        name: &'static str,
        job: F,
    ) -> io::Result<JoinHandle<()>>
    where
        F: FnOnce() + Send + 'static,
    {
        thread::Builder::new().name(name.to_owned()).spawn(job)
    }
}

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

    #[cfg(test)]
    pub(crate) const fn for_test(value: u64) -> Self {
        Self(value)
    }
}

// Move the existing bounded fixture receiver, rather than replacing send or
// fabricating writer acknowledgements. This owner may outlive session teardown.
#[cfg(test)]
pub(crate) struct ProcessInputObserver {
    controls: std::sync::mpsc::Receiver<Control>,
    identity: ProcessIdentity,
    epoch: ProcessEpoch,
    counters: Arc<Counters>,
}

#[cfg(test)]
impl ProcessInputObserver {
    pub(crate) fn take_input(&mut self) -> Result<Option<Vec<u8>>, ProcessFailure> {
        for _ in 0..CONTROL_CAPACITY {
            match self.controls.try_recv() {
                Ok(Control::Input {
                    identity,
                    epoch,
                    payload,
                    ..
                }) => {
                    if identity != self.identity || epoch != self.epoch {
                        return Err(ProcessFailure::io(
                            ProcessStage::Input,
                            &io::Error::new(io::ErrorKind::InvalidData, "foreign fixture input"),
                        ));
                    }
                    return Ok(Some(payload.bytes.to_vec()));
                }
                // Lifecycle controls are not outbound protocol payloads.
                Ok(_) => {}
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => return Ok(None),
            }
        }
        Ok(None)
    }

    pub(crate) fn retained_bytes(&self) -> usize {
        self.counters.retained_bytes.load(Ordering::Acquire)
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
    outstanding_inputs: AtomicUsize,
    submitted_inputs: AtomicU64,
    written_inputs: AtomicU64,
    input_saturations: AtomicU64,
    output_saturations: AtomicU64,
    event_saturations: AtomicU64,
    stale_events: AtomicU64,
    starts: AtomicU64,
    restarts: AtomicU64,
    exits: AtomicU64,
    shutdown_timeouts: AtomicU64,
    wake: Option<ProcessWake>,
}

// Reserve capacity across control admission, writer queuing and the active write.
// Dequeuing or restarting does not release ownership; retirement or write/flush does.
struct InputPermit(Arc<Counters>);

impl InputPermit {
    fn acquire(counters: &Arc<Counters>) -> Result<Self, SubmitError> {
        counters
            .outstanding_inputs
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                current
                    .checked_add(1)
                    .filter(|next| *next <= INPUT_CAPACITY)
            })
            .map_err(|_| {
                counters.input_saturations.fetch_add(1, Ordering::Relaxed);
                SubmitError::Saturated
            })?;
        Ok(Self(Arc::clone(counters)))
    }
}

impl Drop for InputPermit {
    fn drop(&mut self) {
        self.0.outstanding_inputs.fetch_sub(1, Ordering::AcqRel);
    }
}

pub(crate) struct Payload {
    bytes: Box<[u8]>,
    counters: Arc<Counters>,
    // Outputs share the byte budget, but never consume input admission permits.
    input_permit: Option<InputPermit>,
}

impl Payload {
    fn copy(
        bytes: &[u8],
        counters: &Arc<Counters>,
        stage: ProcessStage,
    ) -> Result<Self, ProcessFailure> {
        let previous = counters
            .retained_bytes
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                current
                    .checked_add(bytes.len())
                    .filter(|next| *next <= MAX_RETAINED_PAYLOAD_BYTES)
            })
            .map_err(|_| ProcessFailure::retained(stage))?;
        let next = previous + bytes.len();
        counters
            .peak_retained_bytes
            .fetch_max(next, Ordering::Relaxed);
        Ok(Self {
            bytes: Box::from(bytes),
            counters: Arc::clone(counters),
            input_permit: None,
        })
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
    pub(crate) shutdown_timeouts: u64,
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

/// Instance identity without retaining the transport's counters or wake owner.
/// The weak allocation identity cannot be reused until the token is dropped.
pub(crate) struct ProcessBinding {
    owner: Weak<Counters>,
    epoch: ProcessEpoch,
}

#[cfg(test)]
#[test]
fn binding_does_not_retain_the_transport_owner() {
    let identity = ProcessIdentity::new(1, 1).unwrap_or_else(|| unreachable!());
    let process = LanguageServerProcess::inert_for_test(identity);
    let strong = Arc::strong_count(&process.counters);
    let binding = process.binding();
    assert_eq!(Arc::strong_count(&process.counters), strong);
    assert!(process.owns_binding(&binding));
    drop(process);
    assert!(binding.owner.upgrade().is_none());
}

pub(crate) struct LanguageServerProcess {
    control: Option<SyncSender<Control>>,
    events: Receiver<ProcessEvent>,
    supervisor: Option<JoinHandle<()>>,
    supervisor_complete: Receiver<()>,
    shutdown: Arc<AtomicBool>,
    counters: Arc<Counters>,
    configuration_bytes: usize,
    identity: ProcessIdentity,
    epoch: ProcessEpoch,
    next_sequence: u64,
    #[cfg(test)]
    inert_control: Option<Receiver<Control>>,
    #[cfg(test)]
    inert_events: Option<SyncSender<ProcessEvent>>,
}

impl LanguageServerProcess {
    pub(crate) fn start(
        spec: ProcessSpec,
        identity: ProcessIdentity,
    ) -> Result<Self, ProcessFailure> {
        Self::start_with_spawner(spec, identity, None, &SystemThreadSpawner)
    }

    pub(crate) fn start_with_waker(
        spec: ProcessSpec,
        identity: ProcessIdentity,
        wake: ProcessWake,
    ) -> Result<Self, ProcessFailure> {
        Self::start_with_spawner(spec, identity, Some(wake), &SystemThreadSpawner)
    }

    fn start_with_spawner<S: ThreadSpawner>(
        spec: ProcessSpec,
        identity: ProcessIdentity,
        wake: Option<ProcessWake>,
        spawner: &S,
    ) -> Result<Self, ProcessFailure> {
        let configuration_bytes = spec.retained_bytes;
        let counters = Arc::new(Counters {
            wake,
            ..Counters::default()
        });
        let shutdown = Arc::new(AtomicBool::new(false));
        let (control_sender, control_receiver) = sync_channel(CONTROL_CAPACITY);
        let (event_sender, event_receiver) = sync_channel(EVENT_CAPACITY);
        let (completion_sender, completion_receiver) = sync_channel(1);
        let worker_counters = Arc::clone(&counters);
        let worker_shutdown = Arc::clone(&shutdown);
        let epoch = ProcessEpoch(1);
        let supervisor = spawner
            .spawn(
                ProcessStage::SpawnChild,
                "alpine-lsp-supervisor",
                move || {
                    supervise(
                        spec,
                        identity,
                        epoch,
                        control_receiver,
                        event_sender,
                        worker_shutdown,
                        worker_counters,
                    );
                    let _ = completion_sender.try_send(());
                },
            )
            .map_err(|error| ProcessFailure::io(ProcessStage::SpawnChild, &error))?;
        Ok(Self {
            control: Some(control_sender),
            events: event_receiver,
            supervisor: Some(supervisor),
            supervisor_complete: completion_receiver,
            shutdown,
            counters,
            configuration_bytes,
            identity,
            epoch,
            next_sequence: 0,
            #[cfg(test)]
            inert_control: None,
            #[cfg(test)]
            inert_events: None,
        })
    }

    #[cfg(test)]
    pub(crate) fn inert_for_test(identity: ProcessIdentity) -> Self {
        let counters = Arc::new(Counters::default());
        let shutdown = Arc::new(AtomicBool::new(false));
        let (control_sender, control_receiver) = sync_channel(CONTROL_CAPACITY);
        let (event_sender, event_receiver) = sync_channel(EVENT_CAPACITY);
        let (_, completion_receiver) = sync_channel(1);
        Self {
            control: Some(control_sender),
            events: event_receiver,
            supervisor: None,
            supervisor_complete: completion_receiver,
            shutdown,
            counters,
            configuration_bytes: 0,
            identity,
            epoch: ProcessEpoch(1),
            next_sequence: 0,
            inert_control: Some(control_receiver),
            inert_events: Some(event_sender),
        }
    }

    pub(crate) fn binding(&self) -> ProcessBinding {
        ProcessBinding {
            owner: Arc::downgrade(&self.counters),
            epoch: self.epoch,
        }
    }

    pub(crate) fn owns_binding(&self, binding: &ProcessBinding) -> bool {
        binding.epoch == self.epoch
            && std::ptr::eq(binding.owner.as_ptr(), Arc::as_ptr(&self.counters))
    }

    pub(crate) fn send(&mut self, bytes: &[u8]) -> Result<InputSequence, SubmitError> {
        if bytes.len() > MAX_MESSAGE_BYTES {
            return Err(SubmitError::MessageTooLarge);
        }
        let next = self
            .next_sequence
            .checked_add(1)
            .ok_or(SubmitError::SequenceExhausted)?;
        let Some(sender) = &self.control else {
            return Err(SubmitError::Closed);
        };
        let permit = InputPermit::acquire(&self.counters)?;
        let mut payload = Payload::copy(bytes, &self.counters, ProcessStage::Input)
            .map_err(|_| SubmitError::RetainedBudget)?;
        payload.input_permit = Some(permit);
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

    // Fault-inject a genuinely full management queue without mutating the peer
    // lifecycle. Input saturation intentionally leaves management capacity free.
    #[cfg(test)]
    pub(crate) fn fill_control_for_test(&self) -> Result<usize, SubmitError> {
        let sender = self.control.as_ref().ok_or(SubmitError::Closed)?;
        let mut admitted = 0;
        while admitted < CONTROL_CAPACITY {
            match sender.try_send(Control::Restart {
                identity: self.identity,
                epoch: self.epoch,
            }) {
                Ok(()) => admitted += 1,
                Err(TrySendError::Full(_)) => break,
                Err(TrySendError::Disconnected(_)) => return Err(SubmitError::Closed),
            }
        }
        Ok(admitted)
    }

    #[cfg(test)]
    pub(crate) fn take_input_observer_for_test(
        &mut self,
    ) -> Result<ProcessInputObserver, ProcessFailure> {
        Ok(ProcessInputObserver {
            controls: self
                .inert_control
                .take()
                .ok_or_else(|| broken_pipe(ProcessStage::Input))?,
            identity: self.identity,
            epoch: self.epoch,
            counters: Arc::clone(&self.counters),
        })
    }

    #[cfg(test)]
    pub(crate) fn take_input_for_test(&mut self) -> Result<Option<Vec<u8>>, ProcessFailure> {
        let controls = self
            .inert_control
            .as_ref()
            .ok_or_else(|| broken_pipe(ProcessStage::Input))?;
        match controls.try_recv() {
            Ok(Control::Input {
                identity,
                epoch,
                payload,
                ..
            }) if identity == self.identity && epoch == self.epoch => {
                // Consume actual outbound bytes without fabricating writer
                // acknowledgements or successful-write counters.
                Ok(Some(payload.bytes.to_vec()))
            }
            Ok(_) => Err(ProcessFailure::io(
                ProcessStage::Input,
                &io::Error::new(io::ErrorKind::InvalidData, "unexpected fixture control"),
            )),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Err(broken_pipe(ProcessStage::Input)),
        }
    }

    #[cfg(test)]
    pub(crate) fn inject_event_for_test<F>(&mut self, make: F) -> Result<(), ProcessFailure>
    where
        F: FnOnce(ProcessIdentity, ProcessEpoch) -> ProcessEvent,
    {
        let events = self
            .inert_events
            .as_ref()
            .ok_or_else(|| broken_pipe(ProcessStage::Output))?;
        let event = make(self.identity, self.epoch);
        // Replace the child producer only. Consumer identity checks, wake
        // admission, ordinary limits and terminal reserve remain production.
        let admitted = if matches!(
            event,
            ProcessEvent::Exited { .. }
                | ProcessEvent::Stopped { .. }
                | ProcessEvent::Failed { .. }
        ) {
            emit_terminal(events, event, &self.counters)
        } else {
            emit(events, event, &self.counters)
        };
        if admitted {
            Ok(())
        } else {
            Err(ProcessFailure::io(
                ProcessStage::Output,
                &io::Error::new(io::ErrorKind::WouldBlock, "test event queue rejected input"),
            ))
        }
    }

    #[cfg(test)]
    pub(crate) fn inject_stdout_for_test(&mut self, bytes: &[u8]) -> Result<(), ProcessFailure> {
        let events = self
            .inert_events
            .as_ref()
            .ok_or_else(|| broken_pipe(ProcessStage::Output))?;
        let payload = Payload::copy(bytes, &self.counters, ProcessStage::Output)?;
        // Only replace the child producer. Admission, ownership, framing and
        // the client consumer remain the production path under test.
        if emit(
            events,
            ProcessEvent::Output {
                identity: self.identity,
                epoch: self.epoch,
                stream: ProcessStream::Stdout,
                payload,
            },
            &self.counters,
        ) {
            Ok(())
        } else {
            Err(ProcessFailure::io(
                ProcessStage::Output,
                &io::Error::new(io::ErrorKind::WouldBlock, "test event queue is full"),
            ))
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
            shutdown_timeouts: self.counters.shutdown_timeouts.load(Ordering::Acquire),
        }
    }

    pub(crate) fn shutdown(&mut self) -> ProcessSnapshot {
        self.shutdown_with_budget(SUPERVISOR_SHUTDOWN_TIMEOUT)
    }

    pub(crate) fn shutdown_with_budget(&mut self, budget: Duration) -> ProcessSnapshot {
        self.control.take();
        self.shutdown.store(true, Ordering::Release);
        if let Some(supervisor) = self.supervisor.take() {
            join_supervisor(
                supervisor,
                &self.supervisor_complete,
                budget.min(SUPERVISOR_SHUTDOWN_TIMEOUT),
                &self.counters,
            );
        }
        while self.events.try_recv().is_ok() {
            self.counters.queued_events.fetch_sub(1, Ordering::AcqRel);
        }
        self.snapshot()
    }
}

fn join_supervisor(
    supervisor: JoinHandle<()>,
    completion: &Receiver<()>,
    timeout: Duration,
    counters: &Counters,
) {
    match completion.recv_timeout(timeout) {
        Ok(()) | Err(RecvTimeoutError::Disconnected) => {
            let _ = supervisor.join();
        }
        Err(RecvTimeoutError::Timeout) => {
            counters.shutdown_timeouts.fetch_add(1, Ordering::Relaxed);
            drop(supervisor);
        }
    }
}

impl Drop for LanguageServerProcess {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

#[expect(
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
    #[cfg(test)]
    record_lsp_startup_phase("supervisor-enter", 0);
    let mut running = start_running(&spec, first_identity, first_epoch, &events, &counters);
    let mut continue_supervising = true;
    while continue_supervising && !shutdown.load(Ordering::Acquire) {
        match controls.recv_timeout(SUPERVISOR_POLL) {
            Ok(Control::Input {
                identity,
                epoch,
                sequence,
                payload,
            }) => {
                let request = WriteRequest { sequence, payload };
                handle_input_control(&mut running, identity, epoch, request, &events, &counters);
            }
            Ok(Control::Restart { identity, epoch }) => {
                if let Some(old) = running.take() {
                    stop_for_restart(old, &events, &counters);
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
            terminate = merge_stop_reason(
                terminate,
                forward_outputs(
                    &process.output,
                    process.identity,
                    process.epoch,
                    &events,
                    &counters,
                ),
            );
            terminate = merge_stop_reason(
                terminate,
                forward_writes(
                    &process.writes,
                    process.identity,
                    process.epoch,
                    &events,
                    &counters,
                ),
            );
            let (exited, wait_reason) = interpret_wait(handle_wait(
                process.identity,
                process.epoch,
                process.child.try_wait(),
                &events,
                &counters,
            ));
            if exited && let Some(mut stopped) = running.take() {
                let _ = stop_running(&mut stopped, false);
            }
            terminate = merge_stop_reason(terminate, wait_reason);
        }
        if let Some(reason) = terminate {
            let _ = stop_for_reason(&mut running, reason, &events, &counters);
            continue_supervising = false;
        }
    }
    if let Some(mut process) = running {
        let _ = stop_running(&mut process, true);
    }
}

fn merge_stop_reason(
    current: Option<StopReason>,
    latest: Option<StopReason>,
) -> Option<StopReason> {
    latest.or(current)
}

fn interpret_wait(decision: WaitDecision) -> (bool, Option<StopReason>) {
    match decision {
        WaitDecision::Running => (false, None),
        WaitDecision::Exited => (true, None),
        WaitDecision::Terminate => (false, Some(StopReason::OutputOverflow)),
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
            #[cfg(test)]
            record_lsp_startup_phase("started-ready", u64::from(process.child.id()));
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
            let _ = emit_terminal(
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

fn admit_input(
    running: &mut Option<Running>,
    identity: ProcessIdentity,
    epoch: ProcessEpoch,
    request: WriteRequest,
) -> Result<(), (InputSequence, ProcessFailure, bool)> {
    let Some(process) = running
        .as_mut()
        .filter(|process| process.identity == identity && process.epoch == epoch)
    else {
        return Err((request.sequence, broken_pipe(ProcessStage::Input), false));
    };
    let result = match process.input.as_ref() {
        Some(sender) => sender.try_send(request),
        None => Err(TrySendError::Disconnected(request)),
    };
    result.map_err(|error| match error {
        TrySendError::Full(request) => (
            request.sequence,
            ProcessFailure::saturated(ProcessStage::Input),
            true,
        ),
        TrySendError::Disconnected(request) => {
            (request.sequence, broken_pipe(ProcessStage::Input), true)
        }
    })
}

fn handle_input_control(
    running: &mut Option<Running>,
    identity: ProcessIdentity,
    epoch: ProcessEpoch,
    request: WriteRequest,
    events: &SyncSender<ProcessEvent>,
    counters: &Counters,
) {
    if let Err((sequence, failure, saturated)) = admit_input(running, identity, epoch, request) {
        if saturated {
            counters.input_saturations.fetch_add(1, Ordering::Relaxed);
        }
        let _ = emit(
            events,
            ProcessEvent::InputRejected {
                identity,
                epoch,
                sequence,
                failure,
            },
            counters,
        );
    }
}

fn forward_outputs(
    outputs: &Receiver<OutputPacket>,
    identity: ProcessIdentity,
    epoch: ProcessEpoch,
    events: &SyncSender<ProcessEvent>,
    counters: &Counters,
) -> Option<StopReason> {
    while let Ok(packet) = outputs.try_recv() {
        if !emit(
            events,
            ProcessEvent::Output {
                identity,
                epoch,
                stream: packet.stream,
                payload: packet.payload,
            },
            counters,
        ) {
            return Some(StopReason::EventOverflow);
        }
    }
    None
}

fn forward_writes(
    writes: &Receiver<WriteResult>,
    identity: ProcessIdentity,
    epoch: ProcessEpoch,
    events: &SyncSender<ProcessEvent>,
    counters: &Counters,
) -> Option<StopReason> {
    while let Ok(result) = writes.try_recv() {
        let event = match result.result {
            Ok(()) => {
                counters.written_inputs.fetch_add(1, Ordering::Relaxed);
                ProcessEvent::InputWritten {
                    identity,
                    epoch,
                    sequence: result.sequence,
                    bytes: result.bytes,
                }
            }
            Err(failure) => ProcessEvent::InputRejected {
                identity,
                epoch,
                sequence: result.sequence,
                failure,
            },
        };
        if !emit(events, event, counters) {
            return Some(StopReason::EventOverflow);
        }
    }
    None
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WaitDecision {
    Running,
    Exited,
    Terminate,
}

fn handle_wait(
    identity: ProcessIdentity,
    epoch: ProcessEpoch,
    result: io::Result<Option<ExitStatus>>,
    events: &SyncSender<ProcessEvent>,
    counters: &Counters,
) -> WaitDecision {
    match classify_wait(identity, epoch, result) {
        Ok(Some(event)) => {
            counters.exits.fetch_add(1, Ordering::Relaxed);
            let _ = emit_terminal(events, event, counters);
            WaitDecision::Exited
        }
        Ok(None) => WaitDecision::Running,
        Err(failure) => {
            let _ = emit_terminal(
                events,
                ProcessEvent::Failed {
                    identity,
                    epoch,
                    failure,
                },
                counters,
            );
            WaitDecision::Terminate
        }
    }
}

fn stop_for_reason(
    running: &mut Option<Running>,
    reason: StopReason,
    events: &SyncSender<ProcessEvent>,
    counters: &Counters,
) -> bool {
    let Some(mut process) = running.take() else {
        return false;
    };
    let identity = process.identity;
    let epoch = process.epoch;
    let _ = stop_running(&mut process, true);
    let _ = emit_terminal(
        events,
        ProcessEvent::Stopped {
            identity,
            epoch,
            reason,
        },
        counters,
    );
    reason == StopReason::EventOverflow
}

fn stop_for_restart(mut process: Running, events: &SyncSender<ProcessEvent>, counters: &Counters) {
    let identity = process.identity;
    let epoch = process.epoch;
    let panicked = stop_running(&mut process, true);
    let _ = emit_terminal(
        events,
        ProcessEvent::Stopped {
            identity,
            epoch,
            reason: StopReason::Restart,
        },
        counters,
    );
    if panicked {
        let _ = emit_terminal(
            events,
            ProcessEvent::Failed {
                identity,
                epoch,
                failure: ProcessFailure::panicked(),
            },
            counters,
        );
    }
}

fn emit(events: &SyncSender<ProcessEvent>, event: ProcessEvent, counters: &Counters) -> bool {
    if counters.queued_events.load(Ordering::Acquire) >= EVENT_CAPACITY - TERMINAL_EVENT_RESERVE {
        counters.event_saturations.fetch_add(1, Ordering::Relaxed);
        return false;
    }
    emit_unreserved(events, event, counters)
}

fn emit_terminal(
    events: &SyncSender<ProcessEvent>,
    event: ProcessEvent,
    counters: &Counters,
) -> bool {
    emit_unreserved(events, event, counters)
}

fn emit_unreserved(
    events: &SyncSender<ProcessEvent>,
    event: ProcessEvent,
    counters: &Counters,
) -> bool {
    let queued = counters.queued_events.fetch_add(1, Ordering::AcqRel) + 1;
    counters
        .peak_queued_events
        .fetch_max(queued, Ordering::Relaxed);
    match events.try_send(event) {
        Ok(()) => {
            if queued == 1
                && let Some(wake) = &counters.wake
            {
                wake();
            }
            true
        }
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
    spawn_process_with(spec, identity, epoch, counters, &SystemThreadSpawner)
}

fn spawn_process_with<S: ThreadSpawner>(
    spec: &ProcessSpec,
    identity: ProcessIdentity,
    epoch: ProcessEpoch,
    counters: &Arc<Counters>,
    spawner: &S,
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
    #[cfg(test)]
    record_lsp_startup_phase("spawn-enter", 0);
    let mut child = command
        .spawn()
        .map_err(|error| ProcessFailure::io(ProcessStage::SpawnChild, &error))?;
    #[cfg(test)]
    record_lsp_startup_phase("spawn-return", u64::from(child.id()));
    let stdin = take_pipe(child.stdin.take(), ProcessStage::SpawnInput)?;
    let stdout = take_pipe(child.stdout.take(), ProcessStage::SpawnStdout)?;
    let stderr = take_pipe(child.stderr.take(), ProcessStage::SpawnStderr)?;
    let (input_sender, input_receiver) = sync_channel(INPUT_CAPACITY);
    let (output_sender, output_receiver) = sync_channel(OUTPUT_CAPACITY);
    let (write_sender, write_receiver) = sync_channel(WRITE_RESULT_CAPACITY);
    let overflowed = Arc::new(AtomicBool::new(false));
    let mut helpers = Vec::with_capacity(3);

    match spawner.spawn(ProcessStage::SpawnInput, "alpine-lsp-input", move || {
        writer(stdin, input_receiver, &write_sender);
    }) {
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
    match spawner.spawn(ProcessStage::SpawnStdout, "alpine-lsp-stdout", move || {
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
    match spawner.spawn(ProcessStage::SpawnStderr, "alpine-lsp-stderr", move || {
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
fn writer<W: Write>(
    mut stdin: W,
    requests: Receiver<WriteRequest>,
    results: &SyncSender<WriteResult>,
) {
    while let Ok(request) = requests.recv() {
        let bytes = request.payload.bytes.len();
        #[cfg(test)]
        record_lsp_startup_phase("stdin-write-enter", request.sequence.get());
        let result = stdin
            .write_all(&request.payload.bytes)
            .and_then(|()| stdin.flush())
            .map_err(|error| ProcessFailure::io(ProcessStage::Input, &error));
        let failed = result.is_err();
        #[cfg(test)]
        record_lsp_startup_phase(
            if failed {
                "stdin-write-failed"
            } else {
                "stdin-write-complete"
            },
            request.sequence.get(),
        );
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
        #[cfg(test)]
        record_lsp_startup_phase(
            match stream {
                ProcessStream::Stdout => "stdout-read-enter",
                ProcessStream::Stderr => "stderr-read-enter",
            },
            0,
        );
        let read = match source.read(&mut buffer) {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Ok(0) | Err(_) => {
                #[cfg(test)]
                record_lsp_startup_phase("pipe-read-terminal", 0);
                return;
            }
            Ok(read) => read,
        };
        #[cfg(test)]
        record_lsp_startup_phase(
            match stream {
                ProcessStream::Stdout => "stdout-read-complete",
                ProcessStream::Stderr => "stderr-read-complete",
            },
            u64::try_from(read).unwrap_or(u64::MAX),
        );
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
    join_helpers(process.helpers.drain(..).map(JoinHandle::join))
}

fn join_helpers(joins: impl Iterator<Item = thread::Result<()>>) -> bool {
    // Advancing this iterator performs each owned join. A short-circuiting
    // reduction would detach the remaining helpers after the first panic.
    let mut panicked = false;
    for result in joins {
        panicked |= result.is_err();
    }
    panicked
}

fn take_pipe<T>(pipe: Option<T>, stage: ProcessStage) -> Result<T, ProcessFailure> {
    pipe.ok_or_else(|| broken_pipe(stage))
}

fn classify_wait(
    identity: ProcessIdentity,
    epoch: ProcessEpoch,
    result: io::Result<Option<ExitStatus>>,
) -> Result<Option<ProcessEvent>, ProcessFailure> {
    result
        .map(|status| status.map(|status| exited(identity, epoch, status)))
        .map_err(|error| ProcessFailure::io(ProcessStage::Wait, &error))
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
#[path = "lsp_process_coverage_tests.rs"]
mod tests;
