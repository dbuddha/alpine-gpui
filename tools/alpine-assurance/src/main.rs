//! Validates and reports Alpine qualification, lab, and capture evidence.

mod ax;
mod ax_capture;
mod calibration;
#[cfg(test)]
mod digest;
mod lab;
mod lab_lifecycle;
mod lab_v2;
mod onscreen;
mod qualification;
mod trace_sequence;

use serde::Deserialize;
use std::{
    env, fs,
    path::Path,
    process::{Command, ExitCode},
};

const COMMANDS: &str = "validate-scene-trace, validate-trace-sequence, render-scene-reference, render-scene-native, benchmark-scene-reference, benchmark-scene-native, profile-scene-native, render-trace-sequence-native, validate-qualification, qualification-report, validate-aa-calibration, aa-calibration-report, validate-zed-lab-evidence, zed-lab-evidence-report, validate-onscreen-sdr, onscreen-sdr-report, validate-ax-fixture, validate-ax-evidence, ax-evidence-report, capture-ax-client, or upstream-radar";

#[derive(Debug, Deserialize)]
struct UpstreamRegistry {
    upstreams: Vec<Upstream>,
    #[serde(default)]
    manual_sources: Vec<ManualSource>,
}

#[derive(Debug, Deserialize)]
struct Upstream {
    name: String,
    repository: String,
    baseline_commit: String,
    research_issue: u64,
}

#[derive(Debug, Deserialize)]
struct ManualSource {
    name: String,
    url: String,
    next_review_on: String,
    research_issue: u64,
}

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

