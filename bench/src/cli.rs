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
            [--key-interval-ms N] [--keep-homes] [--rust-analyzer PATH]
  bench baseline <run-dir> --milestone ID [--note TEXT]
  bench workloads | stamp | fixtures | permissions
  bench zed-isolation [--seconds N] [--allow-window] [--open-fixture]

Build the Swift helpers first with bench/helpers/build.sh. The protocol is in
bench/AGENTS.md.";

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
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Run(RunArgs),
    Baseline {
        run_dir: PathBuf,
        milestone: String,
        note: String,
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
                ],
                &["keep-homes"],
            )?;
            parsed.exactly(1, "one workload")?;
            let key_interval_ms: Option<u64> = parsed.number("key-interval-ms")?;
            if key_interval_ms.is_some_and(|interval| interval < MIN_KEY_INTERVAL_MS) {
                return Err(format!(
                    "--key-interval-ms must be at least {MIN_KEY_INTERVAL_MS}"
                ));
            }
            Ok(Command::Run(RunArgs {
                workload: parsed.positional[0].clone(),
                app: parsed.value("app").ok_or("--app is required")?.to_owned(),
                trials: parsed.number("trials")?.unwrap_or(10),
                warmup: parsed.number("warmup")?,
                keep_homes: parsed.flag("keep-homes"),
                rust_analyzer: parsed.value("rust-analyzer").map(PathBuf::from),
                key_interval_ms,
            }))
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
        Command::Workloads => {
            for workload in WORKLOADS {
                let needs = if workload.needs_input() {
                    "Accessibility + Screen Recording"
                } else {
                    "no permission"
                };
                println!("{:<10} {} ({needs})", workload.name, workload.summary);
            }
            Ok(())
        }
        Command::Run(args) => {
            let paths = BenchPaths::discover()?;
            let request = RunRequest {
                workload: workload::find(&args.workload)?,
                app: App::parse(&args.app)?,
                trials: args.trials,
                warmup: args.warmup,
                keep_homes: args.keep_homes,
                rust_analyzer: args.rust_analyzer,
                key_interval_ms: args.key_interval_ms,
            };
            trial::run(&paths, &request).map(|_| ())
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
    use super::{Command, RunArgs, parse};
    use std::path::PathBuf;

    fn args(text: &str) -> Vec<String> {
        text.split_whitespace().map(ToOwned::to_owned).collect()
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
            })
        );
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
