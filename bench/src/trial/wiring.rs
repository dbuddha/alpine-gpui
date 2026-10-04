//! The gate's wiring through `run_with`, `run_trials`, `drive` and
//! `ensure_visible`, run against shell stand-ins for the helpers and a
//! stand-in machine: nothing GUI runs and nothing sleeps.

use super::{
    Context, Driven, Effects, RunRequest, Subject, TrialOutcome, app_binary, check_trial, drive,
    ensure_visible, run_trials, run_with, still_visible, wait_for_gate,
};
use crate::apps::{App, LaunchSpec};
use crate::gate::{Gate, KeepAwake, Power};
use crate::helpers::{Appeared, Helpers, Sampler, SamplerData};
use crate::isolation::Snapshot;
use crate::paths::BenchPaths;
use crate::procs::{LaunchedApp, PsRow};
use crate::stamp;
use crate::workload::{Action, Document, Fixture, Phase, Servers, Workload};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::ffi::OsString;
use std::fmt::Write as _;
use std::fs::{self, File, OpenOptions};
use std::io::Write as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// A workload with no waiting, so a trial's phases take no time.
static QUICK: Workload = Workload {
    name: "quick",
    summary: "no settle and one empty phase, for wiring tests",
    document: Document::Fixture(Fixture::Idle),
    settle_seconds: 0,
    phases: &[Phase {
        name: "idle",
        seconds: 0,
        action: Action::Idle,
        setup: None,
    }],
    warmup_trials: 0,
    servers: Servers::Discovered,
    alpine_only: false,
};

const ONE_MINUTE: Gate = Gate {
    minutes: 1,
    app: "appkit",
};

/// Logs each subcommand, then answers `idle` from the records beside it (a
/// pair of lines per call, the last repeating), `list` and `wait` from their
/// files; no idle records fail it as an unreadable clock does.
const WINDOW: &str = r#"#!/bin/sh
echo "$1" >> "$0.calls"
case "$1" in
  idle)
    n=$(/usr/bin/grep -c '^idle$' "$0.calls")
    out=$(/usr/bin/sed -n "$((2 * n - 1)),$((2 * n))p" "$0.idle" 2>/dev/null)
    [ -n "$out" ] || out=$(/usr/bin/tail -n 2 "$0.idle" 2>/dev/null)
    [ -n "$out" ] && printf '%s\n' "$out"
    ;;
  list)
    f="$0.list"
    if /usr/bin/grep -q '^activate$' "$0.calls" && [ -f "$0.list.after" ]; then
      f="$0.list.after"
    fi
    cat "$f"
    ;;
  activate) printf 'activate\t1\n' ;;
  wait) cat "$0.wait" ;;
  display) printf 'display\t1\t1\t1\t1512\t982\t3024\t1964\t120\t120\n' ;;
  now) printf 'now\t1\t1\t1\n' ;;
esac
"#;

/// Echoes every mark the way `bench-sample` does.
const SAMPLER: &str = r#"#!/bin/sh
printf 'timebase\t1\t1\n'
n=0
while read -r label; do
  n=$((n + 1))
  printf 'mark\t%s\t%s\t%s\n' "$n" "$n" "$label"
done
printf 'end\t%s\t%s\tstdin-closed\n' "$n" "$n"
"#;

/// A `caffeinate` that sleeps in its place.
const AWAKE: &str = "#!/bin/sh\nexec /bin/sleep 600\n";

fn write(path: &Path, text: &str, mode: u32) -> Result<(), String> {
    fs::write(path, text).map_err(|error| format!("{}: {error}", path.display()))?;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .map_err(|error| format!("{}: {error}", path.display()))
}

