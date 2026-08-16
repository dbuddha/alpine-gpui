//! Bounded single-window application runtime for Alpine Studio.

use core::{error::Error, fmt, num::NonZeroUsize};
use std::{
    cell::RefCell,
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc::{Receiver, SyncSender, TryRecvError, TrySendError, sync_channel},
    },
    thread::{self, JoinHandle},
};

use alpine_core::{LinearRgba, Size};
use alpine_platform_macos::{
    NativeSurface, SurfaceDescriptor, SurfaceError, SurfaceEvent, SurfaceFrame,
};
use alpine_scene::{Scene, SceneRevision};

type WorkerJob<T> = Box<dyn FnOnce() -> T + Send + 'static>;
type WorkerWake = Arc<dyn Fn() + Send + Sync + 'static>;

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
    wake: Arc<Mutex<WorkerWake>>,
}

impl<T: Send + 'static> WorkerPool<T> {
    fn new(config: WorkerConfig) -> Result<Self, RuntimeError> {
        let (request_sender, request_receiver) = sync_channel(config.request_capacity());
        let (result_sender, result_receiver) = sync_channel(config.result_capacity());
        let request_receiver = Arc::new(Mutex::new(request_receiver));
        let counters = Arc::new(WorkerCounters::default());
        let wake: Arc<Mutex<WorkerWake>> = Arc::new(Mutex::new(Arc::new(|| {})));
        let mut workers = Vec::with_capacity(config.worker_count());

        for index in 0..config.worker_count() {
            let requests = Arc::clone(&request_receiver);
            let results = result_sender.clone();
            let worker_counters = Arc::clone(&counters);
            let worker_wake = Arc::clone(&wake);
            let spawn = thread::Builder::new()
                .name(format!("alpine-worker-{index}"))
                .spawn(move || worker_loop(&requests, &results, &worker_counters, &worker_wake));
            match spawn {
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
            *installed = wake;
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
    wake: &Mutex<WorkerWake>,
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
                if let Ok(wake) = wake.lock() {
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
    let mut observed = peak.load(Ordering::Relaxed);
    while candidate > observed {
        match peak.compare_exchange_weak(observed, candidate, Ordering::Relaxed, Ordering::Relaxed)
        {
            Ok(_) => return,
            Err(actual) => observed = actual,
        }
    }
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
    pub fn run(mut self, descriptor: &SurfaceDescriptor) -> Result<(), RuntimeError> {
        let surface = NativeSurface::new(descriptor)?;
        if let Some(frame) = self.frame_if_dirty() {
            let (scene, clear) = frame.into_parts();
            let _revision = surface.request_frame(scene, clear)?;
        }
        surface.show()?;

        let state = Rc::new(RefCell::new(self));
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
    use std::{num::NonZeroUsize, sync::Barrier, time::Duration};

    use alpine_core::Size;
    use alpine_platform_macos::{EventTimestamp, SurfaceEvent};
    use alpine_scene::{SceneBuilder, SceneRevision};

    use super::*;

    #[derive(Default)]
    struct TestDelegate {
        events: Vec<SurfaceEvent>,
        results: Vec<(WorkToken, u64)>,
        submit_on_wake: bool,
        invalid_scene: bool,
    }

    impl AppDelegate for TestDelegate {
        type WorkerOutput = u64;

        fn event(&mut self, event: &SurfaceEvent, context: &mut AppContext<'_, u64>) {
            self.events.push(event.clone());
            if self.submit_on_wake && matches!(event, SurfaceEvent::Wake { .. }) {
                self.submit_on_wake = false;
                let _ = context.spawn(|| 41);
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
        let barrier = Arc::new(Barrier::new(2));
        let mut application = runtime(TestDelegate::default())?;
        let worker_barrier = Arc::clone(&barrier);
        {
            let mut context = AppContext {
                workspace_revision: &mut application.workspace_revision,
                document_revision: &mut application.document_revision,
                dirty: &mut application.dirty,
                workers: &mut application.workers,
            };
            context
                .spawn(move || {
                    worker_barrier.wait();
                    89
                })
                .map_err(|_| RuntimeError::Surface(SurfaceError::DriverUnavailable))?;
            assert!(context.advance_document(DocumentRevision::new(1)));
        }
        barrier.wait();
        for _ in 0..100 {
            if application.snapshot().worker().queued_results() > 0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        let _ = application.dispatch(&SurfaceEvent::Wake {
            timestamp: EventTimestamp::new(2),
        });
        assert_eq!(application.snapshot().stale_results(), 1);
        assert!(application.delegate.results.is_empty());
        Ok(())
    }

    #[test]
    fn saturated_submission_returns_without_waiting() -> Result<(), RuntimeError> {
        let barrier = Arc::new(Barrier::new(2));
        let mut application = runtime(TestDelegate::default())?;
        let worker_barrier = Arc::clone(&barrier);
        let first = application.workers.submit(
            WorkspaceRevision::default(),
            DocumentRevision::default(),
            move || {
                worker_barrier.wait();
                1
            },
        );
        assert!(first.is_ok());
        let mut saturated = false;
        for value in 2..=4 {
            if application.workers.submit(
                WorkspaceRevision::default(),
                DocumentRevision::default(),
                move || value,
            ) == Err(SubmitError::Saturated)
            {
                saturated = true;
                break;
            }
        }
        assert!(saturated);
        assert!(application.snapshot().worker().request_saturations() > 0);
        barrier.wait();
        Ok(())
    }

    #[test]
    fn worker_panic_is_contained_and_counted() -> Result<(), RuntimeError> {
        let mut application = runtime(TestDelegate::default())?;
        application
            .workers
            .submit(
                WorkspaceRevision::default(),
                DocumentRevision::default(),
                || std::panic::resume_unwind(Box::new("fault")),
            )
            .map_err(|_| RuntimeError::Surface(SurfaceError::DriverUnavailable))?;
        for _ in 0..100 {
            if application.snapshot().worker().panicked_jobs() > 0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(application.snapshot().worker().panicked_jobs(), 1);
        let _ = application.dispatch(&SurfaceEvent::Wake {
            timestamp: EventTimestamp::new(3),
        });
        assert!(application.delegate.results.is_empty());
        Ok(())
    }
}
