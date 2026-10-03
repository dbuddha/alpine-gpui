//! One `bench run`: a fresh process and disposable home per trial, the
//! visibility gate before measuring, the workload's phases, cleanup of every
//! process the trial started, and raw TSV under `bench/results/<run>/`.

use crate::analysis::{self, Metric, Visibility};
use crate::apps::{self, App, Identity, LaunchInputs, RustAnalyzer};
use crate::fixtures::{self, FixtureFile};
use crate::helpers::{
    self, Appeared, Frame, Helpers, InputEvent, InputOutcome, ProcRow, SampleSet, Sampler,
    SamplerData, SetKind, WindowInfo,
};
use crate::isolation::{self, Snapshot};
use crate::paths::{self, BenchPaths};
use crate::procs::{self, LaunchedApp};
use crate::stamp::{self, Stamp, StampInputs};
use crate::stats::format_value;
use crate::tsv::{self, MISSING};
use crate::workload::{Action, Document, Phase, STARTUP_PHASE, TRIAL_PHASE, Workload, typed_text};
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

pub struct RunRequest {
    pub workload: &'static Workload,
    pub app: App,
    pub trials: u32,
    pub warmup: Option<u32>,
    pub keep_homes: bool,
    pub rust_analyzer: Option<PathBuf>,
    /// Overrides the workload's spacing of timed keys.
    pub key_interval_ms: Option<u64>,
}

const MIN_WINDOW_SIDE: f64 = 100.0;
const WINDOW_TIMEOUT_MS: u64 = 30_000;
const ACTIVATION_ATTEMPTS: u64 = 5;
const STOP_GRACE: Duration = Duration::from_secs(5);
const COOLDOWN: Duration = Duration::from_secs(3);
/// cfprefsd writes preferences after a short delay, so the real-state check
/// waits before comparing.
const STATE_SETTLE: Duration = Duration::from_secs(2);
/// How often every phase re-checks that the window is frontmost and bare.
const VISIBILITY_POLL: Duration = Duration::from_secs(1);
const HOME_MARKER: &str = ".alpine-bench-home";

pub const TRIALS_HEADER: &[&str] = &[
    "run_id",
    "trial",
    "warmup",
    "valid",
    "reason",
    "pid",
    "window_id",
    "window_x",
    "window_y",
    "window_width",
    "window_height",
];
pub const METRICS_HEADER: &[&str] = &[
    "run_id", "app", "workload", "trial", "warmup", "valid", "phase", "metric", "value", "unit",
];
pub const SUMMARY_HEADER: &[&str] = &[
    "run_id",
    "app",
    "workload",
    "phase",
    "metric",
    "unit",
    "n",
    "mean",
    "stddev",
    "ci95_low",
    "ci95_high",
    "median",
    "min",
    "max",
];
const SAMPLES_HEADER: &[&str] = &[
    "run_id",
    "trial",
    "seq",
    "kind",
    "mach",
    "pid",
    "ppid",
    "depth",
    "name",
    "footprint",
    "lifetime_max_footprint",
    "user_ns",
    "system_ns",
    "child_user_ns",
    "child_system_ns",
    "idle_wakeups",
    "interrupt_wakeups",
    "child_idle_wakeups",
    "child_interrupt_wakeups",
    "instructions",
    "cycles",
    "energy_nj",
    "start_mach",
    "exited",
];
const MARKS_HEADER: &[&str] = &["run_id", "trial", "seq", "mach", "label"];
const EVENTS_HEADER: &[&str] = &["run_id", "trial", "phase", "seq", "mach", "kind", "label"];
const FRAMES_HEADER: &[&str] = &[
    "run_id",
    "trial",
    "phase",
    "seq",
    "display_mach",
    "arrival_mach",
    "hash",
];

struct Context<'a> {
    paths: &'a BenchPaths,
    helpers: &'a Helpers,
    request: &'a RunRequest,
    run_id: String,
    tag: String,
    run_dir: PathBuf,
    documents: Vec<PathBuf>,
    identity: Identity,
    rust_analyzer: Option<RustAnalyzer>,
    refresh_hz: u32,
    /// The owner's real state for this app, compared around every trial.
    state_roots: Vec<PathBuf>,
}

struct Driven {
    appeared: Appeared,
    invalid: Option<String>,
    events: Vec<(String, InputEvent)>,
    frames: Vec<(String, Frame)>,
    footprint_tool: Option<u64>,
}

struct TrialOutcome {
    number: u32,
    warmup: bool,
    pid: i32,
    driven: Driven,
    metrics: Vec<Metric>,
    sampler: SamplerData,
}