fn alive(pid: &str) -> bool {
    std::process::Command::new("/bin/kill")
        .args(["-0", pid])
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// A scratch bench: its paths, stand-in helpers and the run request.
struct Rig {
    dir: PathBuf,
    paths: BenchPaths,
    helpers: Helpers,
    request: RunRequest,
}

impl Rig {
    fn new(name: &str, trials: u32, wait_idle: Option<u32>) -> Result<Self, String> {
        let dir = crate::test_dir(name)?;
        let paths = BenchPaths {
            repo_root: dir.join("repo"),
            helpers_dir: dir.join("helpers"),
            results: dir.join("results"),
            baseline: dir.join("baseline.tsv"),
            home_root: dir.join("homes"),
            real_home: dir.join("real-home"),
            alpine_bundle: dir.join("Alpine Editor.app"),
            zed_app: dir.join("Zed.app"),
        };
        let preferences = paths.real_home.join("Library/Preferences");
        for path in [
            &paths.repo_root,
            &paths.helpers_dir,
            &paths.results,
            &preferences,
        ] {
            fs::create_dir_all(path).map_err(|error| format!("{}: {error}", path.display()))?;
        }
        let bin = &paths.helpers_dir;
        write(&bin.join("bench-window"), WINDOW, 0o755)?;
        write(&bin.join("bench-sample"), SAMPLER, 0o755)?;
        write(&bin.join("caffeinate"), AWAKE, 0o755)?;
        for other in ["bench-input", "bench-capture", "bench-reference-appkit"] {
            write(&bin.join(other), "#!/bin/sh\nexit 0\n", 0o755)?;
        }
        let wait = "appeared\t1\t77\t276\t128\t960\t572\n";
        write(&bin.join("bench-window.wait"), wait, 0o644)?;
        let helpers = Helpers::locate(bin)?;
        let request = RunRequest {
            workload: &QUICK,
            app: App::AppKit,
            trials,
            warmup: Some(0),
            keep_homes: false,
            rust_analyzer: None,
            key_interval_ms: None,
            wait_idle,
        };
        let rig = Self {
            dir,
            paths,
            helpers,
            request,
        };
        rig.list(4242, 4242)?;
        Ok(rig)
    }

    fn window(&self, suffix: &str) -> PathBuf {
        self.paths
            .helpers_dir
            .join(format!("bench-window.{suffix}"))
    }

    /// What `bench-window idle` answers on its first, second, ... call, as
    /// if the mouse had moved last.
    fn idle(&self, records: &[(f64, bool)]) -> Result<(), String> {
        let mut text = String::new();
        for (seconds, locked) in records {
            let _ = writeln!(
                text,
                "idle\t{seconds:.3}\tmouseMoved\nlocked\t{}",
                u8::from(*locked)
            );
        }
        write(&self.window("idle"), &text, 0o644)
    }

    /// Like `idle`, with the input type behind each reading.
    fn idle_by(&self, records: &[(f64, &str, bool)]) -> Result<(), String> {
        let mut text = String::new();
        for (seconds, input, locked) in records {
            let locked = u8::from(*locked);
            let _ = writeln!(text, "idle\t{seconds:.3}\t{input}\nlocked\t{locked}");
        }
        write(&self.window("idle"), &text, 0o644)
    }

    fn listing(front: i32, owner: i32) -> String {
        format!("front\t{front}\tapp\nwindow\t0\t77\t{owner}\tapp\t0\t1\t276\t128\t960\t572\n")
    }

    /// `front` is the frontmost process; `owner` has the 960x572 window.
    fn list(&self, front: i32, owner: i32) -> Result<(), String> {
        write(&self.window("list"), &Self::listing(front, owner), 0o644)
    }

    /// The listing once the app has been activated.
    fn list_after_activation(&self, front: i32, owner: i32) -> Result<(), String> {
        write(
            &self.window("list.after"),
            &Self::listing(front, owner),
            0o644,
        )
    }

    /// Every subcommand `bench-window` ran and every effect, in order.
    fn calls(&self) -> Vec<String> {
        fs::read_to_string(self.window("calls"))
            .unwrap_or_default()
            .lines()
            .map(ToOwned::to_owned)
            .collect()
    }

    /// The calls without the gate's power and process reads.
    fn events(&self) -> Vec<String> {
        let mut calls = self.calls();
        calls.retain(|line| line != "@power" && line != "@ps");
        calls
    }

    fn context<'a>(
        &'a self,
        gate: Option<Gate>,
        effects: &'a dyn Effects,
    ) -> Result<Context<'a>, String> {
        let run_dir = self.dir.join("run");
        fs::create_dir_all(&run_dir).map_err(|error| format!("{}: {error}", run_dir.display()))?;
        Ok(Context {
            paths: &self.paths,
            helpers: &self.helpers,
            request: &self.request,
            run_id: "r".to_owned(),
            tag: "t".to_owned(),
            run_dir,
            subjects: vec![Subject {
                language: None,
                documents: vec![self.dir.join("idle-200.txt")],
            }],
            identity: crate::paths::identity(),
            rust_analyzer: None,
            refresh_hz: 120,
            state_roots: Vec::new(),
            gate,
            baseline: RefCell::new(Snapshot::new()),
            installed: stamp::app_entries(
                self.request.app,
                &self.paths.alpine_bundle,
                &self.paths.zed_app,
                &self.paths.helpers_dir,
            ),
            effects,
        })
    }
}

fn next<T: Clone>(queue: &RefCell<VecDeque<T>>) -> Option<T> {
    let mut queue = queue.borrow_mut();
    if queue.len() > 1 {
        queue.pop_front()
    } else {
        queue.front().cloned()
    }
}

/// A stand-in machine. Each effect is appended, `@`-marked, to the stand-in
/// `bench-window`'s call log, so one list orders reads, sleeps, log lines,
/// the display assertion and trials. Scripted values repeat their last.
struct Fake {
    log: PathBuf,
    awake_script: PathBuf,
    binary: String,
    powers: RefCell<VecDeque<Power>>,
    running: RefCell<VecDeque<Vec<i32>>>,
    on_sleep: Box<dyn Fn()>,
    voided: Vec<(u32, String)>,
    awake_pid: Cell<Option<u32>>,
}

