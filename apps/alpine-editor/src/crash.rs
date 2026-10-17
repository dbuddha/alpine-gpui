//! Panic hook: flushes recovery and writes a crash log before an abort. It
//! cannot reach the editor, so it flushes the last published snapshot; only a
//! selection-only change since that publish can be lost.

use std::{
    any::Any,
    ffi::OsString,
    fmt::Write as _,
    io::{self, Write},
    panic::{self, PanicHookInfo},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use crate::{
    logs,
    recovery::{self, FlushHandle, FlushOutcome},
};

const KIND: &str = "crash";
const MAX_MESSAGE_BYTES: usize = 4_096;
const FLUSH_BOUND: Duration = Duration::from_secs(2);
// The flush bound plus the backtrace and log write of the panic in flight.
const WAIT_BOUND: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_millis(5);

/// The shared state of the installed panic hook.
pub(crate) struct CrashHook {
    state: Arc<State>,
}

impl CrashHook {
    /// Chains a hook that flushes recovery, then writes a crash log under
    /// `~/Library/Logs/Alpine Editor/`, before the previous hook runs.
    pub(crate) fn install(home: Option<OsString>) -> Self {
        let state = Arc::new(State::new(
            home,
            logs::bundle_identity_path(),
            FLUSH_BOUND,
            WAIT_BOUND,
        ));
        let previous = panic::take_hook();
        let hook_state = Arc::clone(&state);
        panic::set_hook(Box::new(move |info| {
            // Data first: a full stderr pipe could stall the previous hook.
            let recorded = hook_state.record(&CrashReport::from_hook(info));
            previous(info);
            let mut stderr = io::stderr();
            // A failed write must not become a second panic inside the hook.
            let _ = match recorded {
                Some(Ok(path)) => writeln!(stderr, "alpine-editor: crash log: {}", path.display()),
                Some(Err(error)) => writeln!(stderr, "alpine-editor: no crash log: {error}"),
                None => Ok(()),
            };
        }));
        Self { state }
    }

    /// Lets the hook flush this journal once a panic happens.
    pub(crate) fn attach_recovery(&self, handle: FlushHandle) {
        *self
            .state
            .recovery
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(handle);
    }
}

struct CrashReport {
    message: String,
    location: String,
    thread: String,
}

impl CrashReport {
    fn from_hook(info: &PanicHookInfo<'_>) -> Self {
        Self {
            message: message_of(info.payload()),
            location: info
                .location()
                .map_or_else(|| String::from("<unknown>"), ToString::to_string),
            thread: thread::current().name().unwrap_or("<unnamed>").to_owned(),
        }
    }
}

/// The payload's text, copied only up to the message cap.
fn message_of(payload: &(dyn Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .map_or_else(
            || String::from("<non-string panic payload>"),
            |message| logs::truncated(message, MAX_MESSAGE_BYTES).to_owned(),
        )
}

struct State {
    home: Option<OsString>,
    identity: Option<PathBuf>,
    recovery: Mutex<Option<FlushHandle>>,
    running: AtomicBool,
    flush_bound: Duration,
    wait_bound: Duration,
}

// Clears the reentrancy flag when the hook finishes.
struct Entry<'a>(&'a AtomicBool);

impl Drop for Entry<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl State {
    const fn new(
        home: Option<OsString>,
        identity: Option<PathBuf>,
        flush_bound: Duration,
        wait_bound: Duration,
    ) -> Self {
        Self {
            home,
            identity,
            recovery: Mutex::new(None),
            running: AtomicBool::new(false),
            flush_bound,
            wait_bound,
        }
    }

    /// Flushes recovery, then writes the log. A panic that arrives while this
    /// runs only waits for it, so the abort that follows cannot cut it short.
    fn record(&self, report: &CrashReport) -> Option<io::Result<PathBuf>> {
        let Some(_entry) = self.enter() else {
            self.wait_for_owner();
            return None;
        };
        let recovery = self.flush_recovery();
        Some(self.write_log(report, &recovery))
    }

    fn enter(&self) -> Option<Entry<'_>> {
        self.running
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| Entry(&self.running))
    }

    fn wait_for_owner(&self) {
        let deadline = Instant::now() + self.wait_bound;
        while self.running.load(Ordering::Acquire) && Instant::now() < deadline {
            thread::sleep(POLL);
        }
    }

    fn flush_recovery(&self) -> String {
        let handle = recovery::try_lock(&self.recovery).and_then(|handle| handle.clone());
        match handle.map(|handle| handle.flush(self.flush_bound)) {
            None => String::from("no journal attached"),
            Some(FlushOutcome::Durable) => String::from("already durable"),
            Some(FlushOutcome::Written(generation)) => format!("flushed generation {generation}"),
            Some(FlushOutcome::TimedOut) => {
                format!("not flushed, busy for {:?}", self.flush_bound)
            }
            Some(FlushOutcome::Failed(error)) => format!("flush failed: {error}"),
        }
    }

    fn write_log(&self, report: &CrashReport, recovery: &str) -> io::Result<PathBuf> {
        let directory = logs::directory(self.home.clone())?;
        let unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_secs());
        let text = render(
            unix,
            report,
            recovery,
            &logs::read_identity(self.identity.as_deref()),
            &std::backtrace::Backtrace::force_capture().to_string(),
        );
        let path = write_new(&directory, unix, std::process::id(), &text)?;
        logs::prune(&directory, KIND, logs::KEPT_LOGS, &path);
        Ok(path)
    }
}

fn render(
    unix: u64,
    report: &CrashReport,
    recovery: &str,
    identity: &str,
    backtrace: &str,
) -> String {
    let mut text = String::new();
    let _ = write!(
        text,
        "alpine-editor crash log\ntime_unix: {unix}\npid: {}\nthread: {}\nlocation: {}\nmessage: {}\nrecovery: {recovery}\nbuild identity:\n{identity}\nbacktrace:\n{backtrace}\n",
        std::process::id(),
        report.thread,
        report.location,
        logs::truncated(&report.message, MAX_MESSAGE_BYTES),
    );
    logs::cap(&mut text, logs::MAX_LOG_BYTES);
    text
}

/// Creates `crash-<unix>-<pid>.log`, never replacing an existing file.
fn write_new(directory: &Path, unix: u64, pid: u32, text: &str) -> io::Result<PathBuf> {
    let (path, mut file) = logs::create(directory, KIND, unix, pid)?;
    file.write_all(text.as_bytes())?;
    file.sync_all()?;
    Ok(path)
}

#[cfg(test)]
#[path = "crash_tests.rs"]
mod tests;
