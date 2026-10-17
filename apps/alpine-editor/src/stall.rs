//! Main-thread stall log. Each dispatch stores its start in one atomic word,
//! and a watchdog thread, parked while the editor is idle, logs any dispatch
//! that runs past the threshold. It touches these atomics, never editor state.

use std::{
    ffi::OsString,
    fmt::Write as _,
    fs::File,
    io::{self, Read, Write},
    path::PathBuf,
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        Arc,
        atomic::{
            AtomicBool, AtomicU64,
            Ordering::{Relaxed, SeqCst},
        },
        mpsc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use alpine_platform_macos::SurfaceEvent;

use crate::{EditorWorkerOutput, logs};

const KIND: &str = "stall";
const THRESHOLD: Duration = Duration::from_secs(1);
// Under the threshold, so timer slack cannot let a stall end between wakes.
const INTERVAL: Duration = Duration::from_millis(900);
const MIN_GAP: Duration = Duration::from_secs(30);
const SAMPLER: &str = "/usr/bin/sample";
const SAMPLE_BOUND: Duration = Duration::from_secs(10);
const SAMPLE_READ_BYTES: u64 = 1 << 20;
const POLL: Duration = Duration::from_millis(20);
// Room kept under the cap for the closing duration line.
const END_RESERVE: usize = 64;

// Dispatch word: start micros << 10 | kind << 2 | WATCHED | RUNNING.
const RUNNING: u64 = 1;
const WATCHED: u64 = 1 << 1;
const KIND_SHIFT: u32 = 2;
const START_SHIFT: u32 = 10;
const MAX_MICROS: u64 = u64::MAX >> START_SHIFT;

/// What a main-thread dispatch was doing, by variant name only.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Kind {
    Keyboard = 1,
    Pointer,
    Scroll,
    Focus,
    Ime,
    Clipboard,
    Resize,
    Accessibility,
    Wake,
    CloseRequested,
    Menu,
    Frame,
    WorkerFind,
    WorkerQuickOpen,
    WorkerProjectSearch,
    WorkerFileTree,
    WorkerSettings,
    WorkerLanguage,
    WorkerWorkspaceEdit,
    WorkerWorkspaceEditPublication,
}

const KINDS: [Kind; 20] = [
    Kind::Keyboard,
    Kind::Pointer,
    Kind::Scroll,
    Kind::Focus,
    Kind::Ime,
    Kind::Clipboard,
    Kind::Resize,
    Kind::Accessibility,
    Kind::Wake,
    Kind::CloseRequested,
    Kind::Menu,
    Kind::Frame,
    Kind::WorkerFind,
    Kind::WorkerQuickOpen,
    Kind::WorkerProjectSearch,
    Kind::WorkerFileTree,
    Kind::WorkerSettings,
    Kind::WorkerLanguage,
    Kind::WorkerWorkspaceEdit,
    Kind::WorkerWorkspaceEditPublication,
];

impl Kind {
    pub(crate) const fn of_event(event: &SurfaceEvent) -> Self {
        match event {
            SurfaceEvent::Keyboard { .. } => Self::Keyboard,
            SurfaceEvent::Pointer { .. } => Self::Pointer,
            SurfaceEvent::Scroll { .. } => Self::Scroll,
            SurfaceEvent::Focus { .. } => Self::Focus,
            SurfaceEvent::Ime { .. } => Self::Ime,
            SurfaceEvent::Clipboard { .. } => Self::Clipboard,
            SurfaceEvent::Resize { .. } => Self::Resize,
            SurfaceEvent::Accessibility { .. } => Self::Accessibility,
            SurfaceEvent::Wake { .. } => Self::Wake,
            SurfaceEvent::CloseRequested { .. } => Self::CloseRequested,
            SurfaceEvent::Menu { .. } => Self::Menu,
        }
    }

