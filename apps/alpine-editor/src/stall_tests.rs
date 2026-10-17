use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    thread,
    time::{Duration, Instant},
};

use super::{
    Config, KIND, KIND_SHIFT, Kind, Pending, RUNNING, SAMPLER, START_SHIFT, Shared, State, WATCHED,
    Watch, Watcher, capture_stack, main_thread_stack,
};
use crate::logs::{self, KEPT_LOGS, MAX_LOG_BYTES};

type TestResult = Result<(), Box<dyn std::error::Error>>;

// Bounds only a hang; no assertion depends on it.
const HANG: Duration = Duration::from_mins(1);
// Under an exec storm a fake sampler can take seconds just to start.
const SPAWN_BOUND: Duration = Duration::from_secs(30);

static SEQUENCE: AtomicU64 = AtomicU64::new(1);

const REPORT: &str = r"#!/bin/sh
cat <<EOF
Analysis of sampling alpine-editor (pid $1) every 1 millisecond
Process:         alpine-editor [$1]
Path:            /Users/someone/Applications/Alpine Editor.app/Contents/MacOS/alpine-editor

Call graph:
    785 Thread_459124053   DispatchQueue_1: com.apple.main-thread  (serial)
      785 start  (in dyld) + 6992  [0x182e8c4e4]
        785 args: $*
          785 handle_key  (in alpine-editor) + 12  [0x1]
    785 Thread_459124054
      785 other_thread_frame  (in libsystem_kernel.dylib) + 8  [0x2]

Total number in stack (recursive counted multiple, when >=5):

Binary Images:
       0x102410000 -        0x102410a03 +alpine-editor /Users/someone/alpine-editor
EOF
";

fn scratch(label: &str) -> Result<PathBuf, std::io::Error> {
    let root = std::env::temp_dir().join(format!(
        "alpine-stall-{label}-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root)?;
    Ok(root)
}

fn config(home: &Path, threshold_ms: u64, interval_ms: u64) -> Config {
    Config {
        home: Some(home.as_os_str().to_owned()),
        identity: None,
        threshold: Duration::from_millis(threshold_ms),
        interval: Duration::from_millis(interval_ms),
        min_gap: Duration::ZERO,
        sampler: vec![home.join("no-such-sampler").into_os_string()],
        sample_bound: SPAWN_BOUND,
    }
}

// Run through sh: a first exec of a new file waits on a system scan.
fn fake_sampler(home: &Path, script: &str) -> Result<Vec<OsString>, std::io::Error> {
    let path = home.join("sampler.sh");
    fs::write(&path, script)?;
    Ok(vec![OsString::from("/bin/sh"), path.into_os_string()])
}

fn logs_in(home: &Path) -> PathBuf {
    home.join("Library/Logs/Alpine Editor")
}

fn stall_logs(home: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<_> = fs::read_dir(logs_in(home))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .and_then(|name| logs::stamp(name, KIND))
                .is_some()
        })
        .map(|entry| entry.path())
        .collect();
    paths.sort();
    paths
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_default()
}

fn wait_until(what: &str, mut done: impl FnMut() -> bool) -> Result<(), String> {
    let deadline = Instant::now() + HANG;
    while !done() {
        if Instant::now() >= deadline {
            return Err(format!("hung waiting for {what}"));
        }
        thread::sleep(Duration::from_millis(1));
    }
    Ok(())
}

fn load(watch: &Watch, counter: fn(&Shared) -> &AtomicU64) -> u64 {
    watch
        .inner
        .as_ref()
        .map_or(0, |inner| counter(&inner.shared).load(Ordering::SeqCst))
}

fn wakeups(watch: &Watch) -> u64 {
    load(watch, |shared| &shared.wakeups)
}

fn stalls(watch: &Watch) -> u64 {
    load(watch, |shared| &shared.stalls)
}

fn parked(watch: &Watch) -> bool {
    watch
        .inner
        .as_ref()
        .is_some_and(|inner| inner.shared.parked.load(Ordering::SeqCst))
}

fn field(text: &str, name: &str) -> Result<u128, String> {
    let prefix = format!("\n{name}: ");
    let value = text
        .split(&prefix)
        .nth(1)
        .and_then(|rest| rest.lines().next())
        .ok_or_else(|| format!("no {name} in {text}"))?;
    value.parse().map_err(|_| format!("bad {name}: {value}"))
}

