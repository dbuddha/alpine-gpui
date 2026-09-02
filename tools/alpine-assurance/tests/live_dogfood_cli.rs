//! Process-boundary controls for live Studio dogfood capture sealing.

use std::{fs, path::Path, process::Command};

const REVISION: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SHA: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn command(arguments: &[&str]) -> Result<std::process::Output, Box<dyn std::error::Error>> {
    Ok(Command::new(env!("CARGO_BIN_EXE_alpine-assurance"))
        .args(arguments)
        .output()?)
}

fn invoke_failure(arguments: &[&str]) -> Result<String, Box<dyn std::error::Error>> {
    let output = command(arguments)?;
    if output.status.success() {
        return Err(format!("live dogfood CLI unexpectedly accepted {arguments:?}").into());
    }
    Ok(String::from_utf8(output.stderr)?)
}

fn invoke_success(arguments: &[&str]) -> Result<String, Box<dyn std::error::Error>> {
    let output = command(arguments)?;
    if !output.status.success() {
        return Err(format!(
            "live dogfood CLI rejected {arguments:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(String::from_utf8(output.stdout)?)
}

fn seal_arguments<'a>(pid: &'a str, duration: &'a str, interval: &'a str) -> Vec<&'a str> {
    vec![
        "seal-live-studio-dogfood",
        "draft.toml",
        "internal.json",
        "footprint.json",
        "stdout",
        "stderr",
        "binary",
        "sampler",
        pid,
        duration,
        interval,
        "fixture",
        "fixture-start",
        REVISION,
        "2026-08-30T18:00:00Z",
        "bundle",
    ]
}

fn write_fixture(root: &Path, pid: u32) -> Result<(), Box<dyn std::error::Error>> {
    fs::create_dir_all(root)?;
    fs::write(
        root.join("draft.toml"),
        format!(
            r#"schema = "alpine-studio-dogfood-draft/v2"

[identity]
id = "fixture-live-session"
workload_id = "fixture-live"
workload_version = 1
workspace_fixture = "fixture-workspace"
workspace_fixture_sha256 = "{SHA}"
settings_profile = "fixture-settings"
settings_sha256 = "{SHA}"
opt_in = true
telemetry = false
network_io = false
performance_claim = "none"
coverage = ["launch", "workspace", "editing", "language", "accessibility", "lifecycle", "memory", "shutdown"]
assumptions = ["headless fixture process and fake sampler"]
exclusions = ["telemetry", "network-io", "comparative-claim"]

[identity.environment]
hardware_id = "fixture-apple-silicon"
os_build = "fixture"
architecture = "arm64"
display_refresh_hz = 60
power_source = "ac"
thermal_state = "nominal"
toolchain = "fixture"
locale = "en_US.UTF-8"

[identity.font]
family = "SF Mono"
postscript_name = "SFMono-Regular"
size_milli_points = 13000

[identity.language_server]
name = "none"
version = "none"
executable_sha256 = "none"
"#
        ),
    )?;
    fs::write(
        root.join("internal.json"),
        format!(
            r#"{{
  "schema":"alpine-studio-internal-diagnostic/v1",
  "workload_id":"fixture-live",
  "alpine_revision":"{REVISION}",
  "captured_at_utc":"2026-08-30T18:00:00Z",
  "duration_ms":4000,
  "outcome":"passed",
  "status":"clean native close captured",
  "frames":{{"requested":2,"submitted":2,"completed":2,"presented":1,"omitted":1,"idle_submissions":0,"peak_in_flight":1}},
  "text":{{"shape_calls":2,"rasterize_calls":1,"syntax_cache_hits":1,"syntax_cache_misses":1,"syntax_omitted_lines":0}},
  "language":{{"requests":1,"responses":null,"stale_responses":0,"restarts":0,"current_retained_bytes":8,"peak_retained_bytes":16,"budget_bytes":32}},
  "accessibility":{{"queries":1,"actions":1,"stale_actions":null,"retained_nodes":2,"peak_retained_nodes":2}},
  "lifecycle":{{"close_requests":1,"close_completions":1,"clean_shutdown":true,"post_close_bytes":0,"post_close_limit_bytes":0}},
  "resources":[
    {{"name":"layout-cache","current_bytes":1,"peak_bytes":2,"budget_bytes":4,"omitted":false}},
    {{"name":"syntax-cache","current_bytes":1,"peak_bytes":2,"budget_bytes":4,"omitted":false}},
    {{"name":"glyph-atlas-cpu","current_bytes":1,"peak_bytes":2,"budget_bytes":4,"omitted":false}},
    {{"name":"glyph-atlas-gpu","current_bytes":null,"peak_bytes":null,"budget_bytes":null,"omitted":true}},
    {{"name":"font-cache","current_bytes":null,"peak_bytes":null,"budget_bytes":null,"omitted":true}},
    {{"name":"fallback-cache","current_bytes":null,"peak_bytes":null,"budget_bytes":null,"omitted":true}},
    {{"name":"language-process","current_bytes":8,"peak_bytes":16,"budget_bytes":32,"omitted":false}},
    {{"name":"foreground-queue","current_bytes":1,"peak_bytes":2,"budget_bytes":4,"omitted":false}},
    {{"name":"background-queue","current_bytes":null,"peak_bytes":null,"budget_bytes":null,"omitted":true}},
    {{"name":"upload-staging","current_bytes":1,"peak_bytes":2,"budget_bytes":null,"omitted":false,"omitted_axes":["budget_bytes"]}}
  ],
  "runtime":{{"stale_results":0}},
  "surface":{{"current_retained_bytes":0}},
  "omissions":["accessibility-stale-actions","background-queue","fallback-cache","font-cache","glyph-atlas-gpu","language-responses","process-gpu-bytes","stage-timings","upload-staging-budget"]
}}"#
        ),
    )?;
    fs::write(
        root.join("footprint.json"),
        format!(
            r#"{{"unit":"byte","bytes per unit":1,"samples":[
{{"start_time":{{"wall_time_s":1000.0}},"processes":[{{"pid":{pid},"auxiliary":{{"phys_footprint":100,"phys_footprint_peak":100}}}}],"summary":{{"total":{{"dirty":50}}}}}},
{{"start_time":{{"wall_time_s":1001.0}},"processes":[{{"pid":{pid},"auxiliary":{{"phys_footprint":110,"phys_footprint_peak":110}}}}],"summary":{{"total":{{"dirty":55}}}}}},
{{"start_time":{{"wall_time_s":1002.0}},"processes":[{{"pid":{pid},"auxiliary":{{"phys_footprint":105,"phys_footprint_peak":110}}}}],"summary":{{"total":{{"dirty":52}}}}}},
{{"start_time":{{"wall_time_s":1003.0}},"processes":[{{"pid":{pid},"auxiliary":{{"phys_footprint":108,"phys_footprint_peak":110}}}}],"summary":{{"total":{{"dirty":54}}}}}}
]}}"#
        ),
    )?;
    fs::write(root.join("stdout"), b"fixture stdout\n")?;
    fs::write(root.join("stderr"), b"fixture stderr\n")?;
    fs::write(root.join("binary"), b"fixture binary\n")?;
    fs::write(root.join("sampler"), b"fixture sampler\n")?;
    Ok(())
}