    pub(crate) const fn of_worker(output: &EditorWorkerOutput) -> Self {
        match output {
            EditorWorkerOutput::Find(_) => Self::WorkerFind,
            EditorWorkerOutput::QuickOpen(_) => Self::WorkerQuickOpen,
            EditorWorkerOutput::ProjectSearch(_) => Self::WorkerProjectSearch,
            EditorWorkerOutput::FileTree(_) => Self::WorkerFileTree,
            EditorWorkerOutput::Settings(_) => Self::WorkerSettings,
            EditorWorkerOutput::Language(_) => Self::WorkerLanguage,
            EditorWorkerOutput::WorkspaceEdit(_) => Self::WorkerWorkspaceEdit,
            EditorWorkerOutput::WorkspaceEditPublication(_) => Self::WorkerWorkspaceEditPublication,
        }
    }

    fn name(word: u64) -> String {
        let code = (word >> KIND_SHIFT) & 0xff;
        KINDS
            .into_iter()
            .find(|kind| *kind as u64 == code)
            .map_or_else(|| String::from("unknown"), |kind| format!("{kind:?}"))
    }
}

/// Where the watchdog logs and how long it waits; tests shorten the times.
pub(crate) struct Config {
    home: Option<OsString>,
    identity: Option<PathBuf>,
    threshold: Duration,
    interval: Duration,
    min_gap: Duration,
    // Program and leading arguments; the pid and duration follow.
    sampler: Vec<OsString>,
    sample_bound: Duration,
}

impl Config {
    pub(crate) fn new(home: Option<OsString>) -> Self {
        Self {
            home,
            identity: logs::bundle_identity_path(),
            threshold: THRESHOLD,
            interval: INTERVAL,
            min_gap: MIN_GAP,
            sampler: vec![OsString::from(SAMPLER)],
            sample_bound: SAMPLE_BOUND,
        }
    }
}

struct Shared {
    epoch: Instant,
    dispatch: AtomicU64,
    // End micros plus one of the watched dispatch, 0 until it ends.
    ended: AtomicU64,
    parked: AtomicBool,
    stop: AtomicBool,
    wakeups: AtomicU64,
    stalls: AtomicU64,
}

impl Shared {
    fn new() -> Self {
        Self {
            epoch: Instant::now(),
            dispatch: AtomicU64::new(0),
            ended: AtomicU64::new(0),
            parked: AtomicBool::new(true),
            stop: AtomicBool::new(false),
            wakeups: AtomicU64::new(0),
            stalls: AtomicU64::new(0),
        }
    }

    fn micros(&self) -> u64 {
        u64::try_from(self.epoch.elapsed().as_micros()).map_or(MAX_MICROS, |m| m.min(MAX_MICROS))
    }
}

fn start_of(word: u64) -> Duration {
    Duration::from_micros(word >> START_SHIFT)
}

/// The main thread's end of the watchdog; the default watches nothing.
#[derive(Default)]
pub(crate) struct Watch {
    inner: Option<Inner>,
}

struct Inner {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
    word: u64,
}

impl Watch {
    /// Starts the watchdog thread, parked until the first dispatch.
    pub(crate) fn start(config: Config) -> io::Result<Self> {
        let shared = Arc::new(Shared::new());
        let watched = Arc::clone(&shared);
        let thread = thread::Builder::new()
            .name(String::from("alpine-stall-watch"))
            .spawn(move || Watcher::new(&watched, &config).run())?;
        Ok(Self {
            inner: Some(Inner {
                shared,
                thread: Some(thread),
                word: 0,
            }),
        })
    }

    /// Marks a dispatch's start: a clock read, a store and a flag load.
    pub(crate) fn begin(&mut self, kind: Kind) {
        let Some(inner) = self.inner.as_mut() else {
            return;
        };
        let shared = &inner.shared;
        inner.word = (shared.micros() << START_SHIFT) | ((kind as u64) << KIND_SHIFT) | RUNNING;
        shared.dispatch.store(inner.word, SeqCst);
        if shared.parked.load(SeqCst)
            && shared.parked.swap(false, SeqCst)
            && let Some(thread) = inner.thread.as_ref()
        {
            thread.thread().unpark();
        }
    }

