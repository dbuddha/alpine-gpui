use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

use alpine_text::Buffer;

use super::{
    CrashReport, KEPT_LOGS, MAX_LOG_BYTES, State, TRUNCATED, log_stamp, prune, render, write_new,
};
use crate::recovery::{self, RecoveryCoordinator, RecoveryRequest, RecoverySnapshot};
use crate::session::{
    SESSION_NODE_CAPACITY, SESSION_PANE_CAPACITY, SessionFileTree, SessionNode, SessionPane,
    SessionPanes, SessionState, SessionTab,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

static SEQUENCE: AtomicU64 = AtomicU64::new(1);

fn scratch(label: &str) -> Result<PathBuf, std::io::Error> {
    let root = std::env::temp_dir().join(format!(
        "alpine-crash-{label}-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root)?;
    Ok(root)
}

fn logs_in(home: &Path) -> PathBuf {
    home.join("Library/Logs/Alpine Editor")
}

fn crash_logs(home: &Path) -> Result<Vec<String>, std::io::Error> {
    let mut names = Vec::new();
    for entry in fs::read_dir(logs_in(home))? {
        let name = entry?.file_name().to_string_lossy().into_owned();
        if log_stamp(&name).is_some() {
            names.push(name);
        }
    }
    names.sort();
    Ok(names)
}

fn record(state: &State, message: &str) -> Option<PathBuf> {
    state.record(&report(message)).and_then(Result::ok)
}

fn report(message: &str) -> CrashReport {
    CrashReport {
        message: message.to_owned(),
        location: String::from("src/example.rs:7:9"),
        thread: String::from("worker-7"),
    }
}

fn state(home: &Path) -> State {
    State::new(
        Some(home.as_os_str().to_owned()),
        None,
        Duration::from_millis(200),
        Duration::from_millis(60),
    )
}

fn session_state() -> SessionState {
    let mut nodes = [SessionNode::Empty; SESSION_NODE_CAPACITY];
    nodes[0] = SessionNode::Leaf { pane: 0 };
    let mut panes = [None; SESSION_PANE_CAPACITY];
    panes[0] = Some(SessionPane {
        tab: 0,
        view: crate::documents::DocumentViewState::default(),
    });
    SessionState {
        workspace: None,
        tabs: vec![SessionTab {
            path: None,
            view: crate::documents::DocumentViewState::default(),
        }],
        active_tab: 0,
        panes: SessionPanes {
            nodes,
            panes,
            active_pane: 0,
        },
        file_tree: SessionFileTree::default(),
    }
}

fn request(text: &str) -> RecoveryRequest {
    RecoveryRequest {
        session: session_state(),
        documents: vec![RecoverySnapshot {
            tab: 0,
            base: Buffer::new("base").snapshot(),
            local: Buffer::new(text).snapshot(),
        }],
        authority_revision: 1,
    }
}

#[test]
fn log_names_every_field_and_a_backtrace() -> TestResult {
    let home = scratch("fields")?;
    let identity = home.join("alpine-build-identity.toml");
    fs::write(&identity, "revision = \"0123abc\"\ntree = \"clean\"\n")?;
    let state = State::new(
        Some(home.as_os_str().to_owned()),
        Some(identity),
        Duration::from_millis(50),
        Duration::from_millis(50),
    );

    let path = record(&state, "boom: index out of bounds").ok_or("no crash log was written")?;

    let pid = std::process::id();
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("log name")?;
    assert_eq!(log_stamp(name).map(|stamp| stamp.1), Some(pid));
    let text = fs::read_to_string(&path)?;
    for expected in [
        "alpine-editor crash log\n".to_owned(),
        format!("\npid: {pid}\n"),
        String::from("\nthread: worker-7\n"),
        String::from("\nlocation: src/example.rs:7:9\n"),
        String::from("\nmessage: boom: index out of bounds\n"),
        String::from("\nrecovery: no journal attached\n"),
        String::from("\nrevision = \"0123abc\"\n"),
    ] {
        assert!(text.contains(&expected), "missing {expected:?} in {text}");
    }
    let backtrace = text.split("\nbacktrace:\n").nth(1).ok_or("backtrace")?;
    assert!(!backtrace.trim().is_empty());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(&path)?.permissions().mode() & 0o777, 0o600);
    }
    Ok(())
}

#[test]
fn a_missing_build_identity_is_reported_not_fatal() -> TestResult {
    let home = scratch("identity")?;
    let path = record(&state(&home), "boom").ok_or("log")?;
    assert!(fs::read_to_string(path)?.contains("build identity:\nunavailable\n"));
    Ok(())
}

#[test]
fn log_is_capped_and_a_huge_message_cannot_evict_the_backtrace() -> TestResult {
    let home = scratch("cap")?;
    // 3-byte characters, so neither cap lands on a character boundary.
    let message = "\u{20ac}".repeat(1_000_000);
    let identity = home.join("identity.toml");
    fs::write(&identity, "x".repeat(1_000_000))?;
    let state = State::new(
        Some(home.as_os_str().to_owned()),
        Some(identity),
        Duration::from_millis(50),
        Duration::from_millis(50),
    );

    let path = record(&state, &message).ok_or("log")?;

    let text = fs::read_to_string(path)?;
    assert!(text.len() <= MAX_LOG_BYTES, "{} bytes", text.len());
    assert!(text.contains("\nbacktrace:\n"));

    let oversized = render(1, &report("m"), "r", "i", &"b".repeat(MAX_LOG_BYTES * 2));
    assert_eq!(oversized.len(), MAX_LOG_BYTES);
    assert!(oversized.ends_with(TRUNCATED));
    Ok(())
}

#[test]
fn only_the_newest_ten_crash_logs_survive() -> TestResult {
    let home = scratch("retention")?;
    let logs = logs_in(&home);
    fs::create_dir_all(&logs)?;
    for unix in 1_000..1_014 {
        fs::write(logs.join(format!("crash-{unix}-77.log")), "old")?;
    }
    for unrelated in [
        "notes.txt",
        "perf-5-frames.tsv",
        "crash-abc-1.log",
        "crash-1-2-3.log",
        "crash-+5-7.log",
    ] {
        fs::write(logs.join(unrelated), "keep")?;
    }

    let path = record(&state(&home), "boom").ok_or("log")?;

    let kept = crash_logs(&home)?;
    assert_eq!(kept.len(), KEPT_LOGS);
    let newest = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("name")?;
    assert!(kept.iter().any(|name| name == newest));
    assert!(!kept.contains(&String::from("crash-1000-77.log")));
    assert!(!kept.contains(&String::from("crash-1004-77.log")));
    assert!(kept.contains(&String::from("crash-1005-77.log")));
    assert!(kept.contains(&String::from("crash-1013-77.log")));
    for unrelated in [
        "notes.txt",
        "perf-5-frames.tsv",
        "crash-abc-1.log",
        "crash-+5-7.log",
    ] {
        assert!(logs.join(unrelated).is_file(), "{unrelated} was removed");
    }
    prune(&logs, 0);
    assert!(crash_logs(&home)?.is_empty());
    Ok(())
}

#[test]
fn an_existing_log_is_never_replaced() -> TestResult {
    let home = scratch("create-new")?;
    let first = write_new(&home, 5, 6, "first")?;
    let second = write_new(&home, 5, 6, "second");
    assert_eq!(
        second.map_err(|error| error.kind()),
        Err(std::io::ErrorKind::AlreadyExists)
    );
    assert_eq!(fs::read_to_string(first)?, "first");
    Ok(())
}

#[test]
fn a_panic_while_the_hook_runs_writes_nothing_and_does_not_hang() -> TestResult {
    let home = scratch("reentry")?;
    let state = state(&home);

    let entry = state.enter().ok_or("first entry")?;
    assert!(state.enter().is_none());
    let started = Instant::now();
    assert!(record(&state, "second").is_none());
    assert!(started.elapsed() >= Duration::from_millis(50));
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(!logs_in(&home).exists());

    drop(entry);
    assert!(record(&state, "third").is_some());
    assert_eq!(crash_logs(&home)?.len(), 1);
    Ok(())
}

#[test]
fn a_concurrent_panic_waits_for_the_hook_in_flight() -> TestResult {
    let home = scratch("concurrent")?;
    let state = State::new(
        Some(home.as_os_str().to_owned()),
        None,
        Duration::from_millis(50),
        Duration::from_secs(5),
    );
    let entry = state.enter().ok_or("first entry")?;
    let started = Instant::now();
    let waited = std::thread::scope(|scope| {
        let other = scope.spawn(|| (record(&state, "second"), started.elapsed()));
        std::thread::sleep(Duration::from_millis(100));
        drop(entry);
        other.join()
    })
    .map_err(|_| "waiting thread panicked")?;
    assert!(waited.0.is_none());
    assert!(waited.1 >= Duration::from_millis(90), "{:?}", waited.1);
    assert!(waited.1 < Duration::from_secs(2), "{:?}", waited.1);
    Ok(())
}

#[test]
fn recovery_is_flushed_before_the_log_and_the_log_says_so() -> TestResult {
    let home = scratch("flush")?;
    let journal = home.join("state/recovery-v1.bin");
    let (coordinator, _receiver) = RecoveryCoordinator::without_worker(journal.clone());
    assert_eq!(coordinator.publish(request("typed before the crash"))?, 1);
    assert!(!journal.exists());
    let hook = super::CrashHook {
        state: std::sync::Arc::new(state(&home)),
    };
    hook.attach_recovery(coordinator.flush_handle());

    let path = record(&hook.state, "boom").ok_or("log")?;

    assert!(fs::read_to_string(path)?.contains("\nrecovery: flushed generation 1\n"));
    let recovered = recovery::load(&journal)?;
    assert_eq!(&*recovered.documents[0].local, "typed before the crash");
    Ok(())
}

#[test]
fn a_busy_journal_is_logged_and_never_blocks_the_log() -> TestResult {
    let home = scratch("busy")?;
    let journal = home.join("state/recovery-v1.bin");
    let (coordinator, _receiver) = RecoveryCoordinator::without_worker(journal.clone());
    coordinator.publish(request("pending"))?;
    let hook = super::CrashHook {
        state: std::sync::Arc::new(state(&home)),
    };
    hook.attach_recovery(coordinator.flush_handle());
    let _held = coordinator.hold_snapshot_lock();

    let started = Instant::now();
    let path = record(&hook.state, "boom").ok_or("log")?;

    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(fs::read_to_string(path)?.contains("\nrecovery: not flushed, busy for "));
    assert!(!journal.exists());
    Ok(())
}