pub fn run(paths: &BenchPaths, request: &RunRequest) -> Result<PathBuf, String> {
    let helpers = Helpers::locate(&paths.helpers_dir)?;
    preflight(paths, &helpers, request)?;
    let rust_analyzer = if request.workload.rust_analyzer {
        Some(paths::rust_analyzer(
            paths,
            request.rust_analyzer.as_deref(),
        )?)
    } else {
        None
    };
    let displays = helpers.displays()?;
    let refresh_hz = displays
        .iter()
        .find(|display| display.main)
        .map_or(120, |display| display.max_fps.max(1));
    let stamp = stamp::collect(&StampInputs {
        repo_root: &paths.repo_root,
        alpine_bundle: &paths.alpine_bundle,
        zed_app: &paths.zed_app,
        helpers_dir: &paths.helpers_dir,
        displays: &displays,
    });
    let (_, tag) = stamp::utc(stamp::now_seconds());
    let run_id = format!("{tag}-{}-{}", request.workload.name, request.app.name());
    let run_dir = paths.results.join(&run_id);
    fs::create_dir_all(&run_dir)
        .map_err(|error| format!("create {}: {error}", run_dir.display()))?;
    let (documents, fixture) = documents(paths, request.workload)?;
    let context = Context {
        paths,
        helpers: &helpers,
        request,
        run_id,
        tag,
        run_dir,
        documents,
        identity: paths::identity(),
        rust_analyzer,
        refresh_hz,
        state_roots: apps::real_state_paths(request.app, &paths.real_home, &paths.zed_app),
    };
    write_run_table(&context, &stamp, fixture.as_ref())?;
    let before_run = isolation::snapshot(&context.state_roots);
    run_trials(&context)?;
    // A late cfprefsd write after the last trial still fails the run.
    thread::sleep(COOLDOWN);
    real_state_unchanged(&context, &before_run, "the run")?;
    Ok(context.run_dir)
}

/// Fails loudly if the owner's real state for the app changed.
fn real_state_unchanged(
    context: &Context<'_>,
    before: &Snapshot,
    during: &str,
) -> Result<(), String> {
    let changes = isolation::diff(before, &isolation::snapshot(&context.state_roots));
    if changes.is_empty() {
        return Ok(());
    }
    let listed: Vec<String> = changes.iter().map(isolation::describe).collect();
    Err(format!(
        "REAL {} STATE CHANGED during {during}; stop and inspect:\n  {}",
        context.request.app.name(),
        listed.join("\n  ")
    ))
}

fn run_trials(context: &Context<'_>) -> Result<(), String> {
    let request = context.request;
    let warmup = request.warmup.unwrap_or(request.workload.warmup_trials);
    let total = warmup + request.trials;
    eprintln!(
        "bench: {} on {}: {total} trial(s), {warmup} warm-up. Hands off the keyboard and mouse; \
         a trial is refused if its window is not frontmost and uncovered.",
        request.workload.name,
        request.app.name()
    );
    thread::sleep(Duration::from_secs(3));
    let mut kept: Vec<Metric> = Vec::new();
    for index in 0..total {
        let number = index + 1;
        let warm = index < warmup;
        match run_trial(context, number, warm) {
            Ok(outcome) => {
                record(context, &outcome)?;
                eprintln!("bench: {}", outcome_line(&outcome, total));
                if !outcome.warmup && outcome.driven.invalid.is_none() {
                    kept.extend(outcome.metrics);
                }
            }
            Err(error) => {
                let row = trial_row(&context.run_id, number, warm, Some(&error), 0, None);
                tsv::append(&context.run_dir.join("trials.tsv"), TRIALS_HEADER, &[row])?;
                write_summary(context, &kept)?;
                return Err(format!(
                    "trial {number}: {error}\nresults so far: {}",
                    context.run_dir.display()
                ));
            }
        }
        if number < total {
            thread::sleep(COOLDOWN);
        }
    }
    write_summary(context, &kept)?;
    println!("{}", context.run_dir.display());
    Ok(())
}

fn preflight(paths: &BenchPaths, helpers: &Helpers, request: &RunRequest) -> Result<(), String> {
    let binary = match request.app {
        App::Alpine => paths.alpine_bundle.join("Contents/MacOS/alpine-editor"),
        App::Zed => paths.zed_app.join("Contents/MacOS/zed"),
        App::AppKit => helpers.path(helpers::REFERENCE),
    };
    if !binary.is_file() {
        return Err(format!("{} is missing", binary.display()));
    }
    // A running copy would write the real state the trials compare, and a
    // second Zed exits at its single-instance check.
    let running = procs::running(&procs::ps()?, &binary.to_string_lossy());
    if !running.is_empty() {
        let pids: Vec<String> = running.iter().map(|row| row.pid.to_string()).collect();
        return Err(format!(
            "{} is already running (pid {}); quit it first",
            binary.display(),
            pids.join(", ")
        ));
    }
    if request.workload.needs_input() || request.workload.needs_capture() {
        let permissions = helpers.permissions()?;
        if request.workload.needs_input() && !permissions.post_events {
            return Err(
                "this workload posts input: grant Accessibility to the terminal that runs bench"
                    .to_owned(),
            );
        }
        if request.workload.needs_capture() && !permissions.screen_recording {
            return Err("this workload captures the window: grant Screen Recording to the terminal that runs bench".to_owned());
        }
    }
    if request.trials == 0 {
        return Err("--trials must be at least 1".to_owned());
    }
    Ok(())
}

