//! Bounded single-window application runtime for Alpine Studio.

use core::{error::Error, fmt, num::NonZeroUsize};
use std::{
    sync::{
        Arc, Mutex, TryLockError,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc::{Receiver, SyncSender, TryRecvError, TrySendError, sync_channel},
    },
    thread::{self, JoinHandle},
};

use alpine_core::{LinearRgba, Size};
use alpine_platform_macos::{
    AccessibilityResponse, ClipboardWrite, CloseDisposition, SurfaceDescriptor, SurfaceError,
    SurfaceEvent, SurfaceFrame, SurfaceResponse, SurfaceSnapshot,
};
use alpine_scene::{Scene, SceneRevision};

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
use alpine_platform_macos::NativeSurface;
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
use std::{cell::RefCell, rc::Rc};

type WorkerJob<T> = Box<dyn FnOnce() -> T + Send + 'static>;
type WorkerWake = Arc<dyn Fn() + Send + Sync + 'static>;
type WorkerSpawner<T> = dyn FnMut(
    usize,
    Arc<Mutex<Receiver<WorkerRequest<T>>>>,
    SyncSender<WorkerCompletion<T>>,
    Arc<WorkerCounters>,
    Arc<Mutex<Option<WorkerWake>>>,
) -> std::io::Result<JoinHandle<()>>;

const MAX_WORKER_RESULTS_PER_TURN: usize = 8;
const EXTERNAL_RESULT_CAPACITY: usize = 16;
const MAX_EXTERNAL_RETAINED_BYTES: usize = 8 * 1024 * 1024;
const FIRST_EXTERNAL_SEQUENCE: u64 = 1 << 63;

/// Monotonic identity of the active local workspace state.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct WorkspaceRevision(u64);

impl WorkspaceRevision {
    /// Creates a revision from its persisted integer identity.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the underlying integer identity.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Monotonic identity of the active local document state.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct DocumentRevision(u64);

impl DocumentRevision {
    /// Creates a revision from its persisted integer identity.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the underlying integer identity.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Identity carried by one bounded background request and result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkToken {
    sequence: u64,
    workspace_revision: WorkspaceRevision,
    document_revision: DocumentRevision,
}

impl WorkToken {
    /// Returns the process-local request sequence.
    #[must_use]
    pub const fn sequence(self) -> u64 {
        self.sequence
    }

    /// Returns the workspace revision captured at submission.
    #[must_use]
    pub const fn workspace_revision(self) -> WorkspaceRevision {
        self.workspace_revision
    }

    /// Returns the document revision captured at submission.
    #[must_use]
    pub const fn document_revision(self) -> DocumentRevision {
        self.document_revision
    }
}

/// Fixed worker and channel limits for one application.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkerConfig {
    worker_count: NonZeroUsize,
    request_capacity: NonZeroUsize,
    result_capacity: NonZeroUsize,
}

impl WorkerConfig {
    /// Creates an explicitly bounded worker configuration.
    #[must_use]
    pub const fn new(
        worker_count: NonZeroUsize,
        request_capacity: NonZeroUsize,
        result_capacity: NonZeroUsize,
    ) -> Self {
        Self {
            worker_count,
            request_capacity,
            result_capacity,
        }
    }

    /// Returns the number of owned standard worker threads.
    #[must_use]
    pub const fn worker_count(self) -> usize {
        self.worker_count.get()
    }

    /// Returns the maximum queued request count.
    #[must_use]
    pub const fn request_capacity(self) -> usize {
        self.request_capacity.get()
    }

    /// Returns the maximum queued result count.
    #[must_use]
    pub const fn result_capacity(self) -> usize {
        self.result_capacity.get()
    }
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self::new(NonZeroUsize::MIN, NonZeroUsize::MIN, NonZeroUsize::MIN)
    }
}

/// Failure returned without blocking when background admission is unavailable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SubmitError {
    /// The fixed request channel is full.
    Saturated,
    /// The application has revoked new work.
    Closed,
    /// The process-local request sequence cannot advance.
    SequenceExhausted,
}

impl fmt::Display for SubmitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Saturated => formatter.write_str("the bounded worker request queue is full"),
            Self::Closed => formatter.write_str("the worker pool no longer accepts requests"),
            Self::SequenceExhausted => {
                formatter.write_str("the worker request sequence is exhausted")
            }
        }
    }
}

impl Error for SubmitError {}

/// Result of one nonblocking independent-source result submission.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExternalAdmission {
    /// The owned result entered the fixed queue.
    Admitted,
    /// The fixed item queue or retained-byte budget is full.
    Full,
    /// The runtime queue is no longer connected.
    Disconnected,
    /// Application shutdown has revoked new independent results.
    ShuttingDown,
    /// The process-local independent-result sequence cannot advance.
    SequenceExhausted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExternalRejection {
    Full,
    Disconnected,
    ShuttingDown,
    SequenceExhausted,
}

#[derive(Default)]
struct ExternalCounters {
    current_items: AtomicUsize,
    peak_items: AtomicUsize,
    current_bytes: AtomicUsize,
    peak_bytes: AtomicUsize,
    admitted: AtomicUsize,
    full: AtomicUsize,
    disconnected: AtomicUsize,
    shutting_down: AtomicUsize,
    sequence_exhausted: AtomicUsize,
    wake_requests: AtomicUsize,
    wake_coalesces: AtomicUsize,
    drained: AtomicUsize,
}

/// Handle-free current and peak evidence for independent-source results.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ExternalSnapshot {
    current_items: usize,
    peak_items: usize,
    current_bytes: usize,
    peak_bytes: usize,
    admitted: usize,
    full: usize,
    disconnected: usize,
    shutting_down: usize,
    sequence_exhausted: usize,
    wake_requests: usize,
    wake_coalesces: usize,
    drained: usize,
}

impl ExternalSnapshot {
    /// Returns independent results waiting for the foreground.
    #[must_use]
    pub const fn current_items(self) -> usize {
        self.current_items
    }

    /// Returns the greatest queued independent-result count.
    #[must_use]
    pub const fn peak_items(self) -> usize {
        self.peak_items
    }

    /// Returns bytes attributed to queued independent results.
    #[must_use]
    pub const fn current_bytes(self) -> usize {
        self.current_bytes
    }

    /// Returns the greatest attributed queued byte count.
    #[must_use]
    pub const fn peak_bytes(self) -> usize {
        self.peak_bytes
    }

    /// Returns accepted independent results.
    #[must_use]
    pub const fn admitted(self) -> usize {
        self.admitted
    }

    /// Returns submissions rejected by item or byte capacity.
    #[must_use]
    pub const fn full(self) -> usize {
        self.full
    }

    /// Returns submissions rejected after queue disconnection.
    #[must_use]
    pub const fn disconnected(self) -> usize {
        self.disconnected
    }

    /// Returns submissions rejected by application shutdown.
    #[must_use]
    pub const fn shutting_down(self) -> usize {
        self.shutting_down
    }

    /// Returns submissions rejected by sequence exhaustion.
    #[must_use]
    pub const fn sequence_exhausted(self) -> usize {
        self.sequence_exhausted
    }

    /// Returns run-loop wake requests for empty-to-nonempty transitions.
    #[must_use]
    pub const fn wake_requests(self) -> usize {
        self.wake_requests
    }

    /// Returns accepted results coalesced behind an existing wake.
    #[must_use]
    pub const fn wake_coalesces(self) -> usize {
        self.wake_coalesces
    }

    /// Returns independent results removed by foreground draining.
    #[must_use]
    pub const fn drained(self) -> usize {
        self.drained
    }
}

struct ExternalEnvelope<T> {
    sequence: u64,
    retained_bytes: usize,
    value: T,
}

struct ExternalShared<T> {
    sender: Mutex<Option<SyncSender<ExternalEnvelope<T>>>>,
    shutting_down: AtomicBool,
    next_sequence: AtomicU64,
    counters: ExternalCounters,
    wake: Arc<Mutex<Option<WorkerWake>>>,
}

/// Cloneable handle for one bounded independent local result source.
pub struct ExternalProducer<T> {
    shared: Arc<ExternalShared<T>>,
}

impl<T> Clone for ExternalProducer<T> {
    fn clone(&self) -> Self {
        Self {
            shared: Arc::clone(&self.shared),
        }
    }
}

impl<T> ExternalProducer<T> {
    /// Attempts to enqueue one owned result without waiting for capacity.
    ///
    /// `retained_bytes` is the producer's exact owned-payload attribution. It
    /// does not include channel or runtime bookkeeping.
    #[must_use]
    pub fn submit(&self, value: T, retained_bytes: usize) -> ExternalAdmission {
        let sender_guard = match self.shared.sender.try_lock() {
            Ok(sender) => sender,
            Err(TryLockError::WouldBlock) => return self.reject(ExternalRejection::Full),
            Err(TryLockError::Poisoned(_)) => {
                return self.reject(ExternalRejection::Disconnected);
            }
        };
        if self.shared.shutting_down.load(Ordering::Acquire) {
            return self.reject(ExternalRejection::ShuttingDown);
        }
        let Some(sender) = sender_guard.as_ref() else {
            return self.reject(ExternalRejection::Disconnected);
        };
        let Ok(sequence) = self.shared.next_sequence.fetch_update(
            Ordering::AcqRel,
            Ordering::Acquire,
            |current| current.checked_add(1),
        ) else {
            return self.reject(ExternalRejection::SequenceExhausted);
        };
        let Ok(previous_bytes) = self.shared.counters.current_bytes.fetch_update(
            Ordering::AcqRel,
            Ordering::Acquire,
            |current| {
                current
                    .checked_add(retained_bytes)
                    .filter(|next| *next <= MAX_EXTERNAL_RETAINED_BYTES)
            },
        ) else {
            return self.reject(ExternalRejection::Full);
        };
        let queued = self
            .shared
            .counters
            .current_items
            .fetch_add(1, Ordering::AcqRel)
            + 1;
        let send_result = sender
            .try_send(ExternalEnvelope {
                sequence,
                retained_bytes,
                value,
            })
            .map_err(|error| match error {
                TrySendError::Full(_) => ExternalRejection::Full,
                TrySendError::Disconnected(_) => ExternalRejection::Disconnected,
            });
        if let Err(admission) = send_result {
            self.shared
                .counters
                .current_items
                .fetch_sub(1, Ordering::AcqRel);
            self.shared
                .counters
                .current_bytes
                .fetch_sub(retained_bytes, Ordering::AcqRel);
            return self.reject(admission);
        }
        drop(sender_guard);
        self.admit(queued, previous_bytes + retained_bytes)
    }

