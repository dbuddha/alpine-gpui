//! Command-line parsing and dispatch.

use crate::apps::App;
use crate::baseline;
use crate::fixtures;
use crate::helpers::Helpers;
use crate::isolation;
use crate::paths::BenchPaths;
use crate::stamp::{self, StampInputs};
use crate::trial::{self, RunRequest};
use crate::workload::{self, Fixture, WORKLOADS};
use std::path::PathBuf;

pub const USAGE: &str = "usage:
  bench run <workload> --app alpine|zed|appkit [--trials N] [--warmup N]
            [--key-interval-ms N] [--keep-homes] [--rust-analyzer PATH] [--dry-run]
            [--wait-idle MINUTES]
  bench scene-session <native-fixture> <helper-dir> <new-output-dir> (approved screen only)
  bench scene-evidence <capture.tsv> <action.tsv>  (diagnostic consistency only)
  bench baseline <run-dir> --milestone ID [--note TEXT]
  bench workloads | stamp | fixtures | permissions
  bench zed-isolation [--seconds N] [--allow-window] [--open-fixture]

Build the Swift helpers first with bench/helpers/build.sh. The protocol is in
bench/AGENTS.md.

Scene diagnostics: capture with bench-capture scene --pid PID --window-id ID
--duration-ms 10000 --region X,Y,W,H --scale 1. The region has five equal
vertical grayscale patches: A=0,118,188,225,255; B is reversed. Nine points per
patch are sampled in sRGB, each RGB channel within 12 of its expected value.
The action sidecar contains exactly one TSV record:
scene-action<TAB>alpine-scene-diagnostic/v1<TAB>PID<TAB>ID<TAB>MACH_TICK<TAB>A<TAB>B
scene-session uses the native fixture to record an accepted action.
Its Mach tick is a command-issued lower bound, not native submission latency.
One positive/stale/wrong control each; 30 seconds per case; quiet wait <=5 min.
No live method or physical gate is qualified by offline tests. A hand-written
sidecar proves only replay consistency, never physical or typing qualification.
Capture and replay limits: 2048 frames, 2 MiB capture, 256-byte action file.
Live method, visibility and action provenance remain unqualified.

Build scene-session tools only from clean HEAD:
  export ALPINE_SCENE_REVISION=$(git rev-parse HEAD)
  RUSTFLAGS='--cfg alpine_native_validation' CARGO_TARGET_DIR=target/scene-fixture cargo build --locked -p alpine-platform-macos --example scene_evidence_fixture
  cargo build --manifest-path bench/Cargo.toml
  bench/helpers/build.sh
Run scene-session only with session approval; pass the fixture executable,
bench/target/helpers, and a new output directory outside the repository.
The fixture uses disposable HOME, clears inherited native test overrides,
and records terminal failures and exact cleanup. Capture scale is fixed at 1;
the centered strip is 500x40 points inside the measured outer frame.
Only even integral outer widths 500..504 and heights 300..350 are supported;
source dimensions and strip coordinates must match that measured mapping. Ready/baseline/capture/drain caps
are 5/5/10/5 seconds within a 30-second case execution cap.
Owned process-group termination and socket drain allow up to 0.5 seconds
cleanup per child after the deadline. No process from the owner is signalled. Output is capped at 2 MiB
capture, 64 KiB per child stderr/fixture, and 512 KiB helper evidence.
Source commit, clean tree and executable hashes are retained in stamp.txt.
Only these first-party binaries are supported. OS scheduling/reaping is an
external dependency; cleanup failures invalidate the run. Conditions are
checked at least each second and immediately before activation/capture/action.
Before capture the fixture must own one visible window. After confirmed capture
start, each sample may admit one AppKit-confirmed NSKVONotifying_NSLocalWindowSharingWindow
parented to the pinned window and wholly in its title bar. Queries are bounded
to 32 per case. The stale control needs an A display tick after the supervisor
observes fixture admission and records its second clock sample in admission.tsv.
Every control needs valid full-trace clocks, geometry, identity and cleanup.
Native Metal timestamp failure remains a hold, including when controls agree.";

