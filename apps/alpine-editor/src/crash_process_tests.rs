//! Process-level crash journeys: a child of this test binary edits a document
//! and then panics or is killed; the parent inspects what it left on disk.
//! No window or `AppKit` is involved.

use std::{
    fs::{self, File},
    io::Write,
    os::unix::process::ExitStatusExt,
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant},
};

use super::{
    EditorApp,
    crash::CrashHook,
    recovery::{self, RecoveryCoordinator},
    tests::TestTextSystem,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const MODE: &str = "ALPINE_CRASH_JOURNEY_MODE";
const ROOT: &str = "ALPINE_CRASH_JOURNEY_ROOT";
const TIMEOUT: Duration = Duration::from_secs(30);
const SIGABRT: i32 = 6;
const SIGKILL: i32 = 9;

static SEQUENCE: AtomicU64 = AtomicU64::new(1);

fn journal(root: &Path) -> PathBuf {
    recovery::path_for_session(&root.join("state/session-v1.bin"))
}

fn crash_logs(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(root.join("home/Library/Logs/Alpine Editor")) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension().is_some_and(|extension| extension == "log")
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("crash-"))
        })
        .collect()
}

/// The child half. It does nothing unless a parent test set its environment.
#[test]
fn journey_child() -> TestResult {
    let Some(mode) = std::env::var_os(MODE) else {
        return Ok(());
    };
    let root = PathBuf::from(std::env::var_os(ROOT).ok_or("child root")?);
    match mode.to_str() {
        Some("panic") => panic_journey(&root, false),
        Some("panic-no-writer") => panic_journey(&root, true),
        Some("writer-panic") => writer_panic_journey(&root),
        Some("kill") => kill_journey(&root),
        Some("relaunch") => relaunch_journey(&root),
        _ => Err("unknown journey".into()),
    }
}

fn edited_app(root: &Path) -> Result<EditorApp, Box<dyn std::error::Error>> {
    let mut app = EditorApp::open_file(TestTextSystem, root.join("document.rs"))?;
    app.configure_persistence(root.join("state/session-v1.bin"))
        .map_err(|error| error.to_string())?;
    Ok(app)
}

#[allow(
    clippy::panic,
    reason = "the journey needs a real panic to reach the hook"
)]
fn panic_journey(root: &Path, without_writer: bool) -> TestResult {
    let mut app = edited_app(root)?;
    // With no writer thread, only the hook's flush can put the edit on disk.
    let _signal = without_writer.then(|| {
        let (coordinator, signal) = RecoveryCoordinator::without_worker(journal(root));
        app.recovery = Some(coordinator);
        signal
    });
    let hook = CrashHook::install(std::env::var_os("HOME"));
    hook.attach_recovery(app.recovery.as_ref().ok_or("recovery")?.flush_handle());
    assert!(app.replace_range(0..4, "edited").document_changed);
    app.publish_recovery();
    let worker = thread::Builder::new()
        .name(String::from("alpine-worker-probe"))
        .spawn(|| std::panic::panic_any("journey panic marker"))?;
    let _ = worker.join();
    // A release build has aborted inside the hook by now; a test build
    // unwinds, so end the process the way an abort would.
    std::process::abort();
}

fn writer_panic_journey(root: &Path) -> TestResult {
    let mut app = edited_app(root)?;
    let hook = CrashHook::install(std::env::var_os("HOME"));
    let coordinator = app.recovery.as_ref().ok_or("recovery")?;
    hook.attach_recovery(coordinator.flush_handle());
    wait_until_durable(coordinator)?;
    coordinator.panic_on_next_write();
    assert!(app.replace_range(0..4, "edited").document_changed);
    app.publish_recovery();
    // The join returns once the writer's panic, and so the hook, has run.
    let _ = app.recovery.as_mut().ok_or("recovery")?.shutdown();
    std::process::abort();
}

fn wait_until_durable(coordinator: &RecoveryCoordinator) -> TestResult {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let status = coordinator.status();
        if status.completed_generation >= status.published_generation {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("the recovery writer never caught up".into());
        }
        thread::sleep(Duration::from_millis(5));
    }
}

fn kill_journey(root: &Path) -> TestResult {
    let mut app = edited_app(root)?;
    assert!(app.replace_range(0..4, "edited").document_changed);
    app.publish_recovery();
    wait_until_durable(app.recovery.as_ref().ok_or("recovery")?)?;
    println!("journey-ready");
    std::io::stdout().flush()?;
    thread::sleep(TIMEOUT);
    Err("the parent never killed this child".into())
}

fn relaunch_journey(root: &Path) -> TestResult {
    let state = recovery::load(&journal(root))?;
    let app = EditorApp::from_recovery(TestTextSystem, state).map_err(|error| error.to_string())?;
    println!("journey-recovered:{}", app.buffer().snapshot().text());
    Ok(())
}

struct Run {
    child: Child,
    stdout: PathBuf,
    stderr: PathBuf,
}

