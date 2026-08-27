//! Validates bounded, local-only Alpine Studio dogfood capture bundles.

use std::{
    collections::BTreeSet,
    fmt::Write as _,
    fs,
    path::{Component, Path, PathBuf},
    process::Command,
};

use serde::{Deserialize, Serialize};

const MAX_MANIFEST_BYTES: u64 = 65_536;
const MAX_SNAPSHOT_BYTES: u64 = 1_048_576;
const MAX_DURATION_MS: u64 = 604_800_000;
const MAX_SAMPLES: usize = 4_096;
const MAX_TEXT_BYTES: usize = 4_096;
const MAX_LIST_ITEMS: usize = 32;
const MAX_STAGE_SAMPLES: u64 = 1_000_000_000;
const REQUIRED_STAGES: &[&str] = &[
    "scene-build",
    "adaptation",
    "atlas-upload",
    "encode",
    "commit",
    "gpu-completion",
    "presentation",
    "input-to-present",
    "shutdown-drain",
];
const REQUIRED_RESOURCES: &[&str] = &[
    "layout-cache",
    "syntax-cache",
    "glyph-atlas-cpu",
    "glyph-atlas-gpu",
    "font-cache",
    "fallback-cache",
    "language-process",
    "foreground-queue",
    "background-queue",
    "upload-staging",
];
const COVERAGE_KINDS: &[&str] = &[
    "launch",
    "workspace",
    "editing",
    "language",
    "accessibility",
    "lifecycle",
    "memory",
    "shutdown",
];
const REQUIRED_EXCLUSIONS: &[&str] = &["telemetry", "network-io", "comparative-claim"];

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Capture {
    schema: String,
    id: String,
    captured_at_utc: String,
    alpine_revision: String,
    workload_id: String,
    workload_version: u32,
    duration_ms: u64,
    workspace_fixture: String,
    workspace_fixture_sha256: String,
    settings_profile: String,
    settings_sha256: String,
    output_sha256: String,
    snapshot_file: String,
    snapshot_sha256: String,
    opt_in: bool,
    telemetry: bool,
    network_io: bool,
    performance_claim: String,
    coverage: Vec<String>,
    assumptions: Vec<String>,
    exclusions: Vec<String>,
    environment: Environment,
    font: FontIdentity,
    language_server: LanguageServerIdentity,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Environment {
    hardware_id: String,
    os_build: String,
    architecture: String,
    display_refresh_hz: u32,
    power_source: String,
    thermal_state: String,
    toolchain: String,
    locale: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct FontIdentity {
    family: String,
    postscript_name: String,
    size_milli_points: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct LanguageServerIdentity {
    name: String,
    version: String,
    executable_sha256: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    schema: String,
    workload_id: String,
    duration_ms: u64,
    outcome: String,
    status: String,
    frames: FrameSnapshot,
    language: LanguageSnapshot,
    accessibility: AccessibilitySnapshot,
    lifecycle: LifecycleSnapshot,
    stages: Vec<StageSnapshot>,
    resources: Vec<ResourceSnapshot>,
    samples: Vec<ProcessSample>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FrameSnapshot {
    requested: u64,
    submitted: u64,
    completed: u64,
    presented: u64,
    omitted: u64,
    idle_submissions: u64,
    peak_in_flight: u8,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LanguageSnapshot {
    requests: u64,
    responses: u64,
    stale_responses: u64,
    restarts: u64,
    current_retained_bytes: u64,
    peak_retained_bytes: u64,
    budget_bytes: u64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AccessibilitySnapshot {
    queries: u64,
    actions: u64,
    stale_actions: u64,
    retained_nodes: u32,
    peak_retained_nodes: u32,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LifecycleSnapshot {
    close_requests: u64,
    close_completions: u64,
    clean_shutdown: bool,
    post_close_bytes: u64,
    post_close_limit_bytes: u64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StageSnapshot {
    name: String,
    samples: u64,
    total_ns: u64,
    peak_ns: u64,
    omitted: u64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResourceSnapshot {
    name: String,
    current_bytes: u64,
    peak_bytes: u64,
    budget_bytes: u64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProcessSample {
    sequence: u32,
    elapsed_ms: u64,
    physical_footprint_bytes: u64,
    private_dirty_bytes: u64,
    gpu_bytes: u64,
    alpine_retained_bytes: u64,
}

pub(crate) fn run(command: &str, manifest_path: &Path) -> Result<String, Vec<String>> {
    let capture: Capture = load_toml(manifest_path, MAX_MANIFEST_BYTES)?;
    let bundle = manifest_path.parent().unwrap_or_else(|| Path::new("."));
    let snapshot_path =
        resolve_bundle_file(bundle, &capture.snapshot_file).map_err(|error| vec![error])?;
    let snapshot: Snapshot = load_toml(&snapshot_path, MAX_SNAPSHOT_BYTES)?;
    let mut errors = validate(&capture, &snapshot);
    match calculate_sha256(&snapshot_path) {
        Ok(actual) if actual != capture.snapshot_sha256 => errors.push(format!(
            "snapshot SHA-256 mismatch: declared {}, actual {actual}",
            capture.snapshot_sha256
        )),
        Ok(_) => {}
        Err(error) => errors.push(error),
    }
    errors.sort();
    if !errors.is_empty() {
        return Err(errors);
    }
    match command {
        "validate-studio-dogfood" => Ok(format!(
            "validated Studio dogfood capture {} with {} process samples, {} stage records, and {} bounded resources",
            capture.id,
            snapshot.samples.len(),
            snapshot.stages.len(),
            snapshot.resources.len()
        )),
        "studio-dogfood-report" => Ok(render_report(&capture, &snapshot)),
        _ => Err(vec![format!(
            "unsupported Studio dogfood command {command}"
        )]),
    }
}

pub(crate) fn record(
    draft_path: &Path,
    snapshot_path: &Path,
    destination: &Path,
) -> Result<String, Vec<String>> {
    let mut capture: Capture = load_toml(draft_path, MAX_MANIFEST_BYTES)?;
    let snapshot: Snapshot = load_toml(snapshot_path, MAX_SNAPSHOT_BYTES)?;
    "snapshot.toml".clone_into(&mut capture.snapshot_file);
    capture.snapshot_sha256 = calculate_sha256(snapshot_path).map_err(|error| vec![error])?;
    let mut errors = validate(&capture, &snapshot);
    errors.sort();
    if !errors.is_empty() {
        return Err(errors);
    }
    if destination.exists() {
        return Err(vec![format!(
            "dogfood capture destination {} already exists",
            destination.display()
        )]);
    }
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    if !parent.is_dir() {
        return Err(vec![format!(
            "dogfood capture parent {} is unavailable",
            parent.display()
        )]);
    }
    let staging = staging_path(destination)?;
    if staging.exists() {
        return Err(vec![format!(
            "dogfood capture staging path {} already exists",
            staging.display()
        )]);
    }
    fs::create_dir(&staging).map_err(|error| {
        vec![format!(
            "cannot create dogfood staging directory {}: {error}",
            staging.display()
        )]
    })?;
    let result = write_staged_bundle(&capture, snapshot_path, &staging, destination);
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

fn staging_path(destination: &Path) -> Result<PathBuf, Vec<String>> {
    let name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| vec!["dogfood capture destination needs a UTF-8 file name".to_owned()])?;
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    Ok(parent.join(format!(".{name}.staging")))
}

fn write_staged_bundle(
    capture: &Capture,
    snapshot_path: &Path,
    staging: &Path,
    destination: &Path,
) -> Result<String, Vec<String>> {
    let staged_snapshot = staging.join("snapshot.toml");
    fs::copy(snapshot_path, &staged_snapshot).map_err(|error| {
        vec![format!(
            "cannot stage dogfood snapshot {}: {error}",
            snapshot_path.display()
        )]
    })?;
    let source = toml::to_string_pretty(capture)
        .map_err(|error| vec![format!("cannot encode dogfood manifest: {error}")])?;
    let manifest = staging.join("session.toml");
    fs::write(&manifest, source).map_err(|error| {
        vec![format!(
            "cannot stage dogfood manifest {}: {error}",
            manifest.display()
        )]
    })?;
    run("validate-studio-dogfood", &manifest)?;
    fs::rename(staging, destination).map_err(|error| {
        vec![format!(
            "cannot publish dogfood capture {}: {error}",
            destination.display()
        )]
    })?;
    Ok(format!(
        "recorded Studio dogfood capture {} at {}",
        capture.id,
        destination.display()
    ))
}

fn load_toml<T>(path: &Path, limit: u64) -> Result<T, Vec<String>>
where
    T: for<'de> Deserialize<'de>,
{
    let metadata = fs::metadata(path)
        .map_err(|error| vec![format!("cannot inspect {}: {error}", path.display())])?;
    if metadata.len() > limit {
        return Err(vec![format!(
            "{} is {} bytes; limit is {limit}",
            path.display(),
            metadata.len()
        )]);
    }
    let source = fs::read_to_string(path)
        .map_err(|error| vec![format!("cannot read {}: {error}", path.display())])?;
    toml::from_str(&source)
        .map_err(|error| vec![format!("cannot parse {}: {error}", path.display())])
}

fn resolve_bundle_file(bundle: &Path, relative: &str) -> Result<PathBuf, String> {
    let path = Path::new(relative);
    if relative.is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!(
            "snapshot file {relative:?} must remain within the capture bundle"
        ));
    }
    Ok(bundle.join(path))
}

#[allow(
    clippy::too_many_lines,
    reason = "one validator keeps capture identity and snapshot invariants auditable together"
)]
fn validate(capture: &Capture, snapshot: &Snapshot) -> Vec<String> {
    let mut errors = Vec::new();
    require(
        capture.schema == "alpine-studio-dogfood/v1",
        "capture schema must be alpine-studio-dogfood/v1",
        &mut errors,
    );
    require(
        snapshot.schema == "alpine-studio-diagnostic/v1",
        "snapshot schema must be alpine-studio-diagnostic/v1",
        &mut errors,
    );
    require(
        valid_slug(&capture.id),
        "capture id must be a slug",
        &mut errors,
    );
    require(
        valid_slug(&capture.workload_id),
        "workload id must be a slug",
        &mut errors,
    );
    require(
        capture.workload_id == snapshot.workload_id,
        "capture and snapshot workload identities must match",
        &mut errors,
    );
    require(
        valid_timestamp(&capture.captured_at_utc),
        "capture timestamp must be UTC YYYY-MM-DDTHH:MM:SSZ",
        &mut errors,
    );
    require(
        valid_git_sha(&capture.alpine_revision),
        "Alpine revision must be a lowercase 40-character Git SHA",
        &mut errors,
    );
    require(
        capture.workload_version > 0,
        "workload version must be positive",
        &mut errors,
    );
    require(
        capture.duration_ms > 0 && capture.duration_ms <= MAX_DURATION_MS,
        "capture duration must be within one millisecond and seven days",
        &mut errors,
    );
    require(
        capture.duration_ms == snapshot.duration_ms,
        "capture and snapshot durations must match",
        &mut errors,
    );
    for (name, value) in [
        ("workspace fixture", capture.workspace_fixture.as_str()),
        ("settings profile", capture.settings_profile.as_str()),
        ("hardware id", capture.environment.hardware_id.as_str()),
        ("OS build", capture.environment.os_build.as_str()),
        ("toolchain", capture.environment.toolchain.as_str()),
        ("locale", capture.environment.locale.as_str()),
        ("font family", capture.font.family.as_str()),
        (
            "font PostScript name",
            capture.font.postscript_name.as_str(),
        ),
        (
            "language-server name",
            capture.language_server.name.as_str(),
        ),
        (
            "language-server version",
            capture.language_server.version.as_str(),
        ),
        ("status", snapshot.status.as_str()),
    ] {
        require(
            !value.is_empty() && value.len() <= MAX_TEXT_BYTES,
            format!("{name} must contain 1 to {MAX_TEXT_BYTES} bytes"),
            &mut errors,
        );
    }
    for (name, hash) in [
        (
            "workspace fixture",
            capture.workspace_fixture_sha256.as_str(),
        ),
        ("settings", capture.settings_sha256.as_str()),
        ("output", capture.output_sha256.as_str()),
        ("snapshot", capture.snapshot_sha256.as_str()),
    ] {
        require(
            valid_sha256(hash),
            format!("{name} identity must be a lowercase SHA-256"),
            &mut errors,
        );
    }
    require(
        capture.opt_in,
        "capture must be explicitly opt-in",
        &mut errors,
    );
    require(
        !capture.telemetry,
        "capture must disable telemetry",
        &mut errors,
    );
    require(
        !capture.network_io,
        "capture must perform no network I/O",
        &mut errors,
    );
    require(
        capture.performance_claim == "none",
        "dogfood capture cannot contain a performance claim",
        &mut errors,
    );
    validate_list(
        "coverage",
        &capture.coverage,
        COVERAGE_KINDS,
        false,
        &mut errors,
    );
    validate_list(
        "exclusions",
        &capture.exclusions,
        REQUIRED_EXCLUSIONS,
        true,
        &mut errors,
    );
    validate_free_list("assumptions", &capture.assumptions, &mut errors);
    require(
        capture.environment.architecture == "arm64",
        "dogfood capture architecture must be arm64",
        &mut errors,
    );
    require(
        matches!(capture.environment.display_refresh_hz, 60 | 120),
        "display refresh must be 60 or 120 Hz",
        &mut errors,
    );
    require(
        matches!(capture.environment.power_source.as_str(), "ac" | "battery"),
        "power source must be ac or battery",
        &mut errors,
    );
    require(
        matches!(
            capture.environment.thermal_state.as_str(),
            "nominal" | "fair" | "serious" | "critical"
        ),
        "thermal state is unsupported",
        &mut errors,
    );
    require(
        (1_000..=512_000).contains(&capture.font.size_milli_points),
        "font size must be within 1 and 512 points",
        &mut errors,
    );
    require(
        capture.language_server.executable_sha256 == "none"
            || valid_sha256(&capture.language_server.executable_sha256),
        "language-server executable identity must be none or a lowercase SHA-256",
        &mut errors,
    );
    validate_snapshot(snapshot, &mut errors);
    errors
}

#[allow(
    clippy::too_many_lines,
    reason = "snapshot validation keeps every bounded diagnostic class explicit"
)]
fn validate_snapshot(snapshot: &Snapshot, errors: &mut Vec<String>) {
    require(
        matches!(snapshot.outcome.as_str(), "passed" | "failed"),
        "snapshot outcome must be passed or failed",
        errors,
    );
    require(
        snapshot.frames.presented <= snapshot.frames.completed
            && snapshot.frames.completed <= snapshot.frames.submitted
            && snapshot.frames.submitted <= snapshot.frames.requested,
        "frame counters must preserve requested, submitted, completed, and presented order",
        errors,
    );
    require(
        snapshot.frames.peak_in_flight <= 3,
        "peak in-flight frames must not exceed three",
        errors,
    );
    require(
        snapshot.language.current_retained_bytes <= snapshot.language.peak_retained_bytes
            && snapshot.language.peak_retained_bytes <= snapshot.language.budget_bytes,
        "language retained bytes must remain within peak and budget",
        errors,
    );
    require(
        snapshot.language.responses <= snapshot.language.requests,
        "language responses cannot exceed requests",
        errors,
    );
    require(
        snapshot.accessibility.retained_nodes <= snapshot.accessibility.peak_retained_nodes
            && snapshot.accessibility.peak_retained_nodes <= 271,
        "accessibility nodes must remain within the bounded semantic tree",
        errors,
    );
    require(
        snapshot.lifecycle.close_completions <= snapshot.lifecycle.close_requests,
        "close completions cannot exceed close requests",
        errors,
    );

    let mut stages = BTreeSet::new();
    for stage in &snapshot.stages {
        require(
            REQUIRED_STAGES.contains(&stage.name.as_str()),
            format!("unsupported stage {}", stage.name),
            errors,
        );
        require(
            stages.insert(stage.name.as_str()),
            format!("duplicate stage {}", stage.name),
            errors,
        );
        require(
            stage.samples <= MAX_STAGE_SAMPLES,
            format!("stage {} exceeds the sample bound", stage.name),
            errors,
        );
        require(
            (stage.samples != 0 || (stage.total_ns == 0 && stage.peak_ns == 0))
                && stage.total_ns >= stage.peak_ns,
            format!("stage {} has inconsistent total and peak time", stage.name),
            errors,
        );
        let _ = stage.omitted;
    }
    for required in REQUIRED_STAGES {
        require(
            stages.contains(required),
            format!("snapshot lacks required stage {required}"),
            errors,
        );
    }

    let mut resources = BTreeSet::new();
    for resource in &snapshot.resources {
        require(
            REQUIRED_RESOURCES.contains(&resource.name.as_str()),
            format!("unsupported bounded resource {}", resource.name),
            errors,
        );
        require(
            resources.insert(resource.name.as_str()),
            format!("duplicate bounded resource {}", resource.name),
            errors,
        );
        require(
            resource.current_bytes <= resource.peak_bytes
                && resource.peak_bytes <= resource.budget_bytes,
            format!("resource {} exceeds its peak or budget", resource.name),
            errors,
        );
    }
    for required in REQUIRED_RESOURCES {
        require(
            resources.contains(required),
            format!("snapshot lacks required bounded resource {required}"),
            errors,
        );
    }

    require(
        !snapshot.samples.is_empty() && snapshot.samples.len() <= MAX_SAMPLES,
        format!("snapshot must retain 1 to {MAX_SAMPLES} process samples"),
        errors,
    );
    let mut previous_elapsed = 0;
    for (index, sample) in snapshot.samples.iter().enumerate() {
        require(
            usize::try_from(sample.sequence) == Ok(index),
            format!("process sample {index} has a non-contiguous sequence"),
            errors,
        );
        require(
            sample.elapsed_ms >= previous_elapsed && sample.elapsed_ms <= snapshot.duration_ms,
            format!("process sample {index} has an invalid elapsed time"),
            errors,
        );
        require(
            sample.private_dirty_bytes <= sample.physical_footprint_bytes,
            format!("process sample {index} has private dirty above physical footprint"),
            errors,
        );
        let _ = (sample.gpu_bytes, sample.alpine_retained_bytes);
        previous_elapsed = sample.elapsed_ms;
    }
    if let Some(last) = snapshot.samples.last() {
        require(
            last.elapsed_ms == snapshot.duration_ms,
            "final process sample must coincide with capture duration",
            errors,
        );
    }

    if snapshot.outcome == "passed" {
        require(
            snapshot.frames.idle_submissions == 0,
            "passed capture cannot contain an idle frame submission",
            errors,
        );
        require(
            snapshot.lifecycle.clean_shutdown,
            "passed capture requires clean shutdown",
            errors,
        );
        require(
            snapshot.lifecycle.post_close_bytes <= snapshot.lifecycle.post_close_limit_bytes,
            "passed capture exceeds the post-close byte limit",
            errors,
        );
        require(
            snapshot.lifecycle.close_completions == 1,
            "passed capture requires one completed close",
            errors,
        );
    }
}

fn validate_list(
    name: &str,
    values: &[String],
    allowed_or_required: &[&str],
    require_all: bool,
    errors: &mut Vec<String>,
) {
    require(
        !values.is_empty() && values.len() <= MAX_LIST_ITEMS,
        format!("{name} must contain 1 to {MAX_LIST_ITEMS} items"),
        errors,
    );
    let mut observed = BTreeSet::new();
    for value in values {
        require(
            allowed_or_required.contains(&value.as_str()),
            format!("{name} contains unsupported value {value}"),
            errors,
        );
        require(
            observed.insert(value.as_str()),
            format!("{name} contains duplicate value {value}"),
            errors,
        );
    }
    if require_all {
        for required in allowed_or_required {
            require(
                observed.contains(required),
                format!("{name} lacks required value {required}"),
                errors,
            );
        }
    }
}

fn validate_free_list(name: &str, values: &[String], errors: &mut Vec<String>) {
    require(
        !values.is_empty() && values.len() <= MAX_LIST_ITEMS,
        format!("{name} must contain 1 to {MAX_LIST_ITEMS} items"),
        errors,
    );
    for value in values {
        require(
            !value.is_empty() && value.len() <= MAX_TEXT_BYTES,
            format!("{name} item must contain 1 to {MAX_TEXT_BYTES} bytes"),
            errors,
        );
    }
}

fn require(condition: bool, message: impl Into<String>, errors: &mut Vec<String>) {
    if !condition {
        errors.push(message.into());
    }
}

fn valid_slug(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'.')
        })
}

fn valid_git_sha(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_timestamp(value: &str) -> bool {
    value.len() == 20
        && value.as_bytes().get(4) == Some(&b'-')
        && value.as_bytes().get(7) == Some(&b'-')
        && value.as_bytes().get(10) == Some(&b'T')
        && value.as_bytes().get(13) == Some(&b':')
        && value.as_bytes().get(16) == Some(&b':')
        && value.as_bytes().get(19) == Some(&b'Z')
        && value
            .bytes()
            .enumerate()
            .filter(|(index, _)| !matches!(index, 4 | 7 | 10 | 13 | 16 | 19))
            .all(|(_, byte)| byte.is_ascii_digit())
}

fn calculate_sha256(path: &Path) -> Result<String, String> {
    for (program, arguments) in [("sha256sum", &[][..]), ("shasum", &["-a", "256"][..])] {
        let output = Command::new(program).args(arguments).arg(path).output();
        let Ok(output) = output else {
            continue;
        };
        if !output.status.success() {
            continue;
        }
        if let Some(digest) = sha256_from_output(&output.stdout) {
            return Ok(digest);
        }
    }

    #[cfg(windows)]
    {
        let output = Command::new("certutil")
            .arg("-hashfile")
            .arg(path)
            .arg("SHA256")
            .output();
        if let Ok(output) = output
            && output.status.success()
            && let Some(digest) = sha256_from_output(&output.stdout)
        {
            return Ok(digest);
        }
    }

    Err(format!(
        "cannot calculate SHA-256 for {}; sha256sum, shasum, or certutil is required",
        path.display()
    ))
}

fn sha256_from_output(output: &[u8]) -> Option<String> {
    String::from_utf8_lossy(output)
        .split_ascii_whitespace()
        .map(str::to_ascii_lowercase)
        .find(|candidate| valid_sha256(candidate))
}

fn render_report(capture: &Capture, snapshot: &Snapshot) -> String {
    let mut output = format!(
        "# Alpine Studio dogfood capture {}\n\n- Performance claim: none\n- Alpine revision: {}\n- Workload: {} v{}\n- Outcome: {}\n- Duration: {} ms\n- Hardware: {}\n- OS: {}\n- Display: {} Hz\n- Process samples: {}\n- Frames requested/submitted/completed/presented: {}/{}/{}/{}\n- Idle submissions: {}\n- Peak in-flight frames: {}\n- Clean shutdown: {}\n- Post-close bytes: {} / {}\n\n## Stage evidence\n\n",
        capture.id,
        capture.alpine_revision,
        capture.workload_id,
        capture.workload_version,
        snapshot.outcome,
        capture.duration_ms,
        capture.environment.hardware_id,
        capture.environment.os_build,
        capture.environment.display_refresh_hz,
        snapshot.samples.len(),
        snapshot.frames.requested,
        snapshot.frames.submitted,
        snapshot.frames.completed,
        snapshot.frames.presented,
        snapshot.frames.idle_submissions,
        snapshot.frames.peak_in_flight,
        snapshot.lifecycle.clean_shutdown,
        snapshot.lifecycle.post_close_bytes,
        snapshot.lifecycle.post_close_limit_bytes,
    );
    let _ = write!(
        output,
        "- Omitted frames: {}\n- Language requests/responses/stale/restarts: {}/{}/{}/{}\n- Accessibility queries/actions/stale actions: {}/{}/{}\n\n",
        snapshot.frames.omitted,
        snapshot.language.requests,
        snapshot.language.responses,
        snapshot.language.stale_responses,
        snapshot.language.restarts,
        snapshot.accessibility.queries,
        snapshot.accessibility.actions,
        snapshot.accessibility.stale_actions,
    );
    for stage in &snapshot.stages {
        let _ = writeln!(
            output,
            "- `{}`: {} samples, {} total ns, {} peak ns, {} omitted",
            stage.name, stage.samples, stage.total_ns, stage.peak_ns, stage.omitted
        );
    }
    output.push_str("\n## Bounded resources\n\n");
    for resource in &snapshot.resources {
        let _ = writeln!(
            output,
            "- `{}`: {} current, {} peak, {} budget bytes",
            resource.name, resource.current_bytes, resource.peak_bytes, resource.budget_bytes
        );
    }
    output
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static RECORD_SEQUENCE: AtomicU64 = AtomicU64::new(1);

    fn fixture_paths() -> (PathBuf, PathBuf) {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let bundle = root.join("assurance/dogfood/v1");
        (bundle.join("session.toml"), bundle.join("snapshot.toml"))
    }

    fn fixture() -> Result<(Capture, Snapshot), String> {
        let (manifest, snapshot) = fixture_paths();
        let manifest_source = fs::read_to_string(manifest).map_err(|error| error.to_string())?;
        let snapshot_source = fs::read_to_string(snapshot).map_err(|error| error.to_string())?;
        let capture = toml::from_str(&manifest_source).map_err(|error| error.to_string())?;
        let snapshot = toml::from_str(&snapshot_source).map_err(|error| error.to_string())?;
        Ok((capture, snapshot))
    }

    fn has_error(capture: &Capture, snapshot: &Snapshot, needle: &str) -> bool {
        validate(capture, snapshot)
            .iter()
            .any(|error| error.contains(needle))
    }

    fn temporary_path(label: &str) -> PathBuf {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target");
        let sequence = RECORD_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        root.join(format!("dogfood-{label}-{}-{sequence}", std::process::id()))
    }

    #[test]
    fn canonical_bundle_validates_and_reports_no_claim() {
        let (manifest, _) = fixture_paths();
        assert!(run("validate-studio-dogfood", &manifest).is_ok());
        let report = run("studio-dogfood-report", &manifest);
        assert!(report.is_ok());
        if let Ok(report) = report {
            assert!(report.contains("Performance claim: none"));
            assert!(report.contains("Idle submissions: 0"));
            assert!(report.contains("`glyph-atlas-gpu`"));
        }
    }

    #[test]
    fn sha256_command_output_accepts_unix_and_windows_shapes() {
        let digest = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        assert_eq!(
            sha256_from_output(format!("{digest}  snapshot.toml\n").as_bytes()),
            Some(digest.to_owned())
        );
        assert_eq!(
            sha256_from_output(
                format!(
                    "SHA256 hash of snapshot.toml:\r\n{}\r\nCertUtil: completed successfully.\r\n",
                    digest.to_ascii_uppercase()
                )
                .as_bytes()
            ),
            Some(digest.to_owned())
        );
        assert_eq!(sha256_from_output(b"not-a-digest"), None);
    }

    #[test]
    fn recorder_seals_valid_bundle_and_refuses_overwrite() -> Result<(), String> {
        let (manifest, snapshot) = fixture_paths();
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target");
        fs::create_dir_all(&root).map_err(|error| error.to_string())?;
        let sequence = RECORD_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let destination = root.join(format!("dogfood-record-{}-{sequence}", std::process::id()));
        let result = record(&manifest, &snapshot, &destination);
        assert!(result.is_ok(), "{result:#?}");
        assert!(destination.join("session.toml").is_file());
        assert!(destination.join("snapshot.toml").is_file());
        assert!(run("validate-studio-dogfood", &destination.join("session.toml")).is_ok());
        let repeated = record(&manifest, &snapshot, &destination);
        assert!(
            repeated
                .as_ref()
                .is_err_and(|errors| errors.iter().any(|error| error.contains("already exists")))
        );
        let snapshot_path = destination.join("snapshot.toml");
        let mut tampered = fs::read_to_string(&snapshot_path).map_err(|error| error.to_string())?;
        tampered.push('\n');
        fs::write(&snapshot_path, tampered).map_err(|error| error.to_string())?;
        assert!(
            run("validate-studio-dogfood", &destination.join("session.toml")).is_err_and(
                |errors| errors
                    .iter()
                    .any(|error| error.contains("snapshot SHA-256 mismatch"))
            )
        );
        fs::remove_dir_all(destination).map_err(|error| error.to_string())?;
        Ok(())
    }

    #[test]
    fn file_size_and_bundle_path_boundaries_are_independent() -> Result<(), String> {
        let path = temporary_path("size-boundary.toml");
        let source = "answer = 1\n";
        let limit = 32_u64;
        let padding = usize::try_from(limit).map_err(|error| error.to_string())? - source.len();
        fs::write(&path, format!("{source}{}", " ".repeat(padding)))
            .map_err(|error| error.to_string())?;
        assert!(load_toml::<toml::Value>(&path, limit).is_ok());
        fs::write(&path, format!("{source}{}", " ".repeat(padding + 1)))
            .map_err(|error| error.to_string())?;
        assert!(
            load_toml::<toml::Value>(&path, limit)
                .is_err_and(|errors| { errors.iter().any(|error| error.contains("limit is 32")) })
        );
        fs::remove_file(path).map_err(|error| error.to_string())?;

        let bundle = Path::new("bundle");
        assert_eq!(
            resolve_bundle_file(bundle, "snapshot.toml"),
            Ok(bundle.join("snapshot.toml"))
        );
        assert!(resolve_bundle_file(bundle, "").is_err());
        assert!(resolve_bundle_file(bundle, "/snapshot.toml").is_err());
        assert!(resolve_bundle_file(bundle, "../snapshot.toml").is_err());
        assert_eq!(
            staging_path(Path::new("")),
            Err(vec![
                "dogfood capture destination needs a UTF-8 file name".to_owned()
            ])
        );
        Ok(())
    }

    #[test]
    fn identity_privacy_and_environment_breaks_are_independent() -> Result<(), String> {
        let (mut capture, snapshot) = fixture()?;
        capture.telemetry = true;
        assert!(
            validate(&capture, &snapshot)
                .iter()
                .any(|error| error.contains("telemetry"))
        );
        capture.telemetry = false;
        capture.network_io = true;
        assert!(
            validate(&capture, &snapshot)
                .iter()
                .any(|error| error.contains("network"))
        );
        capture.network_io = false;
        capture.performance_claim = "fastest".to_owned();
        assert!(
            validate(&capture, &snapshot)
                .iter()
                .any(|error| error.contains("performance claim"))
        );
        capture.performance_claim = "none".to_owned();
        capture.environment.display_refresh_hz = 90;
        assert!(
            validate(&capture, &snapshot)
                .iter()
                .any(|error| error.contains("60 or 120"))
        );
        Ok(())
    }

    #[test]
    fn manifest_scalar_and_text_bounds_are_independent() -> Result<(), String> {
        let (mut capture, snapshot) = fixture()?;
        capture.workload_version = 0;
        assert!(has_error(&capture, &snapshot, "workload version"));

        let (mut capture, mut snapshot) = fixture()?;
        capture.duration_ms = 0;
        snapshot.duration_ms = 0;
        assert!(has_error(&capture, &snapshot, "seven days"));
        capture.duration_ms = MAX_DURATION_MS + 1;
        snapshot.duration_ms = MAX_DURATION_MS + 1;
        assert!(has_error(&capture, &snapshot, "seven days"));

        let (mut capture, snapshot) = fixture()?;
        capture.workspace_fixture.clear();
        assert!(has_error(
            &capture,
            &snapshot,
            "workspace fixture must contain"
        ));
        capture.workspace_fixture = "x".repeat(MAX_TEXT_BYTES + 1);
        assert!(has_error(
            &capture,
            &snapshot,
            "workspace fixture must contain"
        ));
        Ok(())
    }

    #[test]
    fn capture_lists_reject_empty_oversized_and_invalid_items() -> Result<(), String> {
        let (mut capture, snapshot) = fixture()?;
        capture.coverage.clear();
        assert!(has_error(&capture, &snapshot, "coverage must contain"));
        capture.coverage = vec!["launch".to_owned(); MAX_LIST_ITEMS + 1];
        assert!(has_error(&capture, &snapshot, "coverage must contain"));
        capture.coverage = vec!["invalid".to_owned()];
        assert!(has_error(&capture, &snapshot, "unsupported value"));

        let (mut capture, snapshot) = fixture()?;
        capture.assumptions.clear();
        assert!(has_error(&capture, &snapshot, "assumptions must contain"));
        capture.assumptions = vec!["bounded".to_owned(); MAX_LIST_ITEMS + 1];
        assert!(has_error(&capture, &snapshot, "assumptions must contain"));
        capture.assumptions = vec![String::new()];
        assert!(has_error(&capture, &snapshot, "assumptions item"));
        capture.assumptions = vec!["x".repeat(MAX_TEXT_BYTES + 1)];
        assert!(has_error(&capture, &snapshot, "assumptions item"));
        Ok(())
    }

    #[test]
    fn passed_snapshot_rejects_idle_frames_leaks_and_unclean_close() -> Result<(), String> {
        let (capture, mut snapshot) = fixture()?;
        snapshot.frames.idle_submissions = 1;
        assert!(
            validate(&capture, &snapshot)
                .iter()
                .any(|error| error.contains("idle frame"))
        );
        snapshot.frames.idle_submissions = 0;
        snapshot.lifecycle.clean_shutdown = false;
        assert!(
            validate(&capture, &snapshot)
                .iter()
                .any(|error| error.contains("clean shutdown"))
        );
        snapshot.lifecycle.clean_shutdown = true;
        snapshot.lifecycle.post_close_bytes = snapshot.lifecycle.post_close_limit_bytes + 1;
        assert!(
            validate(&capture, &snapshot)
                .iter()
                .any(|error| error.contains("post-close"))
        );
        Ok(())
    }

    #[test]
    fn frame_language_and_accessibility_orders_are_independent() -> Result<(), String> {
        let (capture, mut snapshot) = fixture()?;
        snapshot.frames.presented = snapshot.frames.completed + 1;
        assert!(has_error(&capture, &snapshot, "frame counters"));
        let (_, mut snapshot) = fixture()?;
        snapshot.frames.completed = snapshot.frames.submitted + 1;
        assert!(has_error(&capture, &snapshot, "frame counters"));
        let (_, mut snapshot) = fixture()?;
        snapshot.frames.submitted = snapshot.frames.requested + 1;
        assert!(has_error(&capture, &snapshot, "frame counters"));

        let (_, mut snapshot) = fixture()?;
        snapshot.language.current_retained_bytes = snapshot.language.peak_retained_bytes + 1;
        assert!(has_error(&capture, &snapshot, "language retained bytes"));
        let (_, mut snapshot) = fixture()?;
        snapshot.language.peak_retained_bytes = snapshot.language.budget_bytes + 1;
        assert!(has_error(&capture, &snapshot, "language retained bytes"));

        let (_, mut snapshot) = fixture()?;
        snapshot.accessibility.retained_nodes = snapshot.accessibility.peak_retained_nodes + 1;
        assert!(has_error(&capture, &snapshot, "accessibility nodes"));
        let (_, mut snapshot) = fixture()?;
        snapshot.accessibility.peak_retained_nodes = 272;
        assert!(has_error(&capture, &snapshot, "accessibility nodes"));
        Ok(())
    }

    #[test]
    fn stage_resource_and_sample_bounds_are_discriminating() -> Result<(), String> {
        let (capture, mut snapshot) = fixture()?;
        snapshot.stages.pop();
        assert!(
            validate(&capture, &snapshot)
                .iter()
                .any(|error| error.contains("shutdown-drain"))
        );
        let (_, mut snapshot) = fixture()?;
        if let Some(resource) = snapshot.resources.first_mut() {
            resource.current_bytes = resource.budget_bytes + 1;
        }
        assert!(
            validate(&capture, &snapshot)
                .iter()
                .any(|error| error.contains("exceeds"))
        );
        let (_, mut snapshot) = fixture()?;
        if let Some(sample) = snapshot.samples.get_mut(1) {
            sample.sequence = 9;
        }
        assert!(
            validate(&capture, &snapshot)
                .iter()
                .any(|error| error.contains("non-contiguous"))
        );
        Ok(())
    }

    #[test]
    fn stage_timing_and_process_sample_axes_are_independent() -> Result<(), String> {
        let (capture, mut snapshot) = fixture()?;
        let stage = snapshot.stages.first_mut().ok_or("missing stage")?;
        stage.samples = 0;
        stage.total_ns = 0;
        stage.peak_ns = 0;
        assert!(!has_error(
            &capture,
            &snapshot,
            "inconsistent total and peak"
        ));
        let (_, mut snapshot) = fixture()?;
        let stage = snapshot.stages.first_mut().ok_or("missing stage")?;
        stage.samples = 0;
        stage.total_ns = 1;
        stage.peak_ns = 0;
        assert!(has_error(
            &capture,
            &snapshot,
            "inconsistent total and peak"
        ));
        let (_, mut snapshot) = fixture()?;
        let stage = snapshot.stages.first_mut().ok_or("missing stage")?;
        stage.samples = 0;
        stage.total_ns = 0;
        stage.peak_ns = 1;
        assert!(has_error(
            &capture,
            &snapshot,
            "inconsistent total and peak"
        ));
        let (_, mut snapshot) = fixture()?;
        let stage = snapshot.stages.first_mut().ok_or("missing stage")?;
        stage.samples = 1;
        stage.total_ns = 0;
        stage.peak_ns = 1;
        assert!(has_error(
            &capture,
            &snapshot,
            "inconsistent total and peak"
        ));

        let (_, mut snapshot) = fixture()?;
        snapshot.samples.clear();
        assert!(has_error(&capture, &snapshot, "1 to 4096 process samples"));
        let (_, mut snapshot) = fixture()?;
        snapshot.samples = (0..=MAX_SAMPLES)
            .map(|sequence| ProcessSample {
                sequence: u32::try_from(sequence).unwrap_or(u32::MAX),
                elapsed_ms: snapshot.duration_ms,
                physical_footprint_bytes: 1,
                private_dirty_bytes: 1,
                gpu_bytes: 0,
                alpine_retained_bytes: 0,
            })
            .collect();
        assert!(has_error(&capture, &snapshot, "1 to 4096 process samples"));

        let (_, mut snapshot) = fixture()?;
        snapshot.samples[0].elapsed_ms = 1;
        snapshot.samples[1].elapsed_ms = 0;
        assert!(has_error(&capture, &snapshot, "invalid elapsed time"));
        let (_, mut snapshot) = fixture()?;
        snapshot.samples[0].elapsed_ms = snapshot.duration_ms + 1;
        assert!(has_error(&capture, &snapshot, "invalid elapsed time"));
        Ok(())
    }

    #[test]
    fn primitive_identity_and_timestamp_predicates_are_exact() {
        assert!(valid_slug("session-1.0"));
        assert!(!valid_slug("Session"));
        assert!(valid_git_sha(&"a".repeat(40)));
        assert!(!valid_git_sha(&"A".repeat(40)));
        assert!(valid_sha256(&"f".repeat(64)));
        assert!(!valid_sha256(&"g".repeat(64)));
        assert!(valid_timestamp("2026-08-20T12:00:00Z"));
        assert!(!valid_timestamp("2026-08-20 12:00:00Z"));
    }
}