fn documents(
    paths: &BenchPaths,
    workload: &Workload,
) -> Result<(Vec<PathBuf>, Option<FixtureFile>), String> {
    match workload.document {
        Document::Fixture(fixture) => {
            let file = fixtures::ensure(&paths.fixtures_dir(), fixture)?;
            Ok((vec![file.path.clone()], Some(file)))
        }
        Document::RepositoryFile(relative) => {
            let file = paths.repo_root.join(relative);
            if !file.is_file() {
                return Err(format!("{} is missing", file.display()));
            }
            Ok((vec![paths.repo_root.clone(), file], None))
        }
    }
}

fn write_run_table(
    context: &Context<'_>,
    stamp: &Stamp,
    fixture: Option<&FixtureFile>,
) -> Result<(), String> {
    let request = context.request;
    let mut table = tsv::Table::new(&["key", "value"]);
    let mut push = |key: &str, value: String| table.push(vec![key.to_owned(), value]);
    push("run_id", context.run_id.clone())?;
    push("workload", request.workload.name.to_owned())?;
    push("app", request.app.name().to_owned())?;
    push("trials", request.trials.to_string())?;
    let warmup = request.warmup.unwrap_or(request.workload.warmup_trials);
    push("warmup", warmup.to_string())?;
    for (key, value) in &stamp.entries {
        push(key, value.clone())?;
    }
    let documents: Vec<String> = context
        .documents
        .iter()
        .map(|path| path.display().to_string())
        .collect();
    push("documents", documents.join(" "))?;
    if let Some(file) = fixture {
        let name = file.fixture.file_name();
        push(
            "fixture",
            format!("{name} bytes={} fnv64={}", file.bytes, file.fnv64),
        )?;
    }
    if let Some(rust_analyzer) = &context.rust_analyzer {
        push("rust_analyzer", rust_analyzer.binary.display().to_string())?;
    }
    push("home_root", context.paths.home_root.display().to_string())?;
    let window = context.helpers.path(helpers::WINDOW);
    let (_, timebase) = helpers::parse_now(&procs::output(&window, &["now"])?)?;
    push("timebase", format!("{}/{}", timebase.numer, timebase.denom))?;
    tsv::write(&context.run_dir.join("run.tsv"), &table)
}

struct TrialHome {
    root: PathBuf,
    home: PathBuf,
    tmp: PathBuf,
    home_root: PathBuf,
}

impl TrialHome {
    fn create(home_root: &Path, tag: &str, number: u32) -> Result<Self, String> {
        fs::create_dir_all(home_root)
            .map_err(|error| format!("create {}: {error}", home_root.display()))?;
        let root = home_root.join(format!("{tag}-{number:02}"));
        fs::create_dir(&root)
            .map_err(|error| format!("create {} (must be new): {error}", root.display()))?;
        fs::write(root.join(HOME_MARKER), "disposable home created by bench\n")
            .map_err(|error| format!("mark {}: {error}", root.display()))?;
        let home = root.join("home");
        let tmp = root.join("tmp");
        for dir in [&home, &tmp] {
            fs::create_dir(dir).map_err(|error| format!("create {}: {error}", dir.display()))?;
        }
        Ok(Self {
            root,
            home,
            tmp,
            home_root: home_root.to_path_buf(),
        })
    }

    /// Deletes only a directory this bench created: under the home root
    /// and still carrying the marker file.
    fn remove(self) -> Result<(), String> {
        if !removable(
            &self.root,
            &self.home_root,
            self.root.join(HOME_MARKER).is_file(),
        ) {
            return Err(format!("refusing to delete {}", self.root.display()));
        }
        fs::remove_dir_all(&self.root)
            .map_err(|error| format!("delete {}: {error}", self.root.display()))
    }
}

fn removable(root: &Path, home_root: &Path, marked: bool) -> bool {
    marked && root != home_root && root.starts_with(home_root) && root.parent() == Some(home_root)
}

