//! The `--wait-idle` gate. Whether a trial may start, and whether one still
//! stands, are pure functions of samples. The polling loop and the display
//! assertion live here too, so `trial.rs` only supplies the samples.

use crate::procs;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::Duration;

/// Why a trial is void when the owner touched the keyboard or mouse.
pub const OWNER_INPUT: &str = "owner input";
/// How often a closed gate reads its samples again.
pub const POLL: Duration = Duration::from_secs(5);
/// A waiting gate repeats its state every this many polls (five minutes).
const HEARTBEAT_POLLS: u32 = 60;
const PMSET: &str = "/usr/bin/pmset";
const CAFFEINATE: &str = "/usr/bin/caffeinate";
/// A `caffeinate` that rejects its arguments exits within milliseconds.
const AWAKE_CHECK: Duration = Duration::from_millis(200);

/// What `--wait-idle` asks for, before every trial and while it runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Gate {
    /// Minutes without a hardware keyboard or mouse event.
    pub minutes: u32,
    /// The measured app, named in log lines.
    pub app: &'static str,
}

impl Gate {
    fn threshold(&self) -> f64 {
        f64::from(self.minutes) * 60.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Power {
    Ac,
    /// Another source `pmset` names, such as `Battery Power`.
    Other(String),
}

impl Power {
    fn label(&self) -> &str {
        match self {
            Self::Ac => "AC power",
            Self::Other(source) => source,
        }
    }
}

/// The source named by the first line of `pmset -g ps`, such as
/// `Now drawing from 'AC Power'`.
pub fn parse_power(text: &str) -> Result<Power, String> {
    let source = text
        .lines()
        .find_map(|line| {
            line.trim_end()
                .strip_prefix("Now drawing from '")?
                .strip_suffix('\'')
        })
        .ok_or_else(|| format!("no power source line in {text:?}"))?;
    Ok(if source == "AC Power" {
        Power::Ac
    } else {
        Power::Other(source.to_owned())
    })
}

pub fn power() -> Result<Power, String> {
    parse_power(&procs::output(Path::new(PMSET), &["-g", "ps"])?)
}

/// Everything the gate reads before a trial.
#[derive(Clone, Debug, PartialEq)]
pub struct Sample {
    /// Seconds since the owner's last hardware keyboard or mouse event.
    pub idle_seconds: f64,
    pub power: Power,
    /// Pids of copies of the measured app that are already running.
    pub running: Vec<i32>,
}

/// A condition that keeps the gate shut, with the reading that failed it.
#[derive(Clone, Debug, PartialEq)]
pub enum Blocker {
    Owner(f64),
    Power(Power),
    AppRunning(Vec<i32>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    Start,
    Wait(Vec<Blocker>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Watch {
    Continue,
    Abort,
}

/// NaN is not enough, so a broken reading keeps the gate shut.
fn idle_enough(gate: &Gate, idle_seconds: f64) -> bool {
    idle_seconds >= gate.threshold()
}

/// Wait, or start, from one sample taken between trials.
pub fn before_trial(gate: &Gate, sample: &Sample) -> Step {
    let mut blockers = Vec::new();
    if !idle_enough(gate, sample.idle_seconds) {
        blockers.push(Blocker::Owner(sample.idle_seconds));
    }
    if sample.power != Power::Ac {
        blockers.push(Blocker::Power(sample.power.clone()));
    }
    if !sample.running.is_empty() {
        blockers.push(Blocker::AppRunning(sample.running.clone()));
    }
    if blockers.is_empty() {
        Step::Start
    } else {
        Step::Wait(blockers)
    }
}

/// The gate opens at `idle >= threshold` and idle only falls on input, so
/// a lower reading during a trial means the owner touched something.
pub fn during_trial(gate: &Gate, idle_seconds: f64) -> Watch {
    if idle_enough(gate, idle_seconds) {
        Watch::Continue
    } else {
        Watch::Abort
    }
}

/// Why a trial is void after one idle reading, or none to go on. An
/// unreadable clock voids the trial: the gate cannot vouch for it.
pub fn voided_by(gate: &Gate, reading: Result<f64, String>) -> Option<String> {
    match reading {
        Ok(idle) if during_trial(gate, idle) == Watch::Continue => None,
        Ok(_) => Some(OWNER_INPUT.to_owned()),
        Err(error) => Some(format!("cannot read the idle time ({error})")),
    }
}

/// `43m23s` for 2603.3 seconds.
fn age(seconds: f64) -> String {
    let whole = seconds.floor();
    format!("{}m{:02}s", (whole / 60.0).floor(), whole % 60.0)
}

fn wait_text(gate: &Gate, blockers: &[Blocker]) -> String {
    let parts: Vec<String> = blockers
        .iter()
        .map(|blocker| match blocker {
            Blocker::Owner(idle) => {
                format!("owner idle ({} of {}m)", age(*idle), gate.minutes)
            }
            Blocker::Power(power) => format!("AC power (now {})", power.label()),
            Blocker::AppRunning(pids) => {
                let pids: Vec<String> = pids.iter().map(ToString::to_string).collect();
                format!("{} to quit (pid {})", gate.app, pids.join(", "))
            }
        })
        .collect();
    parts.join(", ")
}

fn open_text(gate: &Gate, sample: &Sample) -> String {
    format!(
        "owner idle {} of {}m, AC power, {} not running",
        age(sample.idle_seconds),
        gate.minutes,
        gate.app
    )
}

/// Polls `sample` until a trial may start. `say` gets a line when waiting
/// begins, the state every `HEARTBEAT_POLLS` polls, and a line when the
/// gate opens, though the run's first trial says so even without waiting.
pub fn wait_to_start(
    gate: &Gate,
    first: bool,
    sample: &mut dyn FnMut() -> Result<Sample, String>,
    pause: &mut dyn FnMut(),
    say: &mut dyn FnMut(&str),
) -> Result<(), String> {
    let mut polls: u32 = 0;
    loop {
        let reading = sample()?;
        match before_trial(gate, &reading) {
            Step::Start => {
                if polls > 0 || first {
                    say(&format!("gate open: {}", open_text(gate, &reading)));
                }
                return Ok(());
            }
            Step::Wait(blockers) => {
                if polls == 0 {
                    say(&format!("gate waiting for {}", wait_text(gate, &blockers)));
                } else if polls.is_multiple_of(HEARTBEAT_POLLS) {
                    say(&format!(
                        "gate still waiting for {}",
                        wait_text(gate, &blockers)
                    ));
                }
                polls = polls.saturating_add(1);
                pause();
            }
        }
    }
}

/// `caffeinate` arguments that keep the display and system awake until the
/// process `pid` exits. Never `-u`: it declares user activity, which would
/// reset the idle clock the gate reads.
pub fn caffeinate_args(pid: &str) -> Vec<String> {
    ["-d", "-i", "-w", pid]
        .iter()
        .map(ToString::to_string)
        .collect()
}

/// The dry run's description of the gate.
pub fn plan_lines(gate: &Gate, binary: &Path) -> Vec<String> {
    vec![
        format!(
            "gate\tbefore every trial wait until the owner has had no keyboard or mouse \
             event for {} min, on AC power, with no copy of {} running",
            gate.minutes,
            binary.display()
        ),
        format!(
            "gate\tread every {} s; a trial that sees owner input is void ({OWNER_INPUT}) \
             and the next one waits again",
            POLL.as_secs()
        ),
        format!(
            "gate\t{CAFFEINATE} {} keeps the display and system awake",
            caffeinate_args("<bench pid>").join(" ")
        ),
    ]
}

/// A `caffeinate` held for the run, stopped when it ends. If the bench dies
/// first, `-w` ends `caffeinate` along with it.
pub struct KeepAwake {
    child: Child,
}

impl KeepAwake {
    pub fn start() -> Result<Self, String> {
        let pid = std::process::id().to_string();
        Self::spawn(Path::new(CAFFEINATE), &caffeinate_args(&pid))
    }

    fn spawn(program: &Path, args: &[String]) -> Result<Self, String> {
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("{}: {error}", program.display()))?;
        thread::sleep(AWAKE_CHECK);
        match child.try_wait() {
            Ok(None) => Ok(Self { child }),
            Ok(Some(status)) => Err(format!(
                "{} exited at once ({status}), so the display may sleep",
                program.display()
            )),
            Err(error) => {
                let _ = child.kill();
                Err(format!("{}: {error}", program.display()))
            }
        }
    }
}

impl Drop for KeepAwake {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Blocker, Gate, KeepAwake, OWNER_INPUT, Power, Sample, Step, Watch, age, before_trial,
        caffeinate_args, during_trial, parse_power, plan_lines, voided_by, wait_to_start,
    };
    use std::collections::VecDeque;
    use std::path::Path;

    const TEN: Gate = Gate {
        minutes: 10,
        app: "zed",
    };

    fn sample(idle_seconds: f64, power: Power, running: &[i32]) -> Sample {
        Sample {
            idle_seconds,
            power,
            running: running.to_vec(),
        }
    }

    /// The first text is `pmset -g ps` as the dev Mac printed it on AC power.
    /// The battery text keeps that layout with a discharging battery's source
    /// and state words.
    #[test]
    fn power_sources_parse_from_pmset_output() -> Result<(), String> {
        let ac = "Now drawing from 'AC Power'\n -InternalBattery-0 (id=34799715)\t100%; \
                  charged; 0:00 remaining present: true\n";
        assert_eq!(parse_power(ac)?, Power::Ac);
        let battery = "Now drawing from 'Battery Power'\n -InternalBattery-0 (id=34799715)\t\
                       87%; discharging; 5:07 remaining present: true\n";
        assert_eq!(
            parse_power(battery)?,
            Power::Other("Battery Power".to_owned())
        );
        // A desktop Mac prints only the first line.
        assert_eq!(parse_power("Now drawing from 'AC Power'\n")?, Power::Ac);
        assert_eq!(
            parse_power("Now drawing from 'UPS Power'\n")?,
            Power::Other("UPS Power".to_owned())
        );
        for bad in ["", "Now drawing from AC Power\n", "AC Power\n"] {
            assert!(parse_power(bad).is_err(), "{bad:?}");
        }
        Ok(())
    }

    #[test]
    fn a_trial_starts_only_when_every_condition_holds() {
        let waits = |idle, power, running: &[i32]| match before_trial(
            &TEN,
            &sample(idle, power, running),
        ) {
            Step::Wait(blockers) => blockers,
            Step::Start => Vec::new(),
        };
        assert_eq!(
            before_trial(&TEN, &sample(900.0, Power::Ac, &[])),
            Step::Start
        );
        // Idle below the threshold, however long, keeps the gate shut.
        assert_eq!(
            waits(599.9, Power::Ac, &[]),
            [Blocker::Owner(599.9)],
            "just under ten minutes"
        );
        assert_eq!(waits(0.0, Power::Ac, &[]), [Blocker::Owner(0.0)]);
        // On battery, however idle the owner is.
        let battery = Power::Other("Battery Power".to_owned());
        assert_eq!(
            waits(3600.0, battery.clone(), &[]),
            [Blocker::Power(battery.clone())]
        );
        // Another copy of the app, however idle and plugged in.
        assert_eq!(
            waits(3600.0, Power::Ac, &[812, 9]),
            [Blocker::AppRunning(vec![812, 9])]
        );
        // Every failing condition is reported together, in a fixed order.
        assert_eq!(
            waits(5.0, battery.clone(), &[812]),
            [
                Blocker::Owner(5.0),
                Blocker::Power(battery),
                Blocker::AppRunning(vec![812])
            ]
        );
    }

    #[test]
    fn idle_exactly_at_the_threshold_is_enough() {
        assert_eq!(
            before_trial(&TEN, &sample(600.0, Power::Ac, &[])),
            Step::Start
        );
        assert_eq!(during_trial(&TEN, 600.0), Watch::Continue);
        assert_eq!(during_trial(&TEN, 599.999), Watch::Abort);
        // A broken reading never opens the gate or vouches for a trial.
        let broken = before_trial(&TEN, &sample(f64::NAN, Power::Ac, &[]));
        assert!(matches!(broken, Step::Wait(ref blockers) if blockers.len() == 1));
        assert_eq!(during_trial(&TEN, f64::NAN), Watch::Abort);
    }

    #[test]
    fn input_during_a_trial_is_owner_input() {
        // Idle only grows during a trial, so a reading under the gate's own
        // threshold can only follow a keyboard or mouse event.
        assert_eq!(during_trial(&TEN, 612.4), Watch::Continue);
        assert_eq!(during_trial(&TEN, 3.2), Watch::Abort);
        assert_eq!(voided_by(&TEN, Ok(612.4)), None);
        assert_eq!(voided_by(&TEN, Ok(0.4)).as_deref(), Some(OWNER_INPUT));
        let unreadable = voided_by(&TEN, Err("no such helper".to_owned())).unwrap_or_default();
        assert!(
            unreadable.contains("cannot read the idle time"),
            "{unreadable}"
        );
        assert!(unreadable.contains("no such helper"), "{unreadable}");
    }

    /// Runs the wait over scripted samples; returns the log and the pauses.
    fn waited(
        first: bool,
        script: Vec<Result<Sample, String>>,
    ) -> (Result<(), String>, Vec<String>, usize) {
        let mut script = VecDeque::from(script);
        let mut lines = Vec::new();
        let mut pauses = 0;
        let outcome = wait_to_start(
            &TEN,
            first,
            &mut || {
                script
                    .pop_front()
                    .unwrap_or_else(|| Err("script ended".to_owned()))
            },
            &mut || pauses += 1,
            &mut |line| lines.push(line.to_owned()),
        );
        (outcome, lines, pauses)
    }

    #[test]
    fn the_gate_waits_for_idle_then_opens() {
        let (outcome, lines, pauses) = waited(
            false,
            vec![
                Ok(sample(12.0, Power::Ac, &[])),
                Ok(sample(100.0, Power::Ac, &[])),
                Ok(sample(601.0, Power::Ac, &[])),
            ],
        );
        assert_eq!(outcome, Ok(()));
        assert_eq!(pauses, 2);
        assert_eq!(
            lines,
            [
                "gate waiting for owner idle (0m12s of 10m)",
                "gate open: owner idle 10m01s of 10m, AC power, zed not running",
            ]
        );
    }

    #[test]
    fn an_open_gate_is_silent_except_for_the_first_trial() {
        let open = || vec![Ok(sample(4000.0, Power::Ac, &[]))];
        let (outcome, later, pauses) = waited(false, open());
        assert_eq!((outcome, later.len(), pauses), (Ok(()), 0, 0));
        let (outcome, first, pauses) = waited(true, open());
        assert_eq!((outcome, pauses), (Ok(()), 0));
        assert_eq!(
            first,
            ["gate open: owner idle 66m40s of 10m, AC power, zed not running"]
        );
    }

    #[test]
    fn a_waiting_gate_names_every_blocker_and_repeats_it_every_five_minutes() {
        let battery = Power::Other("Battery Power".to_owned());
        let mut script = vec![Ok(sample(700.0, battery, &[812]))];
        // 59 more shut polls, then the 60th poll repeats the state.
        script.extend((1..=60).map(|_| Ok(sample(700.0, Power::Ac, &[812]))));
        script.push(Ok(sample(700.0, Power::Ac, &[])));
        let (outcome, lines, pauses) = waited(false, script);
        assert_eq!(outcome, Ok(()));
        assert_eq!(pauses, 61);
        assert_eq!(
            lines,
            [
                "gate waiting for AC power (now Battery Power), zed to quit (pid 812)",
                "gate still waiting for zed to quit (pid 812)",
                "gate open: owner idle 11m40s of 10m, AC power, zed not running",
            ]
        );
    }

    #[test]
    fn a_sample_that_fails_stops_the_wait() {
        let (outcome, lines, pauses) = waited(
            true,
            vec![
                Ok(sample(1.0, Power::Ac, &[])),
                Err("bench-window idle failed".to_owned()),
            ],
        );
        assert_eq!(outcome, Err("bench-window idle failed".to_owned()));
        assert_eq!((lines.len(), pauses), (1, 1));
    }

    /// An abort sends the run back to the gate, which waits for idle again
    /// and then starts the next trial.
    #[test]
    fn a_trial_aborted_by_the_owner_resumes_once_idle_again() {
        // Mid-trial: the owner is away, then moves the mouse.
        assert_eq!(during_trial(&TEN, 640.0), Watch::Continue);
        assert_eq!(during_trial(&TEN, 641.0), Watch::Continue);
        assert_eq!(during_trial(&TEN, 1.0), Watch::Abort);
        // Between trials: idle climbs from the owner's last event.
        let (outcome, lines, pauses) = waited(
            false,
            vec![
                Ok(sample(4.0, Power::Ac, &[])),
                Ok(sample(9.0, Power::Ac, &[])),
                Ok(sample(599.0, Power::Ac, &[])),
                Ok(sample(604.0, Power::Ac, &[])),
            ],
        );
        assert_eq!(outcome, Ok(()));
        assert_eq!(pauses, 3);
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("gate waiting for owner idle (0m04s"));
        assert!(lines[1].starts_with("gate open: owner idle 10m04s"));
    }

    #[test]
    fn ages_read_as_minutes_and_seconds() {
        assert_eq!(age(2603.323), "43m23s");
        assert_eq!(age(0.4), "0m00s");
        assert_eq!(age(599.9), "9m59s");
        assert_eq!(age(600.0), "10m00s");
        assert_eq!(age(3_725.0), "62m05s");
    }

    #[test]
    fn caffeinate_keeps_the_display_and_system_awake_without_user_activity() {
        let args = caffeinate_args("4242");
        assert_eq!(args, ["-d", "-i", "-w", "4242"]);
        // -u declares user activity, which resets the idle clock.
        assert!(!args.iter().any(|arg| arg.contains('u')), "{args:?}");
    }

    #[test]
    fn the_dry_run_describes_the_gate() {
        let lines = plan_lines(&TEN, Path::new("/Applications/Zed.app/Contents/MacOS/zed"));
        let text = lines.join("\n");
        assert!(
            lines.iter().all(|line| line.starts_with("gate\t")),
            "{text}"
        );
        assert!(text.contains("for 10 min"), "{text}");
        assert!(text.contains("on AC power"), "{text}");
        assert!(text.contains("no copy of /Applications/Zed.app"), "{text}");
        assert!(text.contains("owner input"), "{text}");
        assert!(
            text.contains("/usr/bin/caffeinate -d -i -w <bench pid>"),
            "{text}"
        );
    }

    fn alive(pid: &str) -> bool {
        std::process::Command::new("/bin/kill")
            .args(["-0", pid])
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }

    /// The guard is dropped on a thread: without a kill, the drop would wait
    /// out the sleeper, and the timeout (never reached otherwise) fails it.
    #[test]
    fn the_awake_guard_stops_its_process() -> Result<(), String> {
        let sleeper = KeepAwake::spawn(Path::new("/bin/sleep"), &["600".to_owned()])?;
        let pid = sleeper.child.id().to_string();
        assert!(alive(&pid));
        let (done, finished) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            drop(sleeper);
            let _ = done.send(());
        });
        let stopped = finished
            .recv_timeout(std::time::Duration::from_secs(10))
            .is_ok();
        if !stopped {
            let _ = std::process::Command::new("/bin/kill").arg(&pid).status();
        }
        assert!(stopped, "dropping the guard waited on pid {pid}");
        assert!(!alive(&pid), "the guard left pid {pid} running");
        Ok(())
    }

    #[test]
    fn a_caffeinate_that_exits_at_once_is_an_error() -> Result<(), String> {
        let refused = KeepAwake::spawn(Path::new("/usr/bin/false"), &[]).err();
        let message = refused.ok_or("a process that exits was accepted")?;
        assert!(message.contains("exited at once"), "{message}");
        Ok(())
    }
}