    fn reject(&self, rejection: ExternalRejection) -> ExternalAdmission {
        let (counter, admission) = match rejection {
            ExternalRejection::Full => (&self.shared.counters.full, ExternalAdmission::Full),
            ExternalRejection::Disconnected => (
                &self.shared.counters.disconnected,
                ExternalAdmission::Disconnected,
            ),
            ExternalRejection::ShuttingDown => (
                &self.shared.counters.shutting_down,
                ExternalAdmission::ShuttingDown,
            ),
            ExternalRejection::SequenceExhausted => (
                &self.shared.counters.sequence_exhausted,
                ExternalAdmission::SequenceExhausted,
            ),
        };
        counter.fetch_add(1, Ordering::Relaxed);
        admission
    }

    fn admit(&self, queued: usize, retained_bytes: usize) -> ExternalAdmission {
        self.shared
            .counters
            .admitted
            .fetch_add(1, Ordering::Relaxed);
        update_peak(&self.shared.counters.peak_items, queued);
        update_peak(&self.shared.counters.peak_bytes, retained_bytes);
        if queued == 1 {
            self.shared
                .counters
                .wake_requests
                .fetch_add(1, Ordering::Relaxed);
            if let Ok(installed) = self.shared.wake.lock()
                && let Some(wake) = installed.as_ref()
            {
                wake();
            }
        } else {
            self.shared
                .counters
                .wake_coalesces
                .fetch_add(1, Ordering::Relaxed);
        }
        ExternalAdmission::Admitted
    }
}

/// Construction or native execution failure for one application.
#[derive(Debug)]
pub enum RuntimeError {
    /// A standard worker thread could not be created.
    WorkerSpawn(std::io::Error),
    /// The native single-window surface failed.
    Surface(SurfaceError),
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WorkerSpawn(error) => {
                write!(formatter, "failed to create bounded worker: {error}")
            }
            Self::Surface(error) => write!(formatter, "native application surface failed: {error}"),
        }
    }
}

impl Error for RuntimeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::WorkerSpawn(error) => Some(error),
            Self::Surface(error) => Some(error),
        }
    }
}

impl From<SurfaceError> for RuntimeError {
    fn from(error: SurfaceError) -> Self {
        Self::Surface(error)
    }
}

struct WorkerRequest<T> {
    token: WorkToken,
    job: WorkerJob<T>,
}

enum WorkerOutcome<T> {
    Completed(T),
    Panicked,
}

struct WorkerCompletion<T> {
    token: WorkToken,
    outcome: WorkerOutcome<T>,
}

#[derive(Default)]
struct WorkerCounters {
    queued_requests: AtomicUsize,
    peak_queued_requests: AtomicUsize,
    queued_results: AtomicUsize,
    peak_queued_results: AtomicUsize,
    request_saturations: AtomicUsize,
    dropped_results: AtomicUsize,
    panicked_jobs: AtomicUsize,
}

/// Handle-free current and peak evidence for bounded background work.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct WorkerSnapshot {
    queued_requests: usize,
    peak_queued_requests: usize,
    queued_results: usize,
    peak_queued_results: usize,
    request_saturations: usize,
    dropped_results: usize,
    panicked_jobs: usize,
}

impl WorkerSnapshot {
    /// Returns requests waiting for a worker.
    #[must_use]
    pub const fn queued_requests(self) -> usize {
        self.queued_requests
    }

    /// Returns the greatest observed queued-request count.
    #[must_use]
    pub const fn peak_queued_requests(self) -> usize {
        self.peak_queued_requests
    }

    /// Returns results waiting for the foreground.
    #[must_use]
    pub const fn queued_results(self) -> usize {
        self.queued_results
    }

    /// Returns the greatest observed queued-result count.
    #[must_use]
    pub const fn peak_queued_results(self) -> usize {
        self.peak_queued_results
    }

    /// Returns nonblocking submissions rejected by the request bound.
    #[must_use]
    pub const fn request_saturations(self) -> usize {
        self.request_saturations
    }

    /// Returns completed results omitted after foreground disconnection.
    #[must_use]
    pub const fn dropped_results(self) -> usize {
        self.dropped_results
    }

    /// Returns jobs that reached the panic boundary.
    #[must_use]
    pub const fn panicked_jobs(self) -> usize {
        self.panicked_jobs
    }
}

struct WorkerPool<T> {
    request_sender: Option<SyncSender<WorkerRequest<T>>>,
    result_receiver: Option<Receiver<WorkerCompletion<T>>>,
    workers: Vec<JoinHandle<()>>,
    counters: Arc<WorkerCounters>,
    next_sequence: u64,
    wake: Arc<Mutex<Option<WorkerWake>>>,
}

struct ExternalQueue<T> {
    receiver: Receiver<ExternalEnvelope<T>>,
    shared: Arc<ExternalShared<T>>,
}

impl<T> ExternalQueue<T> {
    fn new(wake: Arc<Mutex<Option<WorkerWake>>>) -> Self {
        let (sender, receiver) = sync_channel(EXTERNAL_RESULT_CAPACITY);
        Self {
            receiver,
            shared: Arc::new(ExternalShared {
                sender: Mutex::new(Some(sender)),
                shutting_down: AtomicBool::new(false),
                next_sequence: AtomicU64::new(FIRST_EXTERNAL_SEQUENCE),
                counters: ExternalCounters::default(),
                wake,
            }),
        }
    }

    fn producer(&self) -> ExternalProducer<T> {
        ExternalProducer {
            shared: Arc::clone(&self.shared),
        }
    }

    fn try_result(&self) -> Option<ExternalEnvelope<T>> {
        match self.receiver.try_recv() {
            Ok(result) => {
                self.shared
                    .counters
                    .current_items
                    .fetch_sub(1, Ordering::AcqRel);
                self.shared
                    .counters
                    .current_bytes
                    .fetch_sub(result.retained_bytes, Ordering::AcqRel);
                self.shared.counters.drained.fetch_add(1, Ordering::Relaxed);
                Some(result)
            }
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => None,
        }
    }

    fn snapshot(&self) -> ExternalSnapshot {
        let counters = &self.shared.counters;
        ExternalSnapshot {
            current_items: counters.current_items.load(Ordering::Acquire),
            peak_items: counters.peak_items.load(Ordering::Acquire),
            current_bytes: counters.current_bytes.load(Ordering::Acquire),
            peak_bytes: counters.peak_bytes.load(Ordering::Acquire),
            admitted: counters.admitted.load(Ordering::Acquire),
            full: counters.full.load(Ordering::Acquire),
            disconnected: counters.disconnected.load(Ordering::Acquire),
            shutting_down: counters.shutting_down.load(Ordering::Acquire),
            sequence_exhausted: counters.sequence_exhausted.load(Ordering::Acquire),
            wake_requests: counters.wake_requests.load(Ordering::Acquire),
            wake_coalesces: counters.wake_coalesces.load(Ordering::Acquire),
            drained: counters.drained.load(Ordering::Acquire),
        }
    }

    fn close(&self) {
        let Ok(mut sender) = self.shared.sender.lock() else {
            self.shared.shutting_down.store(true, Ordering::Release);
            return;
        };
        self.shared.shutting_down.store(true, Ordering::Release);
        sender.take();
        drop(sender);
        while self.try_result().is_some() {}
    }
}

impl<T> Drop for ExternalQueue<T> {
    fn drop(&mut self) {
        self.close();
    }
}