fn run_trial(context: &Context<'_>, number: u32, warmup: bool) -> Result<TrialOutcome, String> {
    let trial_dir = context.run_dir.join(format!("trial-{number:02}"));
    fs::create_dir_all(&trial_dir)
        .map_err(|error| format!("create {}: {error}", trial_dir.display()))?;
    let home = TrialHome::create(&context.paths.home_root, &context.tag, number)?;
    let reference = context.helpers.path(helpers::REFERENCE);
    let spec = apps::launch_spec(&LaunchInputs {
        app: context.request.app,
        home: &home.home,
        tmpdir: &home.tmp,
        documents: &context.documents,
        identity: &context.identity,
        rust_analyzer: context.rust_analyzer.as_ref(),
        alpine_bundle: &context.paths.alpine_bundle,
        zed_app: &context.paths.zed_app,
        reference_binary: &reference,
    })?;
    for (path, content) in &spec.seed_files {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("create {}: {error}", parent.display()))?;
        }
        fs::write(path, content).map_err(|error| format!("write {}: {error}", path.display()))?;
    }
    let log =
        |name: &str| File::create(trial_dir.join(name)).map_err(|error| format!("{name}: {error}"));
    let logs = (log("app.stdout.log")?, log("app.stderr.log")?);
    let before = isolation::snapshot(&context.state_roots);
    let mut app = LaunchedApp::spawn(&spec, &home.home, logs, &home.root.to_string_lossy())?;
    let pid = app.pid();
    let mut sampler = context
        .helpers
        .spawn_sampler(pid, &trial_dir.join("sampler.stderr.log"))?;
    let driven = match sampler.mark(&analysis::phase_label(STARTUP_PHASE, "start")) {
        Ok(_) => drive(context, &mut app, &mut sampler, &trial_dir),
        Err(error) if app.exited() => Err(launch_failure(
            &trial_dir,
            &format!("the app exited at launch ({error})"),
        )),
        Err(error) => Err(error),
    };
    let known: Vec<i32> = sampler
        .snapshot()
        .sets
        .last()
        .map(|set| set.rows.iter().map(|row| row.pid).collect())
        .unwrap_or_default();
    let data = sampler.finish()?;
    let stopped = app.stop(&known, STOP_GRACE);
    if !stopped.killed.is_empty() {
        eprintln!(
            "bench: trial {number}: KILLed {:?} after the TERM grace period",
            stopped.killed
        );
    }
    thread::sleep(STATE_SETTLE);
    real_state_unchanged(context, &before, &format!("trial {number}"))?;
    let driven =
        driven.map_err(|error| format!("{error} (home kept at {})", home.root.display()))?;
    let metrics = trial_metrics(context, &data, &driven)?;
    if context.request.keep_homes {
        eprintln!(
            "bench: trial {number}: home kept at {}",
            home.root.display()
        );
    } else {
        home.remove()?;
    }
    Ok(TrialOutcome {
        number,
        warmup,
        pid,
        driven,
        metrics,
        sampler: data,
    })
}

fn drive(
    context: &Context<'_>,
    app: &mut LaunchedApp,
    sampler: &mut Sampler,
    trial_dir: &Path,
) -> Result<Driven, String> {
    let helpers = context.helpers;
    let workload = context.request.workload;
    let appeared = helpers
        .wait_for_window(app.pid(), WINDOW_TIMEOUT_MS)
        .map_err(|error| launch_failure(trial_dir, &error))?;
    ensure_visible(helpers, app.pid())?;
    let mut driven = Driven {
        appeared,
        invalid: None,
        events: Vec::new(),
        frames: Vec::new(),
        footprint_tool: None,
    };
    let settle = Duration::from_secs(u64::from(workload.settle_seconds));
    hands_off(context, app.pid(), STARTUP_PHASE, settle, &mut driven);
    sampler.mark(&analysis::phase_label(STARTUP_PHASE, "end"))?;
    for phase in workload.phases {
        if driven.invalid.is_some() {
            break;
        }
        let start = sampler.mark(&analysis::phase_label(phase.name, "start"))?;
        run_phase(
            context,
            phase,
            app.pid(),
            sampler,
            start.seq,
            trial_dir,
            &mut driven,
        )?;
        sampler.mark(&analysis::phase_label(phase.name, "end"))?;
        if app.exited() {
            return Err(format!("the app exited during {}", phase.name));
        }
        if driven.invalid.is_none()
            && let Err(reason) = check_visible(helpers, app.pid())
        {
            driven.invalid = Some(format!("{}: {reason}", phase.name));
        }
    }
    driven.footprint_tool = procs::footprint_bytes(app.pid())
        .map_err(|error| eprintln!("bench: footprint cross-check skipped: {error}"))
        .ok();
    Ok(driven)
}

fn launch_failure(trial_dir: &Path, error: &str) -> String {
    let stdout = fs::read_to_string(trial_dir.join("app.stdout.log")).unwrap_or_default();
    if stdout.contains("zed is already running") {
        return "Zed exited at its single-instance check: another Zed holds the port; quit it and rerun".to_owned();
    }
    let stderr = fs::read_to_string(trial_dir.join("app.stderr.log")).unwrap_or_default();
    let tail: Vec<&str> = stderr.lines().rev().take(5).collect();
    if tail.is_empty() {
        error.to_owned()
    } else {
        let tail: Vec<&str> = tail.into_iter().rev().collect();
        format!("{error}; app stderr ends: {}", tail.join(" | "))
    }
}

fn check_visible(helpers: &Helpers, pid: i32) -> Result<WindowInfo, String> {
    match analysis::visibility(&helpers.window_list()?, pid, MIN_WINDOW_SIDE) {
        Visibility::Visible(window) => Ok(window),
        other => Err(other.describe()),
    }
}

/// Activation is cooperative, so it is requested and then verified; the
/// trial is refused rather than measured behind another app.
fn ensure_visible(helpers: &Helpers, pid: i32) -> Result<WindowInfo, String> {
    let mut last = String::new();
    for attempt in 0..ACTIVATION_ATTEMPTS {
        match check_visible(helpers, pid) {
            Ok(window) => return Ok(window),
            Err(reason) => last = reason,
        }
        let _ = helpers.activate(pid);
        thread::sleep(Duration::from_millis(500 + 250 * attempt));
    }
    Err(format!(
        "refusing to measure: {last}. Bring the window to the front, keep it uncovered and stay hands-off"
    ))
}