impl Fake {
    fn new(rig: &Rig) -> Self {
        Self {
            log: rig.window("calls"),
            awake_script: rig.paths.helpers_dir.join("caffeinate"),
            binary: app_binary(&rig.paths, App::AppKit).display().to_string(),
            powers: RefCell::new(VecDeque::from([Power::Ac])),
            running: RefCell::new(VecDeque::from([Vec::new()])),
            on_sleep: Box::new(|| {}),
            voided: Vec::new(),
            awake_pid: Cell::new(None),
        }
    }

    fn powers(self, powers: Vec<Power>) -> Self {
        Self {
            powers: RefCell::new(powers.into()),
            ..self
        }
    }

    fn running(self, running: Vec<Vec<i32>>) -> Self {
        Self {
            running: RefCell::new(running.into()),
            ..self
        }
    }

    fn on_sleep(self, hook: impl Fn() + 'static) -> Self {
        Self {
            on_sleep: Box::new(hook),
            ..self
        }
    }

    fn voiding(mut self, trial: u32, reason: &str) -> Self {
        self.voided.push((trial, reason.to_owned()));
        self
    }

    fn note(&self, text: &str) {
        if let Ok(mut log) = OpenOptions::new().create(true).append(true).open(&self.log) {
            let _ = writeln!(log, "@{text}");
        }
    }

    /// The pid of the stand-in `caffeinate`, once one was started.
    fn awake_pid(&self) -> Option<String> {
        self.awake_pid.get().map(|pid| pid.to_string())
    }
}

impl Effects for Fake {
    fn power(&self) -> Result<Power, String> {
        self.note("power");
        next(&self.powers).ok_or_else(|| "no scripted power".to_owned())
    }

    fn processes(&self) -> Result<Vec<PsRow>, String> {
        self.note("ps");
        let pids = next(&self.running).ok_or("no scripted processes")?;
        Ok(pids
            .into_iter()
            .map(|pid| PsRow {
                pid,
                ppid: 1,
                command: format!("{} --flag", self.binary),
            })
            .collect())
    }

    fn sleep(&self, length: Duration) {
        self.note(&format!("sleep {}ms", length.as_millis()));
        (self.on_sleep)();
    }

    fn say(&self, line: &str) {
        self.note(&format!("say {line}"));
    }

    fn awake(&self) -> Result<KeepAwake, String> {
        self.note("awake");
        let guard = KeepAwake::start_at(&self.awake_script)?;
        self.awake_pid.set(Some(guard.pid()));
        Ok(guard)
    }

    fn trial(
        &self,
        _context: &Context<'_>,
        _subject: &Subject,
        number: u32,
        warmup: bool,
    ) -> Result<TrialOutcome, String> {
        let awake = self.awake_pid().is_some_and(|pid| alive(&pid));
        self.note(&format!("trial {number} awake={awake}"));
        let invalid = self
            .voided
            .iter()
            .find(|(trial, _)| *trial == number)
            .map(|(_, reason)| reason.clone());
        Ok(TrialOutcome {
            number,
            warmup,
            label: None,
            pid: 4242,
            driven: Driven {
                invalid,
                ..driven()
            },
            metrics: Vec::new(),
            sampler: SamplerData::default(),
        })
    }
}