impl<T: Send + 'static> WorkerPool<T> {
    fn new(config: WorkerConfig) -> Result<Self, RuntimeError> {
        let mut spawn = |index: usize,
                         requests: Arc<Mutex<Receiver<WorkerRequest<T>>>>,
                         results: SyncSender<WorkerCompletion<T>>,
                         counters: Arc<WorkerCounters>,
                         wake: Arc<Mutex<Option<WorkerWake>>>| {
            thread::Builder::new()
                .name(format!("alpine-worker-{index}"))
                .spawn(move || worker_loop(&requests, &results, &counters, &wake))
        };
        Self::new_with_spawner(config, &mut spawn)
    }

    fn new_with_spawner(
        config: WorkerConfig,
        spawn: &mut WorkerSpawner<T>,
    ) -> Result<Self, RuntimeError> {
        let (request_sender, request_receiver) = sync_channel(config.request_capacity());
        let (result_sender, result_receiver) = sync_channel(config.result_capacity());
        let request_receiver = Arc::new(Mutex::new(request_receiver));
        let counters = Arc::new(WorkerCounters::default());
        let wake: Arc<Mutex<Option<WorkerWake>>> = Arc::new(Mutex::new(None));
        let mut workers = Vec::with_capacity(config.worker_count());

        for index in 0..config.worker_count() {
            let requests = Arc::clone(&request_receiver);
            let results = result_sender.clone();
            let worker_counters = Arc::clone(&counters);
            let worker_wake = Arc::clone(&wake);
            match spawn(index, requests, results, worker_counters, worker_wake) {
                Ok(worker) => workers.push(worker),
                Err(error) => {
                    drop(request_sender);
                    for worker in workers {
                        let _ = worker.join();
                    }
                    return Err(RuntimeError::WorkerSpawn(error));
                }
            }
        }
        drop(result_sender);

        Ok(Self {
            request_sender: Some(request_sender),
            result_receiver: Some(result_receiver),
            workers,
            counters,
            next_sequence: 0,
            wake,
        })
    }

    fn set_waker(&mut self, wake: WorkerWake) {
        if let Ok(mut installed) = self.wake.lock() {
            *installed = Some(wake);
        }
    }

    fn wake(&self) {
        if let Ok(installed) = self.wake.lock()
            && let Some(wake) = installed.as_ref()
        {
            wake();
        }
    }

    fn submit<F>(
        &mut self,
        workspace_revision: WorkspaceRevision,
        document_revision: DocumentRevision,
        job: F,
    ) -> Result<WorkToken, SubmitError>
    where
        F: FnOnce() -> T + Send + 'static,
    {
        let sequence = self
            .next_sequence
            .checked_add(1)
            .ok_or(SubmitError::SequenceExhausted)?;
        let token = WorkToken {
            sequence,
            workspace_revision,
            document_revision,
        };
        let Some(sender) = &self.request_sender else {
            return Err(SubmitError::Closed);
        };
        let queued = self.counters.queued_requests.fetch_add(1, Ordering::AcqRel) + 1;
        update_peak(&self.counters.peak_queued_requests, queued);
        match sender.try_send(WorkerRequest {
            token,
            job: Box::new(job),
        }) {
            Ok(()) => {
                self.next_sequence = sequence;
                Ok(token)
            }
            Err(TrySendError::Full(_)) => {
                self.counters.queued_requests.fetch_sub(1, Ordering::AcqRel);
                self.counters
                    .request_saturations
                    .fetch_add(1, Ordering::Relaxed);
                Err(SubmitError::Saturated)
            }
            Err(TrySendError::Disconnected(_)) => {
                self.counters.queued_requests.fetch_sub(1, Ordering::AcqRel);
                Err(SubmitError::Closed)
            }
        }
    }

    fn try_completion(&self) -> Option<WorkerCompletion<T>> {
        let result_receiver = self.result_receiver.as_ref()?;
        match result_receiver.try_recv() {
            Ok(completion) => {
                self.counters.queued_results.fetch_sub(1, Ordering::AcqRel);
                Some(completion)
            }
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => None,
        }
    }

    fn snapshot(&self) -> WorkerSnapshot {
        WorkerSnapshot {
            queued_requests: self.counters.queued_requests.load(Ordering::Acquire),
            peak_queued_requests: self.counters.peak_queued_requests.load(Ordering::Acquire),
            queued_results: self.counters.queued_results.load(Ordering::Acquire),
            peak_queued_results: self.counters.peak_queued_results.load(Ordering::Acquire),
            request_saturations: self.counters.request_saturations.load(Ordering::Acquire),
            dropped_results: self.counters.dropped_results.load(Ordering::Acquire),
            panicked_jobs: self.counters.panicked_jobs.load(Ordering::Acquire),
        }
    }

    #[cfg(any(test, all(target_os = "macos", target_arch = "aarch64")))]
    fn shutdown(&mut self) {
        self.request_sender.take();
        self.result_receiver.take();
        for worker in self.workers.drain(..) {
            if worker.join().is_err() {
                self.counters.panicked_jobs.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

impl<T> Drop for WorkerPool<T> {
    fn drop(&mut self) {
        self.request_sender.take();
        self.result_receiver.take();
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

fn worker_loop<T: Send + 'static>(
    requests: &Mutex<Receiver<WorkerRequest<T>>>,
    results: &SyncSender<WorkerCompletion<T>>,
    counters: &WorkerCounters,
    wake: &Mutex<Option<WorkerWake>>,
) {
    loop {
        let request = match requests.lock() {
            Ok(receiver) => receiver.recv(),
            Err(_) => return,
        };
        let Ok(request) = request else {
            return;
        };
        counters.queued_requests.fetch_sub(1, Ordering::AcqRel);
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(request.job))
            .map_or_else(
                |_| {
                    counters.panicked_jobs.fetch_add(1, Ordering::Relaxed);
                    WorkerOutcome::Panicked
                },
                WorkerOutcome::Completed,
            );
        let queued = counters.queued_results.fetch_add(1, Ordering::AcqRel) + 1;
        update_peak(&counters.peak_queued_results, queued);
        if results
            .send(WorkerCompletion {
                token: request.token,
                outcome,
            })
            .is_ok()
        {
            if let Ok(installed) = wake.lock()
                && let Some(wake) = installed.as_ref()
            {
                wake();
            }
        } else {
            counters.queued_results.fetch_sub(1, Ordering::AcqRel);
            counters.dropped_results.fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn update_peak(peak: &AtomicUsize, candidate: usize) {
    peak.fetch_max(candidate, Ordering::Relaxed);
}

/// Mutable foreground capabilities available during event and result handling.
pub struct AppContext<'a, T> {
    workspace_revision: &'a mut WorkspaceRevision,
    document_revision: &'a mut DocumentRevision,
    dirty: &'a mut bool,
    workers: &'a mut WorkerPool<T>,
    external: &'a ExternalQueue<T>,
    clipboard_write: Option<&'a mut Option<ClipboardWrite>>,
    close_disposition: Option<&'a mut CloseDisposition>,
    accessibility_response: Option<&'a mut Option<AccessibilityResponse>>,
}

impl<T: Send + 'static> AppContext<'_, T> {
    /// Marks the latest application revision for one future scene build.
    pub fn invalidate(&mut self) {
        *self.dirty = true;
    }

    /// Returns the current workspace revision.
    #[must_use]
    pub const fn workspace_revision(&self) -> WorkspaceRevision {
        *self.workspace_revision
    }

    /// Returns the current document revision.
    #[must_use]
    pub const fn document_revision(&self) -> DocumentRevision {
        *self.document_revision
    }

    /// Advances the workspace revision and invalidates when it is newer.
    pub fn advance_workspace(&mut self, revision: WorkspaceRevision) -> bool {
        if revision <= *self.workspace_revision {
            return false;
        }
        *self.workspace_revision = revision;
        *self.dirty = true;
        true
    }

    /// Advances the document revision and invalidates when it is newer.
    pub fn advance_document(&mut self, revision: DocumentRevision) -> bool {
        if revision <= *self.document_revision {
            return false;
        }
        *self.document_revision = revision;
        *self.dirty = true;
        true
    }

    /// Requests one bounded clipboard write for the current native event.
    ///
    /// Returns `false` outside event dispatch or when a write is already set.
    pub fn write_clipboard(&mut self, write: ClipboardWrite) -> bool {
        let Some(slot) = self.clipboard_write.as_deref_mut() else {
            return false;
        };
        if slot.is_some() {
            return false;
        }
        *slot = Some(write);
        true
    }

    /// Cancels the current close request while leaving non-close events unchanged.
    pub fn cancel_close(&mut self) -> bool {
        let Some(disposition) = self.close_disposition.as_deref_mut() else {
            return false;
        };
        if *disposition != CloseDisposition::Allow {
            return false;
        }
        *disposition = CloseDisposition::Cancel;
        true
    }

    /// Returns one exact accessibility response for the current event.
    ///
    /// Returns `false` outside event dispatch or when a response is already set.
    pub fn respond_accessibility(&mut self, response: AccessibilityResponse) -> bool {
        let Some(slot) = self.accessibility_response.as_deref_mut() else {
            return false;
        };
        if slot.is_some() {
            return false;
        }
        *slot = Some(response);
        true
    }

    /// Submits one revision-tagged job without waiting for queue capacity.
    ///
    /// # Errors
    ///
    /// Returns [`SubmitError::Saturated`] when the fixed request queue is full,
    /// [`SubmitError::Closed`] after shutdown, or
    /// [`SubmitError::SequenceExhausted`] if request identity cannot advance.
    pub fn spawn<F>(&mut self, job: F) -> Result<WorkToken, SubmitError>
    where
        F: FnOnce() -> T + Send + 'static,
    {
        self.workers
            .submit(*self.workspace_revision, *self.document_revision, job)
    }

    /// Returns a handle-free producer for independent local results.
    ///
    /// The delegate remains responsible for carrying and validating exact
    /// application identity inside each submitted payload.
    #[must_use]
    pub fn external_producer(&self) -> ExternalProducer<T> {
        self.external.producer()
    }
}

/// Immutable values supplied for one dirty scene build.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowContext {
    scene_revision: SceneRevision,
    viewport: Size,
}

impl WindowContext {
    /// Returns the exact revision the delegate must place in its scene.
    #[must_use]
    pub const fn scene_revision(self) -> SceneRevision {
        self.scene_revision
    }

    /// Returns the latest validated logical viewport.
    #[must_use]
    pub const fn viewport(self) -> Size {
        self.viewport
    }
}

/// Synchronous main-thread application behavior owned by Alpine Studio.
pub trait AppDelegate {
    /// Result type produced by this application's bounded workers.
    type WorkerOutput: Send + 'static;

    /// Mutates foreground state for one native event without blocking.
    fn event(&mut self, event: &SurfaceEvent, context: &mut AppContext<'_, Self::WorkerOutput>);

    /// Applies one current background result on the foreground thread.
    fn worker_result(
        &mut self,
        _token: WorkToken,
        _result: Self::WorkerOutput,
        _context: &mut AppContext<'_, Self::WorkerOutput>,
    ) {
    }

    /// Builds one immutable scene for the supplied latest revision.
    fn frame(&mut self, context: WindowContext) -> Scene;
}

/// Handle-free runtime state and bounded-work evidence.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ApplicationSnapshot {
    workspace_revision: WorkspaceRevision,
    document_revision: DocumentRevision,
    next_scene_revision: u64,
    dirty: bool,
    shutting_down: bool,
    stale_results: usize,
    invalid_scenes: usize,
    worker: WorkerSnapshot,
    external: ExternalSnapshot,
}

impl Default for ApplicationSnapshot {
    fn default() -> Self {
        Self {
            workspace_revision: WorkspaceRevision::default(),
            document_revision: DocumentRevision::default(),
            next_scene_revision: 1,
            dirty: false,
            shutting_down: false,
            stale_results: 0,
            invalid_scenes: 0,
            worker: WorkerSnapshot::default(),
            external: ExternalSnapshot::default(),
        }
    }
}

/// Handle-free final evidence from one completed application run.
#[derive(Clone, Debug, PartialEq)]
pub struct ApplicationCompletion {
    application: ApplicationSnapshot,
    surface: SurfaceSnapshot,
}

impl ApplicationCompletion {
    /// Returns final bounded foreground and worker evidence.
    #[must_use]
    pub const fn application(&self) -> ApplicationSnapshot {
        self.application
    }

    /// Returns final copied native presentation and residency evidence.
    #[must_use]
    pub const fn surface(&self) -> &SurfaceSnapshot {
        &self.surface
    }
}

impl ApplicationSnapshot {
    /// Returns the current workspace revision.
    #[must_use]
    pub const fn workspace_revision(self) -> WorkspaceRevision {
        self.workspace_revision
    }

    /// Returns the current document revision.
    #[must_use]
    pub const fn document_revision(self) -> DocumentRevision {
        self.document_revision
    }

    /// Returns the next scene revision to be admitted.
    #[must_use]
    pub const fn next_scene_revision(self) -> u64 {
        self.next_scene_revision
    }

    /// Returns whether observable state still requires a frame.
    #[must_use]
    pub const fn is_dirty(self) -> bool {
        self.dirty
    }

    /// Returns whether close has revoked new foreground work.
    #[must_use]
    pub const fn is_shutting_down(self) -> bool {
        self.shutting_down
    }

    /// Returns background results rejected by revision identity.
    #[must_use]
    pub const fn stale_results(self) -> usize {
        self.stale_results
    }

    /// Returns delegate scenes rejected for revision or viewport mismatch.
    #[must_use]
    pub const fn invalid_scenes(self) -> usize {
        self.invalid_scenes
    }

    /// Returns bounded worker accounting.
    #[must_use]
    pub const fn worker(self) -> WorkerSnapshot {
        self.worker
    }

    /// Returns bounded independent-source result accounting.
    #[must_use]
    pub const fn external(self) -> ExternalSnapshot {
        self.external
    }
}

/// One single-window foreground state graph and its bounded workers.
pub struct Application<D: AppDelegate> {
    delegate: D,
    workers: WorkerPool<D::WorkerOutput>,
    external: ExternalQueue<D::WorkerOutput>,
    workspace_revision: WorkspaceRevision,
    document_revision: DocumentRevision,
    scene_revision: u64,
    viewport: Size,
    clear: LinearRgba,
    dirty: bool,
    shutting_down: bool,
    stale_results: usize,
    invalid_scenes: usize,
    drain_external_next: bool,
}

impl<D: AppDelegate + 'static> Application<D> {
    /// Creates one dirty application ready to build its first frame.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeError::WorkerSpawn`] if a fixed worker cannot start.
    pub fn new(
        delegate: D,
        viewport: Size,
        clear: LinearRgba,
        worker_config: WorkerConfig,
    ) -> Result<Self, RuntimeError> {
        let workers = WorkerPool::new(worker_config)?;
        let external = ExternalQueue::new(Arc::clone(&workers.wake));
        Ok(Self {
            delegate,
            workers,
            external,
            workspace_revision: WorkspaceRevision::default(),
            document_revision: DocumentRevision::default(),
            scene_revision: 0,
            viewport,
            clear,
            dirty: true,
            shutting_down: false,
            stale_results: 0,
            invalid_scenes: 0,
            drain_external_next: true,
        })
    }

    /// Replaces the worker completion wake callback.
    pub fn set_worker_waker<F>(&mut self, wake: F)
    where
        F: Fn() + Send + Sync + 'static,
    {
        self.workers.set_waker(Arc::new(wake));
    }

    /// Dispatches one event, rejects stale results, and builds at most one frame.
    pub fn dispatch(&mut self, event: &SurfaceEvent) -> Option<SurfaceFrame> {
        self.dispatch_with_response(event).into_frame()
    }

    /// Dispatches one event and returns bounded native side effects plus a frame.
    #[must_use]
    pub fn dispatch_with_response(&mut self, event: &SurfaceEvent) -> SurfaceResponse {
        if self.shutting_down {
            return SurfaceResponse::default();
        }
        let accessibility_kind = match event {
            SurfaceEvent::Accessibility { request, .. } => Some(request.kind()),
            _ => None,
        };
        let defer_accessibility_query_frame = accessibility_kind
            .is_some_and(|kind| kind != alpine_platform_macos::AccessibilityRequestKind::Action);
        let mut clipboard_write = None;
        let mut accessibility_response = None;
        let mut close_disposition = if matches!(event, SurfaceEvent::CloseRequested { .. }) {
            CloseDisposition::Allow
        } else {
            CloseDisposition::NotRequested
        };
        if accessibility_kind.is_none() {
            self.drain_worker_results();
        }
        if let SurfaceEvent::Resize { extent, .. } = event
            && let Some(viewport) = extent.logical_size()
            && viewport != self.viewport
        {
            self.viewport = viewport;
            self.dirty = true;
        }
        {
            let mut context = AppContext {
                workspace_revision: &mut self.workspace_revision,
                document_revision: &mut self.document_revision,
                dirty: &mut self.dirty,
                workers: &mut self.workers,
                external: &self.external,
                clipboard_write: Some(&mut clipboard_write),
                close_disposition: Some(&mut close_disposition),
                accessibility_response: Some(&mut accessibility_response),
            };
            self.delegate.event(event, &mut context);
        }
        if accessibility_kind == Some(alpine_platform_macos::AccessibilityRequestKind::Action) {
            self.drain_worker_results();
        }
        if close_disposition == CloseDisposition::Allow {
            self.external.close();
            self.shutting_down = true;
            self.dirty = false;
            return SurfaceResponse::from_channels(
                None,
                clipboard_write,
                close_disposition,
                accessibility_response,
            );
        }
        let frame = if defer_accessibility_query_frame {
            None
        } else {
            self.frame_if_dirty()
        };
        SurfaceResponse::from_channels(
            frame,
            clipboard_write,
            close_disposition,
            accessibility_response,
        )
    }

    /// Builds the current immutable frame only when observable state is dirty.
    pub fn frame_if_dirty(&mut self) -> Option<SurfaceFrame> {
        if !self.dirty || self.shutting_down {
            return None;
        }
        let Some(next_revision) = self.scene_revision.checked_add(1) else {
            self.invalid_scenes = self.invalid_scenes.saturating_add(1);
            return None;
        };
        let context = WindowContext {
            scene_revision: SceneRevision::new(next_revision),
            viewport: self.viewport,
        };
        let scene = self.delegate.frame(context);
        if scene.revision() != context.scene_revision() || scene.viewport() != context.viewport() {
            self.invalid_scenes = self.invalid_scenes.saturating_add(1);
            return None;
        }
        self.scene_revision = next_revision;
        self.dirty = false;
        Some(SurfaceFrame::new(scene, self.clear))
    }

    /// Returns current handle-free runtime evidence.
    #[must_use]
    pub fn snapshot(&self) -> ApplicationSnapshot {
        ApplicationSnapshot {
            workspace_revision: self.workspace_revision,
            document_revision: self.document_revision,
            next_scene_revision: self.scene_revision.saturating_add(1),
            dirty: self.dirty,
            shutting_down: self.shutting_down,
            stale_results: self.stale_results,
            invalid_scenes: self.invalid_scenes,
            worker: self.workers.snapshot(),
            external: self.external.snapshot(),
        }
    }

    /// Owns one native surface and runs until its production close boundary.
    ///
    /// # Errors
    ///
    /// Returns a structured worker or native surface failure.
    pub fn run(self, descriptor: &SurfaceDescriptor) -> Result<(), RuntimeError> {
        self.run_with_completion(descriptor).map(|_| ())
    }

    /// Owns one native surface and returns copied final evidence after close.
    ///
    /// # Errors
    ///
    /// Returns a structured worker or native surface failure. No completion is
    /// returned unless the production close boundary was observed.
    pub fn run_with_completion(
        self,
        descriptor: &SurfaceDescriptor,
    ) -> Result<ApplicationCompletion, RuntimeError> {
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        {
            let surface = NativeSurface::new(descriptor)?;
            let application =
                self.run_on_native_surface(&surface, |_| Ok(()))?
                    .ok_or(RuntimeError::Surface(SurfaceError::invariant(
                        alpine_platform_macos::SurfaceOperation::Application,
                    )))?;
            Ok(ApplicationCompletion {
                application,
                surface: surface.snapshot(),
            })
        }

        #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
        {
            let _ = descriptor;
            Err(RuntimeError::Surface(SurfaceError::UnsupportedPlatform))
        }
    }

    /// Runs the production application composition against an instrumented surface.
    ///
    /// This is available only to Apple Silicon native-validation builds. The
    /// callback runs after the production show boundary and immediately before
    /// the startup wake and AppKit event loop.
    ///
    /// # Errors
    ///
    /// Returns the same structured worker or surface failure as [`Self::run`],
    /// including a callback setup failure.
    #[cfg(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64"))]
    #[doc(hidden)]
    pub fn run_on_native_surface_for_validation<F>(
        self,
        surface: &NativeSurface,
        before_run: F,
    ) -> Result<Option<ApplicationSnapshot>, RuntimeError>
    where
        F: FnOnce(&NativeSurface) -> Result<(), SurfaceError>,
    {
        self.run_on_native_surface(surface, before_run)
    }

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    fn run_on_native_surface<F>(
        self,
        surface: &NativeSurface,
        before_run: F,
    ) -> Result<Option<ApplicationSnapshot>, RuntimeError>
    where
        F: FnOnce(&NativeSurface) -> Result<(), SurfaceError>,
    {
        let mut application = self;
        let surface_waker = surface.waker();
        let startup_waker = surface_waker.clone();
        application.set_worker_waker(move || {
            let _ = surface_waker.wake();
        });
        if let Some(frame) = application.frame_if_dirty() {
            let (scene, clear) = frame.into_parts();
            let _revision = surface.request_frame(scene, clear)?;
        }
        surface.show()?;
        before_run(surface)?;
        let _ = startup_waker.wake();

        let state = Rc::new(RefCell::new(application));
        let callback_state = Rc::clone(&state);
        let run_result = surface.run_with_event_handler(move |event| {
            callback_state.try_borrow_mut().map_or_else(
                |_| SurfaceResponse::default(),
                |mut application| application.dispatch_with_response(&event),
            )
        });
        let snapshot = state.try_borrow_mut().ok().and_then(|mut application| {
            let close_observed = application.shutting_down;
            application.external.close();
            application.shutting_down = true;
            application.dirty = false;
            application.workers.shutdown();
            close_observed.then(|| application.snapshot())
        });
        run_result.map_err(RuntimeError::Surface)?;
        Ok(snapshot)
    }

    fn drain_worker_results(&mut self) {
        for _ in 0..MAX_WORKER_RESULTS_PER_TURN {
            let external = self
                .drain_external_next
                .then(|| self.external.try_result())
                .flatten();
            if let Some(result) = external {
                self.drain_external_next = false;
                self.apply_result(
                    WorkToken {
                        sequence: result.sequence,
                        workspace_revision: self.workspace_revision,
                        document_revision: self.document_revision,
                    },
                    result.value,
                );
                continue;
            }
            if let Some(completion) = self.workers.try_completion() {
                self.drain_external_next = true;
                if completion.token.workspace_revision != self.workspace_revision
                    || completion.token.document_revision != self.document_revision
                {
                    self.stale_results = self.stale_results.saturating_add(1);
                    continue;
                }
                let WorkerOutcome::Completed(result) = completion.outcome else {
                    continue;
                };
                self.apply_result(completion.token, result);
                continue;
            }
            if !self.drain_external_next
                && let Some(result) = self.external.try_result()
            {
                self.drain_external_next = false;
                self.apply_result(
                    WorkToken {
                        sequence: result.sequence,
                        workspace_revision: self.workspace_revision,
                        document_revision: self.document_revision,
                    },
                    result.value,
                );
                continue;
            }
            return;
        }
        if self.workers.snapshot().queued_results() > 0
            || self.external.snapshot().current_items() > 0
        {
            self.workers.wake();
        }
    }

    fn apply_result(&mut self, token: WorkToken, result: D::WorkerOutput) {
        let mut context = AppContext {
            workspace_revision: &mut self.workspace_revision,
            document_revision: &mut self.document_revision,
            dirty: &mut self.dirty,
            workers: &mut self.workers,
            external: &self.external,
            clipboard_write: None,
            close_disposition: None,
            accessibility_response: None,
        };
        self.delegate.worker_result(token, result, &mut context);
    }
}

impl<D: AppDelegate> Drop for Application<D> {
    fn drop(&mut self) {
        self.external.close();
    }
}

#[cfg(test)]
mod tests {
    use std::{
        num::NonZeroUsize,
        sync::{
            atomic::AtomicBool,
            mpsc::{RecvTimeoutError, sync_channel},
        },
        time::Duration,
    };

    use alpine_core::Size;
    use alpine_platform_macos::{
        AccessibilityError, AccessibilityRequest, AccessibilityRequestId, AccessibilityResponse,
        AccessibilityRevision, ClipboardOperation, ClipboardText, EventTimestamp, SurfaceEvent,
        SurfaceExtent,
    };
    use alpine_scene::{SceneBuilder, SceneRevision};

    use super::*;

    static ROLLBACK_WORKER_FINISHED: AtomicBool = AtomicBool::new(false);
    const APPLICATION_INVARIANT: SurfaceError = SurfaceError::InvariantViolation {
        operation: alpine_platform_macos::SurfaceOperation::Application,
    };
    const APPLICATION_RUNTIME_INVARIANT: RuntimeError =
        RuntimeError::Surface(APPLICATION_INVARIANT);

    fn rollback_worker_probe() {
        thread::sleep(Duration::from_millis(100));
        ROLLBACK_WORKER_FINISHED.store(true, Ordering::Release);
    }

    fn spawn_rollback_worker(
        index: usize,
        _requests: Arc<Mutex<Receiver<WorkerRequest<u64>>>>,
        _results: SyncSender<WorkerCompletion<u64>>,
        _counters: Arc<WorkerCounters>,
        _wake: Arc<Mutex<Option<WorkerWake>>>,
    ) -> std::io::Result<JoinHandle<()>> {
        if index == 1 {
            return Err(std::io::Error::other("injected spawn failure"));
        }
        thread::Builder::new()
            .name("alpine-test-worker".to_owned())
            .spawn(rollback_worker_probe)
    }

    #[derive(Default)]
    struct TestDelegate {
        events: Vec<SurfaceEvent>,
        results: Vec<(WorkToken, u64)>,
        accessibility_result_counts: Vec<usize>,
        invalid_scene: bool,
        cancel_close: bool,
        clipboard_write: Option<ClipboardWrite>,
        respond_accessibility: bool,
    }

    impl AppDelegate for TestDelegate {
        type WorkerOutput = u64;

        fn event(&mut self, event: &SurfaceEvent, context: &mut AppContext<'_, u64>) {
            self.events.push(event.clone());
            if matches!(event, SurfaceEvent::Accessibility { .. }) {
                self.accessibility_result_counts.push(self.results.len());
            }
            if let Some(write) = self.clipboard_write.take() {
                assert!(context.write_clipboard(write));
            }
            if self.cancel_close && matches!(event, SurfaceEvent::CloseRequested { .. }) {
                assert!(context.cancel_close());
            }
            if self.respond_accessibility
                && let SurfaceEvent::Accessibility { request, .. } = event
            {
                assert!(
                    context.respond_accessibility(AccessibilityResponse::failure(
                        request,
                        AccessibilityRevision::new(0, 0),
                        AccessibilityError::InvalidTree,
                    ))
                );
            }
        }

        fn worker_result(
            &mut self,
            token: WorkToken,
            result: u64,
            context: &mut AppContext<'_, u64>,
        ) {
            self.results.push((token, result));
            context.invalidate();
        }

        fn frame(&mut self, context: WindowContext) -> Scene {
            let revision = if self.invalid_scene {
                SceneRevision::new(context.scene_revision().get().saturating_add(1))
            } else {
                context.scene_revision()
            };
            SceneBuilder::new(revision, context.viewport()).finish()
        }
    }

    fn runtime<D>(delegate: D) -> Result<Application<D>, RuntimeError>
    where
        D: AppDelegate<WorkerOutput = u64> + 'static,
    {
        let viewport = Size::new(96.0, 64.0).ok_or(RuntimeError::Surface(
            SurfaceError::invariant(alpine_platform_macos::SurfaceOperation::Application),
        ))?;
        let clear = LinearRgba::new(0.0, 0.0, 0.0, 1.0).ok_or(RuntimeError::Surface(
            SurfaceError::invariant(alpine_platform_macos::SurfaceOperation::Application),
        ))?;
        Application::new(
            delegate,
            viewport,
            clear,
            WorkerConfig::new(NonZeroUsize::MIN, NonZeroUsize::MIN, NonZeroUsize::MIN),
        )
    }

    fn token(sequence: u64) -> WorkToken {
        WorkToken {
            sequence,
            workspace_revision: WorkspaceRevision::new(7),
            document_revision: DocumentRevision::new(11),
        }
    }

    #[test]
    fn public_values_and_errors_preserve_exact_identity() {
        let config = WorkerConfig::new(
            NonZeroUsize::new(2).unwrap_or(NonZeroUsize::MIN),
            NonZeroUsize::new(3).unwrap_or(NonZeroUsize::MIN),
            NonZeroUsize::new(4).unwrap_or(NonZeroUsize::MIN),
        );
        assert_eq!(config.worker_count(), 2);
        assert_eq!(config.request_capacity(), 3);
        assert_eq!(config.result_capacity(), 4);
        assert_eq!(WorkerConfig::default().worker_count(), 1);

        let application = ApplicationSnapshot::default();
        assert_eq!(
            application.workspace_revision(),
            WorkspaceRevision::default()
        );
        assert_eq!(application.document_revision(), DocumentRevision::default());
        assert_eq!(application.next_scene_revision(), 1);
        assert!(!application.is_dirty());
        assert!(!application.is_shutting_down());
        assert_eq!(application.stale_results(), 0);
        assert_eq!(application.invalid_scenes(), 0);
        assert_eq!(application.worker(), WorkerSnapshot::default());
        assert_eq!(application.external(), ExternalSnapshot::default());
        let surface = SurfaceSnapshot::empty_for_test();
        let completion = ApplicationCompletion {
            application,
            surface,
        };
        assert_eq!(completion.application(), application);
        assert_eq!(*completion.surface(), surface);

        let work = token(13);
        assert_eq!(work.sequence(), 13);
        assert_eq!(work.workspace_revision().get(), 7);
        assert_eq!(work.document_revision().get(), 11);

        let snapshot = WorkerSnapshot {
            queued_requests: 2,
            peak_queued_requests: 3,
            queued_results: 4,
            peak_queued_results: 5,
            request_saturations: 6,
            dropped_results: 7,
            panicked_jobs: 8,
        };
        assert_eq!(snapshot.queued_requests(), 2);
        assert_eq!(snapshot.peak_queued_requests(), 3);
        assert_eq!(snapshot.queued_results(), 4);
        assert_eq!(snapshot.peak_queued_results(), 5);
        assert_eq!(snapshot.request_saturations(), 6);
        assert_eq!(snapshot.dropped_results(), 7);
        assert_eq!(snapshot.panicked_jobs(), 8);

        let external = ExternalSnapshot {
            current_items: 1,
            peak_items: 2,
            current_bytes: 3,
            peak_bytes: 4,
            admitted: 5,
            full: 6,
            disconnected: 7,
            shutting_down: 8,
            sequence_exhausted: 9,
            wake_requests: 10,
            wake_coalesces: 11,
            drained: 12,
        };
        assert_eq!(external.current_items(), 1);
        assert_eq!(external.peak_items(), 2);
        assert_eq!(external.current_bytes(), 3);
        assert_eq!(external.peak_bytes(), 4);
        assert_eq!(external.admitted(), 5);
        assert_eq!(external.full(), 6);
        assert_eq!(external.disconnected(), 7);
        assert_eq!(external.shutting_down(), 8);
        assert_eq!(external.sequence_exhausted(), 9);
        assert_eq!(external.wake_requests(), 10);
        assert_eq!(external.wake_coalesces(), 11);
        assert_eq!(external.drained(), 12);

        assert_eq!(
            SubmitError::Saturated.to_string(),
            "the bounded worker request queue is full"
        );
        assert_eq!(
            SubmitError::Closed.to_string(),
            "the worker pool no longer accepts requests"
        );
        assert_eq!(
            SubmitError::SequenceExhausted.to_string(),
            "the worker request sequence is exhausted"
        );
        let worker_error = RuntimeError::WorkerSpawn(std::io::Error::other("fault"));
        assert_eq!(
            worker_error.to_string(),
            "failed to create bounded worker: fault"
        );
        assert!(worker_error.source().is_some());
        let surface_error = RuntimeError::from(SurfaceError::UnsupportedPlatform);
        assert!(
            surface_error
                .to_string()
                .starts_with("native application surface failed:")
        );
        assert!(surface_error.source().is_some());
    }

    #[test]
    fn foreground_context_only_accepts_newer_revisions() -> Result<(), RuntimeError> {
        let mut application = runtime(TestDelegate::default())?;
        application.dirty = false;
        let mut clipboard_write = None;
        let mut close_disposition = CloseDisposition::Allow;
        let mut accessibility_response = None;
        let mut context = AppContext {
            workspace_revision: &mut application.workspace_revision,
            document_revision: &mut application.document_revision,
            dirty: &mut application.dirty,
            workers: &mut application.workers,
            external: &application.external,
            clipboard_write: Some(&mut clipboard_write),
            close_disposition: Some(&mut close_disposition),
            accessibility_response: Some(&mut accessibility_response),
        };
        assert_eq!(context.workspace_revision().get(), 0);
        assert_eq!(context.document_revision().get(), 0);
        assert!(context.advance_workspace(WorkspaceRevision::new(2)));
        assert!(!context.advance_workspace(WorkspaceRevision::new(2)));
        assert!(context.advance_document(DocumentRevision::new(3)));
        assert!(!context.advance_document(DocumentRevision::new(1)));
        *context.dirty = false;
        context.invalidate();
        let text = ClipboardText::new("response").map_err(|_| APPLICATION_INVARIANT)?;
        let write = ClipboardWrite::new(ClipboardOperation::Copy, text)
            .map_err(|_| APPLICATION_INVARIANT)?;
        assert!(context.write_clipboard(write.clone()));
        assert!(!context.write_clipboard(write.clone()));
        assert!(context.cancel_close());
        assert!(!context.cancel_close());
        let request = AccessibilityRequest::snapshot(AccessibilityRequestId::new(1))
            .map_err(|_| APPLICATION_RUNTIME_INVARIANT)?;
        let response = AccessibilityResponse::failure(
            &request,
            AccessibilityRevision::new(2, 3),
            AccessibilityError::InvalidTree,
        );
        assert!(context.respond_accessibility(response.clone()));
        assert!(!context.respond_accessibility(response.clone()));
        assert_eq!(context.workspace_revision().get(), 2);
        assert_eq!(context.document_revision().get(), 3);
        let external = context.external_producer();
        assert_eq!(clipboard_write.as_ref(), Some(&write));
        assert_eq!(close_disposition, CloseDisposition::Cancel);
        assert_eq!(accessibility_response.as_ref(), Some(&response));
        assert_eq!(external.submit(17, 4), ExternalAdmission::Admitted);
        let mut no_response_context = AppContext {
            workspace_revision: &mut application.workspace_revision,
            document_revision: &mut application.document_revision,
            dirty: &mut application.dirty,
            workers: &mut application.workers,
            external: &application.external,
            clipboard_write: None,
            close_disposition: None,
            accessibility_response: None,
        };
        assert!(!no_response_context.write_clipboard(write));
        assert!(!no_response_context.cancel_close());
        assert!(!no_response_context.respond_accessibility(response));
        assert!(application.dirty);
        assert_eq!(application.snapshot().external().current_items(), 1);
        Ok(())
    }

    #[test]
    fn current_result_wakes_and_mutates_the_delegate() -> Result<(), RuntimeError> {
        let mut application = runtime(TestDelegate::default())?;
        let (wake_sender, wake_receiver) = sync_channel(1);
        application.set_worker_waker(move || {
            let _ = wake_sender.try_send(());
        });
        let submitted = application.workers.submit(
            WorkspaceRevision::default(),
            DocumentRevision::default(),
            || 55,
        );
        assert!(submitted.is_ok());
        assert_eq!(wake_receiver.recv_timeout(Duration::from_secs(1)), Ok(()));
        let evidence = application.snapshot().worker();
        assert_eq!(evidence.queued_results(), 1);
        assert_eq!(evidence.peak_queued_requests(), 1);
        assert_eq!(evidence.peak_queued_results(), 1);
        let _ = application.dispatch(&SurfaceEvent::Wake {
            timestamp: EventTimestamp::new(4),
        });
        assert_eq!(application.delegate.results.len(), 1);
        assert_eq!(application.delegate.results[0].1, 55);
        assert_eq!(application.snapshot().worker().queued_results(), 0);
        Ok(())
    }

    #[test]
    fn accessibility_query_preserves_complete_projection_until_next_wake()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut application = runtime(TestDelegate {
            respond_accessibility: true,
            ..TestDelegate::default()
        })?;
        assert!(application.frame_if_dirty().is_some());
        let producer = application.external.producer();
        assert_eq!(producer.submit(55, 0), ExternalAdmission::Admitted);
        let request = AccessibilityRequest::snapshot(AccessibilityRequestId::new(17))?;
        let query = application.dispatch_with_response(&SurfaceEvent::Accessibility {
            timestamp: EventTimestamp::new(21),
            request,
        });
        assert!(query.frame().is_none());
        assert!(application.delegate.results.is_empty());
        assert_eq!(application.delegate.accessibility_result_counts, vec![0]);
        assert_eq!(application.snapshot().external().current_items(), 1);
        assert!(!application.snapshot().is_dirty());

        let wake = application.dispatch_with_response(&SurfaceEvent::Wake {
            timestamp: EventTimestamp::new(22),
        });
        assert!(wake.frame().is_some());
        assert_eq!(application.delegate.results.len(), 1);
        assert_eq!(application.delegate.results[0].1, 55);
        assert_eq!(application.snapshot().external().current_items(), 0);
        assert!(!application.snapshot().is_dirty());
        assert!(
            application
                .dispatch_with_response(&SurfaceEvent::Wake {
                    timestamp: EventTimestamp::new(23),
                })
                .frame()
                .is_none()
        );
        Ok(())
    }

    #[test]
    fn accessibility_action_precedes_concurrent_result_and_coalesces_one_frame()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut application = runtime(TestDelegate {
            respond_accessibility: true,
            ..TestDelegate::default()
        })?;
        assert!(application.frame_if_dirty().is_some());
        let producer = application.external.producer();
        assert_eq!(producer.submit(89, 0), ExternalAdmission::Admitted);
        let mut fixture_executions = 0_u8;
        for request in [
            AccessibilityRequest::action(
                AccessibilityRequestId::new(18),
                alpine_platform_macos::AccessibilityAction::activate(
                    AccessibilityRevision::new(0, 0),
                    alpine_platform_macos::AccessibilityNodeId::new(7),
                ),
            ),
            AccessibilityRequest::action(
                AccessibilityRequestId::new(0),
                alpine_platform_macos::AccessibilityAction::activate(
                    AccessibilityRevision::new(0, 0),
                    alpine_platform_macos::AccessibilityNodeId::new(7),
                ),
            ),
        ]
        .into_iter()
        .flatten()
        {
            fixture_executions = fixture_executions.saturating_add(1);
            let action = application.dispatch_with_response(&SurfaceEvent::Accessibility {
                timestamp: EventTimestamp::new(24),
                request,
            });
            assert_eq!(application.delegate.accessibility_result_counts, vec![0]);
            assert_eq!(application.delegate.results.len(), 1);
            assert_eq!(application.delegate.results[0].1, 89);
            assert_eq!(application.snapshot().external().current_items(), 0);
            assert!(action.frame().is_some());
            assert!(!application.snapshot().is_dirty());
            assert!(
                application
                    .dispatch_with_response(&SurfaceEvent::Wake {
                        timestamp: EventTimestamp::new(25),
                    })
                    .frame()
                    .is_none()
            );
        }
        assert_eq!(fixture_executions, 1);
        Ok(())
    }

    #[test]
    fn resize_builds_latest_viewport_and_close_revokes_frames() -> Result<(), RuntimeError> {
        let mut application = runtime(TestDelegate::default())?;
        assert!(!application.snapshot().is_shutting_down());
        assert_eq!(application.snapshot().invalid_scenes(), 0);
        let _ = application.frame_if_dirty();
        let extent = SurfaceExtent::new(120.0, 80.0, 1.0)?;
        let resized = application
            .dispatch(&SurfaceEvent::Resize {
                timestamp: EventTimestamp::new(5),
                extent,
            })
            .ok_or(RuntimeError::Surface(SurfaceError::invariant(
                alpine_platform_macos::SurfaceOperation::Application,
            )))?;
        assert_eq!(
            resized.scene().viewport(),
            Size::new(120.0, 80.0).unwrap_or_default()
        );
        assert!(
            application
                .dispatch(&SurfaceEvent::CloseRequested {
                    timestamp: EventTimestamp::new(6),
                })
                .is_none()
        );
        let snapshot = application.snapshot();
        assert_eq!(snapshot.workspace_revision().get(), 0);
        assert_eq!(snapshot.document_revision().get(), 0);
        assert_eq!(snapshot.stale_results(), 0);
        assert!(snapshot.is_shutting_down());
        assert!(!snapshot.is_dirty());
        assert!(
            application
                .dispatch(&SurfaceEvent::Wake {
                    timestamp: EventTimestamp::new(7),
                })
                .is_none()
        );
        Ok(())
    }

    #[test]
    fn cancelled_close_returns_one_bounded_response_and_stays_live() -> Result<(), RuntimeError> {
        let text = ClipboardText::new("selected").map_err(|_| APPLICATION_INVARIANT)?;
        let write = ClipboardWrite::new(ClipboardOperation::Cut, text)
            .map_err(|_| APPLICATION_INVARIANT)?;
        let delegate = TestDelegate {
            cancel_close: true,
            clipboard_write: Some(write.clone()),
            ..TestDelegate::default()
        };
        let mut application = runtime(delegate)?;
        let _ = application.frame_if_dirty();

        let cancelled = application.dispatch_with_response(&SurfaceEvent::CloseRequested {
            timestamp: EventTimestamp::new(8),
        });
        assert_eq!(cancelled.frame(), None);
        assert_eq!(cancelled.clipboard_write(), Some(&write));
        assert_eq!(cancelled.close_disposition(), CloseDisposition::Cancel);
        assert!(!application.snapshot().is_shutting_down());

        application.delegate.cancel_close = false;
        let allowed = application.dispatch_with_response(&SurfaceEvent::CloseRequested {
            timestamp: EventTimestamp::new(9),
        });
        assert_eq!(allowed.into_parts(), (None, None, CloseDisposition::Allow));
        assert!(application.snapshot().is_shutting_down());
        Ok(())
    }

    #[test]
    fn exhausted_scene_and_work_sequences_are_rejected() -> Result<(), RuntimeError> {
        let mut application = runtime(TestDelegate::default())?;
        application.scene_revision = u64::MAX;
        assert!(application.frame_if_dirty().is_none());
        assert_eq!(application.snapshot().invalid_scenes(), 1);
        application.workers.next_sequence = u64::MAX;
        assert_eq!(
            application.workers.submit(
                WorkspaceRevision::default(),
                DocumentRevision::default(),
                u64::default,
            ),
            Err(SubmitError::SequenceExhausted)
        );
        Ok(())
    }

    #[test]
    fn spawn_failure_rolls_back_started_workers() {
        ROLLBACK_WORKER_FINISHED.store(false, Ordering::Release);
        let config = WorkerConfig::new(
            NonZeroUsize::new(2).unwrap_or(NonZeroUsize::MIN),
            NonZeroUsize::MIN,
            NonZeroUsize::MIN,
        );
        let result = WorkerPool::<u64>::new_with_spawner(config, &mut spawn_rollback_worker);
        assert!(
            matches!(result, Err(RuntimeError::WorkerSpawn(error)) if error.kind() == std::io::ErrorKind::Other)
        );
        assert!(ROLLBACK_WORKER_FINISHED.load(Ordering::Acquire));
    }

    #[test]
    fn disconnected_and_shutdown_pools_reject_admission() -> Result<(), RuntimeError> {
        let (request_sender, request_receiver) = sync_channel(1);
        drop(request_receiver);
        let (result_sender, result_receiver) = sync_channel(1);
        drop(result_sender);
        let mut disconnected = WorkerPool::<u64> {
            request_sender: Some(request_sender),
            result_receiver: Some(result_receiver),
            workers: Vec::new(),
            counters: Arc::new(WorkerCounters::default()),
            next_sequence: 0,
            wake: Arc::new(Mutex::new(None)),
        };
        assert_eq!(
            disconnected.submit(
                WorkspaceRevision::default(),
                DocumentRevision::default(),
                u64::default
            ),
            Err(SubmitError::Closed)
        );

        let mut shutdown = WorkerPool::<u64>::new(WorkerConfig::default())?;
        shutdown.workers.push(thread::spawn(|| {
            std::panic::resume_unwind(Box::new("injected worker teardown panic"));
        }));
        shutdown.shutdown();
        assert_eq!(shutdown.snapshot().panicked_jobs(), 1);
        assert_eq!(
            shutdown.submit(
                WorkspaceRevision::default(),
                DocumentRevision::default(),
                u64::default
            ),
            Err(SubmitError::Closed)
        );
        Ok(())
    }

    #[test]
    fn drop_waits_for_admitted_work_to_finish() -> Result<(), RuntimeError> {
        let (started_sender, started_receiver) = sync_channel(1);
        let (release_sender, release_receiver) = sync_channel(1);
        let mut pool = WorkerPool::<u64>::new(WorkerConfig::default())?;
        let submitted = pool.submit(
            WorkspaceRevision::default(),
            DocumentRevision::default(),
            move || {
                let _ = started_sender.send(());
                let _ = release_receiver.recv();
                1
            },
        );
        assert!(submitted.is_ok());
        assert_eq!(
            started_receiver.recv_timeout(Duration::from_secs(1)),
            Ok(())
        );
        let (done_sender, done_receiver) = sync_channel(1);
        let dropper = thread::spawn(move || {
            drop(pool);
            let _ = done_sender.send(());
        });
        assert_eq!(
            done_receiver.recv_timeout(Duration::from_millis(20)),
            Err(RecvTimeoutError::Timeout)
        );
        assert_eq!(release_sender.send(()), Ok(()));
        assert_eq!(done_receiver.recv_timeout(Duration::from_secs(1)), Ok(()));
        assert!(dropper.join().is_ok());
        Ok(())
    }

    #[test]
    fn worker_loop_backpressures_results_and_counts_disconnect_and_poison()
    -> Result<(), Box<dyn std::error::Error>> {
        let (request_sender, request_receiver) = sync_channel(1);
        let (result_sender, result_receiver) = sync_channel(0);
        let (started_sender, started_receiver) = sync_channel(1);
        let (done_sender, done_receiver) = sync_channel(1);
        let counters = Arc::new(WorkerCounters::default());
        counters.queued_requests.store(1, Ordering::Release);
        assert!(
            request_sender
                .try_send(WorkerRequest {
                    token: token(2),
                    job: Box::new(move || {
                        let _ = started_sender.send(());
                        2
                    }),
                })
                .is_ok()
        );
        drop(request_sender);
        let worker_counters = Arc::clone(&counters);
        let worker = thread::spawn(move || {
            worker_loop(
                &Mutex::new(request_receiver),
                &result_sender,
                &worker_counters,
                &Mutex::new(None),
            );
            let _ = done_sender.send(());
        });
        assert_eq!(
            started_receiver.recv_timeout(Duration::from_secs(1)),
            Ok(())
        );
        assert_eq!(
            done_receiver.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        );
        let completion = result_receiver.recv_timeout(Duration::from_secs(1))?;
        counters.queued_results.fetch_sub(1, Ordering::AcqRel);
        assert_eq!(completion.token.sequence(), 2);
        assert!(matches!(completion.outcome, WorkerOutcome::Completed(2)));
        assert_eq!(done_receiver.recv_timeout(Duration::from_secs(1)), Ok(()));
        assert!(worker.join().is_ok());
        assert_eq!(counters.dropped_results.load(Ordering::Acquire), 0);
        assert_eq!(counters.queued_results.load(Ordering::Acquire), 0);
        assert_eq!(counters.peak_queued_results.load(Ordering::Acquire), 1);

        let (request_sender, request_receiver) = sync_channel(1);
        let (result_sender, result_receiver) = sync_channel(1);
        let disconnected = WorkerCounters::default();
        disconnected.queued_requests.store(1, Ordering::Release);
        assert!(
            request_sender
                .try_send(WorkerRequest {
                    token: token(3),
                    job: Box::new(|| 3),
                })
                .is_ok()
        );
        drop(request_sender);
        drop(result_receiver);
        let wake: Mutex<Option<WorkerWake>> = Mutex::new(None);
        worker_loop(
            &Mutex::new(request_receiver),
            &result_sender,
            &disconnected,
            &wake,
        );
        assert_eq!(disconnected.dropped_results.load(Ordering::Acquire), 1);
        assert_eq!(disconnected.queued_results.load(Ordering::Acquire), 0);

        let (_sender, receiver) = sync_channel::<WorkerRequest<u64>>(1);
        let poisoned = Mutex::new(receiver);
        let fault = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = poisoned
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            std::panic::resume_unwind(Box::new("poison request receiver"));
        }));
        assert!(fault.is_err());
        let (results, _results_receiver) = sync_channel(1);
        worker_loop(&poisoned, &results, &WorkerCounters::default(), &wake);
        Ok(())
    }

    struct DefaultResultDelegate;

    impl AppDelegate for DefaultResultDelegate {
        type WorkerOutput = u64;

        fn event(&mut self, _event: &SurfaceEvent, _context: &mut AppContext<'_, u64>) {}

        fn frame(&mut self, context: WindowContext) -> Scene {
            SceneBuilder::new(context.scene_revision(), context.viewport()).finish()
        }
    }

    #[test]
    fn default_worker_result_is_a_noop() -> Result<(), RuntimeError> {
        let viewport = Size::new(10.0, 10.0).ok_or(RuntimeError::Surface(
            SurfaceError::invariant(alpine_platform_macos::SurfaceOperation::Application),
        ))?;
        let clear = LinearRgba::new(0.0, 0.0, 0.0, 1.0).ok_or(RuntimeError::Surface(
            SurfaceError::invariant(alpine_platform_macos::SurfaceOperation::Application),
        ))?;
        let config = WorkerConfig::default();
        let mut application = Application::new(DefaultResultDelegate, viewport, clear, config)?;
        assert!(
            application
                .dispatch(&SurfaceEvent::Wake {
                    timestamp: EventTimestamp::new(8),
                })
                .is_some()
        );
        assert!(!application.dirty);
        let mut context = AppContext {
            workspace_revision: &mut application.workspace_revision,
            document_revision: &mut application.document_revision,
            dirty: &mut application.dirty,
            workers: &mut application.workers,
            external: &application.external,
            clipboard_write: None,
            close_disposition: None,
            accessibility_response: None,
        };
        AppDelegate::worker_result(&mut application.delegate, token(3), 9, &mut context);
        assert!(!application.dirty);
        Ok(())
    }

    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    #[test]
    fn application_run_rejects_unsupported_host() -> Result<(), SurfaceError> {
        let descriptor = SurfaceDescriptor::new("Alpine", 10.0, 10.0, 1.0)?;
        let application = runtime(TestDelegate::default()).map_err(|_| APPLICATION_INVARIANT)?;
        assert!(matches!(
            application.run(&descriptor),
            Err(RuntimeError::Surface(SurfaceError::UnsupportedPlatform))
        ));
        let application = runtime(TestDelegate::default()).map_err(|_| APPLICATION_INVARIANT)?;
        assert!(matches!(
            application.run_with_completion(&descriptor),
            Err(RuntimeError::Surface(SurfaceError::UnsupportedPlatform))
        ));
        Ok(())
    }

    #[test]
    fn clean_events_do_not_build_or_submit_another_frame() -> Result<(), RuntimeError> {
        let mut application = runtime(TestDelegate::default())?;
        let first =
            application
                .frame_if_dirty()
                .ok_or(RuntimeError::Surface(SurfaceError::invariant(
                    alpine_platform_macos::SurfaceOperation::Application,
                )))?;
        assert_eq!(first.scene().revision(), SceneRevision::new(1));
        assert!(
            application
                .dispatch(&SurfaceEvent::Wake {
                    timestamp: EventTimestamp::new(1),
                })
                .is_none()
        );
        assert!(!application.snapshot().is_dirty());
        assert_eq!(application.snapshot().next_scene_revision(), 2);
        Ok(())
    }

    #[test]
    fn mismatched_delegate_scene_is_rejected_and_remains_dirty() -> Result<(), RuntimeError> {
        let mut application = runtime(TestDelegate {
            invalid_scene: true,
            ..TestDelegate::default()
        })?;
        assert!(application.frame_if_dirty().is_none());
        assert!(application.snapshot().is_dirty());
        assert_eq!(application.snapshot().invalid_scenes(), 1);
        Ok(())
    }

    #[test]
    fn stale_background_result_never_mutates_delegate_state() -> Result<(), RuntimeError> {
        let mut application = runtime(TestDelegate::default())?;
        let (started_sender, started_receiver) = sync_channel(1);
        let (release_sender, release_receiver) = sync_channel(1);
        let (wake_sender, wake_receiver) = sync_channel(1);
        application.set_worker_waker(move || {
            let _ = wake_sender.try_send(());
        });
        {
            let mut context = AppContext {
                workspace_revision: &mut application.workspace_revision,
                document_revision: &mut application.document_revision,
                dirty: &mut application.dirty,
                workers: &mut application.workers,
                external: &application.external,
                clipboard_write: None,
                close_disposition: None,
                accessibility_response: None,
            };
            let submitted = context.spawn(move || {
                let _ = started_sender.send(());
                let _ = release_receiver.recv();
                89
            });
            assert!(submitted.is_ok());
            assert!(context.advance_document(DocumentRevision::new(1)));
        }
        assert_eq!(
            started_receiver.recv_timeout(Duration::from_secs(1)),
            Ok(())
        );
        assert_eq!(release_sender.send(()), Ok(()));
        assert_eq!(wake_receiver.recv_timeout(Duration::from_secs(1)), Ok(()));
        let _ = application.dispatch(&SurfaceEvent::Wake {
            timestamp: EventTimestamp::new(2),
        });
        assert_eq!(application.snapshot().stale_results(), 1);
        assert!(application.delegate.results.is_empty());
        Ok(())
    }

    #[test]
    fn saturated_submission_returns_without_waiting() -> Result<(), RuntimeError> {
        let mut application = runtime(TestDelegate::default())?;
        let (started_sender, started_receiver) = sync_channel(1);
        let (release_sender, release_receiver) = sync_channel(1);
        let first = application.workers.submit(
            WorkspaceRevision::default(),
            DocumentRevision::default(),
            move || {
                let _ = started_sender.send(());
                let _ = release_receiver.recv();
                1
            },
        );
        assert!(first.is_ok());
        assert_eq!(
            started_receiver.recv_timeout(Duration::from_secs(1)),
            Ok(())
        );
        assert!(
            application
                .workers
                .submit(
                    WorkspaceRevision::default(),
                    DocumentRevision::default(),
                    u64::default,
                )
                .is_ok()
        );
        assert_eq!(
            application.workers.submit(
                WorkspaceRevision::default(),
                DocumentRevision::default(),
                u64::default,
            ),
            Err(SubmitError::Saturated)
        );
        assert!(application.snapshot().worker().request_saturations() > 0);
        assert_eq!(release_sender.send(()), Ok(()));
        Ok(())
    }

    #[test]
    fn worker_panic_is_contained_and_counted() -> Result<(), RuntimeError> {
        let mut application = runtime(TestDelegate::default())?;
        let (wake_sender, wake_receiver) = sync_channel(1);
        application.set_worker_waker(move || {
            let _ = wake_sender.try_send(());
        });
        let submitted = application.workers.submit(
            WorkspaceRevision::default(),
            DocumentRevision::default(),
            || std::panic::resume_unwind(Box::new("fault")),
        );
        assert!(submitted.is_ok());
        assert_eq!(wake_receiver.recv_timeout(Duration::from_secs(1)), Ok(()));
        assert_eq!(application.snapshot().worker().panicked_jobs(), 1);
        let _ = application.dispatch(&SurfaceEvent::Wake {
            timestamp: EventTimestamp::new(3),
        });
        assert!(application.delegate.results.is_empty());
        Ok(())
    }

    #[test]
    fn foreground_drain_is_bounded_and_reschedules_remaining_results() -> Result<(), RuntimeError> {
        let mut application = runtime(TestDelegate::default())?;
        let (request_sender, _request_receiver) = sync_channel(1);
        let (result_sender, result_receiver) = sync_channel(16);
        let counters = Arc::new(WorkerCounters::default());
        let wakes = Arc::new(AtomicUsize::new(0));
        let wake_counter = Arc::clone(&wakes);
        application.workers = WorkerPool {
            request_sender: Some(request_sender),
            result_receiver: Some(result_receiver),
            workers: Vec::new(),
            counters: Arc::clone(&counters),
            next_sequence: 0,
            wake: Arc::new(Mutex::new(Some(Arc::new(move || {
                wake_counter.fetch_add(1, Ordering::Relaxed);
            })))),
        };
        for sequence in 1..=8 {
            assert!(
                result_sender
                    .send(WorkerCompletion {
                        token: WorkToken {
                            sequence,
                            workspace_revision: WorkspaceRevision::default(),
                            document_revision: DocumentRevision::default(),
                        },
                        outcome: WorkerOutcome::Completed(sequence),
                    })
                    .is_ok()
            );
        }
        counters.queued_results.store(8, Ordering::Release);
        counters.peak_queued_results.store(8, Ordering::Release);
        let _ = application.frame_if_dirty();

        let exact_budget = application.dispatch(&SurfaceEvent::Wake {
            timestamp: EventTimestamp::new(10),
        });
        assert!(exact_budget.is_some());
        assert_eq!(application.delegate.results.len(), 8);
        assert_eq!(application.snapshot().worker().queued_results(), 0);
        assert_eq!(wakes.load(Ordering::Acquire), 0);

        for sequence in 9..=17 {
            assert!(
                result_sender
                    .send(WorkerCompletion {
                        token: WorkToken {
                            sequence,
                            workspace_revision: WorkspaceRevision::default(),
                            document_revision: DocumentRevision::default(),
                        },
                        outcome: WorkerOutcome::Completed(sequence),
                    })
                    .is_ok()
            );
        }
        counters.queued_results.store(9, Ordering::Release);
        counters.peak_queued_results.store(9, Ordering::Release);

        let over_budget = application.dispatch(&SurfaceEvent::Wake {
            timestamp: EventTimestamp::new(11),
        });
        assert!(over_budget.is_some());
        assert_eq!(application.delegate.results.len(), 16);
        assert_eq!(application.snapshot().worker().queued_results(), 1);
        assert_eq!(wakes.load(Ordering::Acquire), 1);

        let remainder = application.dispatch(&SurfaceEvent::Wake {
            timestamp: EventTimestamp::new(12),
        });
        assert!(remainder.is_some());
        assert_eq!(application.delegate.results.len(), 17);
        assert_eq!(application.snapshot().worker().queued_results(), 0);
        assert_eq!(wakes.load(Ordering::Acquire), 1);
        Ok(())
    }

    #[test]
    fn external_results_cross_runtime_revisions_and_keep_exact_accounting()
    -> Result<(), RuntimeError> {
        let mut application = runtime(TestDelegate::default())?;
        let _ = application.frame_if_dirty();
        let (wake_sender, wake_receiver) = sync_channel(1);
        application.set_worker_waker(move || {
            let _ = wake_sender.try_send(());
        });
        let producer = application.external.producer().clone();
        application.document_revision = DocumentRevision::new(9);
        assert_eq!(producer.submit(55, 13), ExternalAdmission::Admitted);
        assert_eq!(wake_receiver.recv_timeout(Duration::from_secs(1)), Ok(()));
        let queued = application.snapshot().external();
        assert_eq!(queued.current_items(), 1);
        assert_eq!(queued.current_bytes(), 13);
        assert_eq!(queued.wake_requests(), 1);

        assert!(
            application
                .dispatch(&SurfaceEvent::Wake {
                    timestamp: EventTimestamp::new(20),
                })
                .is_some()
        );
        let (token, result) = application.delegate.results[0];
        assert_eq!(result, 55);
        assert_eq!(token.document_revision(), DocumentRevision::new(9));
        assert!(token.sequence() >= FIRST_EXTERNAL_SEQUENCE);
        assert_eq!(application.snapshot().stale_results(), 0);
        let drained = application.snapshot().external();
        assert_eq!(drained.current_items(), 0);
        assert_eq!(drained.current_bytes(), 0);
        assert_eq!(drained.peak_items(), 1);
        assert_eq!(drained.peak_bytes(), 13);
        assert_eq!(drained.admitted(), 1);
        assert_eq!(drained.drained(), 1);
        Ok(())
    }

    #[test]
    fn external_queue_coalesces_saturates_and_continues_under_one_drain_budget()
    -> Result<(), RuntimeError> {
        let mut application = runtime(TestDelegate::default())?;
        let _ = application.frame_if_dirty();
        let wakes = Arc::new(AtomicUsize::new(0));
        let wake_counter = Arc::clone(&wakes);
        application.set_worker_waker(move || {
            wake_counter.fetch_add(1, Ordering::Relaxed);
        });
        let producer = application.external.producer();
        for value in 0..EXTERNAL_RESULT_CAPACITY {
            assert_eq!(
                producer.submit(value as u64, 1),
                ExternalAdmission::Admitted
            );
        }
        assert_eq!(producer.submit(99, 1), ExternalAdmission::Full);
        let full = application.snapshot().external();
        assert_eq!(full.current_items(), EXTERNAL_RESULT_CAPACITY);
        assert_eq!(full.current_bytes(), EXTERNAL_RESULT_CAPACITY);
        assert_eq!(full.peak_items(), EXTERNAL_RESULT_CAPACITY);
        assert_eq!(full.wake_requests(), 1);
        assert_eq!(full.wake_coalesces(), EXTERNAL_RESULT_CAPACITY - 1);
        assert_eq!(full.full(), 1);
        assert_eq!(wakes.load(Ordering::Acquire), 1);

        assert!(
            application
                .dispatch(&SurfaceEvent::Wake {
                    timestamp: EventTimestamp::new(21),
                })
                .is_some()
        );
        assert_eq!(
            application.delegate.results.len(),
            MAX_WORKER_RESULTS_PER_TURN
        );
        assert_eq!(application.snapshot().external().current_items(), 8);
        assert_eq!(wakes.load(Ordering::Acquire), 2);
        assert!(
            application
                .dispatch(&SurfaceEvent::Wake {
                    timestamp: EventTimestamp::new(22),
                })
                .is_some()
        );
        assert_eq!(application.delegate.results.len(), EXTERNAL_RESULT_CAPACITY);
        assert_eq!(application.snapshot().external().current_items(), 0);
        assert_eq!(
            application.snapshot().external().drained(),
            EXTERNAL_RESULT_CAPACITY
        );
        Ok(())
    }

    #[test]
    fn external_result_without_delegate_invalidation_requests_no_frame() -> Result<(), RuntimeError>
    {
        let mut application = runtime(DefaultResultDelegate)?;
        let _ = application.frame_if_dirty();
        let producer = application.external.producer();
        assert_eq!(producer.submit(1, 0), ExternalAdmission::Admitted);
        assert!(
            application
                .dispatch(&SurfaceEvent::Wake {
                    timestamp: EventTimestamp::new(23),
                })
                .is_none()
        );
        assert!(!application.snapshot().is_dirty());
        Ok(())
    }

    #[test]
    fn external_producer_rejects_budget_exhaustion_shutdown_and_poison() -> Result<(), RuntimeError>
    {
        let application = runtime(TestDelegate::default())?;
        let producer = application.external.producer();
        assert_eq!(
            producer.submit(1, MAX_EXTERNAL_RETAINED_BYTES + 1),
            ExternalAdmission::Full
        );
        assert_eq!(application.snapshot().external().current_items(), 0);
        drop(application);
        assert_eq!(producer.submit(2, 1), ExternalAdmission::ShuttingDown);

        let poisoned_application = runtime(TestDelegate::default())?;
        let poisoned = poisoned_application.external.producer();
        let shared = Arc::clone(&poisoned.shared);
        let fault = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = shared
                .sender
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            std::panic::resume_unwind(Box::new("poison external sender"));
        }));
        assert!(fault.is_err());
        assert_eq!(poisoned.submit(3, 1), ExternalAdmission::Disconnected);

        let contended_application = runtime(TestDelegate::default())?;
        let contended = contended_application.external.producer();
        let contended_shared = Arc::clone(&contended.shared);
        let contended_guard = contended_shared
            .sender
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_eq!(contended.submit(4, 1), ExternalAdmission::Full);
        drop(contended_guard);

        let missing_application = runtime(TestDelegate::default())?;
        let missing = missing_application.external.producer();
        let removed_sender = missing
            .shared
            .sender
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        assert!(removed_sender.is_some());
        assert_eq!(missing.submit(5, 1), ExternalAdmission::Disconnected);

        let (disconnected_sender, disconnected_receiver) = sync_channel(1);
        drop(disconnected_receiver);
        let disconnected = ExternalProducer {
            shared: Arc::new(ExternalShared {
                sender: Mutex::new(Some(disconnected_sender)),
                shutting_down: AtomicBool::new(false),
                next_sequence: AtomicU64::new(FIRST_EXTERNAL_SEQUENCE),
                counters: ExternalCounters::default(),
                wake: Arc::new(Mutex::new(None)),
            }),
        };
        assert_eq!(disconnected.submit(6, 1), ExternalAdmission::Disconnected);
        assert_eq!(
            disconnected
                .shared
                .counters
                .current_items
                .load(Ordering::Acquire),
            0
        );
        assert_eq!(
            disconnected
                .shared
                .counters
                .current_bytes
                .load(Ordering::Acquire),
            0
        );
        Ok(())
    }

    #[test]
    fn external_sequence_exhaustion_fails_before_queue_mutation() -> Result<(), RuntimeError> {
        let application = runtime(TestDelegate::default())?;
        let producer = application.external.producer();
        producer
            .shared
            .next_sequence
            .store(u64::MAX, Ordering::Release);
        assert_eq!(producer.submit(1, 1), ExternalAdmission::SequenceExhausted);
        let snapshot = application.snapshot().external();
        assert_eq!(snapshot.sequence_exhausted(), 1);
        assert_eq!(snapshot.current_items(), 0);
        Ok(())
    }
}