fn run_phase(
    context: &Context<'_>,
    phase: &Phase,
    pid: i32,
    sampler: &Sampler,
    start_seq: u64,
    trial_dir: &Path,
    driven: &mut Driven,
) -> Result<(), String> {
    match effective_action(context, phase) {
        Action::Idle => {
            let length = Duration::from_secs(u64::from(phase.seconds));
            hands_off(context, pid, phase.name, length, driven);
        }
        Action::UntilQuiet {
            quiet_core_pct,
            quiet_seconds,
            max_seconds,
        } => {
            let started = Instant::now();
            let window_ns = u64::from(quiet_seconds) * 1_000_000_000;
            while started.elapsed() < Duration::from_secs(u64::from(max_seconds))
                && still_visible(context, pid, phase.name, driven)
            {
                thread::sleep(VISIBILITY_POLL);
                let data = sampler.snapshot();
                let Some(timebase) = data.timebase else {
                    continue;
                };
                let points = analysis::cpu_points(&data.sets, start_seq, timebase);
                if analysis::quiet_since(&points, quiet_core_pct, window_ns).is_some() {
                    break;
                }
            }
        }
        action => run_input_phase(context, phase, action, pid, trial_dir, driven)?,
    }
    Ok(())
}

/// Records the first visibility failure as the reason the trial is invalid
/// and returns whether the trial can still be measured.
fn still_visible(context: &Context<'_>, pid: i32, phase: &str, driven: &mut Driven) -> bool {
    if driven.invalid.is_some() {
        return false;
    }
    match check_visible(context.helpers, pid) {
        Ok(_) => true,
        Err(reason) => {
            driven.invalid = Some(format!("{phase}: {reason}"));
            false
        }
    }
}

/// Waits `length` hands-off, checking the window once a second; stops early
/// once the trial is invalid.
fn hands_off(
    context: &Context<'_>,
    pid: i32,
    phase: &str,
    length: Duration,
    driven: &mut Driven,
) -> bool {
    let deadline = Instant::now() + length;
    loop {
        if !still_visible(context, pid, phase, driven) {
            return false;
        }
        let now = Instant::now();
        if now >= deadline {
            return true;
        }
        thread::sleep((deadline - now).min(VISIBILITY_POLL));
    }
}

/// The phase's action with the run's key spacing, when one was given.
fn effective_action(context: &Context<'_>, phase: &Phase) -> Action {
    context
        .request
        .key_interval_ms
        .map_or(phase.action, |interval| {
            phase.action.with_key_interval(interval)
        })
}

fn run_input_phase(
    context: &Context<'_>,
    phase: &Phase,
    action: Action,
    pid: i32,
    trial_dir: &Path,
    driven: &mut Driven,
) -> Result<(), String> {
    if !still_visible(context, pid, phase.name, driven) {
        return Ok(());
    }
    let window = check_visible(context.helpers, pid)?;
    if let Some(setup) = phase.setup {
        let log = trial_dir.join(format!("setup-{}.stderr.log", phase.name));
        context
            .helpers
            .run_input(&input_args(setup, pid, &window), &log)?;
        thread::sleep(Duration::from_millis(300));
    }
    let duration_ms = action.script_ms() + u64::from(phase.seconds) * 1_000 + 1_000;
    let (x, y, width, height) = analysis::capture_region(window.width, window.height);
    let region = [x, y, width, height].map(format_value).join(",");
    let capture_args: Vec<String> = [
        "run",
        "--pid",
        &pid.to_string(),
        "--window-id",
        &window.id.to_string(),
        "--duration-ms",
        &duration_ms.to_string(),
        "--region",
        &region,
    ]
    .iter()
    .map(ToString::to_string)
    .collect();
    let capture_log = trial_dir.join(format!("capture-{}.stderr.log", phase.name));
    let mut capture = context.helpers.spawn_capture(&capture_args, &capture_log)?;
    capture.wait_ready(Duration::from_secs(5))?;
    let input_log = trial_dir.join(format!("input-{}.stderr.log", phase.name));
    let mut input = context
        .helpers
        .spawn_input(&input_args(action, pid, &window), &input_log)?;
    let mut checked = Instant::now();
    let posted = loop {
        match input.finished() {
            Some(InputOutcome::Done(events)) => break events,
            Some(InputOutcome::LostForeground(reason)) => {
                driven
                    .invalid
                    .get_or_insert(format!("{}: {reason}", phase.name));
                break Vec::new();
            }
            Some(InputOutcome::Failed(error)) => return Err(error),
            None => {}
        }
        if checked.elapsed() >= VISIBILITY_POLL {
            checked = Instant::now();
            if !still_visible(context, pid, phase.name, driven) {
                input.stop();
                break Vec::new();
            }
        }
        thread::sleep(Duration::from_millis(20));
    };
    let tail = Duration::from_secs(u64::from(phase.seconds));
    if driven.invalid.is_some() || !hands_off(context, pid, phase.name, tail, driven) {
        // Dropping the capture stops it; an invalid trial keeps no frames.
        return Ok(());
    }
    let frames = capture.finish(Duration::from_millis(duration_ms + 10_000))?;
    driven.events.extend(
        posted
            .into_iter()
            .map(|event| (phase.name.to_owned(), event)),
    );
    driven.frames.extend(
        frames
            .into_iter()
            .map(|frame| (phase.name.to_owned(), frame)),
    );
    Ok(())
}

