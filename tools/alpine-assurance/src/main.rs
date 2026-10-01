//! Validates onscreen SDR evidence and captures raw AX client evidence.

mod ax_capture;
mod onscreen;

use std::{env, path::Path, process::ExitCode};

const COMMANDS: &str = "validate-onscreen-sdr, onscreen-sdr-report, or capture-ax-client";

fn main() -> ExitCode {
    match run() {
        Ok(output) => {
            println!("{output}");
            ExitCode::SUCCESS
        }
        Err(errors) => {
            for error in errors {
                eprintln!("assurance error: {error}");
            }
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<String, Vec<String>> {
    let mut arguments = env::args().skip(1);
    let Some(command) = arguments.next() else {
        return Err(vec![format!("a command is required; expected {COMMANDS}")]);
    };
    if matches!(
        command.as_str(),
        "validate-onscreen-sdr" | "onscreen-sdr-report"
    ) {
        return run_onscreen_command(&command, &mut arguments);
    }
    if command == "capture-ax-client" {
        return run_ax_capture_command(&mut arguments);
    }
    Err(vec![format!(
        "unknown command {command:?}; expected {COMMANDS}"
    )])
}

fn run_ax_capture_command(
    arguments: &mut impl Iterator<Item = String>,
) -> Result<String, Vec<String>> {
    let values = arguments.collect::<Vec<_>>();
    if values.len() != 5 {
        return Err(vec![
            "capture-ax-client requires PID, generation, pre-action milliseconds, post-action milliseconds, and an output directory"
                .to_owned(),
        ]);
    }
    let pid = values[0]
        .parse::<i32>()
        .map_err(|_| vec!["capture-ax-client PID must be a positive integer".to_owned()])?;
    let generation = values[1]
        .parse::<u64>()
        .map_err(|_| vec!["capture-ax-client generation must be a positive integer".to_owned()])?;
    let pre_action_ms = values[2].parse::<u64>().map_err(|_| {
        vec!["capture-ax-client pre-action duration must be unsigned milliseconds".to_owned()]
    })?;
    let post_action_ms = values[3].parse::<u64>().map_err(|_| {
        vec!["capture-ax-client post-action duration must be unsigned milliseconds".to_owned()]
    })?;
    ax_capture::run_native(
        pid,
        generation,
        pre_action_ms,
        post_action_ms,
        Path::new(&values[4]),
    )
}

fn run_onscreen_command(
    command: &str,
    arguments: &mut impl Iterator<Item = String>,
) -> Result<String, Vec<String>> {
    let Some(path) = arguments.next() else {
        return Err(vec![format!("{command} requires an artifact bundle path")]);
    };
    if arguments.next().is_some() {
        return Err(vec![format!("{command} accepts exactly one bundle path")]);
    }
    onscreen::run(command, Path::new(&path))
}
