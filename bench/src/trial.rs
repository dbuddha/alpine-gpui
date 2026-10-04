//! One `bench run`: a fresh process and disposable home per trial, the
//! visibility gate before measuring, the workload's phases, cleanup of every
//! process the trial started, and raw TSV under `bench/results/<run>/`.

use crate::analysis::{self, Metric, Visibility};
use crate::apps::{self, App, Identity, LaunchInputs, RustAnalyzer};
use crate::fixtures::{self, FixtureFile};
use crate::gate::{self, Gate, Power, Sample};
use crate::helpers::{
    self, Appeared, Frame, Helpers, InputEvent, InputOutcome, ProcRow, SampleSet, Sampler,
    SamplerData, SetKind, Timebase, WindowInfo,
};
use crate::isolation::{self, Snapshot};
use crate::paths::{self, BenchPaths};
use crate::procs::{self, LaunchedApp, PsRow};
use crate::recorder_log::{self, FirstBuild};
use crate::stamp::{self, Stamp, StampInputs};
use crate::stats::{format_value, to_f64};
use crate::tsv::{self, MISSING};
use crate::workload::{
    Action, Document, Fixture, Language, Phase, STARTUP_PHASE, Servers, TRIAL_PHASE, Workload,
    typed_text,
};
use std::cell::RefCell;
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
    /// Minutes of owner idle each trial waits for and then keeps.
    pub wait_idle: Option<u32>,
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
/// Time for the palette to open, and to filter once the query is typed.
const PALETTE_PAUSE: Duration = Duration::from_millis(400);
/// Saving two small TSV files takes milliseconds.
const SAVE_TIMEOUT: Duration = Duration::from_secs(10);
/// The metric criterion 2.1's 100 ms budget applies to.
pub const FIRST_FRAME_METRIC: &str = "highlight_first_frame_ms";

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
    subjects: Vec<Subject>,
    identity: Identity,
    rust_analyzer: Option<RustAnalyzer>,
    refresh_hz: u32,
    /// The owner's real state for this app, compared around every trial.
    state_roots: Vec<PathBuf>,
    /// Set by `--wait-idle`: where every trial waits, and what voids it.
    gate: Option<Gate>,
    /// The real state as the last gate opened, or as the run began with no
    /// gate: what the end of the run is compared with.
    baseline: RefCell<Snapshot>,
    /// The measured app's identity as stamped; every gate that opens checks
    /// the install against it.
    installed: Vec<(String, String)>,
    effects: &'a dyn Effects,
}

impl Context<'_> {
    /// Waits at the gate before a trial (never, without `--wait-idle`). After
    /// a wait the owner may have used the app, so the baseline starts again;
    /// a gate that opens at once keeps it and the bench's late writes show.
    fn wait_at_gate(&self) -> Result<(), String> {
        let Some(gate) = &self.gate else {
            return Ok(());
        };
        let setup = (self.paths, self.helpers, self.request);
        let waited = wait_for_gate(self.effects, setup, gate, false)?;
        self.install_unchanged()?;
        if waited {
            self.retake_baseline();
        }
        Ok(())
    }

    /// Stops the run loudly if the measured app was replaced or rebuilt since
    /// the stamp, which a long wait makes possible.
    fn install_unchanged(&self) -> Result<(), String> {
        let now = stamp::app_entries(
            self.request.app,
            &self.paths.alpine_bundle,
            &self.paths.zed_app,
            &self.paths.helpers_dir,
        );
        match stamp::changed(&self.installed, &now) {
            None => Ok(()),
            Some(change) => Err(format!(
                "the {} install changed since the run was stamped ({change}); stop and inspect",
                self.request.app.name()
            )),
        }
    }

    fn retake_baseline(&self) {
        *self.baseline.borrow_mut() = isolation::snapshot(&self.state_roots);
    }
}

/// What a run does to the machine that a unit test must not: power and
/// process reads, sleeping, log lines, the display assertion and one trial.
trait Effects {
    fn power(&self) -> Result<Power, String>;
    fn processes(&self) -> Result<Vec<PsRow>, String>;
    fn sleep(&self, length: Duration);
    fn say(&self, line: &str);
    fn awake(&self) -> Result<gate::KeepAwake, String>;
    fn trial(
        &self,
        context: &Context<'_>,
        subject: &Subject,
        number: u32,
        warmup: bool,
    ) -> Result<TrialOutcome, String>;
}

/// The real machine.
struct Machine;

impl Effects for Machine {
    fn power(&self) -> Result<Power, String> {
        gate::power()
    }

    fn processes(&self) -> Result<Vec<PsRow>, String> {
        procs::ps()
    }

    fn sleep(&self, length: Duration) {
        thread::sleep(length);
    }

    fn say(&self, line: &str) {
        eprintln!("bench: {} {line}", stamp::utc(stamp::now_seconds()).0);
    }

    fn awake(&self) -> Result<gate::KeepAwake, String> {
        gate::KeepAwake::start()
    }

    fn trial(
        &self,
        context: &Context<'_>,
        subject: &Subject,
        number: u32,
        warmup: bool,
    ) -> Result<TrialOutcome, String> {
        run_trial(context, subject, number, warmup)
    }
}

/// What one launch opens. A language group's subject names its metrics.
struct Subject {
    language: Option<Language>,
    documents: Vec<PathBuf>,
}

impl Subject {
    fn label(&self) -> Option<&'static str> {
        self.language.map(Language::id)
    }

    /// Prefixes the group, so a refusal names the fixture it happened on.
    fn named(&self, text: &str) -> String {
        self.label()
            .map_or_else(|| text.to_owned(), |label| format!("{label}: {text}"))
    }
}