/// One dispatch that runs until the watchdog has detected it as a stall.
fn stall(watch: &mut Watch, kind: Kind) -> Result<(), String> {
    let before = stalls(watch);
    watch.begin(kind);
    wait_until("stall detection", || stalls(watch) > before)?;
    watch.end();
    Ok(())
}

fn finished(path: &Path) -> Result<String, String> {
    wait_until("the duration line", || {
        read(path).contains("\nduration_ms: ")
    })?;
    Ok(read(path))
}

fn process_alive(pid: &str) -> bool {
    Command::new("/bin/kill")
        .args(["-0", pid])
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[test]
fn a_dispatch_past_the_threshold_writes_one_log_with_its_duration() -> TestResult {
    let home = scratch("stall")?;
    let identity = home.join("identity.toml");
    fs::write(&identity, "revision = \"0123abc\"\n")?;
    let sampler = fake_sampler(&home, REPORT)?;
    let mut watch = Watch::start(Config {
        identity: Some(identity),
        sampler,
        ..config(&home, 50, 25)
    })?;

    stall(&mut watch, Kind::Keyboard)?;
    wait_until("the stall log", || stall_logs(&home).len() == 1)?;
    let path = stall_logs(&home).pop().ok_or("stall log")?;
    let text = finished(&path)?;
    drop(watch);

    assert_eq!(stall_logs(&home), std::slice::from_ref(&path));
    let pid = std::process::id();
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("name")?;
    assert_eq!(logs::stamp(name, KIND).map(|stamp| stamp.1), Some(pid));
    for expected in [
        String::from("alpine-editor stall log\n"),
        format!("\npid: {pid}\n"),
        String::from("\ndispatch: Keyboard\n"),
        String::from("\nthreshold_ms: 50\n"),
        String::from("\nbuild identity:\nrevision = \"0123abc\"\n"),
        String::from("\nmain thread stack:\n"),
        String::from("com.apple.main-thread"),
        format!("args: {pid} 1 -file /dev/stdout\n"),
        String::from("handle_key  (in alpine-editor)"),
    ] {
        assert!(text.contains(&expected), "missing {expected:?} in {text}");
    }
    for leaked in ["Path:", "other_thread_frame", "Binary Images", "/Users/"] {
        assert!(!text.contains(leaked), "{leaked:?} leaked into {text}");
    }
    let started = field(&text, "started_unix_ms")?;
    let detected = field(&text, "elapsed_at_detection_ms")?;
    let duration = field(&text, "duration_ms")?;
    assert!(started > 1_600_000_000_000, "{started}");
    assert!(detected >= 50, "{detected}");
    assert!(duration >= detected, "{duration} < {detected}");
    assert!(text.ends_with(&format!("\nduration_ms: {duration}\n")));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(&path)?.permissions().mode() & 0o777, 0o600);
    }
    fs::remove_dir_all(home)?;
    Ok(())
}

#[test]
fn dispatches_under_the_threshold_write_nothing() -> TestResult {
    let home = scratch("short")?;
    let mut watch = Watch::start(config(&home, 2_000, 5))?;

    for _ in 0..10 {
        watch.begin(Kind::Scroll);
        let seen = wakeups(&watch);
        // The watchdog checks this dispatch at least twice while it runs.
        wait_until("two watchdog checks", || wakeups(&watch) >= seen + 2)?;
        watch.end();
    }
    wait_until("the watchdog to park", || parked(&watch))?;

    assert_eq!(stalls(&watch), 0);
    drop(watch);
    assert!(stall_logs(&home).is_empty());
    assert!(!logs_in(&home).exists());
    fs::remove_dir_all(home)?;
    Ok(())
}