/// Arguments for `bench-input`; scrolls aim at the window's center.
fn input_args(action: Action, pid: i32, window: &WindowInfo) -> Vec<String> {
    let pid = pid.to_string();
    let args: Vec<String> = match action {
        Action::Type {
            phrase,
            count,
            interval_ms,
        } => vec![
            "type".into(),
            "--pid".into(),
            pid,
            "--text".into(),
            typed_text(phrase, count),
            "--interval-ms".into(),
            interval_ms.to_string(),
        ],
        Action::Keys {
            keycode,
            count,
            interval_ms,
        } => vec![
            "keys".into(),
            "--pid".into(),
            pid,
            "--keycode".into(),
            keycode.to_string(),
            "--count".into(),
            count.to_string(),
            "--interval-ms".into(),
            interval_ms.to_string(),
        ],
        Action::Scroll {
            pixels,
            count,
            interval_ms,
        } => {
            let center = format!(
                "{},{}",
                format_value((window.x + window.width / 2.0).round()),
                format_value((window.y + window.height / 2.0).round())
            );
            vec![
                "scroll".into(),
                "--pid".into(),
                pid,
                "--window-id".into(),
                window.id.to_string(),
                "--at".into(),
                center,
                "--pixels".into(),
                pixels.to_string(),
                "--count".into(),
                count.to_string(),
                "--interval-ms".into(),
                interval_ms.to_string(),
            ]
        }
        Action::Idle | Action::UntilQuiet { .. } => Vec::new(),
    };
    args
}

fn trial_metrics(
    context: &Context<'_>,
    data: &SamplerData,
    driven: &Driven,
) -> Result<Vec<Metric>, String> {
    if let Some(error) = data.errors.first() {
        return Err(format!("sampler output: {error}"));
    }
    let timebase = data.timebase.ok_or("the sampler reported no timebase")?;
    let mut metrics = analysis::trial_metrics(
        TRIAL_PHASE,
        &data.sets,
        &driven.appeared,
        driven.footprint_tool,
        timebase,
    );
    for window in analysis::phase_windows(&data.marks)? {
        let from_launch = window.name == STARTUP_PHASE;
        metrics.extend(analysis::phase_metrics(
            &window,
            &data.sets,
            timebase,
            from_launch,
        )?);
        let Some(phase) = context
            .request
            .workload
            .phases
            .iter()
            .find(|phase| phase.name == window.name)
        else {
            continue;
        };
        let events: Vec<InputEvent> = driven
            .events
            .iter()
            .filter(|(name, _)| name == phase.name)
            .map(|(_, event)| event.clone())
            .collect();
        let frames: Vec<Frame> = driven
            .frames
            .iter()
            .filter(|(name, _)| name == phase.name)
            .map(|(_, frame)| *frame)
            .collect();
        let action = effective_action(context, phase);
        match action {
            Action::Type { .. } | Action::Keys { .. } => {
                let spacing_ms = action.key_interval_ms().unwrap_or_default();
                let spacing = timebase.ticks(spacing_ms.saturating_mul(1_000_000));
                let latencies = analysis::key_latencies(&events, &frames, timebase, spacing);
                metrics.extend(analysis::latency_metrics(
                    phase.name,
                    &latencies,
                    events.len(),
                ));
            }
            Action::Scroll { .. } => {
                let start = events.first().map_or(window.start_mach, |event| event.mach);
                let end = events.last().map_or(window.end_mach, |event| {
                    event.mach + timebase.ticks(100_000_000)
                });
                let intervals = analysis::frame_intervals(&frames, start, end, timebase);
                metrics.extend(analysis::interval_metrics(
                    phase.name,
                    &intervals,
                    context.refresh_hz,
                ));
            }
            Action::Idle | Action::UntilQuiet { .. } => {}
        }
    }
    Ok(metrics)
}

fn trial_row(
    run_id: &str,
    number: u32,
    warmup: bool,
    reason: Option<&str>,
    pid: i32,
    window: Option<&Appeared>,
) -> Vec<String> {
    let mut row = vec![
        run_id.to_owned(),
        number.to_string(),
        u8::from(warmup).to_string(),
        u8::from(reason.is_none()).to_string(),
        reason.unwrap_or("").to_owned(),
        pid.to_string(),
    ];
    match window {
        Some(window) => row.extend([
            window.id.to_string(),
            format_value(window.x),
            format_value(window.y),
            format_value(window.width),
            format_value(window.height),
        ]),
        None => row.extend(std::iter::repeat_n(MISSING.to_owned(), 5)),
    }
    row
}