    /// Marks the dispatch's end; reads the clock only if the watchdog asked.
    pub(crate) fn end(&mut self) {
        let Some(inner) = self.inner.as_ref() else {
            return;
        };
        let shared = &inner.shared;
        if shared.dispatch.swap(inner.word & !RUNNING, SeqCst) & WATCHED != 0 {
            shared.ended.store(shared.micros() + 1, SeqCst);
        }
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        let Some(inner) = self.inner.as_mut() else {
            return;
        };
        inner.shared.stop.store(true, SeqCst);
        if let Some(thread) = inner.thread.take() {
            thread.thread().unpark();
            let _ = thread.join();
        }
    }
}

enum Next {
    Park,
    Sleep(Duration),
}

enum State {
    Watching,
    Logged(Record),
    Suppressed,
}

/// The one dispatch whose word carries WATCHED, so its end time is stored.
struct Pending {
    word: u64,
    state: State,
}

struct Watcher<'a> {
    shared: &'a Shared,
    config: &'a Config,
    seen: u64,
    pending: Option<Pending>,
    last_log: Option<Instant>,
}

impl<'a> Watcher<'a> {
    const fn new(shared: &'a Shared, config: &'a Config) -> Self {
        Self {
            shared,
            config,
            seen: 0,
            pending: None,
            last_log: None,
        }
    }

    fn run(mut self) {
        while !self.shared.stop.load(SeqCst) {
            match self.step() {
                Next::Park => self.park(),
                Next::Sleep(duration) => {
                    thread::park_timeout(duration);
                    self.shared.wakeups.fetch_add(1, Relaxed);
                }
            }
        }
        self.resolve(self.shared.dispatch.load(SeqCst));
    }

    fn step(&mut self) -> Next {
        let word = self.shared.dispatch.load(SeqCst);
        if !self.resolve(word) {
            return Next::Sleep(POLL);
        }
        let now = self.shared.epoch.elapsed();
        let threshold = self.config.threshold;
        let pending = self
            .pending
            .as_ref()
            .map(|pending| (pending.word, matches!(pending.state, State::Watching)));
        if let Some((pending, watching)) = pending {
            let elapsed = now.saturating_sub(start_of(pending));
            if watching && elapsed >= threshold {
                let state = self.detect(pending, elapsed);
                self.pending = Some(Pending {
                    word: pending,
                    state,
                });
            }
        } else if word & RUNNING != 0 {
            let elapsed = now.saturating_sub(start_of(word));
            let watched =
                self.shared
                    .dispatch
                    .compare_exchange(word, word | WATCHED, SeqCst, SeqCst);
            if watched.is_err() {
                // It ended after the load, so its end time is about now.
                if elapsed >= threshold {
                    self.log_ended(word, elapsed);
                }
            } else {
                let state = if elapsed >= threshold {
                    self.detect(word, elapsed)
                } else {
                    State::Watching
                };
                self.pending = Some(Pending { word, state });
            }
        }
        let next = match self.pending.as_ref() {
            Some(Pending {
                word,
                state: State::Watching,
            }) => {
                let due = start_of(*word) + threshold;
                Next::Sleep(due.saturating_sub(now).min(self.config.interval))
            }
            None if word == self.seen => Next::Park,
            _ => Next::Sleep(self.config.interval),
        };
        self.seen = word;
        next
    }

    /// Parks until a dispatch begins. Either `begin` sees the flag or the
    /// recheck sees the dispatch, so no wakeup is lost.
    fn park(&self) {
        let shared = self.shared;
        shared.parked.store(true, SeqCst);
        if shared.dispatch.load(SeqCst) != self.seen {
            // Withdraw the flag, or take the unpark `begin` already sent.
            if !shared.parked.swap(false, SeqCst) {
                thread::park();
                shared.wakeups.fetch_add(1, Relaxed);
            }
            return;
        }
        while shared.parked.load(SeqCst) && !shared.stop.load(SeqCst) {
            thread::park();
            shared.wakeups.fetch_add(1, Relaxed);
        }
    }