/// Timed keys closer than this mostly come back late on any editor.
const MIN_KEY_INTERVAL_MS: u64 = 100;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunArgs {
    pub workload: String,
    pub app: String,
    pub trials: u32,
    pub warmup: Option<u32>,
    pub keep_homes: bool,
    pub rust_analyzer: Option<PathBuf>,
    pub key_interval_ms: Option<u64>,
    /// Prints the launches and input a run would make, starting nothing.
    pub dry_run: bool,
    /// Minutes of owner idle to wait for before every trial.
    pub wait_idle: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Run(RunArgs),
    Baseline {
        run_dir: PathBuf,
        milestone: String,
        note: String,
    },
    SceneSession {
        fixture: PathBuf,
        helpers: PathBuf,
        output: PathBuf,
    },
    SceneEvidence {
        capture: PathBuf,
        actions: PathBuf,
    },
    Workloads,
    Stamp,
    Fixtures,
    Permissions,
    ZedIsolation {
        seconds: u64,
        allow_window: bool,
        open_fixture: bool,
    },
    Help,
}

/// Splits `--name value` options and bare flags from positional arguments.
struct Parsed {
    positional: Vec<String>,
    values: Vec<(String, String)>,
    flags: Vec<String>,
}

fn split(args: &[String], value_options: &[&str], flag_options: &[&str]) -> Result<Parsed, String> {
    let mut parsed = Parsed {
        positional: Vec::new(),
        values: Vec::new(),
        flags: Vec::new(),
    };
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if let Some(name) = arg.strip_prefix("--") {
            if flag_options.contains(&name) {
                parsed.flags.push(name.to_owned());
            } else if value_options.contains(&name) {
                let value = iter
                    .next()
                    .ok_or_else(|| format!("--{name} needs a value"))?;
                if parsed.values.iter().any(|(existing, _)| existing == name) {
                    return Err(format!("--{name} given twice"));
                }
                parsed.values.push((name.to_owned(), value.clone()));
            } else {
                return Err(format!("unknown option --{name}"));
            }
        } else {
            parsed.positional.push(arg.clone());
        }
    }
    Ok(parsed)
}