fn record(context: &Context<'_>, outcome: &TrialOutcome) -> Result<(), String> {
    let dir = &context.run_dir;
    let run_id = context.run_id.as_str();
    let trial = outcome.number.to_string();
    let valid = outcome.driven.invalid.is_none();
    tsv::append(
        &dir.join("trials.tsv"),
        TRIALS_HEADER,
        &[trial_row(
            run_id,
            outcome.number,
            outcome.warmup,
            outcome.driven.invalid.as_deref(),
            outcome.pid,
            Some(&outcome.driven.appeared),
        )],
    )?;
    let metric_rows: Vec<Vec<String>> = outcome
        .metrics
        .iter()
        .map(|metric| {
            vec![
                run_id.to_owned(),
                context.request.app.name().to_owned(),
                context.request.workload.name.to_owned(),
                trial.clone(),
                u8::from(outcome.warmup).to_string(),
                u8::from(valid).to_string(),
                metric.phase.clone(),
                metric.name.clone(),
                format_value(metric.value),
                metric.unit.to_owned(),
            ]
        })
        .collect();
    tsv::append(&dir.join("metrics.tsv"), METRICS_HEADER, &metric_rows)?;
    let sample_rows: Vec<Vec<String>> = outcome
        .sampler
        .sets
        .iter()
        .flat_map(|set| set.rows.iter().map(move |row| sample_row(set, row)))
        .map(|fields| prefixed(run_id, &trial, fields))
        .collect();
    tsv::append(&dir.join("samples.tsv"), SAMPLES_HEADER, &sample_rows)?;
    let mark_rows: Vec<Vec<String>> = outcome
        .sampler
        .marks
        .iter()
        .map(|mark| {
            let fields = vec![
                mark.seq.to_string(),
                mark.mach.to_string(),
                mark.label.clone(),
            ];
            prefixed(run_id, &trial, fields)
        })
        .collect();
    tsv::append(&dir.join("marks.tsv"), MARKS_HEADER, &mark_rows)?;
    record_input(context, outcome, &trial)
}

fn prefixed(run_id: &str, trial: &str, fields: Vec<String>) -> Vec<String> {
    let mut row = vec![run_id.to_owned(), trial.to_owned()];
    row.extend(fields);
    row
}

fn sample_row(set: &SampleSet, row: &ProcRow) -> Vec<String> {
    let kind = match set.kind {
        SetKind::Tick => "tick",
        SetKind::Mark => "mark",
    };
    let mut fields = vec![
        set.seq.to_string(),
        kind.to_owned(),
        set.mach.to_string(),
        row.pid.to_string(),
        row.ppid.to_string(),
        row.depth.to_string(),
        row.name.clone(),
    ];
    fields.extend(
        [
            row.footprint,
            row.lifetime_max_footprint,
            row.user_ns,
            row.system_ns,
            row.child_user_ns,
            row.child_system_ns,
            row.idle_wakeups,
            row.interrupt_wakeups,
            row.child_idle_wakeups,
            row.child_interrupt_wakeups,
            row.instructions,
            row.cycles,
            row.energy_nj,
            row.start_mach,
            u64::from(row.exited),
        ]
        .iter()
        .map(ToString::to_string),
    );
    fields
}

fn record_input(context: &Context<'_>, outcome: &TrialOutcome, trial: &str) -> Result<(), String> {
    let run_id = context.run_id.as_str();
    if !outcome.driven.events.is_empty() {
        let rows: Vec<Vec<String>> = outcome
            .driven
            .events
            .iter()
            .map(|(phase, event)| {
                let fields = vec![
                    phase.clone(),
                    event.seq.to_string(),
                    event.mach.to_string(),
                    event.kind.clone(),
                    event.label.clone(),
                ];
                prefixed(run_id, trial, fields)
            })
            .collect();
        tsv::append(&context.run_dir.join("events.tsv"), EVENTS_HEADER, &rows)?;
    }
    if !outcome.driven.frames.is_empty() {
        let rows: Vec<Vec<String>> = outcome
            .driven
            .frames
            .iter()
            .map(|(phase, frame)| {
                let fields = vec![
                    phase.clone(),
                    frame.seq.to_string(),
                    frame.display_mach.to_string(),
                    frame.arrival_mach.to_string(),
                    format!("{:x}", frame.hash),
                ];
                prefixed(run_id, trial, fields)
            })
            .collect();
        tsv::append(&context.run_dir.join("frames.tsv"), FRAMES_HEADER, &rows)?;
    }
    Ok(())
}

fn optional(value: Option<f64>) -> String {
    value.map_or_else(|| MISSING.to_owned(), format_value)
}

fn write_summary(context: &Context<'_>, kept: &[Metric]) -> Result<(), String> {
    let mut table = tsv::Table::new(SUMMARY_HEADER);
    for row in analysis::summarize(kept) {
        let summary = &row.summary;
        table.push(vec![
            context.run_id.clone(),
            context.request.app.name().to_owned(),
            context.request.workload.name.to_owned(),
            row.phase,
            row.name,
            row.unit.to_owned(),
            summary.n.to_string(),
            format_value(summary.mean),
            optional(summary.stddev),
            optional(summary.ci95.map(|ci| ci.0)),
            optional(summary.ci95.map(|ci| ci.1)),
            format_value(summary.median),
            format_value(summary.min),
            format_value(summary.max),
        ])?;
    }
    tsv::write(&context.run_dir.join("summary.tsv"), &table)
}