#[test]
fn an_idle_watchdog_parks_and_makes_no_wakeups() -> TestResult {
    let home = scratch("idle")?;
    let mut watch = Watch::start(config(&home, 1_000, 5))?;
    thread::sleep(Duration::from_millis(50));
    assert_eq!(wakeups(&watch), 0, "woke before any dispatch");

    for _ in 0..20 {
        watch.begin(Kind::Wake);
        watch.end();
        thread::sleep(Duration::from_millis(1));
    }
    // A last dispatch the watchdog sees, so it can park only after its end.
    watch.begin(Kind::Wake);
    let seen = wakeups(&watch);
    wait_until("two watchdog checks", || wakeups(&watch) >= seen + 2)?;
    watch.end();
    wait_until("the watchdog to park", || parked(&watch))?;
    let at_park = wakeups(&watch);
    assert!(at_park > 0);
    // Twenty intervals of idle.
    thread::sleep(Duration::from_millis(100));
    assert_eq!(wakeups(&watch), at_park, "woke while idle");
    assert!(parked(&watch));

    watch.begin(Kind::Keyboard);
    watch.end();
    wait_until("the next dispatch to unpark it", || {
        wakeups(&watch) > at_park
    })?;
    drop(watch);
    fs::remove_dir_all(home)?;
    Ok(())
}

#[test]
fn steady_dispatches_wake_the_watchdog_once_per_interval() -> TestResult {
    let home = scratch("steady")?;
    let interval = Duration::from_millis(20);
    let mut watch = Watch::start(config(&home, 10_000, 20))?;

    let started = Instant::now();
    let mut dispatches = 0_u32;
    while dispatches < 400 {
        watch.begin(Kind::Pointer);
        watch.end();
        dispatches += 1;
        thread::sleep(Duration::from_micros(500));
    }
    let woke = wakeups(&watch);
    let intervals = started.elapsed().as_micros() / interval.as_micros();
    drop(watch);

    // One unpark, then at most one wakeup per interval.
    assert!(woke >= 1, "never woke");
    assert!(
        u128::from(woke) <= intervals + 3,
        "{woke} wakeups in {intervals} intervals for {dispatches} dispatches"
    );
    fs::remove_dir_all(home)?;
    Ok(())
}

#[test]
fn retention_keeps_ten_stall_logs_and_leaves_other_files() -> TestResult {
    let home = scratch("retention")?;
    let logs = logs_in(&home);
    fs::create_dir_all(&logs)?;
    for unix in 1_000..1_014 {
        fs::write(logs.join(format!("stall-{unix}-77.log")), "old")?;
    }
    let unrelated = [
        "crash-1000-77.log",
        "notes.txt",
        "perf-5-frames.tsv",
        "stall-abc-1.log",
        "stall-1-2-3.log",
        "stalls-5-7.log",
    ];
    for name in unrelated {
        fs::write(logs.join(name), "keep")?;
    }
    let mut watch = Watch::start(config(&home, 30, 10))?;

    stall(&mut watch, Kind::Menu)?;
    wait_until("the new log", || {
        stall_logs(&home)
            .iter()
            .any(|path| read(path).contains("dispatch: Menu"))
    })?;
    drop(watch);

    let kept = stall_logs(&home);
    assert_eq!(kept.len(), KEPT_LOGS, "{kept:?}");
    assert!(!logs.join("stall-1004-77.log").exists());
    assert!(logs.join("stall-1005-77.log").is_file());
    for name in unrelated {
        assert!(logs.join(name).is_file(), "{name} was removed");
    }
    fs::remove_dir_all(home)?;
    Ok(())
}

#[test]
fn a_huge_stack_is_capped_and_the_duration_still_fits() -> TestResult {
    let home = scratch("cap")?;
    let script = "#!/bin/sh\necho '    9 Thread_1   DispatchQueue_1: com.apple.main-thread'\n\
                  /usr/bin/yes '      9 deep_frame  (in alpine-editor) + 1  [0x1]' | /usr/bin/head -n 40000\n";
    let sampler = fake_sampler(&home, script)?;
    let mut watch = Watch::start(Config {
        sampler,
        ..config(&home, 30, 10)
    })?;

    stall(&mut watch, Kind::Frame)?;
    wait_until("the stall log", || stall_logs(&home).len() == 1)?;
    let path = stall_logs(&home).pop().ok_or("stall log")?;
    let text = finished(&path)?;
    drop(watch);

    assert!(text.len() <= MAX_LOG_BYTES, "{} bytes", text.len());
    assert!(text.len() > MAX_LOG_BYTES - 200, "{} bytes", text.len());
    assert!(text.contains("deep_frame"));
    assert!(text.contains(logs::TRUNCATED));
    field(&text, "duration_ms")?;
    fs::remove_dir_all(home)?;
    Ok(())
}