    /// Closes the pending dispatch once it has ended. False while its end
    /// time is not yet published.
    fn resolve(&mut self, word: u64) -> bool {
        let Some(pending) = self.pending.as_ref() else {
            return true;
        };
        if word == pending.word | WATCHED {
            return true;
        }
        let ended = self.shared.ended.swap(0, SeqCst);
        if ended == 0 {
            return false;
        }
        let duration = Duration::from_micros(ended - 1).saturating_sub(start_of(pending.word));
        match self
            .pending
            .take()
            .map(|pending| (pending.word, pending.state))
        {
            Some((_, State::Logged(mut record))) => record.finish(duration),
            Some((word, State::Watching)) if duration >= self.config.threshold => {
                self.log_ended(word, duration);
            }
            _ => {}
        }
        true
    }

    /// Logs a stall still running, with its stack, unless rate-limited.
    fn detect(&mut self, word: u64, elapsed: Duration) -> State {
        self.shared.stalls.fetch_add(1, Relaxed);
        if !self.admit() {
            return State::Suppressed;
        }
        let Ok(mut record) = self.record(word, elapsed) else {
            return State::Suppressed;
        };
        let stack = match capture_stack(
            &self.config.sampler,
            self.config.sample_bound,
            &self.shared.stop,
        ) {
            Ok(stack) => format!("main thread stack:\n{stack}"),
            Err(reason) => format!("main thread stack: unavailable ({reason})\n"),
        };
        let _ = record.append(&stack);
        State::Logged(record)
    }

    /// Logs a stall that ended before the watchdog saw it pass the threshold.
    fn log_ended(&mut self, word: u64, duration: Duration) {
        self.shared.stalls.fetch_add(1, Relaxed);
        if self.admit()
            && let Ok(mut record) = self.record(word, duration)
        {
            let _ = record.append("main thread stack: not captured, it had ended\n");
            record.finish(duration);
        }
    }

    fn record(&self, word: u64, elapsed: Duration) -> io::Result<Record> {
        let age = self.shared.epoch.elapsed().saturating_sub(start_of(word));
        Record::create(self.config, word, age, elapsed)
    }

    /// At most one log per `min_gap`.
    fn admit(&mut self) -> bool {
        let now = Instant::now();
        if self
            .last_log
            .is_some_and(|last| now.duration_since(last) < self.config.min_gap)
        {
            return false;
        }
        self.last_log = Some(now);
        true
    }
}

/// One open stall log, capped at the crash log's size.
struct Record {
    file: File,
    written: usize,
}

impl Record {
    /// `age` is the time since the dispatch began, read just now.
    fn create(config: &Config, word: u64, age: Duration, elapsed: Duration) -> io::Result<Self> {
        let directory = logs::directory(config.home.clone())?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let pid = std::process::id();
        let (path, file) = logs::create(&directory, KIND, now.as_secs(), pid)?;
        logs::prune(&directory, KIND, logs::KEPT_LOGS, &path);
        let mut header = String::new();
        let _ = write!(
            header,
            "alpine-editor stall log\npid: {pid}\ndispatch: {}\nstarted_unix_ms: {}\nthreshold_ms: {}\nelapsed_at_detection_ms: {}\nbuild identity:\n{}",
            Kind::name(word),
            now.saturating_sub(age).as_millis(),
            config.threshold.as_millis(),
            elapsed.as_millis(),
            logs::read_identity(config.identity.as_deref()),
        );
        let mut record = Self { file, written: 0 };
        record.append(&header)?;
        Ok(record)
    }

    /// Appends within the cap, keeping room for the duration line.
    fn append(&mut self, text: &str) -> io::Result<()> {
        let room = (logs::MAX_LOG_BYTES - END_RESERVE).saturating_sub(self.written);
        if room <= logs::TRUNCATED.len() {
            return Ok(());
        }
        // Cut the borrowed text first, so a long stack is never copied.
        let (head, marker) = if text.len() > room {
            let limit = room.saturating_sub(logs::TRUNCATED.len());
            (logs::truncated(text, limit), logs::TRUNCATED)
        } else {
            (text, "")
        };
        self.file.write_all(head.as_bytes())?;
        self.file.write_all(marker.as_bytes())?;
        self.file.sync_all()?;
        self.written += head.len() + marker.len();
        Ok(())
    }

