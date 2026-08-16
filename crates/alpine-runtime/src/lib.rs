//! Bounded single-window application runtime for Alpine Studio.

use core::{error::Error, fmt, num::NonZeroUsize};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc::{Receiver, SyncSender, TryRecvError, TrySendError, sync_channel},
    },
    thread::{self, JoinHandle},
};

use alpine_core::{LinearRgba, Size};
use alpine_platform_macos::{SurfaceDescriptor, SurfaceError, SurfaceEvent, SurfaceFrame};
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

    /// Returns completed results omitted by the result bound.
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
    result_receiver: Receiver<WorkerCompletion<T>>,
    workers: Vec<JoinHandle<()>>,
    counters: Arc<WorkerCounters>,
    next_sequence: u64,
    wake: Arc<Mutex<Option<WorkerWake>>>,
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
            result_receiver,
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
        match self.result_receiver.try_recv() {
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
        match results.try_send(WorkerCompletion {
            token: request.token,
            outcome,
        }) {
            Ok(()) => {
                if let Ok(installed) = wake.lock()
                    && let Some(wake) = installed.as_ref()
                {
                    wake();
                }
            }
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
                counters.queued_results.fetch_sub(1, Ordering::AcqRel);
                counters.dropped_results.fetch_add(1, Ordering::Relaxed);
            }
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
}