#[test]
fn one_log_per_stall_and_none_inside_the_gap() -> TestResult {
    let home = scratch("rate")?;
    let mut watch = Watch::start(Config {
        min_gap: Duration::from_hours(1),
        ..config(&home, 30, 10)
    })?;

    watch.begin(Kind::Keyboard);
    wait_until("stall detection", || stalls(&watch) == 1)?;
    let detected_at = wakeups(&watch);
    // The watchdog sees the same stall five more times.
    wait_until("more checks", || wakeups(&watch) >= detected_at + 5)?;
    watch.end();
    let first = stall_logs(&home).pop().ok_or("first log")?;
    finished(&first)?;
    // A second log would need a new second in its name.
    let stamp = |path: &Path| {
        path.file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| logs::stamp(name, KIND))
            .map(|stamp| stamp.0)
    };
    let unix_now = || {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_secs())
    };
    wait_until("the next second", || stamp(&first) < Some(unix_now()))?;
    stall(&mut watch, Kind::Pointer)?;
    wait_until("the watchdog to park", || parked(&watch))?;

    assert_eq!(stalls(&watch), 2);
    drop(watch);
    assert_eq!(stall_logs(&home), [first]);
    fs::remove_dir_all(home)?;
    Ok(())
}

#[test]
fn a_stall_that_ends_between_checks_is_still_logged() -> TestResult {
    let home = scratch("between")?;
    let config = config(&home, 30, 10);
    let shared = Shared::new();
    let mut watcher = Watcher::new(&shared, &config);
    let watch = |start_us: u64, kind: Kind| {
        let word = (start_us << START_SHIFT) | ((kind as u64) << KIND_SHIFT) | RUNNING;
        shared.dispatch.store(word | WATCHED, Ordering::SeqCst);
        Pending {
            word,
            state: State::Watching,
        }
    };

    // A short dispatch that ends between checks writes nothing.
    let short = watch(1, Kind::Wake);
    let ended = short.word & !RUNNING;
    watcher.pending = Some(short);
    shared.dispatch.store(ended, Ordering::SeqCst);
    assert!(!watcher.resolve(ended), "resolved before its end time");
    shared.ended.store(20_000 + 1, Ordering::SeqCst);
    assert!(watcher.resolve(ended));
    assert!(stall_logs(&home).is_empty());

    // A 150 ms dispatch the checks never saw past 30 ms is logged at its end.
    let long = watch(100_000, Kind::Scroll);
    let ended = long.word & !RUNNING;
    watcher.pending = Some(long);
    shared.dispatch.store(ended, Ordering::SeqCst);
    shared.ended.store(250_000 + 1, Ordering::SeqCst);
    assert!(watcher.resolve(ended));

    assert!(watcher.pending.is_none());
    assert_eq!(shared.stalls.load(Ordering::SeqCst), 1);
    let path = stall_logs(&home).pop().ok_or("stall log")?;
    let text = read(&path);
    for expected in [
        "\ndispatch: Scroll\n",
        "\nelapsed_at_detection_ms: 150\n",
        "\nmain thread stack: not captured, it had ended\n",
        "\nduration_ms: 150\n",
    ] {
        assert!(text.contains(expected), "missing {expected:?} in {text}");
    }
    fs::remove_dir_all(home)?;
    Ok(())
}

#[test]
fn a_missing_sampler_still_writes_the_log() -> TestResult {
    let home = scratch("no-sampler")?;
    let mut watch = Watch::start(config(&home, 30, 10))?;

    stall(&mut watch, Kind::WorkerLanguage)?;
    wait_until("the stall log", || stall_logs(&home).len() == 1)?;
    let path = stall_logs(&home).pop().ok_or("stall log")?;
    let text = finished(&path)?;
    drop(watch);

    assert!(text.contains("\ndispatch: WorkerLanguage\n"), "{text}");
    assert!(
        text.contains("\nmain thread stack: unavailable (spawn failed: NotFound)\n"),
        "{text}"
    );
    fs::remove_dir_all(home)?;
    Ok(())
}