fn outcome_line(outcome: &TrialOutcome, total: u32) -> String {
    let find = |phase: &str, name: &str| {
        outcome
            .metrics
            .iter()
            .find(|metric| metric.phase == phase && metric.name == name)
            .map(|metric| metric.value)
    };
    let last_phase = outcome
        .metrics
        .iter()
        .rev()
        .find(|metric| metric.name == "cpu_time");
    let mut parts = vec![format!("trial {}/{total}", outcome.number)];
    if outcome.warmup {
        parts.push("warm-up".to_owned());
    }
    parts.push(match &outcome.driven.invalid {
        None => "valid".to_owned(),
        Some(reason) => format!("INVALID ({reason})"),
    });
    if let Some(launch) = find(TRIAL_PHASE, "launch_to_window") {
        parts.push(format!("window after {launch:.0} ms"));
    }
    if let Some(metric) = last_phase {
        let phase = metric.phase.as_str();
        parts.push(format!("{phase} cpu {:.1} ms", metric.value));
        if let Some(bytes) = find(phase, "footprint_app_p95") {
            parts.push(format!(
                "{phase} app footprint p95 {:.1} MiB",
                bytes / 1_048_576.0
            ));
        }
    }
    parts.join(", ")
}

#[cfg(test)]
mod tests {
    use super::{input_args, removable, trial_row};
    use crate::helpers::{Appeared, WindowInfo};
    use crate::workload::Action;
    use std::path::Path;

    fn window() -> WindowInfo {
        WindowInfo {
            order: 3,
            id: 77,
            pid: 9,
            owner: "app".to_owned(),
            layer: 0,
            alpha: 1.0,
            x: 276.0,
            y: 221.0,
            width: 960.0,
            height: 568.0,
        }
    }

    #[test]
    fn input_arguments_follow_the_workload_action() {
        let typed = input_args(
            Action::Type {
                phrase: "ab ",
                count: 5,
                interval_ms: 120,
            },
            9,
            &window(),
        );
        assert_eq!(
            typed,
            [
                "type",
                "--pid",
                "9",
                "--text",
                "ab ab",
                "--interval-ms",
                "120"
            ]
        );
        let keys = input_args(
            Action::Keys {
                keycode: 125,
                count: 3,
                interval_ms: 50,
            },
            9,
            &window(),
        );
        assert_eq!(keys[0], "keys");
        assert!(keys.windows(2).any(|pair| pair == ["--keycode", "125"]));
        let scroll = input_args(
            Action::Scroll {
                pixels: -66,
                count: 10,
                interval_ms: 8,
            },
            9,
            &window(),
        );
        assert!(scroll.windows(2).any(|pair| pair == ["--at", "756,505"]));
        assert!(scroll.windows(2).any(|pair| pair == ["--pixels", "-66"]));
        assert!(scroll.windows(2).any(|pair| pair == ["--window-id", "77"]));
        assert!(input_args(Action::Idle, 9, &window()).is_empty());
    }

    /// `bench-input` refuses to post without a target PID, so every script
    /// the orchestrator builds must name the measured app's PID.
    #[test]
    fn every_input_script_targets_the_measured_pid() {
        let actions = [
            Action::Type {
                phrase: "ab ",
                count: 3,
                interval_ms: 120,
            },
            Action::Keys {
                keycode: 125,
                count: 2,
                interval_ms: 120,
            },
            Action::Scroll {
                pixels: -66,
                count: 2,
                interval_ms: 8,
            },
        ];
        for action in actions {
            let args = input_args(action, 4242, &window());
            let targets: Vec<&[String]> =
                args.windows(2).filter(|pair| pair[0] == "--pid").collect();
            assert_eq!(targets.len(), 1, "{action:?}");
            assert_eq!(targets[0][1], "4242", "{action:?}");
        }
    }

    #[test]
    fn only_marked_children_of_the_home_root_are_deleted() {
        let root = Path::new("/tmp/alpine-bench");
        assert!(removable(&root.join("20261002T183005Z-01"), root, true));
        assert!(!removable(&root.join("20261002T183005Z-01"), root, false));
        assert!(!removable(root, root, true));
        assert!(!removable(Path::new("/Users/me"), root, true));
        assert!(!removable(&root.join("a/b"), root, true));
    }

    #[test]
    fn trial_rows_mark_refusals_invalid() {
        let refused = trial_row("r", 2, false, Some("refusing to measure"), 0, None);
        assert_eq!(refused[3], "0");
        assert_eq!(refused.len(), super::TRIALS_HEADER.len());
        let appeared = Appeared {
            mach: 1,
            id: 5,
            x: 1.0,
            y: 2.0,
            width: 960.0,
            height: 568.0,
        };
        let valid = trial_row("r", 1, true, None, 42, Some(&appeared));
        assert_eq!(valid[2], "1");
        assert_eq!(valid[3], "1");
        assert_eq!(valid[9], "960");
    }
}