struct Driven {
    appeared: Appeared,
    invalid: Option<String>,
    events: Vec<(String, InputEvent)>,
    frames: Vec<(String, Frame)>,
    footprint_tool: Option<u64>,
    /// The startup build's duration from Alpine's recorder.
    first_build_ns: Option<u64>,
}

struct TrialOutcome {
    number: u32,
    warmup: bool,
    label: Option<&'static str>,
    pid: i32,
    driven: Driven,
    metrics: Vec<Metric>,
    sampler: SamplerData,
}

pub fn run(paths: &BenchPaths, request: &RunRequest) -> Result<PathBuf, String> {
    request.workload.check_app(request.app)?;
    let helpers = Helpers::locate(&paths.helpers_dir)?;
    run_with(paths, &helpers, request, &Machine)
}

fn run_with(
    paths: &BenchPaths,
    helpers: &Helpers,
    request: &RunRequest,
    effects: &dyn Effects,
) -> Result<PathBuf, String> {
    preflight(paths, helpers, request)?;
    let rust_analyzer = rust_analyzer(paths, request)?;
    let displays = helpers.displays()?;
    let refresh_hz = displays
        .iter()
        .find(|display| display.main)
        .map_or(120, |display| display.max_fps.max(1));
    // The first gate precedes the stamp and the real-state snapshot, so the
    // owner using the app or a new bundle during a long wait is not the run's
    // fault. The display assertion starts once it opens.
    let gate = gate_for(request);
    let _awake = match &gate {
        Some(gate) => {
            wait_for_gate(effects, (paths, helpers, request), gate, true)?;
            Some(effects.awake()?)
        }
        None => None,
    };
    let stamp = stamp::collect(&StampInputs {
        repo_root: &paths.repo_root,
        alpine_bundle: &paths.alpine_bundle,
        zed_app: &paths.zed_app,
        helpers_dir: &paths.helpers_dir,
        displays: &displays,
    });
    let installed = stamp.installed(request.app);
    let (_, tag) = stamp::utc(stamp::now_seconds());
    let run_id = format!("{tag}-{}-{}", request.workload.name, request.app.name());
    let run_dir = paths.results.join(&run_id);
    fs::create_dir_all(&run_dir)
        .map_err(|error| format!("create {}: {error}", run_dir.display()))?;
    let (subjects, fixtures) = subjects(paths, request.workload)?;
    let context = Context {
        paths,
        helpers,
        request,
        run_id,
        tag,
        run_dir,
        subjects,
        identity: paths::identity(),
        rust_analyzer,
        refresh_hz,
        state_roots: apps::real_state_paths(request.app, &paths.real_home, &paths.zed_app),
        gate,
        baseline: RefCell::new(Snapshot::new()),
        installed,
        effects,
    };
    write_run_table(&context, &stamp, &fixtures)?;
    context.retake_baseline();
    run_trials(&context)?;
    // A late cfprefsd write after the last trial still fails the run.
    effects.sleep(COOLDOWN);
    real_state_unchanged(&context, &context.baseline.borrow(), "the run")?;
    Ok(context.run_dir)
}

/// Prints what `run` would open, launch and post. It writes the fixtures but
/// starts no app, helper or input, so it needs no permission or window.
pub fn dry_run(paths: &BenchPaths, request: &RunRequest) -> Result<(), String> {
    request.workload.check_app(request.app)?;
    let rust_analyzer = rust_analyzer(paths, request)?;
    let (subjects, fixtures) = subjects(paths, request.workload)?;
    for line in plan(paths, request, &subjects, &fixtures, rust_analyzer.as_ref())? {
        println!("{line}");
    }
    Ok(())
}