/// A trial that has just been launched.
fn driven() -> Driven {
    Driven {
        appeared: Appeared {
            mach: 1,
            id: 77,
            x: 276.0,
            y: 128.0,
            width: 960.0,
            height: 572.0,
        },
        invalid: None,
        events: Vec::new(),
        frames: Vec::new(),
        footprint_tool: None,
        first_build_ns: None,
    }
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

const OPEN_AWAY: &str =
    "@say gate open: owner idle 66m40s of 1m, AC power, screen unlocked, appkit not running";

// ---- the gate's reads ----

/// One condition fails per poll (idle, the lock, power, another copy of the
/// app), so a condition that is not read opens the gate early and shortens
/// the list of five-second sleeps.
#[test]
fn every_gate_condition_is_read_and_each_poll_waits_five_seconds() -> Result<(), String> {
    let rig = Rig::new("gate-conditions", 1, Some(1))?;
    let records = [
        (3.0, false),
        (4000.0, true),
        (4000.0, false),
        (4000.0, false),
        (4000.0, false),
    ];
    rig.idle(&records)?;
    let battery = Power::Other("Battery Power".to_owned());
    let fake = Fake::new(&rig)
        .powers(vec![Power::Ac, Power::Ac, battery, Power::Ac])
        .running(vec![vec![], vec![], vec![], vec![812], vec![]]);
    wait_for_gate(
        &fake,
        (&rig.paths, &rig.helpers, &rig.request),
        &ONE_MINUTE,
        true,
    )?;
    assert_eq!(
        rig.events(),
        strings(&[
            "idle",
            "@say gate waiting for owner idle (0m03s of 1m)",
            "@sleep 5000ms",
            "idle",
            "@sleep 5000ms",
            "idle",
            "@sleep 5000ms",
            "idle",
            "@sleep 5000ms",
            "idle",
            OPEN_AWAY,
        ])
    );
    Ok(())
}

// ---- the run ----

/// The first gate says it is open even when it did not wait, the display
/// assertion starts after it and lasts through every trial, and the
/// per-trial gates that do not wait say nothing.
#[test]
fn the_first_gate_says_it_is_open_and_the_display_is_held_for_every_trial() -> Result<(), String> {
    let rig = Rig::new("first-gate", 2, Some(1))?;
    rig.idle(&[(4000.0, false)])?;
    let fake = Fake::new(&rig);
    run_with(&rig.paths, &rig.helpers, &rig.request, &fake)?;
    assert_eq!(
        rig.events(),
        strings(&[
            "display",
            "idle",
            OPEN_AWAY,
            "@awake",
            "now",
            "@sleep 3000ms",
            "idle",
            "@trial 1 awake=true",
            "@sleep 3000ms",
            "idle",
            "@trial 2 awake=true",
            "@sleep 3000ms",
        ])
    );
    let pid = fake.awake_pid().ok_or("no display assertion was started")?;
    assert!(!alive(&pid), "the display assertion outlived the run");
    Ok(())
}

/// The reference editor's preferences file, which the run guards.
fn preferences(rig: &Rig) -> PathBuf {
    rig.paths
        .real_home
        .join("Library/Preferences/bench-reference-appkit.plist")
}

/// A sleep hook that runs `action` on the `nth` sleep.
fn on_nth_sleep(nth: u32, action: impl Fn() + 'static) -> impl Fn() + 'static {
    let sleeps = Cell::new(0);
    move || {
        sleeps.set(sleeps.get() + 1);
        if sleeps.get() == nth {
            action();
        }
    }
}

fn stamp_text(hash: &str) -> String {
    format!(
        "swiftc\tApple Swift version 6.1 (swiftlang)\n\
         source\treference-appkit/ReferenceEditor.swift\t{hash}0000\n"
    )
}

/// The owner uses the app during the first wait and the bundle is replaced
/// during the second. Neither may fail the run or leave the old stamp.
#[test]
fn a_long_wait_does_not_age_the_stamp_or_the_real_state_snapshot() -> Result<(), String> {
    let rig = Rig::new("stamp-after-gate", 1, Some(1))?;
    rig.idle(&[(3.0, false), (4.0, false), (4000.0, false)])?;
    let preferences = preferences(&rig);
    let stamp = rig.paths.helpers_dir.join("stamp.tsv");
    write(&preferences, "state before the wait", 0o644)?;
    write(&stamp, &stamp_text("aaaaaaaaaaaa"), 0o644)?;
    let waits = Cell::new(0);
    let (used, rebuilt) = (preferences, stamp);
    let fake = Fake::new(&rig).on_sleep(move || {
        waits.set(waits.get() + 1);
        if waits.get() == 1 {
            let _ = fs::write(&used, "state after the owner used the app");
        }
        if waits.get() == 2 {
            let _ = fs::write(&rebuilt, stamp_text("bbbbbbbbbbbb"));
        }
    });
    let run_dir = run_with(&rig.paths, &rig.helpers, &rig.request, &fake)?;
    let table =
        fs::read_to_string(run_dir.join("run.tsv")).map_err(|error| format!("run.tsv: {error}"))?;
    assert!(table.contains("src-bbbbbbbbbbbb"), "{table}");
    assert!(!table.contains("src-aaaaaaaaaaaa"), "{table}");
    Ok(())
}

/// The owner uses the app while trial 2's gate waits. Every trial is fine,
/// so the run is too: the baseline starts again when a gate opens.
#[test]
fn owner_use_of_the_app_while_a_later_gate_waits_does_not_fail_the_run() -> Result<(), String> {
    let rig = Rig::new("baseline-retaken", 2, Some(1))?;
    // The first gates open at once; trial 2's finds the owner back.
    rig.idle(&[
        (4000.0, false),
        (4000.0, false),
        (3.0, false),
        (4000.0, false),
    ])?;
    let used = preferences(&rig);
    write(&used, "state before the run", 0o644)?;
    // Sleeps: the banner, the cooldown after trial 1, trial 2's gate poll.
    let owner = on_nth_sleep(3, move || {
        let _ = fs::write(&used, "state after the owner used the app");
    });
    let fake = Fake::new(&rig).on_sleep(owner);
    run_with(&rig.paths, &rig.helpers, &rig.request, &fake).map(drop)
}

/// A write after the last gate, here in the final cooldown, still fails.
#[test]
fn a_late_write_after_the_last_gate_still_fails_a_gated_run() -> Result<(), String> {
    let rig = Rig::new("baseline-late-write", 2, Some(1))?;
    rig.idle(&[(4000.0, false)])?;
    let late = preferences(&rig);
    write(&late, "state before the run", 0o644)?;
    // Sleeps: the banner, the cooldown after trial 1, the final cooldown.
    let write_late = on_nth_sleep(3, move || {
        let _ = fs::write(&late, "a late write");
    });
    let fake = Fake::new(&rig).on_sleep(write_late);
    let failure = run_with(&rig.paths, &rig.helpers, &rig.request, &fake)
        .err()
        .ok_or("a late write went unnoticed")?;
    assert!(
        failure.contains("REAL appkit STATE CHANGED during the run"),
        "{failure}"
    );
    Ok(())
}

/// The app is replaced during the cooldown after trial 1. Every gate opening
/// compares the install with the stamp, so the run stops before trial 2.
#[test]
fn a_replaced_app_stops_a_gated_run_at_the_next_gate() -> Result<(), String> {
    let rig = Rig::new("install-replaced", 3, Some(1))?;
    rig.idle(&[(4000.0, false)])?;
    let stamp = rig.paths.helpers_dir.join("stamp.tsv");
    write(&stamp, &stamp_text("aaaaaaaaaaaa"), 0o644)?;
    // Sleeps: the banner, then the cooldown after trial 1.
    let rebuilt = on_nth_sleep(2, move || {
        let _ = fs::write(&stamp, stamp_text("bbbbbbbbbbbb"));
    });
    let fake = Fake::new(&rig).on_sleep(rebuilt);
    let failure = run_with(&rig.paths, &rig.helpers, &rig.request, &fake)
        .err()
        .ok_or("a replaced app went unnoticed")?;
    assert!(failure.contains("before trial 2"), "{failure}");
    assert!(failure.contains("appkit_version"), "{failure}");
    assert!(failure.contains("src-aaaaaaaaaaaa"), "{failure}");
    assert!(failure.contains("src-bbbbbbbbbbbb"), "{failure}");
    let trials: Vec<String> = rig
        .calls()
        .into_iter()
        .filter(|call| call.starts_with("@trial"))
        .collect();
    assert_eq!(trials, ["@trial 1 awake=true"]);
    Ok(())
}

/// A gate that opens at once leaves the baseline alone, so a write in the
/// cooldown between gated trials (the bench's own late write, say) still
/// fails the run, as it does without a gate.
#[test]
fn a_write_between_gated_trials_still_fails_when_every_gate_opens_at_once() -> Result<(), String> {
    let rig = Rig::new("baseline-gap-write", 2, Some(1))?;
    rig.idle(&[(4000.0, false)])?;
    let touched = preferences(&rig);
    write(&touched, "state before the run", 0o644)?;
    // Sleeps: the banner, then the cooldown between the trials.
    let between = on_nth_sleep(2, move || {
        let _ = fs::write(&touched, "a write between the trials");
    });
    let fake = Fake::new(&rig).on_sleep(between);
    let failure = run_with(&rig.paths, &rig.helpers, &rig.request, &fake)
        .err()
        .ok_or("a write between the trials went unnoticed")?;
    assert!(
        failure.contains("REAL appkit STATE CHANGED during the run"),
        "{failure}"
    );
    Ok(())
}

/// State the run never touches passes, so the baseline has to exist from the
/// start: without a gate nothing else would take it.
#[test]
fn a_run_that_leaves_the_real_state_alone_passes_with_and_without_a_gate() -> Result<(), String> {
    for (name, wait_idle) in [("state-alone-plain", None), ("state-alone-gated", Some(1))] {
        let rig = Rig::new(name, 2, wait_idle)?;
        rig.idle(&[(4000.0, false)])?;
        write(&preferences(&rig), "state before the run", 0o644)?;
        run_with(&rig.paths, &rig.helpers, &rig.request, &Fake::new(&rig))?;
    }
    Ok(())
}

/// With no gate there is one baseline, taken as the run began.
#[test]
fn a_run_without_wait_idle_keeps_its_one_baseline() -> Result<(), String> {
    let rig = Rig::new("baseline-ungated", 2, None)?;
    let touched = preferences(&rig);
    write(&touched, "state before the run", 0o644)?;
    // Sleeps: the banner, then the cooldown between the trials.
    let between = on_nth_sleep(2, move || {
        let _ = fs::write(&touched, "a write between the trials");
    });
    let fake = Fake::new(&rig).on_sleep(between);
    let failure = run_with(&rig.paths, &rig.helpers, &rig.request, &fake)
        .err()
        .ok_or("a write between the trials went unnoticed")?;
    assert!(
        failure.contains("REAL appkit STATE CHANGED during the run"),
        "{failure}"
    );
    Ok(())
}

#[test]
fn a_run_without_wait_idle_reads_no_clock_and_holds_no_display() -> Result<(), String> {
    let rig = Rig::new("ungated-run", 2, None)?;
    let fake = Fake::new(&rig);
    run_with(&rig.paths, &rig.helpers, &rig.request, &fake)?;
    assert_eq!(
        rig.calls(),
        strings(&[
            "display",
            "now",
            "@sleep 3000ms",
            "@trial 1 awake=false",
            "@sleep 3000ms",
            "@trial 2 awake=false",
            "@sleep 3000ms",
        ])
    );
    Ok(())
}

// ---- the trial loop ----

/// Every trial waits at the gate first. The owner is back after trial 1, so
/// trial 2 waits, and the voided trial 1 neither stops the run nor is
/// repeated.
#[test]
fn every_trial_waits_at_the_gate_and_a_voided_trial_does_not_stop_the_run() -> Result<(), String> {
    let rig = Rig::new("trial-loop", 3, Some(1))?;
    rig.idle(&[(4000.0, false), (3.0, false), (4000.0, false)])?;
    let fake = Fake::new(&rig).voiding(1, "typing: owner input (mouseMoved 3.0 s < 60 s)");
    let context = rig.context(Some(ONE_MINUTE), &fake)?;
    run_trials(&context)?;
    assert_eq!(
        rig.events(),
        strings(&[
            "@sleep 3000ms",
            "idle",
            "@trial 1 awake=false",
            "@sleep 3000ms",
            "idle",
            "@say gate waiting for owner idle (0m03s of 1m)",
            "@sleep 5000ms",
            "idle",
            OPEN_AWAY,
            "@trial 2 awake=false",
            "@sleep 3000ms",
            "idle",
            "@trial 3 awake=false",
        ])
    );
    let trials = fs::read_to_string(context.run_dir.join("trials.tsv"))
        .map_err(|error| format!("trials.tsv: {error}"))?;
    let rows: Vec<Vec<&str>> = trials
        .lines()
        .map(|line| line.split('\t').collect())
        .collect();
    assert_eq!(rows.len(), 4, "{trials}");
    assert_eq!(
        (rows[1][3], rows[1][4]),
        ("0", "typing: owner input (mouseMoved 3.0 s < 60 s)")
    );
    assert_eq!((rows[2][3], rows[3][3]), ("1", "1"));
    Ok(())
}

// ---- one trial ----

/// Starts a stand-in app and its sampler, for `drive`.
fn start_trial(rig: &Rig) -> Result<(LaunchedApp, Sampler), String> {
    let spec = LaunchSpec {
        program: PathBuf::from("/bin/sleep"),
        args: vec![OsString::from("600")],
        env: Vec::new(),
        seed_files: Vec::new(),
    };
    let log =
        |name: &str| File::create(rig.dir.join(name)).map_err(|error| format!("{name}: {error}"));
    let logs = (log("app.stdout.log")?, log("app.stderr.log")?);
    let marker = rig.dir.to_string_lossy();
    let app = LaunchedApp::spawn(&spec, &rig.dir, logs, &marker)?;
    let sampler = rig
        .helpers
        .spawn_sampler(app.pid(), &rig.dir.join("sampler.stderr.log"))?;
    Ok((app, sampler))
}

fn finish_trial(mut app: LaunchedApp, sampler: Sampler) -> Result<(), String> {
    sampler.finish().map(drop)?;
    app.stop(&[], Duration::from_secs(2));
    Ok(())
}

/// Runs `drive` on a stand-in app; returns what it returned and the calls.
fn driven_by(
    rig: &Rig,
    gate: Option<Gate>,
    fake: &Fake,
    app: &mut LaunchedApp,
    sampler: &mut Sampler,
) -> Result<Result<Driven, String>, String> {
    let context = rig.context(gate, fake)?;
    Ok(drive(&context, app, sampler, &rig.dir, &rig.dir))
}

/// The owner is back before the window is in front: the trial is void, the
/// run goes on, and the app is never activated against the owner.
#[test]
fn a_startup_refusal_the_owner_caused_voids_the_trial_without_taking_focus() -> Result<(), String> {
    let rig = Rig::new("drive-owner-back", 1, Some(1))?;
    let (mut app, mut sampler) = start_trial(&rig)?;
    rig.list(99, app.pid())?;
    rig.idle(&[(3.0, false)])?;
    let fake = Fake::new(&rig);
    let driven = driven_by(&rig, Some(ONE_MINUTE), &fake, &mut app, &mut sampler)?;
    let reason = driven?.invalid;
    assert_eq!(
        reason.as_deref(),
        Some("startup: owner input (mouseMoved 3.0 s < 60 s)")
    );
    assert_eq!(rig.calls(), strings(&["wait", "list", "idle", "idle"]));
    finish_trial(app, sampler)
}

/// With no owner involved, a refusal still stops the run, after the five
/// requests that activation gets.
#[test]
fn a_startup_refusal_nobody_caused_still_stops_the_run() -> Result<(), String> {
    let rig = Rig::new("drive-refused", 1, None)?;
    let (mut app, mut sampler) = start_trial(&rig)?;
    rig.list(99, app.pid())?;
    let fake = Fake::new(&rig);
    let driven = driven_by(&rig, None, &fake, &mut app, &mut sampler)?;
    let refusal = driven.err().ok_or("a refused window was accepted")?;
    assert!(refusal.contains("refusing to measure"), "{refusal}");
    let mut expected = vec!["wait".to_owned()];
    for delay in ["500", "750", "1000", "1250", "1500"] {
        expected.extend(strings(&["list", "activate"]));
        expected.push(format!("@sleep {delay}ms"));
    }
    assert_eq!(rig.calls(), expected);
    finish_trial(app, sampler)
}

/// Input in the last second of a phase is caught by the look at its end.
#[test]
fn the_look_at_the_end_of_a_phase_reads_the_owner() -> Result<(), String> {
    let rig = Rig::new("drive-phase-end", 1, Some(1))?;
    let (mut app, mut sampler) = start_trial(&rig)?;
    rig.list(app.pid(), app.pid())?;
    rig.idle(&[(4000.0, false), (4000.0, false), (3.0, false)])?;
    let fake = Fake::new(&rig);
    let driven = driven_by(&rig, Some(ONE_MINUTE), &fake, &mut app, &mut sampler)?;
    let reason = driven?.invalid;
    assert_eq!(
        reason.as_deref(),
        Some("idle: owner input (mouseMoved 3.0 s < 60 s)")
    );
    assert_eq!(
        rig.calls(),
        strings(&[
            "wait",
            "list",
            "idle",
            "list",
            "idle",
            "list",
            "idle",
            "@say idle: owner input (mouseMoved 3.0 s < 60 s); stopping the trial",
        ])
    );
    finish_trial(app, sampler)
}

/// A lock that starts mid-trial voids it as locked, and `drive` returns the
/// void trial so the run re-gates instead of stopping.
#[test]
fn a_lock_that_starts_mid_trial_voids_the_trial_as_locked() -> Result<(), String> {
    let rig = Rig::new("drive-locks", 1, Some(1))?;
    let (mut app, mut sampler) = start_trial(&rig)?;
    rig.list(app.pid(), app.pid())?;
    rig.idle(&[(4000.0, false), (4000.0, false), (4000.0, true)])?;
    let fake = Fake::new(&rig);
    let driven = driven_by(&rig, Some(ONE_MINUTE), &fake, &mut app, &mut sampler)?;
    assert_eq!(driven?.invalid.as_deref(), Some("idle: screen locked"));
    finish_trial(app, sampler)
}

/// The same trial with the owner away all the way through is valid.
#[test]
fn a_trial_the_owner_leaves_alone_stands_and_reads_the_clock_three_times() -> Result<(), String> {
    let rig = Rig::new("drive-valid", 1, Some(1))?;
    let (mut app, mut sampler) = start_trial(&rig)?;
    rig.list(app.pid(), app.pid())?;
    rig.idle(&[(4000.0, false)])?;
    let fake = Fake::new(&rig);
    let driven = driven_by(&rig, Some(ONE_MINUTE), &fake, &mut app, &mut sampler)?;
    assert_eq!(driven?.invalid, None);
    assert_eq!(
        rig.calls(),
        strings(&[
            "wait", "list", "idle", "list", "idle", "list", "idle", "list"
        ])
    );
    finish_trial(app, sampler)
}

// ---- focus ----

/// The owner comes back after the first request: no second request is made.
#[test]
fn activation_stops_the_moment_the_owner_is_back() -> Result<(), String> {
    let rig = Rig::new("focus-owner", 1, Some(1))?;
    rig.list(99, 4242)?;
    rig.idle(&[(4000.0, false), (3.0, false)])?;
    let fake = Fake::new(&rig);
    let context = rig.context(Some(ONE_MINUTE), &fake)?;
    let refusal = ensure_visible(&context, 4242).err().ok_or("accepted")?;
    assert!(refusal.contains("refusing to measure"), "{refusal}");
    assert_eq!(
        rig.calls(),
        strings(&["list", "idle", "activate", "@sleep 500ms", "list", "idle"])
    );
    Ok(())
}

/// With the owner away it activates five times, waiting longer each time.
#[test]
fn an_owner_who_stays_away_gets_five_activations_and_then_a_refusal() -> Result<(), String> {
    let rig = Rig::new("focus-away", 1, Some(1))?;
    rig.list(99, 4242)?;
    rig.idle(&[(4000.0, false)])?;
    let fake = Fake::new(&rig);
    let context = rig.context(Some(ONE_MINUTE), &fake)?;
    assert!(ensure_visible(&context, 4242).is_err());
    let mut expected = Vec::new();
    for delay in ["500", "750", "1000", "1250", "1500"] {
        expected.extend(strings(&["list", "idle", "activate"]));
        expected.push(format!("@sleep {delay}ms"));
    }
    assert_eq!(rig.calls(), expected);
    Ok(())
}

#[test]
fn a_window_that_comes_to_the_front_after_a_request_is_accepted() -> Result<(), String> {
    let rig = Rig::new("focus-comes", 1, Some(1))?;
    rig.list(99, 4242)?;
    rig.list_after_activation(4242, 4242)?;
    rig.idle(&[(4000.0, false)])?;
    let fake = Fake::new(&rig);
    let context = rig.context(Some(ONE_MINUTE), &fake)?;
    let window = ensure_visible(&context, 4242)?;
    assert_eq!(window.id, 77);
    assert_eq!(
        rig.calls(),
        strings(&["list", "idle", "activate", "@sleep 500ms", "list"])
    );
    Ok(())
}

// ---- the trial's checks ----

/// The idle clock is read at every check of a gated trial and never in a
/// run without `--wait-idle`, whose trials stand as they always did. The
/// abort is logged at once.
#[test]
fn owner_input_voids_a_gated_trial_at_once_and_a_plain_run_never_reads_it() -> Result<(), String> {
    let rig = Rig::new("check-owner-input", 1, Some(1))?;
    rig.idle_by(&[(3.0, "scrollWheel", false)])?;
    let fake = Fake::new(&rig);
    let gated = rig.context(Some(ONE_MINUTE), &fake)?;
    let mut trial = driven();
    assert!(check_trial(&gated, 4242, "typing", &mut trial).is_none());
    assert_eq!(
        trial.invalid.as_deref(),
        Some("typing: owner input (scrollWheel 3.0 s < 60 s)")
    );
    // The trial is void, so the window is not even listed.
    assert_eq!(
        rig.calls(),
        strings(&[
            "idle",
            "@say typing: owner input (scrollWheel 3.0 s < 60 s); stopping the trial"
        ])
    );
    let plain = rig.context(None, &fake)?;
    let mut trial = driven();
    assert!(check_trial(&plain, 4242, "typing", &mut trial).is_some());
    assert_eq!(trial.invalid, None);
    assert_eq!(rig.calls().last().map(String::as_str), Some("list"));
    assert_eq!(rig.calls().iter().filter(|call| *call == "idle").count(), 1);
    // The 1 Hz poll reaches the same verdicts.
    assert!(still_visible(&plain, 4242, "typing", &mut trial));
    assert!(!still_visible(&gated, 4242, "typing", &mut driven()));
    Ok(())
}

#[test]
fn a_gated_trial_stands_while_the_owner_is_away_and_the_window_is_bare() -> Result<(), String> {
    let rig = Rig::new("check-away", 1, Some(1))?;
    rig.idle(&[(640.25, false)])?;
    let fake = Fake::new(&rig);
    let gated = rig.context(Some(ONE_MINUTE), &fake)?;
    let mut trial = driven();
    let window = check_trial(&gated, 4242, "typing", &mut trial);
    assert_eq!(window.map(|window| window.id), Some(77));
    assert_eq!(trial.invalid, None);
    assert_eq!(rig.calls(), strings(&["idle", "list"]));
    // A window that is not frontmost still voids it, with its own reason.
    let mut elsewhere = driven();
    assert!(check_trial(&gated, 99, "typing", &mut elsewhere).is_none());
    let reason = elsewhere.invalid.unwrap_or_default();
    assert!(reason.starts_with("typing: "), "{reason}");
    assert!(!reason.contains("owner input"), "{reason}");
    Ok(())
}

#[test]
fn a_locked_screen_voids_a_gated_trial_as_locked_before_the_window_is_looked_at()
-> Result<(), String> {
    let rig = Rig::new("check-locked", 1, Some(1))?;
    rig.idle(&[(4000.0, true)])?;
    let fake = Fake::new(&rig);
    let gated = rig.context(Some(ONE_MINUTE), &fake)?;
    let mut trial = driven();
    assert!(check_trial(&gated, 4242, "typing", &mut trial).is_none());
    assert_eq!(trial.invalid.as_deref(), Some("typing: screen locked"));
    assert_eq!(
        rig.calls(),
        strings(&["idle", "@say typing: screen locked; stopping the trial"])
    );
    Ok(())
}

/// A helper built before the per-type clocks names no input type. It reads
/// the any-input clock that screen capture resets, so it is not trusted.
#[test]
fn a_helper_that_names_no_input_type_is_refused_with_the_rebuild_hint() -> Result<(), String> {
    let rig = Rig::new("check-old-helper", 1, Some(1))?;
    write(&rig.window("idle"), "idle\t4000.000\nlocked\t0\n", 0o644)?;
    let fake = Fake::new(&rig);
    let gated = rig.context(Some(ONE_MINUTE), &fake)?;
    let mut trial = driven();
    assert!(check_trial(&gated, 4242, "typing", &mut trial).is_none());
    let reason = trial.invalid.unwrap_or_default();
    assert!(
        reason.starts_with("typing: cannot read the idle time"),
        "{reason}"
    );
    assert!(reason.contains("no input type"), "{reason}");
    assert!(reason.contains("bench/helpers/build.sh"), "{reason}");
    Ok(())
}

#[test]
fn an_unreadable_clock_voids_a_gated_trial_and_the_first_reason_stands() -> Result<(), String> {
    let rig = Rig::new("check-no-clock", 1, Some(1))?;
    let fake = Fake::new(&rig);
    let gated = rig.context(Some(ONE_MINUTE), &fake)?;
    let mut trial = driven();
    assert!(check_trial(&gated, 4242, "typing", &mut trial).is_none());
    let reason = trial.invalid.clone().unwrap_or_default();
    assert!(
        reason.starts_with("typing: cannot read the idle time"),
        "{reason}"
    );
    // A helper built before `idle` existed says how to fix itself.
    assert!(reason.contains("bench/helpers/build.sh"), "{reason}");
    // A trial already void is not looked at again.
    trial.invalid = Some("typing: lost the foreground".to_owned());
    assert!(check_trial(&gated, 4242, "typing", &mut trial).is_none());
    assert_eq!(
        trial.invalid.as_deref(),
        Some("typing: lost the foreground")
    );
    assert_eq!(rig.calls().iter().filter(|call| *call == "idle").count(), 1);
    Ok(())
}