fn run_child(mode: &str, root: &Path) -> Result<Run, Box<dyn std::error::Error>> {
    let stdout = root.join(format!("{mode}.stdout"));
    let stderr = root.join(format!("{mode}.stderr"));
    let child = Command::new(std::env::current_exe()?)
        .args([
            "--exact",
            "crash_process_tests::journey_child",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(MODE, mode)
        .env(ROOT, root)
        .env("HOME", root.join("home"))
        .env_remove("RUST_BACKTRACE")
        .stdout(File::create(&stdout)?)
        .stderr(File::create(&stderr)?)
        .spawn()?;
    Ok(Run {
        child,
        stdout,
        stderr,
    })
}

impl Run {
    fn wait(&mut self) -> Result<ExitStatus, Box<dyn std::error::Error>> {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if let Some(status) = self.child.try_wait()? {
                return Ok(status);
            }
            if Instant::now() >= deadline {
                self.child.kill()?;
                self.child.wait()?;
                return Err(format!("child timed out; stderr: {}", self.stderr()).into());
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn stdout(&self) -> String {
        fs::read_to_string(&self.stdout).unwrap_or_default()
    }

    fn stderr(&self) -> String {
        fs::read_to_string(&self.stderr).unwrap_or_default()
    }
}

fn journey_root(label: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let root = fs::canonicalize(std::env::temp_dir())?.join(format!(
        "alpine-crash-journey-{label}-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(root.join("home"))?;
    fs::write(root.join("document.rs"), "base")?;
    Ok(root)
}

const PROBE: Panicked = Panicked {
    thread: "alpine-worker-probe",
    message: "journey panic marker",
};

struct Panicked {
    thread: &'static str,
    message: &'static str,
}

#[test]
fn worker_panic_leaves_a_crash_log_and_the_edit_on_disk() -> TestResult {
    let root = journey_root("panic")?;
    let mut run = run_child("panic", &root)?;
    assert_panic_journey(&root, &mut run, &PROBE, |flushed| {
        flushed == "already durable" || flushed.starts_with("flushed generation ")
    })
}

#[test]
fn worker_panic_flushes_an_edit_the_writer_never_wrote() -> TestResult {
    let root = journey_root("panic-no-writer")?;
    let mut run = run_child("panic-no-writer", &root)?;
    assert_panic_journey(&root, &mut run, &PROBE, |flushed| {
        flushed == "flushed generation 1"
    })
}

#[test]
fn recovery_writer_panic_flushes_the_edit_it_was_writing() -> TestResult {
    let root = journey_root("writer-panic")?;
    let mut run = run_child("writer-panic", &root)?;
    let writer = Panicked {
        thread: "alpine-recovery",
        message: "recovery writer panic marker",
    };
    // Generation 1 is the launch snapshot; the edit is generation 2.
    assert_panic_journey(&root, &mut run, &writer, |flushed| {
        flushed == "flushed generation 2"
    })
}

fn assert_panic_journey(
    root: &Path,
    run: &mut Run,
    panicked: &Panicked,
    flushed_as_expected: impl Fn(&str) -> bool,
) -> TestResult {
    let status = run.wait()?;

    assert_eq!(status.signal(), Some(SIGABRT), "stderr: {}", run.stderr());
    let logs = crash_logs(root);
    assert_eq!(logs.len(), 1, "stderr: {}", run.stderr());
    let log = fs::read_to_string(&logs[0])?;
    for expected in [
        format!("\nthread: {}\n", panicked.thread),
        format!("\nmessage: {}\n", panicked.message),
        String::from("\nbacktrace:\n"),
    ] {
        assert!(log.contains(&expected), "missing {expected:?} in {log}");
    }
    let flushed = log
        .lines()
        .find_map(|line| line.strip_prefix("recovery: "))
        .ok_or("recovery line")?;
    assert!(flushed_as_expected(flushed), "{flushed}");
    assert!(run.stderr().contains(panicked.message));
    let state = recovery::load(&journal(root))?;
    assert_eq!(state.documents.len(), 1);
    assert_eq!(&*state.documents[0].local, "edited");
    assert_eq!(&*state.documents[0].base, "base");
    assert_eq!(fs::read_to_string(root.join("document.rs"))?, "base");
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn sigkill_after_an_edit_is_recovered_by_the_next_launch() -> TestResult {
    let root = journey_root("kill")?;
    let mut run = run_child("kill", &root)?;
    let deadline = Instant::now() + TIMEOUT;
    while !run.stdout().contains("journey-ready") {
        if Instant::now() >= deadline || run.child.try_wait()?.is_some() {
            return Err(format!("child never became ready; stderr: {}", run.stderr()).into());
        }
        thread::sleep(Duration::from_millis(10));
    }

    run.child.kill()?;
    let status = run.wait()?;

    assert_eq!(status.signal(), Some(SIGKILL));
    assert!(crash_logs(&root).is_empty());
    let mut relaunch = run_child("relaunch", &root)?;
    let status = relaunch.wait()?;
    assert!(status.success(), "stderr: {}", relaunch.stderr());
    assert!(
        relaunch.stdout().contains("journey-recovered:edited\n"),
        "stdout: {}",
        relaunch.stdout()
    );
    fs::remove_dir_all(root)?;
    Ok(())
}