/// The dry run's lines; `<pid>` and the `<tag>-<trial>` home stand for
/// values that only a real trial has.
fn plan(
    paths: &BenchPaths,
    request: &RunRequest,
    subjects: &[Subject],
    fixtures: &[FixtureFile],
    rust_analyzer: Option<&RustAnalyzer>,
) -> Result<Vec<String>, String> {
    let workload = request.workload;
    let warmup = request.warmup.unwrap_or(workload.warmup_trials);
    let mut lines = vec![format!(
        "dry run\t{} on {}: {} round(s) of {} launch(es), {warmup} warm-up; nothing starts",
        workload.name,
        request.app.name(),
        warmup + request.trials,
        subjects.len()
    )];
    if let Some(gate) = gate_for(request) {
        let binary = app_binary(paths, request.app);
        lines.extend(gate::plan_lines(&gate, &binary, VISIBILITY_POLL));
    }
    for file in fixtures {
        lines.push(format!(
            "fixture\t{}\t{} bytes\tfnv64={}",
            file.path.display(),
            file.bytes,
            file.fnv64
        ));
    }
    let root = paths.home_root.join("<tag>-<trial>");
    let (home, tmp, empty) = (root.join("home"), root.join("tmp"), root.join("empty-path"));
    let identity = paths::identity();
    let reference = paths.helpers_dir.join(helpers::REFERENCE);
    for subject in subjects {
        let spec = apps::launch_spec(&LaunchInputs {
            app: request.app,
            home: &home,
            tmpdir: &tmp,
            documents: &subject.documents,
            identity: &identity,
            rust_analyzer,
            path: (workload.servers == Servers::None).then_some(empty.as_path()),
            alpine_bundle: &paths.alpine_bundle,
            zed_app: &paths.zed_app,
            reference_binary: &reference,
        })?;
        let mut command = vec![spec.program.display().to_string()];
        command.extend(
            spec.args
                .iter()
                .map(|arg| arg.to_string_lossy().into_owned()),
        );
        let label = subject.label().unwrap_or("-");
        lines.push(format!("launch\t{label}\t{}", command.join(" ")));
        for (name, value) in &spec.env {
            lines.push(format!("env\t{label}\t{name}={}", value.to_string_lossy()));
        }
    }
    lines.push(format!(
        "{STARTUP_PHASE}\tsettle {} s once the window is frontmost and uncovered",
        workload.settle_seconds
    ));
    for phase in workload.phases {
        match effective_action(request, phase) {
            Action::SaveRecorderLog => {
                for args in recorder_log::save_steps("<pid>") {
                    lines.push(format!(
                        "{}\t{}\t{}",
                        phase.name,
                        helpers::INPUT,
                        args.join(" ")
                    ));
                }
                lines.push(format!(
                    "{}\twait for perf-<s>-samples.tsv in {}, then read perf-<s>-frames.tsv",
                    phase.name,
                    recorder_log::log_dir(&home).display()
                ));
            }
            action => lines.push(format!(
                "{}\t{action:?}, then {} s",
                phase.name, phase.seconds
            )),
        }
    }
    if workload.servers == Servers::None {
        lines.push("guard\ta child process of the app voids the trial".to_owned());
    }
    Ok(lines)
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
    let rounds = warmup + request.trials;
    let per_round = u32::try_from(context.subjects.len()).unwrap_or(u32::MAX);
    let total = rounds.saturating_mul(per_round);
    let scope = if per_round > 1 {
        format!(
            "{rounds} rounds of {per_round} language groups ({total} trials), {warmup} warm-up round(s)"
        )
    } else {
        format!("{total} trial(s), {warmup} warm-up")
    };
    eprintln!(
        "bench: {} on {}: {scope}. Hands off the keyboard and mouse; \
         a trial is refused if its window is not frontmost and uncovered.",
        request.workload.name,
        request.app.name()
    );
    context.effects.sleep(Duration::from_secs(3));
    let mut kept: Vec<Vec<Metric>> = Vec::new();
    let mut number = 0;
    // Each round opens every group once, so drift reaches all groups alike.
    for round in 0..rounds {
        let warm = round < warmup;
        for subject in &context.subjects {
            number += 1;
            if let Err(error) = context.wait_at_gate() {
                write_summary(context, &kept)?;
                return Err(format!(
                    "before trial {number}: {error}\nresults so far: {}",
                    context.run_dir.display()
                ));
            }
            match context.effects.trial(context, subject, number, warm) {
                Ok(outcome) => {
                    record(context, &outcome)?;
                    eprintln!("bench: {}", outcome_line(&outcome, total));
                    if !outcome.warmup && outcome.driven.invalid.is_none() {
                        kept.push(outcome.metrics);
                    }
                }
                Err(error) => {
                    let error = subject.named(&error);
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
                context.effects.sleep(COOLDOWN);
            }
        }
    }
    write_summary(context, &kept)?;
    println!("{}", context.run_dir.display());
    Ok(())
}

/// The executable a trial of `app` launches.
fn app_binary(paths: &BenchPaths, app: App) -> PathBuf {
    match app {
        App::Alpine => paths.alpine_bundle.join("Contents/MacOS/alpine-editor"),
        App::Zed => paths.zed_app.join("Contents/MacOS/zed"),
        App::AppKit => paths.helpers_dir.join(helpers::REFERENCE),
    }
}

fn gate_for(request: &RunRequest) -> Option<Gate> {
    request.wait_idle.map(|minutes| Gate {
        minutes,
        app: request.app.name(),
    })
}

fn preflight(paths: &BenchPaths, helpers: &Helpers, request: &RunRequest) -> Result<(), String> {
    let binary = app_binary(paths, request.app);
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

fn rust_analyzer(paths: &BenchPaths, request: &RunRequest) -> Result<Option<RustAnalyzer>, String> {
    if request.workload.servers != Servers::RustAnalyzer {
        return Ok(None);
    }
    paths::rust_analyzer(paths, request.rust_analyzer.as_deref()).map(Some)
}

/// One subject per launch in a round, and every fixture the run opens.
fn subjects(
    paths: &BenchPaths,
    workload: &Workload,
) -> Result<(Vec<Subject>, Vec<FixtureFile>), String> {
    let fixture = |fixture, language| -> Result<(Subject, FixtureFile), String> {
        let file = fixtures::ensure(&paths.fixtures_dir(), fixture)?;
        let documents = vec![file.path.clone()];
        Ok((
            Subject {
                language,
                documents,
            },
            file,
        ))
    };
    match workload.document {
        Document::Fixture(file) => {
            let (subject, file) = fixture(file, None)?;
            Ok((vec![subject], vec![file]))
        }
        Document::RepositoryFile(relative) => {
            let file = paths.repo_root.join(relative);
            if !file.is_file() {
                return Err(format!("{} is missing", file.display()));
            }
            let documents = vec![paths.repo_root.clone(), file];
            Ok((
                vec![Subject {
                    language: None,
                    documents,
                }],
                Vec::new(),
            ))
        }
        Document::Languages(languages) => languages
            .iter()
            .map(|language| fixture(Fixture::Source(*language), Some(*language)))
            .collect::<Result<Vec<_>, _>>()
            .map(|pairs| pairs.into_iter().unzip()),
    }
}

fn write_run_table(
    context: &Context<'_>,
    stamp: &Stamp,
    fixtures: &[FixtureFile],
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
        .subjects
        .iter()
        .flat_map(|subject| &subject.documents)
        .map(|path| path.display().to_string())
        .collect();
    push("documents", documents.join(" "))?;
    for file in fixtures {
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

    /// An empty directory for PATH, so the app finds no language server.
    fn empty_path(&self) -> Result<PathBuf, String> {
        let dir = self.root.join("empty-path");
        fs::create_dir(&dir).map_err(|error| format!("create {}: {error}", dir.display()))?;
        Ok(dir)
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

fn run_trial(
    context: &Context<'_>,
    subject: &Subject,
    number: u32,
    warmup: bool,
) -> Result<TrialOutcome, String> {
    let name = match subject.label() {
        Some(label) => format!("trial-{number:02}-{label}"),
        None => format!("trial-{number:02}"),
    };
    let trial_dir = context.run_dir.join(name);
    fs::create_dir_all(&trial_dir)
        .map_err(|error| format!("create {}: {error}", trial_dir.display()))?;
    let home = TrialHome::create(&context.paths.home_root, &context.tag, number)?;
    let empty_path = match context.request.workload.servers {
        Servers::None => Some(home.empty_path()?),
        Servers::Discovered | Servers::RustAnalyzer => None,
    };
    let reference = context.helpers.path(helpers::REFERENCE);
    let spec = apps::launch_spec(&LaunchInputs {
        app: context.request.app,
        home: &home.home,
        tmpdir: &home.tmp,
        documents: &subject.documents,
        identity: &context.identity,
        rust_analyzer: context.rust_analyzer.as_ref(),
        path: empty_path.as_deref(),
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
        Ok(_) => drive(context, &mut app, &mut sampler, &trial_dir, &home.home),
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
    let mut driven =
        driven.map_err(|error| format!("{error} (home kept at {})", home.root.display()))?;
    if context.request.workload.servers == Servers::None
        && let Some(evidence) = analysis::child_process_evidence(&data.sets)
    {
        driven
            .invalid
            .get_or_insert(format!("{evidence}, but this workload runs with no server"));
    }
    if let Some(reason) = driven.invalid.take() {
        driven.invalid = Some(subject.named(&reason));
    }
    let metrics = trial_metrics(context, subject, &data, &driven)?;
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
        label: subject.label(),
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
    home: &Path,
) -> Result<Driven, String> {
    let helpers = context.helpers;
    let workload = context.request.workload;
    let appeared = helpers
        .wait_for_window(app.pid(), WINDOW_TIMEOUT_MS)
        .map_err(|error| launch_failure(trial_dir, &error))?;
    let visible = ensure_visible(context, app.pid());
    let mut driven = Driven {
        appeared,
        invalid: None,
        events: Vec::new(),
        frames: Vec::new(),
        footprint_tool: None,
        first_build_ns: None,
    };
    if let Err(refusal) = visible {
        driven.invalid = Some(refusal_or_void(refusal, void_reason(context))?);
    }
    let settle = Duration::from_secs(u64::from(workload.settle_seconds));
    hands_off(context, app.pid(), STARTUP_PHASE, settle, &mut driven);
    sampler.mark(&analysis::phase_label(STARTUP_PHASE, "end"))?;
    let mut last_window = None;
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
            (trial_dir, home),
            &mut driven,
        )?;
        sampler.mark(&analysis::phase_label(phase.name, "end"))?;
        if app.exited() {
            return Err(format!("the app exited during {}", phase.name));
        }
        // The last poll can be a second old; this look is at the phase's end.
        if let Some(window) = check_trial(context, app.pid(), phase.name, &mut driven) {
            last_window = Some(window);
        }
    }
    settle_trial_frame(&mut driven, last_window);
    driven.footprint_tool = procs::footprint_bytes(app.pid())
        .map_err(|error| eprintln!("bench: footprint cross-check skipped: {error}"))
        .ok();
    Ok(driven)
}

/// Records the window as read after the last phase, since the first sighting
/// can land in the window-open animation, and voids the trial if that frame
/// is not the requested one.
fn settle_trial_frame(driven: &mut Driven, last_window: Option<WindowInfo>) {
    let Some(window) = last_window else {
        return;
    };
    let (settled, off) = analysis::settle_frame(driven.appeared, &window, apps::REQUESTED_FRAME);
    driven.appeared = settled;
    if let Some(reason) = off {
        driven.invalid.get_or_insert(reason);
    }
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

/// Waits at the `--wait-idle` gate, and returns whether it had to wait.
/// `first` is the run's first gate, which reports itself open even when it
/// did not have to wait.
fn wait_for_gate(
    effects: &dyn Effects,
    (paths, helpers, request): (&BenchPaths, &Helpers, &RunRequest),
    gate: &Gate,
    first: bool,
) -> Result<bool, String> {
    let binary = app_binary(paths, request.app)
        .to_string_lossy()
        .into_owned();
    gate::wait_to_start(
        gate,
        first,
        &mut || {
            let presence = helpers.presence()?;
            Ok(Sample {
                idle_seconds: presence.idle_seconds,
                locked: presence.locked,
                power: effects.power()?,
                running: procs::running(&effects.processes()?, &binary)
                    .iter()
                    .map(|row| row.pid)
                    .collect(),
            })
        },
        &mut || effects.sleep(gate::POLL),
        &mut |line| effects.say(line),
    )
}

/// Why a gated trial is void now: the owner touched the keyboard or mouse,
/// or the screen locked. A run without `--wait-idle` reads nothing and is
/// never void.
fn void_reason(context: &Context<'_>) -> Option<String> {
    let gate = context.gate.as_ref()?;
    gate::voided_by(gate, context.helpers.presence())
}

/// A refusal to measure stops the run, unless the owner or a lock caused it:
/// then the trial is void and the run goes on.
fn refusal_or_void(refusal: String, void: Option<String>) -> Result<String, String> {
    void.map_or(Err(refusal), |reason| {
        Ok(format!("{STARTUP_PHASE}: {reason}"))
    })
}

fn check_visible(helpers: &Helpers, pid: i32) -> Result<WindowInfo, String> {
    match analysis::visibility(&helpers.window_list()?, pid, MIN_WINDOW_SIDE) {
        Visibility::Visible(window) => Ok(window),
        other => Err(other.describe()),
    }
}

/// Activation is cooperative, so it is requested and then verified; the
/// trial is refused rather than measured behind another app. A gated run
/// reads the owner before each request: no fight for focus.
fn ensure_visible(context: &Context<'_>, pid: i32) -> Result<WindowInfo, String> {
    let helpers = context.helpers;
    let mut last = String::new();
    for attempt in 0..ACTIVATION_ATTEMPTS {
        match check_visible(helpers, pid) {
            Ok(window) => return Ok(window),
            Err(reason) => last = reason,
        }
        if void_reason(context).is_some() {
            break;
        }
        let _ = helpers.activate(pid);
        context
            .effects
            .sleep(Duration::from_millis(500 + 250 * attempt));
    }
    Err(format!(
        "refusing to measure: {last}. Bring the window to the front, keep it uncovered and stay hands-off"
    ))
}

/// `dirs` is the trial's output directory and the app's disposable HOME.
fn run_phase(
    context: &Context<'_>,
    phase: &Phase,
    pid: i32,
    sampler: &Sampler,
    start_seq: u64,
    dirs: (&Path, &Path),
    driven: &mut Driven,
) -> Result<(), String> {
    let (trial_dir, _) = dirs;
    match effective_action(context.request, phase) {
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
        Action::SaveRecorderLog => save_recorder_log(context, phase, pid, dirs, driven)?,
        action => {
            let timebase = sampler
                .snapshot()
                .timebase
                .ok_or("the sampler reported no timebase")?;
            run_input_phase(context, phase, action, pid, (trial_dir, timebase), driven)?;
        }
    }
    Ok(())
}

/// Records the first reason the trial is void (owner input or a lock, then a
/// window that is not frontmost and bare) and returns the window while it
/// stands.
fn check_trial(
    context: &Context<'_>,
    pid: i32,
    phase: &str,
    driven: &mut Driven,
) -> Option<WindowInfo> {
    if driven.invalid.is_some() {
        return None;
    }
    if let Some(reason) = void_reason(context) {
        context
            .effects
            .say(&format!("{phase}: {reason}; stopping the trial"));
        driven.invalid = Some(format!("{phase}: {reason}"));
        return None;
    }
    match check_visible(context.helpers, pid) {
        Ok(window) => Some(window),
        Err(reason) => {
            driven.invalid = Some(format!("{phase}: {reason}"));
            None
        }
    }
}

/// Whether the trial can still be measured, as `check_trial` finds it.
fn still_visible(context: &Context<'_>, pid: i32, phase: &str, driven: &mut Driven) -> bool {
    check_trial(context, pid, phase, driven).is_some()
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
fn effective_action(request: &RunRequest, phase: &Phase) -> Action {
    request.key_interval_ms.map_or(phase.action, |interval| {
        phase.action.with_key_interval(interval)
    })
}

fn run_input_phase(
    context: &Context<'_>,
    phase: &Phase,
    action: Action,
    pid: i32,
    (trial_dir, timebase): (&Path, Timebase),
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
    refuse_idle_input(driven, phase.name, action, (&posted, &frames), timebase);
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

/// Voids the trial if a finished input phase did nothing where the captured
/// frames can show it: a scroll must post events and leave the window in many
/// distinct states. A scroll that moved nothing failed; it was not fast.
fn refuse_idle_input(
    driven: &mut Driven,
    phase: &str,
    action: Action,
    (posted, frames): (&[InputEvent], &[Frame]),
    timebase: Timebase,
) {
    if let Action::Scroll { .. } = action
        && let Some(reason) = analysis::scroll_refusal(posted, frames, timebase)
    {
        driven.invalid.get_or_insert(format!("{phase}: {reason}"));
    }
}

/// Runs one bench-input script to its end. False means the trial became
/// invalid: the window lost the foreground or stopped being visible.
fn run_step(
    context: &Context<'_>,
    phase: &str,
    pid: i32,
    args: &[String],
    log: &Path,
    driven: &mut Driven,
) -> Result<bool, String> {
    let mut input = context.helpers.spawn_input(args, log)?;
    let mut checked = Instant::now();
    loop {
        match input.finished() {
            Some(InputOutcome::Done(_)) => return Ok(true),
            Some(InputOutcome::LostForeground(reason)) => {
                driven.invalid.get_or_insert(format!("{phase}: {reason}"));
                return Ok(false);
            }
            Some(InputOutcome::Failed(error)) => return Err(error),
            None => {}
        }
        if checked.elapsed() >= VISIBILITY_POLL {
            checked = Instant::now();
            if !still_visible(context, pid, phase, driven) {
                input.stop();
                return Ok(false);
            }
        }
        thread::sleep(Duration::from_millis(20));
    }
}

/// Saves Alpine's recorder from the palette, keeps the frames file with the
/// trial and reads the startup build from it.
fn save_recorder_log(
    context: &Context<'_>,
    phase: &Phase,
    pid: i32,
    (trial_dir, home): (&Path, &Path),
    driven: &mut Driven,
) -> Result<(), String> {
    let pid_text = pid.to_string();
    for (index, args) in recorder_log::save_steps(&pid_text).iter().enumerate() {
        if !still_visible(context, pid, phase.name, driven) {
            return Ok(());
        }
        let log = trial_dir.join(format!("input-{}-{}.stderr.log", phase.name, index + 1));
        if !run_step(context, phase.name, pid, args, &log, driven)? {
            return Ok(());
        }
        thread::sleep(PALETTE_PAUSE);
    }
    let dir = recorder_log::log_dir(home);
    let started = Instant::now();
    let frames = loop {
        if let Some(saved) = recorder_log::saved_frames(&dir)? {
            break saved;
        }
        if started.elapsed() >= SAVE_TIMEOUT {
            return Err(format!(
                "the palette saved no performance log in {} within {} s; if `/usr/bin/log show \
                 --last 5m --predicate 'process == \"WindowServer\" AND eventMessage CONTAINS \
                 \"prohibited\"'` lists any event, WindowServer dropped the input",
                dir.display(),
                SAVE_TIMEOUT.as_secs()
            ));
        }
        thread::sleep(Duration::from_millis(100));
    };
    fs::copy(&frames, trial_dir.join("recorder-frames.tsv"))
        .map_err(|error| format!("copy {}: {error}", frames.display()))?;
    let text = fs::read_to_string(&frames)
        .map_err(|error| format!("read {}: {error}", frames.display()))?;
    match recorder_log::first_build(&recorder_log::parse_frames(&text)?) {
        FirstBuild::Startup { build_ns } => driven.first_build_ns = Some(build_ns),
        FirstBuild::Refused(reason) => {
            driven
                .invalid
                .get_or_insert(format!("{}: {reason}", phase.name));
        }
    }
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
        Action::Idle | Action::UntilQuiet { .. } | Action::SaveRecorderLog => Vec::new(),
    };
    args
}

fn trial_metrics(
    context: &Context<'_>,
    subject: &Subject,
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
        let action = effective_action(context.request, phase);
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
                let span = analysis::scroll_span(&events, timebase)
                    .unwrap_or((window.start_mach, window.end_mach));
                metrics.extend(analysis::scroll_metrics(
                    phase.name,
                    &frames,
                    span,
                    (timebase, context.refresh_hz),
                ));
            }
            Action::SaveRecorderLog => {
                if let Some(ns) = driven.first_build_ns {
                    let group = subject.label().unwrap_or(phase.name);
                    metrics.push(Metric::new(
                        group,
                        FIRST_FRAME_METRIC,
                        to_f64(ns) / 1e6,
                        "ms",
                    ));
                }
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

fn write_summary(context: &Context<'_>, kept: &[Vec<Metric>]) -> Result<(), String> {
    let mut table = tsv::Table::new(SUMMARY_HEADER);
    for row in analysis::summarize_trials(kept) {
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
    let mut parts = vec![match outcome.label {
        Some(label) => format!("trial {}/{total} ({label})", outcome.number),
        None => format!("trial {}/{total}", outcome.number),
    }];
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
    if let Some(metric) = outcome
        .metrics
        .iter()
        .find(|metric| metric.name == FIRST_FRAME_METRIC)
    {
        parts.push(format!("first frame built in {:.2} ms", metric.value));
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
    use super::{
        Driven, RunRequest, Subject, app_binary, gate_for, input_args, plan, refusal_or_void,
        refuse_idle_input, removable, settle_trial_frame, trial_row,
    };
    use crate::apps::{App, DOCK_PATH};
    use crate::helpers::{Appeared, Frame, InputEvent, Timebase, WindowInfo};
    use crate::paths::BenchPaths;
    use crate::workload::{Action, Language, find};
    use std::path::{Path, PathBuf};

    fn bench_paths() -> BenchPaths {
        BenchPaths {
            repo_root: PathBuf::from("/repo"),
            helpers_dir: PathBuf::from("/repo/bench/target/helpers"),
            results: PathBuf::from("/repo/bench/results"),
            baseline: PathBuf::from("/repo/bench/baseline.tsv"),
            home_root: PathBuf::from("/tmp/alpine-bench"),
            real_home: PathBuf::from("/Users/me"),
            alpine_bundle: PathBuf::from("/Users/me/Applications/Alpine Editor.app"),
            zed_app: PathBuf::from("/Applications/Zed.app"),
        }
    }

    fn request(workload: &str) -> Result<RunRequest, String> {
        Ok(RunRequest {
            workload: find(workload)?,
            app: App::Alpine,
            trials: 10,
            warmup: None,
            keep_homes: false,
            rust_analyzer: None,
            key_interval_ms: None,
            wait_idle: None,
        })
    }

    #[test]
    fn the_highlight_dry_run_shows_an_empty_path_and_the_palette_keys() -> Result<(), String> {
        let subjects = [Language::Cpp, Language::Java].map(|language| Subject {
            language: Some(language),
            documents: vec![PathBuf::from(format!(
                "/f/highlight-20000.{}",
                language.id()
            ))],
        });
        let lines = plan(&bench_paths(), &request("highlight")?, &subjects, &[], None)?;
        let has = |text: &str| lines.iter().any(|line| line == text);
        assert!(
            lines[0].contains("11 round(s) of 2 launch(es), 1 warm-up"),
            "{lines:?}"
        );
        assert!(has(
            "env\tcpp\tPATH=/tmp/alpine-bench/<tag>-<trial>/empty-path"
        ));
        assert!(has(
            "env\tjava\tPATH=/tmp/alpine-bench/<tag>-<trial>/empty-path"
        ));
        assert!(lines.iter().any(|line| line.starts_with("launch\tjava\t")
            && line.ends_with("alpine-editor /f/highlight-20000.java")));
        assert!(has(
            "save\tbench-input\tchord --pid <pid> --keycode 35 --modifiers command,shift"
        ));
        assert!(has(
            "save\tbench-input\ttype --pid <pid> --text save performance log --interval-ms 60"
        ));
        assert!(has(
            "save\tbench-input\tkeys --pid <pid> --keycode 36 --count 1"
        ));
        assert!(has("guard\ta child process of the app voids the trial"));
        let idle = Subject {
            language: None,
            documents: vec![PathBuf::from("/f/idle-200.txt")],
        };
        let lines = plan(&bench_paths(), &request("idle")?, &[idle], &[], None)?;
        assert!(
            lines.contains(&format!("env\t-\tPATH={DOCK_PATH}")),
            "{lines:?}"
        );
        assert!(!lines.iter().any(|line| line.starts_with("guard")));
        Ok(())
    }

    /// The only difference `--wait-idle` makes to a dry run is the gate lines.
    #[test]
    fn the_dry_run_prints_the_gate_only_with_wait_idle() -> Result<(), String> {
        let idle = Subject {
            language: None,
            documents: vec![PathBuf::from("/f/idle-200.txt")],
        };
        let subjects = [idle];
        let plain = plan(&bench_paths(), &request("idle")?, &subjects, &[], None)?;
        assert!(
            !plain.iter().any(|line| line.starts_with("gate")),
            "{plain:?}"
        );
        let mut gated_request = request("idle")?;
        gated_request.wait_idle = Some(10);
        let gated = plan(&bench_paths(), &gated_request, &subjects, &[], None)?;
        let (gate_lines, rest): (Vec<_>, Vec<_>) = gated
            .iter()
            .cloned()
            .partition(|line| line.starts_with("gate\t"));
        assert_eq!(rest, plain);
        let text = gate_lines.join("\n");
        assert!(text.contains("for 10 min"), "{text}");
        assert!(text.contains("on AC power"), "{text}");
        assert!(
            text.contains("Alpine Editor.app/Contents/MacOS/alpine-editor"),
            "{text}"
        );
        assert!(text.contains("owner input"), "{text}");
        assert!(text.contains("caffeinate -d -i -w <bench pid>"), "{text}");
        assert!(!text.contains(" -u"), "{text}");
        Ok(())
    }

    #[test]
    fn the_gate_exists_only_for_a_run_that_asks() -> Result<(), String> {
        assert_eq!(gate_for(&request("idle")?), None);
        let mut asked = request("idle")?;
        asked.wait_idle = Some(15);
        let gate = gate_for(&asked).ok_or("no gate for --wait-idle 15")?;
        assert_eq!((gate.minutes, gate.app), (15, "alpine"));
        Ok(())
    }

    #[test]
    fn each_app_has_the_binary_the_gate_looks_for() {
        let paths = bench_paths();
        assert_eq!(
            app_binary(&paths, App::Alpine),
            Path::new("/Users/me/Applications/Alpine Editor.app/Contents/MacOS/alpine-editor")
        );
        assert_eq!(
            app_binary(&paths, App::Zed),
            Path::new("/Applications/Zed.app/Contents/MacOS/zed")
        );
        assert_eq!(
            app_binary(&paths, App::AppKit),
            Path::new("/repo/bench/target/helpers/bench-reference-appkit")
        );
    }

    /// A refusal to measure stops the run; the owner's doing, or a lock, only
    /// voids the trial.
    #[test]
    fn a_refusal_the_owner_caused_voids_the_trial_and_any_other_stops_the_run() {
        let refusal = "refusing to measure: Mail is frontmost".to_owned();
        assert_eq!(refusal_or_void(refusal.clone(), None), Err(refusal.clone()));
        assert_eq!(
            refusal_or_void(
                refusal.clone(),
                Some("owner input (idle 3.0 s < 60 s)".to_owned())
            ),
            Ok("startup: owner input (idle 3.0 s < 60 s)".to_owned())
        );
        assert_eq!(
            refusal_or_void(refusal, Some("screen locked".to_owned())),
            Ok("startup: screen locked".to_owned())
        );
    }

    #[test]
    fn a_language_subject_names_its_group() {
        let java = Subject {
            language: Some(Language::Java),
            documents: Vec::new(),
        };
        assert_eq!(java.named("refused"), "java: refused");
        let plain = Subject {
            language: None,
            documents: Vec::new(),
        };
        assert_eq!(plain.named("refused"), "refused");
    }

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

    fn posted(count: u64) -> Vec<InputEvent> {
        (1..=count)
            .map(|seq| InputEvent {
                seq,
                mach: 100 + seq,
                kind: "scroll".to_owned(),
                label: "-66".to_owned(),
            })
            .collect()
    }

    /// `count` frames from the first event on, cycling through `states` hashes.
    fn frames_of(count: u64, states: u64) -> Vec<Frame> {
        (0..count)
            .map(|index| Frame {
                seq: index,
                display_mach: 100 + index,
                arrival_mach: 100 + index,
                hash: index % states,
            })
            .collect()
    }

    /// The reference editor's first sighting on 2026-10-03, mid-animation.
    fn opening() -> Appeared {
        Appeared {
            mach: 9,
            id: 5,
            x: 284.0,
            y: 133.0,
            width: 944.0,
            height: 562.0,
        }
    }

    fn driven() -> Driven {
        Driven {
            appeared: opening(),
            invalid: None,
            events: Vec::new(),
            frames: Vec::new(),
            footprint_tool: None,
            first_build_ns: None,
        }
    }

    /// A tick is a nanosecond, so a scroll's span ends 100 ms after its last
    /// event.
    const UNIT: Timebase = Timebase { numer: 1, denom: 1 };

    fn scroll() -> Action {
        Action::Scroll {
            pixels: -66,
            count: 3_334,
            interval_ms: 8,
        }
    }

    fn keys() -> Action {
        Action::Keys {
            keycode: 125,
            count: 3_334,
            interval_ms: 8,
        }
    }

    /// What the refusal leaves in `invalid` for a phase named "scroll".
    fn verdict(action: Action, posted: &[InputEvent], frames: &[Frame]) -> Option<String> {
        let mut trial = driven();
        refuse_idle_input(&mut trial, "scroll", action, (posted, frames), UNIT);
        trial.invalid
    }

    #[test]
    fn a_scroll_that_left_the_window_unchanged_voids_the_trial() {
        let events = posted(3_334);
        // The 2026-10-03 shape: thousands of frames, two states.
        let reason = verdict(scroll(), &events, &frames_of(3_300, 2)).unwrap_or_default();
        assert!(reason.starts_with("scroll: "), "{reason}");
        assert!(reason.contains("2 distinct states"), "{reason}");
        assert!(reason.contains("did not land"), "{reason}");
        // A scroll that changed the window on every frame passes.
        assert_eq!(verdict(scroll(), &events, &frames_of(3_300, 3_300)), None);
    }

    #[test]
    fn the_minimum_follows_the_events_posted_not_the_frames_shown() {
        // 3,334 events need 66 states. 30 states in 500 frames are too few,
        // though 500 frames alone would only ask for 20.
        let reason = verdict(scroll(), &posted(3_334), &frames_of(500, 30)).unwrap_or_default();
        assert!(reason.contains("30 distinct states"), "{reason}");
        assert!(reason.contains("fewer than the 66"), "{reason}");
    }

    #[test]
    fn a_state_before_the_first_event_does_not_count() {
        let events = posted(3_334);
        let before = Frame {
            seq: 0,
            display_mach: 50,
            arrival_mach: 50,
            hash: 9_999,
        };
        // The states shown from the first event on, plus one shown before it.
        let with_extra = |states: u64| [vec![before], frames_of(3_300, states)].concat();
        let reason = verdict(scroll(), &events, &with_extra(65)).unwrap_or_default();
        assert!(reason.contains("65 distinct states"), "{reason}");
        assert_eq!(verdict(scroll(), &events, &with_extra(66)), None);
    }

    #[test]
    fn a_scroll_that_posted_nothing_voids_the_trial_but_a_key_run_does_not() {
        let frames = frames_of(3_300, 3_300);
        let reason = verdict(scroll(), &[], &frames).unwrap_or_default();
        assert!(reason.starts_with("scroll: "), "{reason}");
        assert!(reason.contains("posted no scroll events"), "{reason}");
        // Frames that never changed say nothing about a key press.
        assert_eq!(verdict(keys(), &posted(3_334), &frames_of(3_300, 2)), None);
        assert_eq!(verdict(keys(), &[], &frames), None);
    }

    #[test]
    fn the_first_reason_stands() {
        let mut trial = driven();
        trial.invalid = Some("scroll: lost the foreground".to_owned());
        let still = frames_of(3_300, 2);
        refuse_idle_input(
            &mut trial,
            "scroll",
            scroll(),
            (&posted(3_334), &still),
            UNIT,
        );
        refuse_idle_input(&mut trial, "scroll", scroll(), (&[], &still), UNIT);
        assert_eq!(
            trial.invalid.as_deref(),
            Some("scroll: lost the foreground")
        );
    }

    #[test]
    fn a_trial_records_its_settled_frame_and_a_wrong_one_voids_it() {
        let frame_of = |x, y, width, height| WindowInfo {
            x,
            y,
            width,
            height,
            ..window()
        };
        // The reference editor settles to the request after its opening frame.
        let mut reference = driven();
        settle_trial_frame(&mut reference, Some(frame_of(276.0, 128.0, 960.0, 572.0)));
        assert_eq!(reference.invalid, None);
        let row = trial_row("r", 1, false, None, 42, Some(&reference.appeared));
        assert_eq!(row[7..11], ["276", "128", "960", "572"]);
        // Zed at its default size is recorded as it is, and voids the trial.
        let mut zed = driven();
        settle_trial_frame(&mut zed, Some(frame_of(13.0, 41.0, 1486.0, 933.0)));
        let reason = zed.invalid.clone().unwrap_or_default();
        assert!(reason.contains("1486x933"), "{reason}");
        let row = trial_row(
            "r",
            2,
            false,
            zed.invalid.as_deref(),
            42,
            Some(&zed.appeared),
        );
        assert_eq!(row[3], "0");
        assert_eq!(row[7..11], ["13", "41", "1486", "933"]);
        // The opening frame itself would have failed the same gate.
        let mut mid_animation = driven();
        settle_trial_frame(
            &mut mid_animation,
            Some(frame_of(284.0, 133.0, 944.0, 562.0)),
        );
        assert!(mid_animation.invalid.is_some());
        // A trial that read no window keeps its first sighting.
        let mut unread = driven();
        settle_trial_frame(&mut unread, None);
        assert_eq!((unread.appeared, unread.invalid), (opening(), None));
        // The first reason stands.
        let mut earlier = driven();
        earlier.invalid = Some("scroll: lost the foreground".to_owned());
        settle_trial_frame(&mut earlier, Some(frame_of(13.0, 41.0, 1486.0, 933.0)));
        assert_eq!(
            earlier.invalid.as_deref(),
            Some("scroll: lost the foreground")
        );
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

#[cfg(test)]
mod wiring;