#[allow(
    clippy::too_many_lines,
    reason = "the bounded CLI keeps one exhaustive dispatch boundary with exact argument ownership"
)]
fn run() -> Result<String, Vec<String>> {
    let mut arguments = env::args().skip(1);
    let Some(command) = arguments.next() else {
        return Err(vec![format!("a command is required; expected {COMMANDS}")]);
    };
    if command == "upstream-radar" {
        return run_upstream_radar();
    }
    if matches!(
        command.as_str(),
        "validate-zed-lab-evidence" | "zed-lab-evidence-report"
    ) {
        return run_lab_command(&command, &mut arguments);
    }
    if matches!(
        command.as_str(),
        "validate-onscreen-sdr" | "onscreen-sdr-report"
    ) {
        return run_onscreen_command(&command, &mut arguments);
    }
    if matches!(
        command.as_str(),
        "validate-ax-fixture" | "validate-ax-evidence" | "ax-evidence-report"
    ) {
        return run_ax_command(&command, &mut arguments);
    }
    if command == "capture-ax-client" {
        return run_ax_capture_command(&mut arguments);
    }
    if command == "validate-trace-sequence" {
        let Some(path) = arguments.next() else {
            return Err(vec![format!("{command} requires a manifest path")]);
        };
        if arguments.next().is_some() {
            return Err(vec![format!("{command} accepts exactly one manifest path")]);
        }
        return trace_sequence::validate(Path::new(&path), Path::new("."));
    }
    if command == "render-trace-sequence-native" {
        let Some(manifest) = arguments.next() else {
            return Err(vec![format!(
                "{command} requires a manifest and output path"
            )]);
        };
        let Some(output) = arguments.next() else {
            return Err(vec![format!(
                "{command} requires a manifest and output path"
            )]);
        };
        if arguments.next().is_some() {
            return Err(vec![format!("{command} accepts exactly two paths")]);
        }
        return trace_sequence::render_native(
            Path::new(&manifest),
            Path::new(&output),
            Path::new("."),
        );
    }
    if matches!(
        command.as_str(),
        "validate-qualification"
            | "qualification-report"
            | "validate-aa-calibration"
            | "aa-calibration-report"
            | "validate-scene-trace"
    ) {
        return run_qualification_command(&command, &mut arguments);
    }
    if matches!(
        command.as_str(),
        "render-scene-reference" | "render-scene-native"
    ) {
        let Some(manifest) = arguments.next() else {
            return Err(vec![format!(
                "{command} requires a scene trace and output path"
            )]);
        };
        let Some(output) = arguments.next() else {
            return Err(vec![format!(
                "{command} requires a scene trace and output path"
            )]);
        };
        if arguments.next().is_some() {
            return Err(vec![format!("{command} accepts exactly two paths")]);
        }
        return qualification::render_scene(
            command == "render-scene-native",
            Path::new(&manifest),
            Path::new(&output),
        );
    }
    if matches!(
        command.as_str(),
        "benchmark-scene-reference" | "benchmark-scene-native" | "profile-scene-native"
    ) {
        let values = arguments.collect::<Vec<_>>();
        if values.len() != 4 {
            return Err(vec![format!(
                "{command} requires a scene trace, output path, warmup count, and sample count"
            )]);
        }
        let warmup_iterations = values[2].parse::<u64>().map_err(|_| {
            vec![format!(
                "{command} warmup count must be an unsigned integer"
            )]
        })?;
        let sample_count = values[3].parse::<u64>().map_err(|_| {
            vec![format!(
                "{command} sample count must be an unsigned integer"
            )]
        })?;
        if command == "profile-scene-native" {
            return qualification::profile_native_scene(
                Path::new(&values[0]),
                Path::new(&values[1]),
                warmup_iterations,
                sample_count,
            );
        }
        return qualification::benchmark_scene(
            command == "benchmark-scene-native",
            Path::new(&values[0]),
            Path::new(&values[1]),
            warmup_iterations,
            sample_count,
        );
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

fn run_lab_command(
    command: &str,
    arguments: &mut impl Iterator<Item = String>,
) -> Result<String, Vec<String>> {
    let Some(path) = arguments.next() else {
        return Err(vec![format!("{command} requires an evidence path")]);
    };
    if arguments.next().is_some() {
        return Err(vec![format!("{command} accepts exactly one evidence path")]);
    }
    let path = Path::new(&path);
    if lab_lifecycle::is_lifecycle_evidence(path) {
        lab_lifecycle::run(command, path)
    } else if lab_v2::is_v2_evidence(path) {
        lab_v2::run(command, path)
    } else {
        lab::run(command, path)
    }
}

fn run_qualification_command(
    command: &str,
    arguments: &mut impl Iterator<Item = String>,
) -> Result<String, Vec<String>> {
    let Some(path) = arguments.next() else {
        return Err(vec![format!("{command} requires a manifest path")]);
    };
    if arguments.next().is_some() {
        return Err(vec![format!("{command} accepts exactly one manifest path")]);
    }
    if matches!(command, "validate-aa-calibration" | "aa-calibration-report") {
        return calibration::run(command, Path::new(&path), Path::new("."));
    }
    if command == "validate-scene-trace" {
        return qualification::run_scene(Path::new(&path), Path::new("."));
    }
    qualification::run(command, Path::new(&path), Path::new("."))
}

fn run_ax_command(
    command: &str,
    arguments: &mut impl Iterator<Item = String>,
) -> Result<String, Vec<String>> {
    let Some(path) = arguments.next() else {
        return Err(vec![format!("{command} requires an artifact bundle path")]);
    };
    if arguments.next().is_some() {
        return Err(vec![format!("{command} accepts exactly one bundle path")]);
    }
    ax::run(command, Path::new(&path))
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

fn run_upstream_radar() -> Result<String, Vec<String>> {
    let path = Path::new("assurance/upstreams.toml");
    let source = fs::read_to_string(path)
        .map_err(|error| vec![format!("cannot read {}: {error}", path.display())])?;
    let registry: UpstreamRegistry = toml::from_str(&source)
        .map_err(|error| vec![format!("cannot parse {}: {error}", path.display())])?;
    let destination = env::var("GH_REPOSITORY")
        .map_err(|_| vec!["upstream-radar requires GH_REPOSITORY".to_owned()])?;
    let mut messages = Vec::new();
    let mut errors = Vec::new();

    for upstream in registry.upstreams {
        let Some(head) = command_output(
            "gh",
            &[
                "api",
                &format!("repos/{}/commits/HEAD", upstream.repository),
                "--jq",
                ".sha",
            ],
        ) else {
            errors.push(format!("cannot retrieve {} HEAD", upstream.repository));
            continue;
        };
        if head == upstream.baseline_commit {
            messages.push(format!("{} remains at reviewed baseline", upstream.name));
            continue;
        }
        let title = format!("Research: re-evaluate {} upstream changes", upstream.name);
        let body = format!(
            "Upstream radar detected a change after research #{}.\n\nRepository: https://github.com/{}\nBaseline: {}\nCurrent HEAD: {}\n\nReview the changed architecture, behavior, tests, license, and candidate Alpine claims. Update the durable case study and baseline only after review.",
            upstream.research_issue, upstream.repository, upstream.baseline_commit, head
        );
        match ensure_research_issue(&destination, &title, &body) {
            Some(result) => messages.push(result),
            None => errors.push(format!(
                "cannot create or find radar issue for {}",
                upstream.name
            )),
        }
    }

    let today = command_output("date", &["+%F"])
        .ok_or_else(|| vec!["cannot determine current date".to_owned()])?;
    for source in registry.manual_sources {
        if today < source.next_review_on {
            messages.push(format!("{} manual review is not due", source.name));
            continue;
        }
        let title = format!("Research: re-evaluate {} documentation", source.name);
        let body = format!(
            "The scheduled manual upstream review is due after research #{}.\n\nSource: {}\nReview due: {}\n\nRecord the exact documentation versions or page revisions available, platform behavior changes, and derived Alpine claims. Update next_review_on only after review.",
            source.research_issue, source.url, source.next_review_on
        );
        match ensure_research_issue(&destination, &title, &body) {
            Some(result) => messages.push(result),
            None => errors.push(format!(
                "cannot create or find radar issue for {}",
                source.name
            )),
        }
    }

    if errors.is_empty() {
        Ok(messages.join("\n"))
    } else {
        Err(errors)
    }
}

fn ensure_research_issue(repository: &str, title: &str, body: &str) -> Option<String> {
    let count = command_output(
        "gh",
        &[
            "issue",
            "list",
            "--repo",
            repository,
            "--state",
            "open",
            "--search",
            &format!("{title} in:title"),
            "--json",
            "number",
            "--jq",
            "length",
        ],
    )?;
    if count != "0" {
        return Some(format!("open radar issue already exists: {title}"));
    }
    command_output(
        "gh",
        &[
            "issue",
            "create",
            "--repo",
            repository,
            "--title",
            title,
            "--body",
            body,
            "--label",
            "kind:research",
            "--label",
            "release:none",
        ],
    )
    .map(|url| format!("opened {url}"))
}

fn command_output(program: &str, arguments: &[&str]) -> Option<String> {
    let output = Command::new(program).args(arguments).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}