impl Parsed {
    fn value(&self, name: &str) -> Option<&str> {
        self.values
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    fn number<T: std::str::FromStr>(&self, name: &str) -> Result<Option<T>, String> {
        self.value(name)
            .map(|text| {
                text.parse()
                    .map_err(|_| format!("--{name} must be a number, found {text:?}"))
            })
            .transpose()
    }

    fn flag(&self, name: &str) -> bool {
        self.flags.iter().any(|flag| flag == name)
    }

    fn exactly(&self, count: usize, what: &str) -> Result<(), String> {
        if self.positional.len() == count {
            Ok(())
        } else {
            Err(format!("expected {what}"))
        }
    }
}

pub fn parse(args: &[String]) -> Result<Command, String> {
    let Some((command, rest)) = args.split_first() else {
        return Ok(Command::Help);
    };
    match command.as_str() {
        "run" => {
            let parsed = split(
                rest,
                &[
                    "app",
                    "trials",
                    "warmup",
                    "rust-analyzer",
                    "key-interval-ms",
                    "wait-idle",
                ],
                &["keep-homes", "dry-run"],
            )?;
            parsed.exactly(1, "one workload")?;
            let key_interval_ms: Option<u64> = parsed.number("key-interval-ms")?;
            if key_interval_ms.is_some_and(|interval| interval < MIN_KEY_INTERVAL_MS) {
                return Err(format!(
                    "--key-interval-ms must be at least {MIN_KEY_INTERVAL_MS}"
                ));
            }
            let wait_idle: Option<u32> = parsed.number("wait-idle")?;
            if wait_idle == Some(0) {
                return Err("--wait-idle must be at least 1 minute".to_owned());
            }
            Ok(Command::Run(RunArgs {
                workload: parsed.positional[0].clone(),
                app: parsed.value("app").ok_or("--app is required")?.to_owned(),
                trials: parsed.number("trials")?.unwrap_or(10),
                warmup: parsed.number("warmup")?,
                keep_homes: parsed.flag("keep-homes"),
                rust_analyzer: parsed.value("rust-analyzer").map(PathBuf::from),
                key_interval_ms,
                dry_run: parsed.flag("dry-run"),
                wait_idle,
            }))
        }
        "scene-session" => {
            let parsed = split(rest, &[], &[])?;
            parsed.exactly(
                3,
                "native fixture, helper directory and new output directory",
            )?;
            Ok(Command::SceneSession {
                fixture: PathBuf::from(&parsed.positional[0]),
                helpers: PathBuf::from(&parsed.positional[1]),
                output: PathBuf::from(&parsed.positional[2]),
            })
        }
        "scene-evidence" => {
            let parsed = split(rest, &[], &[])?;
            parsed.exactly(2, "capture.tsv and single-action.tsv")?;
            Ok(Command::SceneEvidence {
                capture: PathBuf::from(&parsed.positional[0]),
                actions: PathBuf::from(&parsed.positional[1]),
            })
        }
        "baseline" => {
            let parsed = split(rest, &["milestone", "note"], &[])?;
            parsed.exactly(1, "one run directory")?;
            Ok(Command::Baseline {
                run_dir: PathBuf::from(&parsed.positional[0]),
                milestone: parsed
                    .value("milestone")
                    .ok_or("--milestone is required")?
                    .to_owned(),
                note: parsed.value("note").unwrap_or_default().to_owned(),
            })
        }
        "zed-isolation" => {
            let parsed = split(rest, &["seconds"], &["allow-window", "open-fixture"])?;
            parsed.exactly(0, "no positional arguments")?;
            Ok(Command::ZedIsolation {
                seconds: parsed.number("seconds")?.unwrap_or(8),
                allow_window: parsed.flag("allow-window"),
                open_fixture: parsed.flag("open-fixture"),
            })
        }
        simple
        @ ("workloads" | "stamp" | "fixtures" | "permissions" | "help" | "--help" | "-h") => {
            split(rest, &[], &[])?.exactly(0, "no arguments")?;
            Ok(match simple {
                "workloads" => Command::Workloads,
                "stamp" => Command::Stamp,
                "fixtures" => Command::Fixtures,
                "permissions" => Command::Permissions,
                _ => Command::Help,
            })
        }
        other => Err(format!("unknown command {other:?}\n{USAGE}")),
    }
}

pub fn execute(command: Command) -> Result<(), String> {
    match command {
        Command::Help => {
            println!("{USAGE}");
            Ok(())
        }
        Command::SceneSession {
            fixture,
            helpers,
            output,
        } => crate::scene_session::run(&fixture, &helpers, &output),
        Command::SceneEvidence { capture, actions } => {
            println!("{}", crate::physical_evidence::run(&capture, &actions)?);
            Ok(())
        }
        Command::Workloads => {
            for workload in WORKLOADS {
                let only = if workload.alpine_only {
                    ", alpine only"
                } else {
                    ""
                };
                let needs = workload.permissions();
                println!("{:<10} {} ({needs}{only})", workload.name, workload.summary);
            }
            Ok(())
        }
        Command::Run(args) => {
            let paths = BenchPaths::discover()?;
            let request = request_for(&args)?;
            if args.dry_run {
                trial::dry_run(&paths, &request)
            } else {
                trial::run(&paths, &request).map(|_| ())
            }
        }
        Command::Baseline {
            run_dir,
            milestone,
            note,
        } => {
            let paths = BenchPaths::discover()?;
            let rows = baseline::append(&run_dir, &paths.baseline, &milestone, &note)?;
            println!("appended {rows} rows to {}", paths.baseline.display());
            Ok(())
        }
        Command::Stamp => {
            print_stamp(&BenchPaths::discover()?);
            Ok(())
        }
        Command::Fixtures => {
            let paths = BenchPaths::discover()?;
            for fixture in Fixture::ALL {
                let file = fixtures::ensure(&paths.fixtures_dir(), fixture)?;
                let path = file.path.display();
                println!("{path}\t{}\tfnv64={}", file.bytes, file.fnv64);
            }
            Ok(())
        }
        Command::Permissions => {
            let paths = BenchPaths::discover()?;
            let permissions = Helpers::locate(&paths.helpers_dir)?.permissions()?;
            println!("accessibility\t{}", permissions.accessibility);
            println!("post-events\t{}", permissions.post_events);
            println!("screen-recording\t{}", permissions.screen_recording);
            Ok(())
        }
        Command::ZedIsolation {
            seconds,
            allow_window,
            open_fixture,
        } => zed_isolation(
            &BenchPaths::discover()?,
            seconds,
            allow_window,
            open_fixture,
        ),
    }
}

fn request_for(args: &RunArgs) -> Result<RunRequest, String> {
    Ok(RunRequest {
        workload: workload::find(&args.workload)?,
        app: App::parse(&args.app)?,
        trials: args.trials,
        warmup: args.warmup,
        keep_homes: args.keep_homes,
        rust_analyzer: args.rust_analyzer.clone(),
        key_interval_ms: args.key_interval_ms,
        wait_idle: args.wait_idle,
    })
}

fn print_stamp(paths: &BenchPaths) {
    let displays = Helpers::locate(&paths.helpers_dir)
        .and_then(|helpers| helpers.displays())
        .unwrap_or_default();
    let stamp = stamp::collect(&StampInputs {
        repo_root: &paths.repo_root,
        alpine_bundle: &paths.alpine_bundle,
        zed_app: &paths.zed_app,
        helpers_dir: &paths.helpers_dir,
        displays: &displays,
    });
    for (key, value) in stamp.entries {
        println!("{key}\t{value}");
    }
}

fn zed_isolation(
    paths: &BenchPaths,
    seconds: u64,
    allow_window: bool,
    open_fixture: bool,
) -> Result<(), String> {
    let probe = isolation::run(paths, seconds, allow_window, open_fixture)?;
    for line in &probe.report {
        println!("{line}");
    }
    if probe.clean {
        println!(
            "verdict: no change under the checked real Zed paths; recent documents need Full Disk Access and are not checked"
        );
        Ok(())
    } else {
        Err("verdict: real Zed state changed or Zed was updated; see above".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::{Command, RunArgs, parse, request_for};
    use std::path::PathBuf;

    fn args(text: &str) -> Vec<String> {
        text.split_whitespace().map(ToOwned::to_owned).collect()
    }

    #[test]
    fn scene_session_requires_three_paths() -> Result<(), String> {
        assert!(matches!(
            parse(&args("scene-session fixture helpers output"))?,
            Command::SceneSession { .. }
        ));
        for input in [
            "scene-session",
            "scene-session fixture helpers",
            "scene-session fixture helpers output extra",
            "scene-session --approve fixture helpers output",
        ] {
            assert!(parse(&args(input)).is_err());
        }
        Ok(())
    }

    #[test]
    fn run_defaults_to_ten_trials() -> Result<(), String> {
        assert_eq!(
            parse(&args("run idle --app alpine"))?,
            Command::Run(RunArgs {
                workload: "idle".to_owned(),
                app: "alpine".to_owned(),
                trials: 10,
                warmup: None,
                keep_homes: false,
                rust_analyzer: None,
                key_interval_ms: None,
                dry_run: false,
                wait_idle: None,
            })
        );
        Ok(())
    }

    #[test]
    fn wait_idle_is_off_unless_given_and_takes_whole_minutes() -> Result<(), String> {
        let wait = |text: &str| match parse(&args(text))? {
            Command::Run(run) => Ok(run.wait_idle),
            other => Err(format!("not a run: {other:?}")),
        };
        assert_eq!(wait("run idle --app zed")?, None);
        assert_eq!(wait("run idle --app zed --wait-idle 10")?, Some(10));
        assert_eq!(wait("run idle --app zed --wait-idle 1 --dry-run")?, Some(1));
        for bad in [
            "run idle --app zed --wait-idle 0",
            "run idle --app zed --wait-idle",
            "run idle --app zed --wait-idle soon",
            "run idle --app zed --wait-idle 2.5",
            "run idle --app zed --wait-idle -3",
            "run idle --app zed --wait-idle 5 --wait-idle 6",
        ] {
            assert!(parse(&args(bad)).is_err(), "{bad}");
        }
        Ok(())
    }

    #[test]
    fn the_flag_reaches_the_run_request() -> Result<(), String> {
        let request = |text: &str| match parse(&args(text))? {
            Command::Run(run) => request_for(&run),
            other => Err(format!("not a run: {other:?}")),
        };
        assert_eq!(request("run idle --app zed")?.wait_idle, None);
        let gated = request("run idle --app zed --wait-idle 10 --trials 3")?;
        assert_eq!((gated.wait_idle, gated.trials), (Some(10), 3));
        assert!(request("run nothing --app zed --wait-idle 10").is_err());
        Ok(())
    }

    /// Each option reaches the request, so a dropped one fails here.
    #[test]
    fn every_run_option_reaches_the_run_request() -> Result<(), String> {
        let Command::Run(run) = parse(&args(
            "run caret --app alpine --trials 7 --warmup 2 --keep-homes --rust-analyzer /ra \
             --key-interval-ms 400 --wait-idle 12",
        ))?
        else {
            return Err("not a run".to_owned());
        };
        let request = request_for(&run)?;
        assert_eq!(request.workload.name, "caret");
        assert_eq!(request.app, crate::apps::App::Alpine);
        assert_eq!((request.trials, request.warmup), (7, Some(2)));
        assert!(request.keep_homes);
        assert_eq!(request.rust_analyzer, Some(PathBuf::from("/ra")));
        assert_eq!(request.key_interval_ms, Some(400));
        assert_eq!(request.wait_idle, Some(12));
        Ok(())
    }

    #[test]
    fn run_accepts_the_gate_invocation_and_flags() -> Result<(), String> {
        let Command::Run(run) = parse(&args(
            "run open-repo --app zed --trials 1 --warmup 0 --keep-homes --rust-analyzer /ra \
             --key-interval-ms 400",
        ))?
        else {
            return Err("not a run".to_owned());
        };
        assert_eq!((run.trials, run.warmup, run.keep_homes), (1, Some(0), true));
        assert_eq!(run.rust_analyzer, Some(PathBuf::from("/ra")));
        assert_eq!(run.key_interval_ms, Some(400));
        assert!(!run.dry_run);
        let Command::Run(dry) = parse(&args("run highlight --app alpine --dry-run"))? else {
            return Err("not a run".to_owned());
        };
        assert!(dry.dry_run && dry.workload == "highlight");
        Ok(())
    }

    #[test]
    fn bad_arguments_are_rejected() {
        for bad in [
            "run idle",
            "run --app alpine",
            "run idle extra --app alpine",
            "run idle --app alpine --trials many",
            "run idle --app alpine --app zed",
            "run idle --app alpine --bogus",
            "run typing --app alpine --key-interval-ms 40",
            "baseline --milestone M1",
            "baseline results/x",
            "workloads extra",
            "frobnicate",
        ] {
            assert!(parse(&args(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn scene_replay_requires_two_files_and_no_extra_actions() -> Result<(), String> {
        assert_eq!(
            parse(&args("scene-evidence capture.tsv action.tsv"))?,
            Command::SceneEvidence {
                capture: PathBuf::from("capture.tsv"),
                actions: PathBuf::from("action.tsv")
            }
        );
        for bad in [
            "scene-evidence",
            "scene-evidence capture.tsv",
            "scene-evidence capture.tsv action.tsv second.tsv",
            "scene-evidence capture.tsv --action 3",
        ] {
            assert!(parse(&args(bad)).is_err());
        }
        Ok(())
    }

    #[test]
    fn other_commands_parse() -> Result<(), String> {
        assert_eq!(parse(&[])?, Command::Help);
        assert_eq!(parse(&args("workloads"))?, Command::Workloads);
        assert_eq!(
            parse(&args("baseline results/run --milestone M1 --note first"))?,
            Command::Baseline {
                run_dir: PathBuf::from("results/run"),
                milestone: "M1".to_owned(),
                note: "first".to_owned(),
            }
        );
        assert_eq!(
            parse(&args("zed-isolation --seconds 5"))?,
            Command::ZedIsolation {
                seconds: 5,
                allow_window: false,
                open_fixture: false,
            }
        );
        assert_eq!(
            parse(&args("zed-isolation --allow-window --open-fixture"))?,
            Command::ZedIsolation {
                seconds: 8,
                allow_window: true,
                open_fixture: true,
            }
        );
        Ok(())
    }
}