/// One single-window foreground state graph and its bounded workers.
pub struct Application<D: AppDelegate> {
    delegate: D,
    workers: WorkerPool<D::WorkerOutput>,
    workspace_revision: WorkspaceRevision,
    document_revision: DocumentRevision,
    scene_revision: u64,
    viewport: Size,
    clear: LinearRgba,
    dirty: bool,
    shutting_down: bool,
    stale_results: usize,
    invalid_scenes: usize,
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
        Ok(Self {
            delegate,
            workers: WorkerPool::new(worker_config)?,
            workspace_revision: WorkspaceRevision::default(),
            document_revision: DocumentRevision::default(),
            scene_revision: 0,
            viewport,
            clear,
            dirty: true,
            shutting_down: false,
            stale_results: 0,
            invalid_scenes: 0,
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
        if self.shutting_down {
            return None;
        }
        self.drain_worker_results();
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
            };
            self.delegate.event(event, &mut context);
        }
        if matches!(event, SurfaceEvent::CloseRequested { .. }) {
            self.shutting_down = true;
            self.dirty = false;
            return None;
        }
        self.frame_if_dirty()
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
        }
    }

    /// Owns one native surface and runs until its production close boundary.
    ///
    /// # Errors
    ///
    /// Returns a structured worker or native surface failure.
    pub fn run(self, descriptor: &SurfaceDescriptor) -> Result<(), RuntimeError> {
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        {
            let mut application = self;
            let surface = NativeSurface::new(descriptor)?;
            if let Some(frame) = application.frame_if_dirty() {
                let (scene, clear) = frame.into_parts();
                let _revision = surface.request_frame(scene, clear)?;
            }
            surface.show()?;

            let state = Rc::new(RefCell::new(application));
            let callback_state = Rc::clone(&state);
            let run_result = surface.run_with_event_handler(move |event| {
                callback_state
                    .try_borrow_mut()
                    .ok()
                    .and_then(|mut application| application.dispatch(&event))
            });
            if let Ok(mut application) = state.try_borrow_mut() {
                application.shutting_down = true;
                application.dirty = false;
                application.workers.shutdown();
            }
            run_result.map_err(RuntimeError::Surface)
        }

        #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
        {
            let _ = descriptor;
            Err(RuntimeError::Surface(SurfaceError::UnsupportedPlatform))
        }
    }

    fn drain_worker_results(&mut self) {
        while let Some(completion) = self.workers.try_completion() {
            if completion.token.workspace_revision != self.workspace_revision
                || completion.token.document_revision != self.document_revision
            {
                self.stale_results = self.stale_results.saturating_add(1);
                continue;
            }
            let WorkerOutcome::Completed(result) = completion.outcome else {
                continue;
            };
            let mut context = AppContext {
                workspace_revision: &mut self.workspace_revision,
                document_revision: &mut self.document_revision,
                dirty: &mut self.dirty,
                workers: &mut self.workers,
            };
            self.delegate
                .worker_result(completion.token, result, &mut context);
        }
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
    use alpine_platform_macos::{EventTimestamp, SurfaceEvent, SurfaceExtent};
    use alpine_scene::{SceneBuilder, SceneRevision};

    use super::*;

    static ROLLBACK_WORKER_FINISHED: AtomicBool = AtomicBool::new(false);

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
        invalid_scene: bool,
    }

    impl AppDelegate for TestDelegate {
        type WorkerOutput = u64;

        fn event(&mut self, event: &SurfaceEvent, _context: &mut AppContext<'_, u64>) {
            self.events.push(event.clone());
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

    fn runtime(delegate: TestDelegate) -> Result<Application<TestDelegate>, RuntimeError> {
        let viewport =
            Size::new(96.0, 64.0).ok_or(RuntimeError::Surface(SurfaceError::DriverUnavailable))?;
        let clear = LinearRgba::new(0.0, 0.0, 0.0, 1.0)
            .ok_or(RuntimeError::Surface(SurfaceError::DriverUnavailable))?;
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
        let mut context = AppContext {
            workspace_revision: &mut application.workspace_revision,
            document_revision: &mut application.document_revision,
            dirty: &mut application.dirty,
            workers: &mut application.workers,
        };
        assert_eq!(context.workspace_revision().get(), 0);
        assert_eq!(context.document_revision().get(), 0);
        assert!(context.advance_workspace(WorkspaceRevision::new(2)));
        assert!(!context.advance_workspace(WorkspaceRevision::new(2)));
        assert!(context.advance_document(DocumentRevision::new(3)));
        assert!(!context.advance_document(DocumentRevision::new(1)));
        *context.dirty = false;
        context.invalidate();
        assert_eq!(context.workspace_revision().get(), 2);
        assert_eq!(context.document_revision().get(), 3);
        assert!(application.dirty);
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
            .ok_or(RuntimeError::Surface(SurfaceError::DriverUnavailable))?;
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
            result_receiver,
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
    fn worker_loop_counts_bounded_result_omission_and_poison() {
        let (request_sender, request_receiver) = sync_channel(1);
        let (result_sender, result_receiver) = sync_channel(1);
        let counters = WorkerCounters::default();
        counters.queued_requests.store(1, Ordering::Release);
        let wake: Mutex<Option<WorkerWake>> = Mutex::new(None);
        let occupied = WorkerCompletion {
            token: token(1),
            outcome: WorkerOutcome::Completed(1_u64),
        };
        assert!(result_sender.try_send(occupied).is_ok());
        assert!(
            request_sender
                .try_send(WorkerRequest {
                    token: token(2),
                    job: Box::new(|| 2),
                })
                .is_ok()
        );
        drop(request_sender);
        worker_loop(
            &Mutex::new(request_receiver),
            &result_sender,
            &counters,
            &wake,
        );
        assert_eq!(counters.dropped_results.load(Ordering::Acquire), 1);
        assert_eq!(counters.queued_results.load(Ordering::Acquire), 0);
        drop(result_receiver);

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
        let viewport =
            Size::new(10.0, 10.0).ok_or(RuntimeError::Surface(SurfaceError::DriverUnavailable))?;
        let clear = LinearRgba::new(0.0, 0.0, 0.0, 1.0)
            .ok_or(RuntimeError::Surface(SurfaceError::DriverUnavailable))?;
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
        };
        AppDelegate::worker_result(&mut application.delegate, token(3), 9, &mut context);
        assert!(!application.dirty);
        Ok(())
    }

    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    #[test]
    fn application_run_rejects_unsupported_host() -> Result<(), SurfaceError> {
        let descriptor = SurfaceDescriptor::new("Alpine", 10.0, 10.0, 1.0)?;
        let application =
            runtime(TestDelegate::default()).map_err(|_| SurfaceError::DriverUnavailable)?;
        assert!(matches!(
            application.run(&descriptor),
            Err(RuntimeError::Surface(SurfaceError::UnsupportedPlatform))
        ));
        Ok(())
    }

    #[test]
    fn clean_events_do_not_build_or_submit_another_frame() -> Result<(), RuntimeError> {
        let mut application = runtime(TestDelegate::default())?;
        let first = application
            .frame_if_dirty()
            .ok_or(RuntimeError::Surface(SurfaceError::DriverUnavailable))?;
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
}