#[test]
fn seal_dispatch_rejects_missing_and_malformed_numeric_arguments()
-> Result<(), Box<dyn std::error::Error>> {
    let missing_draft = invoke_failure(&["validate-live-studio-dogfood-draft"])?;
    assert!(missing_draft.contains("requires a draft path"));

    let missing = invoke_failure(&["seal-live-studio-dogfood"])?;
    assert!(missing.contains("requires draft, internal JSON"));

    let bad_pid = invoke_failure(&seal_arguments("pid", "3000", "1000"))?;
    assert!(bad_pid.contains("PID must be an unsigned integer"));

    let bad_duration = invoke_failure(&seal_arguments("42", "duration", "1000"))?;
    assert!(bad_duration.contains("duration must be unsigned milliseconds"));

    let bad_interval = invoke_failure(&seal_arguments("42", "3000", "interval"))?;
    assert!(bad_interval.contains("interval must be unsigned milliseconds"));
    Ok(())
}

#[test]
fn seals_validates_reports_and_rejects_tampering_through_the_binary()
-> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::temp_dir().join(format!("alpine-live-dogfood-cli-{}", std::process::id()));
    if root.exists() {
        fs::remove_dir_all(&root)?;
    }
    write_fixture(&root, std::process::id())?;

    let draft = root.join("draft.toml");
    let draft_text = draft.to_string_lossy();
    let preflight = invoke_success(&["validate-live-studio-dogfood-draft", &draft_text])?;
    assert!(preflight.contains("validated live Studio dogfood draft fixture-live-session"));

    let invalid_draft = root.join("invalid-draft.toml");
    fs::write(
        &invalid_draft,
        fs::read_to_string(&draft)?
            .replace("alpine-studio-dogfood-draft/v2", "alpine-studio-dogfood/v2"),
    )?;
    let invalid_text = invalid_draft.to_string_lossy();
    let rejected = invoke_failure(&["validate-live-studio-dogfood-draft", &invalid_text])?;
    assert!(rejected.contains("draft schema must be alpine-studio-dogfood-draft/v2"));

    let pid = std::process::id().to_string();
    let relative = seal_arguments(&pid, "3000", "1000");
    let owned = relative
        .iter()
        .enumerate()
        .map(|(index, argument)| {
            if matches!(index, 1..=7 | 15) {
                root.join(argument).to_string_lossy().into_owned()
            } else {
                (*argument).to_owned()
            }
        })
        .collect::<Vec<_>>();
    let arguments = owned.iter().map(String::as_str).collect::<Vec<_>>();
    invoke_success(&arguments)?;

    let manifest = root.join("bundle/session.toml");
    let manifest_text = manifest.to_string_lossy();
    let validation = invoke_success(&["validate-studio-dogfood", &manifest_text])?;
    assert!(validation.contains("4 physical samples"));
    let report = invoke_success(&["studio-dogfood-report", &manifest_text])?;
    assert!(report.contains("Evidence scope:"));
    assert!(report.contains("GPU process sampling: omitted, unavailable"));

    fs::write(root.join("bundle/footprint.json"), b"tampered\n")?;
    let tamper = invoke_failure(&["validate-studio-dogfood", &manifest_text])?;
    assert!(tamper.contains("SHA-256 differs"));

    fs::remove_dir_all(root)?;
    Ok(())
}