#[test]
fn a_hung_sampler_is_killed_at_its_bound() -> TestResult {
    let home = scratch("hung")?;
    let pid_file = home.join("sampler.pid");
    let script = format!(
        "#!/bin/sh\necho $$ > '{}'\nexec /bin/sleep 30\n",
        pid_file.display()
    );
    let sampler = fake_sampler(&home, &script)?;
    let mut watch = Watch::start(Config {
        sampler,
        sample_bound: Duration::from_secs(2),
        ..config(&home, 30, 10)
    })?;

    stall(&mut watch, Kind::Resize)?;
    wait_until("the stall log", || stall_logs(&home).len() == 1)?;
    let path = stall_logs(&home).pop().ok_or("stall log")?;
    let text = finished(&path)?;
    drop(watch);

    assert!(
        text.contains("\nmain thread stack: unavailable (timed out after 2000 ms)\n"),
        "{text}"
    );
    // No pid file means the kill came before the script's first line.
    if let Ok(sampler_pid) = fs::read_to_string(&pid_file) {
        assert!(!process_alive(sampler_pid.trim()), "sampler survived");
    }
    fs::remove_dir_all(home)?;
    Ok(())
}

#[test]
fn dropping_the_watch_stops_a_capture_in_flight() -> TestResult {
    let home = scratch("stop")?;
    let pid_file = home.join("sampler.pid");
    let script = format!(
        "#!/bin/sh\necho $$ > '{}'\nexec /bin/sleep 600\n",
        pid_file.display()
    );
    let sampler = fake_sampler(&home, &script)?;
    let mut watch = Watch::start(Config {
        sampler,
        sample_bound: Duration::from_mins(2),
        ..config(&home, 30, 10)
    })?;

    stall(&mut watch, Kind::Ime)?;
    wait_until("the sampler", || !read(&pid_file).is_empty())?;
    let dropped = Instant::now();
    drop(watch);

    // The capture polls for the stop, so the join never waits on the bound.
    assert!(dropped.elapsed() < SPAWN_BOUND, "{:?}", dropped.elapsed());
    let path = stall_logs(&home).pop().ok_or("stall log")?;
    let text = finished(&path)?;
    assert!(
        text.contains("\nmain thread stack: unavailable (stopped)\n"),
        "{text}"
    );
    assert!(!process_alive(read(&pid_file).trim()), "sampler survived");
    fs::remove_dir_all(home)?;
    Ok(())
}

#[test]
fn the_stack_is_the_main_thread_section_of_a_sample_report() {
    let report = "Process:         alpine-editor [42]\n\
                  Path:            /Users/someone/alpine-editor\n\
                  \n\
                  Call graph:\n\
                  \x20   12 Thread_7   DispatchQueue_1: com.apple.main-thread  (serial)\n\
                  \x20   + 12 start  (in dyld) + 6992  [0x1]\n\
                  \x20   +   12 save  (in alpine-editor) + 8  [0x2]  lib.rs:12\n\
                  \x20   12 Thread_8: alpine-recovery\n\
                  \x20   + 12 other  (in alpine-editor) + 8  [0x3]\n\
                  \n\
                  Binary Images:\n";

    let stack = main_thread_stack(report);

    assert_eq!(
        stack.as_deref(),
        Some(
            "    12 Thread_7   DispatchQueue_1: com.apple.main-thread  (serial)\n    \
             + 12 start  (in dyld) + 6992  [0x1]\n    \
             +   12 save  (in alpine-editor) + 8  [0x2]  lib.rs:12\n"
        )
    );
    assert_eq!(main_thread_stack("Call graph:\n    1 Thread_9\n"), None);
}

#[test]
#[ignore = "runs /usr/bin/sample on this test process for about 3 s"]
fn the_real_sampler_finds_this_process_main_thread() -> TestResult {
    let stack = capture_stack(
        &[OsString::from(SAMPLER)],
        Duration::from_secs(30),
        &AtomicBool::new(false),
    )?;
    assert!(stack.contains("com.apple.main-thread"), "{stack}");
    assert!(stack.lines().count() > 1, "{stack}");
    Ok(())
}