    fn finish(&mut self, duration: Duration) {
        let line = format!("duration_ms: {}\n", duration.as_millis());
        let _ = self
            .file
            .write_all(line.as_bytes())
            .and_then(|()| self.file.sync_all());
    }
}

/// Samples this process for one second and returns the main thread's call
/// graph. The sampler is killed at the bound or on stop; output is capped.
fn capture_stack(
    sampler: &[OsString],
    bound: Duration,
    stop: &AtomicBool,
) -> Result<String, String> {
    let (program, leading) = sampler
        .split_first()
        .ok_or_else(|| String::from("no sampler"))?;
    let mut child = Command::new(program)
        .args(leading)
        .args([&std::process::id().to_string(), "1", "-file", "/dev/stdout"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("spawn failed: {:?}", error.kind()))?;
    let deadline = Instant::now() + bound;
    let (sender, receiver) = mpsc::sync_channel(1);
    let reader = child.stdout.take().map(|mut output| {
        thread::Builder::new()
            .name(String::from("alpine-stall-sample"))
            .spawn(move || {
                let mut bytes = Vec::new();
                let _ = (&mut output)
                    .take(SAMPLE_READ_BYTES)
                    .read_to_end(&mut bytes);
                // Drain the rest so a long report cannot block the sampler.
                let _ = io::copy(&mut output, &mut io::sink());
                let _ = sender.send(bytes);
            })
    });
    if !matches!(reader, Some(Ok(_))) {
        let _ = finish(&mut child, Instant::now(), stop);
        return Err(String::from("no reader thread"));
    }
    let outcome = loop {
        match receiver.recv_timeout(POLL) {
            Ok(bytes) => break Ok(bytes),
            Err(mpsc::RecvTimeoutError::Disconnected) => break Err(String::from("no output")),
            Err(mpsc::RecvTimeoutError::Timeout)
                if stop.load(SeqCst) || Instant::now() >= deadline =>
            {
                break Err(cut_short(bound, stop));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    };
    let status = finish(&mut child, deadline, stop);
    let bytes = outcome?;
    match status {
        Some(status) if status.success() => {}
        Some(status) => return Err(format!("sampler failed: {status}")),
        None => return Err(cut_short(bound, stop)),
    }
    main_thread_stack(&String::from_utf8_lossy(&bytes))
        .ok_or_else(|| String::from("no main thread in the report"))
}

fn cut_short(bound: Duration, stop: &AtomicBool) -> String {
    if stop.load(SeqCst) {
        String::from("stopped")
    } else {
        format!("timed out after {} ms", bound.as_millis())
    }
}

/// Waits for the sampler until the deadline or a stop, then kills it.
fn finish(child: &mut Child, deadline: Instant, stop: &AtomicBool) -> Option<ExitStatus> {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) if !stop.load(SeqCst) && Instant::now() < deadline => thread::sleep(POLL),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

/// The main thread's section of a `sample` call graph: no header, no paths.
fn main_thread_stack(report: &str) -> Option<String> {
    let mut lines = report
        .lines()
        .skip_while(|line| !(is_thread(line) && line.contains("com.apple.main-thread")));
    let mut stack = format!("{}\n", lines.next()?);
    for line in lines.take_while(|line| !line.trim().is_empty() && !is_thread(line)) {
        stack.push_str(line);
        stack.push('\n');
    }
    Some(stack)
}

fn is_thread(line: &str) -> bool {
    line.trim_start()
        .trim_start_matches(|c: char| c.is_ascii_digit())
        .trim_start()
        .starts_with("Thread_")
}

#[cfg(test)]
#[path = "stall_tests.rs"]
mod tests;
