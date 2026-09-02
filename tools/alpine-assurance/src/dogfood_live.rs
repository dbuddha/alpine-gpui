//! Seals and validates omission-aware live Alpine Studio dogfood bundles.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write as _,
    fs,
    path::{Component, Path, PathBuf},
    process::Command,
};

use serde::{Deserialize, Serialize};

const MAX_DRAFT_BYTES: u64 = 65_536;
const MAX_MANIFEST_BYTES: u64 = 131_072;
const MAX_INTERNAL_BYTES: u64 = 262_144;
const MAX_FOOTPRINT_BYTES: u64 = 67_108_864;
const MAX_STREAM_BYTES: u64 = 1_048_576;
const MAX_SNAPSHOT_BYTES: u64 = 1_048_576;
const MAX_SAMPLES: usize = 4_096;
const MAX_ITEMS: usize = 32;
const MAX_TEXT_BYTES: usize = 4_096;
const REQUIRED_COVERAGE: &[&str] = &[
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
const REQUIRED_ARTIFACTS: &[(&str, &str, u64)] = &[
    (
        "internal-diagnostic",
        "internal-diagnostic.json",
        MAX_INTERNAL_BYTES,
    ),
    ("process-footprint", "footprint.json", MAX_FOOTPRINT_BYTES),
    ("studio-stdout", "studio.stdout", MAX_STREAM_BYTES),
    ("studio-stderr", "studio.stderr", MAX_STREAM_BYTES),
    ("snapshot", "snapshot.toml", MAX_SNAPSHOT_BYTES),
];

#[derive(Clone, Debug)]
pub(crate) struct SealRequest {
    pub(crate) draft: PathBuf,
    pub(crate) internal: PathBuf,
    pub(crate) footprint: PathBuf,
    pub(crate) stdout: PathBuf,
    pub(crate) stderr: PathBuf,
    pub(crate) binary: PathBuf,
    pub(crate) sampler: PathBuf,
    pub(crate) expected_pid: u32,
    pub(crate) requested_duration_ms: u64,
    pub(crate) interval_ms: u64,
    pub(crate) evidence_scope: String,
    pub(crate) process_start: String,
    pub(crate) expected_revision: String,
    pub(crate) expected_captured_at: String,
    pub(crate) destination: PathBuf,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Draft {
    schema: String,
    identity: Identity,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Identity {
    id: String,
    workload_id: String,
    workload_version: u32,
    workspace_fixture: String,
    workspace_fixture_sha256: String,
    settings_profile: String,
    settings_sha256: String,
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

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema: String,
    captured_at_utc: String,
    alpine_revision: String,
    duration_ms: u64,
    requested_duration_ms: u64,
    interval_ms: u64,
    process_pid: u32,
    process_start: String,
    evidence_scope: String,
    binary_sha256: String,
    binary_bytes: u64,
    sampler_sha256: String,
    sampler_bytes: u64,
    gpu_process_sampling: String,
    identity: Identity,
    artifacts: Vec<Artifact>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct Artifact {
    name: String,
    file: String,
    sha256: String,
    bytes: u64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InternalDiagnostic {
    schema: String,
    workload_id: String,
    alpine_revision: String,
    captured_at_utc: String,
    duration_ms: u64,
    outcome: String,
    status: String,
    frames: FrameSnapshot,
    text: TextSnapshot,
    language: LanguageSnapshot,
    accessibility: AccessibilitySnapshot,
    lifecycle: LifecycleSnapshot,
    resources: Vec<ResourceSnapshot>,
    runtime: serde_json::Value,
    surface: serde_json::Value,
    omissions: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    schema: String,
    workload_id: String,
    application_duration_ms: u64,
    duration_ms: u64,
    outcome: String,
    status: String,
    frames: FrameSnapshot,
    text: TextSnapshot,
    language: LanguageSnapshot,
    accessibility: AccessibilitySnapshot,
    lifecycle: LifecycleSnapshot,
    stages: Vec<StageSnapshot>,
    resources: Vec<ResourceSnapshot>,
    samples: Vec<ProcessSample>,
    omissions: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct TextSnapshot {
    shape_calls: u64,
    rasterize_calls: u64,
    syntax_cache_hits: u64,
    syntax_cache_misses: u64,
    syntax_omitted_lines: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct LanguageSnapshot {
    requests: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    responses: Option<u64>,
    stale_responses: u64,
    restarts: u64,
    current_retained_bytes: u64,
    peak_retained_bytes: u64,
    budget_bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct AccessibilitySnapshot {
    queries: u64,
    actions: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    stale_actions: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    retained_nodes: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    peak_retained_nodes: Option<u32>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct LifecycleSnapshot {
    close_requests: u64,
    close_completions: u64,
    clean_shutdown: bool,
    post_close_bytes: u64,
    post_close_limit_bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct StageSnapshot {
    name: String,
    omitted: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct ResourceSnapshot {
    name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    current_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    peak_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    budget_bytes: Option<u64>,
    omitted: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    omitted_axes: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct ProcessSample {
    sequence: u32,
    elapsed_ms: u64,
    physical_footprint_bytes: u64,
    physical_peak_bytes: u64,
    private_dirty_bytes: u64,
}

#[derive(Debug, Deserialize)]
struct Footprint {
    unit: String,
    #[serde(rename = "bytes per unit")]
    bytes_per_unit: u64,
    samples: Vec<FootprintSample>,
}

#[derive(Debug, Deserialize)]
struct FootprintSample {
    start_time: StartTime,
    processes: Vec<FootprintProcess>,
    summary: FootprintSummary,
}

#[derive(Debug, Deserialize)]
struct StartTime {
    wall_time_s: f64,
}

#[derive(Debug, Deserialize)]
struct FootprintProcess {
    pid: u32,
    auxiliary: FootprintAuxiliary,
}

#[derive(Debug, Deserialize)]
struct FootprintAuxiliary {
    phys_footprint: u64,
    phys_footprint_peak: u64,
}

#[derive(Debug, Deserialize)]
struct FootprintSummary {
    total: FootprintTotal,
}

#[derive(Debug, Deserialize)]
struct FootprintTotal {
    dirty: u64,
}

pub(crate) fn is_v2(manifest_path: &Path) -> bool {
    read_limited(manifest_path, MAX_MANIFEST_BYTES)
        .ok()
        .and_then(|source| String::from_utf8(source).ok())
        .and_then(|source| toml::from_str::<toml::Value>(&source).ok())
        .and_then(|document| {
            document
                .get("schema")
                .and_then(toml::Value::as_str)
                .map(str::to_owned)
        })
        .is_some_and(|schema| schema == "alpine-studio-dogfood/v2")
}

pub(crate) fn run(command: &str, manifest_path: &Path) -> Result<String, Vec<String>> {
    let (manifest, snapshot) = validate_bundle(manifest_path)?;
    match command {
        "validate-studio-dogfood" => Ok(format!(
            "validated live Studio dogfood capture {} with {} physical samples and {} explicit omissions",
            manifest.identity.id,
            snapshot.samples.len(),
            snapshot.omissions.len()
        )),
        "studio-dogfood-report" => Ok(render_report(&manifest, &snapshot)),
        _ => Err(vec![format!(
            "unsupported live Studio dogfood command {command}"
        )]),
    }
}

pub(crate) fn seal(request: &SealRequest) -> Result<String, Vec<String>> {
    let draft: Draft = load_toml(&request.draft, MAX_DRAFT_BYTES)?;
    let mut errors = validate_identity(&draft);
    validate_seal_request(request, &mut errors);
    if !errors.is_empty() {
        errors.sort();
        return Err(errors);
    }

    let internal_bytes = read_limited(&request.internal, MAX_INTERNAL_BYTES)?;
    let internal: InternalDiagnostic = serde_json::from_slice(&internal_bytes)
        .map_err(|error| vec![format!("cannot parse internal diagnostic: {error}")])?;
    let footprint_bytes = read_limited(&request.footprint, MAX_FOOTPRINT_BYTES)?;
    let footprint: Footprint = serde_json::from_slice(&footprint_bytes)
        .map_err(|error| vec![format!("cannot parse footprint JSON: {error}")])?;
    let snapshot = derive_snapshot(&internal, &footprint, request.expected_pid)?;
    if internal.workload_id != draft.identity.workload_id {
        return Err(vec![
            "internal and draft workload identities differ".to_owned(),
        ]);
    }
    if internal.alpine_revision != request.expected_revision {
        return Err(vec![
            "internal revision does not match the expected repository revision".to_owned(),
        ]);
    }
    if internal.captured_at_utc != request.expected_captured_at {
        return Err(vec![
            "internal timestamp does not match the launcher capture identity".to_owned(),
        ]);
    }
    if !sample_window_matches(
        snapshot.duration_ms,
        request.requested_duration_ms,
        request.interval_ms,
    ) {
        return Err(vec![
            "physical sample window does not match the requested duration".to_owned(),
        ]);
    }

    if request.destination.exists() {
        return Err(vec![format!(
            "live dogfood destination {} already exists",
            request.destination.display()
        )]);
    }
    let parent = request
        .destination
        .parent()
        .unwrap_or_else(|| Path::new("."));
    if !parent.is_dir() {
        return Err(vec![format!(
            "live dogfood destination parent {} is unavailable",
            parent.display()
        )]);
    }
    let staging = staging_path(&request.destination)?;
    if staging.exists() {
        return Err(vec![format!(
            "live dogfood staging path {} already exists",
            staging.display()
        )]);
    }
    fs::create_dir(&staging).map_err(|error| {
        vec![format!(
            "cannot create live dogfood staging directory {}: {error}",
            staging.display()
        )]
    })?;

    let result = write_bundle(request, draft.identity, &snapshot, &staging);
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

fn sample_window_matches(actual_ms: u64, requested_ms: u64, interval_ms: u64) -> bool {
    actual_ms > 0 && actual_ms.abs_diff(requested_ms) <= interval_ms
}

fn validate_seal_request(request: &SealRequest, errors: &mut Vec<String>) {
    require(
        request.expected_pid > 0,
        "expected PID must be positive",
        errors,
    );
    require(
        request.requested_duration_ms > 0 && request.requested_duration_ms <= 604_800_000,
        "requested duration must be within one millisecond and seven days",
        errors,
    );
    require(
        request.interval_ms > 0 && request.interval_ms < request.requested_duration_ms,
        "sample interval must be positive and shorter than duration",
        errors,
    );
    let sample_bound = request
        .requested_duration_ms
        .checked_div(request.interval_ms)
        .and_then(|samples| samples.checked_add(2));
    require(
        sample_bound.is_some_and(|sample_bound| {
            (4..=u64::try_from(MAX_SAMPLES).unwrap_or(u64::MAX)).contains(&sample_bound)
        }),
        "requested sample bound must remain within 4 and 4096",
        errors,
    );
    require(
        matches!(request.evidence_scope.as_str(), "physical" | "fixture"),
        "evidence scope must be physical or fixture",
        errors,
    );
    require(
        !request.process_start.is_empty() && request.process_start.len() <= 256,
        "process start identity must contain 1 to 256 bytes",
        errors,
    );
    require(
        valid_git_sha(&request.expected_revision),
        "expected repository revision must be a full lowercase Git SHA",
        errors,
    );
    require(
        valid_timestamp(&request.expected_captured_at),
        "expected capture timestamp must be UTC YYYY-MM-DDTHH:MM:SSZ",
        errors,
    );
    for (name, path) in [
        ("internal diagnostic", request.internal.as_path()),
        ("footprint", request.footprint.as_path()),
        ("Studio stdout", request.stdout.as_path()),
        ("Studio stderr", request.stderr.as_path()),
        ("binary", request.binary.as_path()),
        ("sampler", request.sampler.as_path()),
    ] {
        require(
            path.is_file(),
            format!("{name} must identify a file"),
            errors,
        );
    }
}

fn write_bundle(
    request: &SealRequest,
    identity: Identity,
    snapshot: &Snapshot,
    staging: &Path,
) -> Result<String, Vec<String>> {
    let sources = [
        (&request.internal, "internal-diagnostic.json"),
        (&request.footprint, "footprint.json"),
        (&request.stdout, "studio.stdout"),
        (&request.stderr, "studio.stderr"),
    ];
    for (source, name) in sources {
        fs::copy(source, staging.join(name)).map_err(|error| {
            vec![format!(
                "cannot retain live dogfood artifact {name}: {error}"
            )]
        })?;
    }
    let snapshot_source = toml::to_string_pretty(&snapshot)
        .map_err(|error| vec![format!("cannot encode live dogfood snapshot: {error}")])?;
    fs::write(staging.join("snapshot.toml"), snapshot_source)
        .map_err(|error| vec![format!("cannot write live dogfood snapshot: {error}")])?;

    let artifacts = REQUIRED_ARTIFACTS
        .iter()
        .map(|(name, file, _)| artifact(staging, name, file))
        .collect::<Result<Vec<_>, _>>()?;
    let binary_metadata = fs::metadata(&request.binary)
        .map_err(|error| vec![format!("cannot inspect Studio binary: {error}")])?;
    let sampler_metadata = fs::metadata(&request.sampler)
        .map_err(|error| vec![format!("cannot inspect footprint sampler: {error}")])?;
    let manifest = Manifest {
        schema: "alpine-studio-dogfood/v2".to_owned(),
        captured_at_utc: snapshot_source_identity(staging)?.captured_at_utc,
        alpine_revision: snapshot_source_identity(staging)?.alpine_revision,
        duration_ms: snapshot.duration_ms,
        requested_duration_ms: request.requested_duration_ms,
        interval_ms: request.interval_ms,
        process_pid: request.expected_pid,
        process_start: request.process_start.clone(),
        evidence_scope: request.evidence_scope.clone(),
        binary_sha256: calculate_sha256(&request.binary)?,
        binary_bytes: binary_metadata.len(),
        sampler_sha256: calculate_sha256(&request.sampler)?,
        sampler_bytes: sampler_metadata.len(),
        gpu_process_sampling: "omitted-unavailable".to_owned(),
        identity,
        artifacts,
    };
    let manifest_source = toml::to_string_pretty(&manifest)
        .map_err(|error| vec![format!("cannot encode live dogfood manifest: {error}")])?;
    let manifest_path = staging.join("session.toml");
    fs::write(&manifest_path, manifest_source)
        .map_err(|error| vec![format!("cannot write live dogfood manifest: {error}")])?;
    validate_bundle(&manifest_path)?;
    fs::rename(staging, &request.destination).map_err(|error| {
        vec![format!(
            "cannot publish live dogfood bundle {}: {error}",
            request.destination.display()
        )]
    })?;
    Ok(format!(
        "sealed live Studio dogfood capture {} at {}",
        manifest.identity.id,
        request.destination.display()
    ))
}

fn snapshot_source_identity(staging: &Path) -> Result<InternalSourceIdentity, Vec<String>> {
    let source = read_limited(
        &staging.join("internal-diagnostic.json"),
        MAX_INTERNAL_BYTES,
    )?;
    serde_json::from_slice(&source)
        .map_err(|error| vec![format!("cannot recover internal source identity: {error}")])
}

#[derive(Deserialize)]
struct InternalSourceIdentity {
    captured_at_utc: String,
    alpine_revision: String,
}

fn artifact(bundle: &Path, name: &str, file: &str) -> Result<Artifact, Vec<String>> {
    let path = bundle.join(file);
    let metadata = fs::metadata(&path)
        .map_err(|error| vec![format!("cannot inspect retained artifact {file}: {error}")])?;
    Ok(Artifact {
        name: name.to_owned(),
        file: file.to_owned(),
        sha256: calculate_sha256(&path)?,
        bytes: metadata.len(),
    })
}

#[allow(
    clippy::too_many_lines,
    reason = "one validator keeps raw artifacts, normalized evidence, and manifest identity auditable together"
)]
fn validate_bundle(manifest_path: &Path) -> Result<(Manifest, Snapshot), Vec<String>> {
    let manifest: Manifest = load_toml(manifest_path, MAX_MANIFEST_BYTES)?;
    let mut errors = Vec::new();
    require(
        manifest.schema == "alpine-studio-dogfood/v2",
        "live dogfood schema must be alpine-studio-dogfood/v2",
        &mut errors,
    );
    validate_identity_fields(&manifest.identity, &mut errors);
    require(
        valid_timestamp(&manifest.captured_at_utc),
        "capture timestamp is invalid",
        &mut errors,
    );
    require(
        valid_git_sha(&manifest.alpine_revision),
        "Alpine revision is invalid",
        &mut errors,
    );
    require(
        sample_window_matches(
            manifest.duration_ms,
            manifest.requested_duration_ms,
            manifest.interval_ms,
        ),
        "captured duration is outside the requested sample window",
        &mut errors,
    );
    require(
        manifest.interval_ms > 0 && manifest.interval_ms < manifest.requested_duration_ms,
        "manifest interval is invalid",
        &mut errors,
    );
    require(
        manifest.process_pid > 0,
        "manifest PID must be positive",
        &mut errors,
    );
    require(
        !manifest.process_start.is_empty() && manifest.process_start.len() <= 256,
        "manifest process start identity is invalid",
        &mut errors,
    );
    require(
        matches!(manifest.evidence_scope.as_str(), "physical" | "fixture"),
        "manifest evidence scope is invalid",
        &mut errors,
    );
    require(
        valid_sha256(&manifest.binary_sha256),
        "binary SHA-256 is invalid",
        &mut errors,
    );
    require(
        manifest.binary_bytes > 0,
        "binary byte count must be positive",
        &mut errors,
    );
    require(
        valid_sha256(&manifest.sampler_sha256),
        "sampler SHA-256 is invalid",
        &mut errors,
    );
    require(
        manifest.sampler_bytes > 0,
        "sampler byte count must be positive",
        &mut errors,
    );
    require(
        manifest.gpu_process_sampling == "omitted-unavailable",
        "GPU process sampling must remain an explicit omission",
        &mut errors,
    );

    let bundle = manifest_path.parent().unwrap_or_else(|| Path::new("."));
    let artifacts = validate_artifacts(bundle, &manifest.artifacts, &mut errors);
    if !errors.is_empty() {
        errors.sort();
        return Err(errors);
    }
    let snapshot_path = artifacts
        .get("snapshot")
        .ok_or_else(|| vec!["live dogfood snapshot artifact is missing".to_owned()])?;
    let snapshot: Snapshot = load_toml(snapshot_path, MAX_SNAPSHOT_BYTES)?;
    let internal_path = artifacts
        .get("internal-diagnostic")
        .ok_or_else(|| vec!["internal diagnostic artifact is missing".to_owned()])?;
    let footprint_path = artifacts
        .get("process-footprint")
        .ok_or_else(|| vec!["process footprint artifact is missing".to_owned()])?;
    let internal: InternalDiagnostic =
        serde_json::from_slice(&read_limited(internal_path, MAX_INTERNAL_BYTES)?).map_err(
            |error| {
                vec![format!(
                    "cannot parse retained internal diagnostic: {error}"
                )]
            },
        )?;
    let footprint: Footprint =
        serde_json::from_slice(&read_limited(footprint_path, MAX_FOOTPRINT_BYTES)?)
            .map_err(|error| vec![format!("cannot parse retained footprint JSON: {error}")])?;
    let expected = derive_snapshot(&internal, &footprint, manifest.process_pid)?;
    require(
        snapshot == expected,
        "snapshot does not reproduce retained raw evidence",
        &mut errors,
    );
    require(
        snapshot.workload_id == manifest.identity.workload_id,
        "manifest workload differs from snapshot",
        &mut errors,
    );
    require(
        snapshot.duration_ms == manifest.duration_ms,
        "manifest duration differs from snapshot",
        &mut errors,
    );
    require(
        internal.alpine_revision == manifest.alpine_revision,
        "manifest revision differs from internal output",
        &mut errors,
    );
    require(
        internal.captured_at_utc == manifest.captured_at_utc,
        "manifest timestamp differs from internal output",
        &mut errors,
    );
    errors.sort();
    if errors.is_empty() {
        Ok((manifest, snapshot))
    } else {
        Err(errors)
    }
}

fn validate_artifacts<'a>(
    bundle: &'a Path,
    artifacts: &'a [Artifact],
    errors: &mut Vec<String>,
) -> BTreeMap<&'a str, PathBuf> {
    require(
        artifacts.len() == REQUIRED_ARTIFACTS.len(),
        "live dogfood artifact inventory has the wrong size",
        errors,
    );
    let mut resolved = BTreeMap::new();
    for artifact in artifacts {
        let Some((_, expected_file, limit)) = REQUIRED_ARTIFACTS
            .iter()
            .find(|(name, _, _)| *name == artifact.name)
        else {
            errors.push(format!(
                "unsupported live dogfood artifact {}",
                artifact.name
            ));
            continue;
        };
        require(
            artifact.file == *expected_file,
            format!("artifact {} must use file {expected_file}", artifact.name),
            errors,
        );
        require(
            resolved
                .insert(artifact.name.as_str(), bundle.join(&artifact.file))
                .is_none(),
            format!("duplicate live dogfood artifact {}", artifact.name),
            errors,
        );
        let path = bundle.join(&artifact.file);
        let lexical = Path::new(&artifact.file);
        require(
            !artifact.file.is_empty()
                && !lexical.is_absolute()
                && lexical
                    .components()
                    .all(|component| matches!(component, Component::Normal(_))),
            format!("artifact {} escapes its bundle", artifact.name),
            errors,
        );
        let metadata = fs::symlink_metadata(&path);
        match metadata {
            Ok(metadata) => {
                require(
                    metadata.file_type().is_file(),
                    format!("artifact {} is not a regular file", artifact.name),
                    errors,
                );
                require(
                    metadata.len() == artifact.bytes,
                    format!("artifact {} byte count differs", artifact.name),
                    errors,
                );
                require(
                    metadata.len() <= *limit,
                    format!("artifact {} exceeds its byte limit", artifact.name),
                    errors,
                );
            }
            Err(error) => errors.push(format!(
                "cannot inspect artifact {}: {error}",
                artifact.name
            )),
        }
        require(
            valid_sha256(&artifact.sha256),
            format!("artifact {} SHA-256 is invalid", artifact.name),
            errors,
        );
        match calculate_sha256(&path) {
            Ok(actual) => require(
                actual == artifact.sha256,
                format!("artifact {} SHA-256 differs", artifact.name),
                errors,
            ),
            Err(mut hash_errors) => errors.append(&mut hash_errors),
        }
    }
    for (required, _, _) in REQUIRED_ARTIFACTS {
        require(
            resolved.contains_key(required),
            format!("artifact inventory lacks {required}"),
            errors,
        );
    }
    resolved
}

fn derive_snapshot(
    internal: &InternalDiagnostic,
    footprint: &Footprint,
    expected_pid: u32,
) -> Result<Snapshot, Vec<String>> {
    let mut errors = Vec::new();
    require(
        internal.schema == "alpine-studio-internal-diagnostic/v1",
        "internal diagnostic schema is unsupported",
        &mut errors,
    );
    require(
        valid_slug(&internal.workload_id),
        "internal workload id is invalid",
        &mut errors,
    );
    require(
        valid_git_sha(&internal.alpine_revision),
        "internal revision is invalid",
        &mut errors,
    );
    require(
        valid_timestamp(&internal.captured_at_utc),
        "internal timestamp is invalid",
        &mut errors,
    );
    require(
        internal.duration_ms > 0,
        "internal duration must be positive",
        &mut errors,
    );
    require(
        matches!(internal.outcome.as_str(), "passed" | "failed"),
        "internal outcome is invalid",
        &mut errors,
    );
    require(
        !internal.status.is_empty() && internal.status.len() <= MAX_TEXT_BYTES,
        "internal status is invalid",
        &mut errors,
    );
    validate_frames(&internal.frames, &mut errors);
    validate_language(&internal.language, &internal.omissions, &mut errors);
    validate_accessibility(&internal.accessibility, &internal.omissions, &mut errors);
    validate_lifecycle(&internal.lifecycle, &internal.outcome, &mut errors);
    let _ = (&internal.runtime, &internal.surface);
    let resources = validate_resources(&internal.resources, &internal.omissions, &mut errors);
    let samples = derive_process_samples(footprint, expected_pid, &mut errors);
    let duration_ms = samples.last().map_or(0, |sample| sample.elapsed_ms);
    require(
        duration_ms > 0,
        "physical sample duration must be positive",
        &mut errors,
    );
    require(
        internal.duration_ms >= duration_ms,
        "application ended before the physical sample window",
        &mut errors,
    );
    let mut omissions = internal.omissions.clone();
    omissions.retain(|item| item != "process-samples");
    for required in [
        "process-gpu-bytes",
        "process-alpine-retained-bytes",
        "stage-timings",
    ] {
        if !omissions.iter().any(|item| item == required) {
            omissions.push(required.to_owned());
        }
    }
    omissions.sort();
    omissions.dedup();
    require(
        !omissions.is_empty() && omissions.len() <= MAX_ITEMS,
        "omission inventory is invalid",
        &mut errors,
    );
    errors.sort();
    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(Snapshot {
        schema: "alpine-studio-diagnostic/v2".to_owned(),
        workload_id: internal.workload_id.clone(),
        application_duration_ms: internal.duration_ms,
        duration_ms,
        outcome: internal.outcome.clone(),
        status: internal.status.clone(),
        frames: internal.frames.clone(),
        text: internal.text.clone(),
        language: internal.language.clone(),
        accessibility: internal.accessibility.clone(),
        lifecycle: internal.lifecycle.clone(),
        stages: REQUIRED_STAGES
            .iter()
            .map(|name| StageSnapshot {
                name: (*name).to_owned(),
                omitted: true,
            })
            .collect(),
        resources,
        samples,
        omissions,
    })
}

fn validate_frames(frames: &FrameSnapshot, errors: &mut Vec<String>) {
    require(
        frames.presented <= frames.completed
            && frames.completed <= frames.submitted
            && frames.submitted <= frames.requested,
        "internal frame counters are out of order",
        errors,
    );
    require(
        frames.peak_in_flight <= 3,
        "internal peak in-flight exceeds three",
        errors,
    );
    let _ = frames.omitted;
}

fn validate_language(language: &LanguageSnapshot, omissions: &[String], errors: &mut Vec<String>) {
    require(
        language.current_retained_bytes <= language.peak_retained_bytes
            && language.peak_retained_bytes <= language.budget_bytes,
        "language bytes exceed peak or budget",
        errors,
    );
    match language.responses {
        Some(responses) => require(
            responses <= language.requests,
            "language responses exceed requests",
            errors,
        ),
        None => require(
            omissions.iter().any(|item| item == "language-responses"),
            "missing language responses lack an omission",
            errors,
        ),
    }
}

fn validate_accessibility(
    accessibility: &AccessibilitySnapshot,
    omissions: &[String],
    errors: &mut Vec<String>,
) {
    match (
        accessibility.retained_nodes,
        accessibility.peak_retained_nodes,
    ) {
        (Some(current), Some(peak)) => require(
            current <= peak && peak <= 271,
            "accessibility nodes exceed their bound",
            errors,
        ),
        (None, None) => require(
            omissions.iter().any(|item| item == "accessibility-tree"),
            "missing accessibility tree lacks an omission",
            errors,
        ),
        _ => errors.push("accessibility node axes must be present or omitted together".to_owned()),
    }
    if accessibility.stale_actions.is_none() {
        require(
            omissions
                .iter()
                .any(|item| item == "accessibility-stale-actions"),
            "missing stale accessibility actions lack an omission",
            errors,
        );
    }
}

fn validate_lifecycle(lifecycle: &LifecycleSnapshot, outcome: &str, errors: &mut Vec<String>) {
    require(
        lifecycle.close_completions <= lifecycle.close_requests,
        "close completions exceed close requests",
        errors,
    );
    if outcome == "passed" {
        require(
            lifecycle.close_completions == 1,
            "passed capture needs one completed close",
            errors,
        );
        require(
            lifecycle.clean_shutdown,
            "passed capture needs clean shutdown",
            errors,
        );
        require(
            lifecycle.post_close_bytes <= lifecycle.post_close_limit_bytes,
            "passed capture exceeds its post-close byte limit",
            errors,
        );
    }
}

fn validate_resources(
    resources: &[ResourceSnapshot],
    omissions: &[String],
    errors: &mut Vec<String>,
) -> Vec<ResourceSnapshot> {
    let mut names = BTreeSet::new();
    for resource in resources {
        require(
            REQUIRED_RESOURCES.contains(&resource.name.as_str()),
            format!("unsupported resource {}", resource.name),
            errors,
        );
        require(
            names.insert(resource.name.as_str()),
            format!("duplicate resource {}", resource.name),
            errors,
        );
        if resource.omitted {
            require(
                resource.current_bytes.is_none()
                    && resource.peak_bytes.is_none()
                    && resource.budget_bytes.is_none(),
                format!("omitted resource {} contains bytes", resource.name),
                errors,
            );
            require(
                omissions.iter().any(|item| item == &resource.name),
                format!("resource {} lacks a matching omission", resource.name),
                errors,
            );
        } else {
            match (
                resource.current_bytes,
                resource.peak_bytes,
                resource.budget_bytes,
            ) {
                (Some(current), Some(peak), Some(budget)) => require(
                    current <= peak && peak <= budget,
                    format!("resource {} exceeds peak or budget", resource.name),
                    errors,
                ),
                (Some(current), Some(peak), None) => {
                    require(
                        current <= peak,
                        format!("resource {} current bytes exceed peak", resource.name),
                        errors,
                    );
                    require(
                        resource.omitted_axes == ["budget_bytes"],
                        format!(
                            "resource {} missing budget lacks an exact omission",
                            resource.name
                        ),
                        errors,
                    );
                }
                _ => errors.push(format!(
                    "resource {} has an incomplete byte tuple",
                    resource.name
                )),
            }
        }
    }
    for required in REQUIRED_RESOURCES {
        require(
            names.contains(required),
            format!("resource inventory lacks {required}"),
            errors,
        );
    }
    resources.to_vec()
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the rounded value is checked finite, nonnegative, and bounded to seven days before conversion"
)]
fn derive_process_samples(
    footprint: &Footprint,
    expected_pid: u32,
    errors: &mut Vec<String>,
) -> Vec<ProcessSample> {
    require(
        footprint.unit == "byte",
        "footprint unit must be byte",
        errors,
    );
    require(
        footprint.bytes_per_unit == 1,
        "footprint byte scale must be one",
        errors,
    );
    require(
        (4..=MAX_SAMPLES).contains(&footprint.samples.len()),
        "footprint sample count must remain within 4 and 4096",
        errors,
    );
    let first = footprint
        .samples
        .first()
        .map_or(0.0, |sample| sample.start_time.wall_time_s);
    let mut output = Vec::with_capacity(footprint.samples.len());
    let mut previous_wall = None;
    let mut previous_peak = 0;
    for (index, sample) in footprint.samples.iter().enumerate() {
        require(
            sample.processes.len() == 1,
            format!("footprint sample {index} must contain one process"),
            errors,
        );
        let Some(process) = sample.processes.first() else {
            continue;
        };
        require(
            process.pid == expected_pid,
            format!("footprint sample {index} PID differs"),
            errors,
        );
        let wall = sample.start_time.wall_time_s;
        require(
            wall.is_finite(),
            format!("footprint sample {index} wall time is not finite"),
            errors,
        );
        if let Some(previous) = previous_wall {
            require(
                wall > previous,
                format!("footprint sample {index} wall time is not increasing"),
                errors,
            );
        }
        require(
            process.auxiliary.phys_footprint_peak >= process.auxiliary.phys_footprint,
            format!("footprint sample {index} peak is below current bytes"),
            errors,
        );
        require(
            process.auxiliary.phys_footprint_peak >= previous_peak,
            format!("footprint sample {index} peak moved backward"),
            errors,
        );
        require(
            sample.summary.total.dirty <= process.auxiliary.phys_footprint,
            format!("footprint sample {index} dirty bytes exceed physical footprint"),
            errors,
        );
        let elapsed = ((wall - first) * 1_000.0).round();
        require(
            elapsed.is_finite() && (0.0..=604_800_000.0).contains(&elapsed),
            format!("footprint sample {index} elapsed time is invalid"),
            errors,
        );
        output.push(ProcessSample {
            sequence: u32::try_from(index).unwrap_or(u32::MAX),
            elapsed_ms: if elapsed.is_finite() && (0.0..=604_800_000.0).contains(&elapsed) {
                elapsed as u64
            } else {
                0
            },
            physical_footprint_bytes: process.auxiliary.phys_footprint,
            physical_peak_bytes: process.auxiliary.phys_footprint_peak,
            private_dirty_bytes: sample.summary.total.dirty,
        });
        previous_wall = Some(wall);
        previous_peak = process.auxiliary.phys_footprint_peak;
    }
    output
}

fn validate_identity(draft: &Draft) -> Vec<String> {
    let mut errors = Vec::new();
    require(
        draft.schema == "alpine-studio-dogfood-draft/v2",
        "draft schema must be alpine-studio-dogfood-draft/v2",
        &mut errors,
    );
    validate_identity_fields(&draft.identity, &mut errors);
    errors
}

#[allow(
    clippy::too_many_lines,
    reason = "one validator keeps the complete no-telemetry dogfood identity contract auditable"
)]
fn validate_identity_fields(identity: &Identity, errors: &mut Vec<String>) {
    require(
        valid_slug(&identity.id),
        "capture id must be a bounded slug",
        errors,
    );
    require(
        valid_slug(&identity.workload_id),
        "workload id must be a bounded slug",
        errors,
    );
    require(
        identity.workload_version > 0,
        "workload version must be positive",
        errors,
    );
    for (name, value) in [
        ("workspace fixture", identity.workspace_fixture.as_str()),
        ("settings profile", identity.settings_profile.as_str()),
        ("hardware id", identity.environment.hardware_id.as_str()),
        ("OS build", identity.environment.os_build.as_str()),
        ("toolchain", identity.environment.toolchain.as_str()),
        ("locale", identity.environment.locale.as_str()),
        ("font family", identity.font.family.as_str()),
        (
            "font PostScript name",
            identity.font.postscript_name.as_str(),
        ),
        (
            "language-server name",
            identity.language_server.name.as_str(),
        ),
        (
            "language-server version",
            identity.language_server.version.as_str(),
        ),
    ] {
        require(
            !value.is_empty() && value.len() <= MAX_TEXT_BYTES,
            format!("{name} is empty or too long"),
            errors,
        );
    }
    for (name, hash) in [
        (
            "workspace fixture",
            identity.workspace_fixture_sha256.as_str(),
        ),
        ("settings", identity.settings_sha256.as_str()),
    ] {
        require(
            valid_sha256(hash),
            format!("{name} SHA-256 is invalid"),
            errors,
        );
    }
    require(identity.opt_in, "capture must be explicitly opt-in", errors);
    require(
        !identity.telemetry,
        "capture must disable telemetry",
        errors,
    );
    require(
        !identity.network_io,
        "capture must perform no network I/O",
        errors,
    );
    require(
        identity.performance_claim == "none",
        "capture cannot contain a performance claim",
        errors,
    );
    validate_fixed_list(
        "coverage",
        &identity.coverage,
        REQUIRED_COVERAGE,
        false,
        errors,
    );
    validate_fixed_list(
        "exclusions",
        &identity.exclusions,
        REQUIRED_EXCLUSIONS,
        true,
        errors,
    );
    require(
        !identity.assumptions.is_empty() && identity.assumptions.len() <= MAX_ITEMS,
        "assumptions must contain 1 to 32 items",
        errors,
    );
    for assumption in &identity.assumptions {
        require(
            !assumption.is_empty() && assumption.len() <= MAX_TEXT_BYTES,
            "assumption is empty or too long",
            errors,
        );
    }
    require(
        identity.environment.architecture == "arm64",
        "capture architecture must be arm64",
        errors,
    );
    require(
        matches!(identity.environment.display_refresh_hz, 60 | 120),
        "display refresh must be 60 or 120 Hz",
        errors,
    );
    require(
        matches!(identity.environment.power_source.as_str(), "ac" | "battery"),
        "power source is invalid",
        errors,
    );
    require(
        matches!(
            identity.environment.thermal_state.as_str(),
            "nominal" | "fair" | "serious" | "critical"
        ),
        "thermal state is invalid",
        errors,
    );
    require(
        (1_000..=512_000).contains(&identity.font.size_milli_points),
        "font size is invalid",
        errors,
    );
    require(
        identity.language_server.executable_sha256 == "none"
            || valid_sha256(&identity.language_server.executable_sha256),
        "language-server executable SHA-256 is invalid",
        errors,
    );
}

fn validate_fixed_list(
    name: &str,
    values: &[String],
    expected: &[&str],
    require_all: bool,
    errors: &mut Vec<String>,
) {
    require(
        !values.is_empty() && values.len() <= MAX_ITEMS,
        format!("{name} must contain 1 to 32 items"),
        errors,
    );
    let mut seen = BTreeSet::new();
    for value in values {
        require(
            expected.contains(&value.as_str()),
            format!("{name} contains unsupported value {value}"),
            errors,
        );
        require(
            seen.insert(value.as_str()),
            format!("{name} contains duplicate value {value}"),
            errors,
        );
    }
    if require_all {
        for value in expected {
            require(
                seen.contains(value),
                format!("{name} lacks required value {value}"),
                errors,
            );
        }
    }
}

fn render_report(manifest: &Manifest, snapshot: &Snapshot) -> String {
    let mut output = String::from("# Live Alpine Studio dogfood capture\n\n");
    let _ = write!(
        output,
        "- Capture: `{}`\n- Revision: `{}`\n- Evidence scope: `{}`\n- Workload: `{}`\n- Physical samples: {}\n- Physical window: {} ms\n- Application duration: {} ms\n- Peak in-flight frames: {}\n- Idle submissions: {}\n- GPU process sampling: omitted, unavailable\n- Performance claim: none\n\n",
        manifest.identity.id,
        manifest.alpine_revision,
        manifest.evidence_scope,
        manifest.identity.workload_id,
        snapshot.samples.len(),
        snapshot.duration_ms,
        snapshot.application_duration_ms,
        snapshot.frames.peak_in_flight,
        snapshot.frames.idle_submissions,
    );
    output.push_str("## Explicit omissions\n\n");
    for omission in &snapshot.omissions {
        let _ = writeln!(output, "- `{omission}`");
    }
    output
}

fn staging_path(destination: &Path) -> Result<PathBuf, Vec<String>> {
    let name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| vec!["live dogfood destination needs a UTF-8 file name".to_owned()])?;
    Ok(destination
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!(".{name}.staging")))
}

fn load_toml<T>(path: &Path, limit: u64) -> Result<T, Vec<String>>
where
    T: for<'de> Deserialize<'de>,
{
    let source = read_limited(path, limit)?;
    let source = String::from_utf8(source)
        .map_err(|error| vec![format!("{} is not UTF-8: {error}", path.display())])?;
    toml::from_str(&source)
        .map_err(|error| vec![format!("cannot parse {}: {error}", path.display())])
}

fn read_limited(path: &Path, limit: u64) -> Result<Vec<u8>, Vec<String>> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| vec![format!("cannot inspect {}: {error}", path.display())])?;
    if !metadata.file_type().is_file() {
        return Err(vec![format!("{} must be a regular file", path.display())]);
    }
    if metadata.len() > limit {
        return Err(vec![format!(
            "{} is {} bytes; limit is {limit}",
            path.display(),
            metadata.len()
        )]);
    }
    fs::read(path).map_err(|error| vec![format!("cannot read {}: {error}", path.display())])
}

fn calculate_sha256(path: &Path) -> Result<String, Vec<String>> {
    if let Ok(metadata) = fs::metadata(path)
        && metadata.is_file()
        && metadata.len() == 0
    {
        return Ok("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".to_owned());
    }

    for (program, arguments) in [("sha256sum", &[][..]), ("shasum", &["-a", "256"][..])] {
        let output = Command::new(program).args(arguments).arg(path).output();
        let Ok(output) = output else {
            continue;
        };
        if output.status.success()
            && let Some(digest) = sha256_from_output(&output.stdout)
        {
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

    Err(vec![format!(
        "cannot calculate SHA-256 for {}; sha256sum, shasum, or certutil is required",
        path.display()
    )])
}

fn sha256_from_output(output: &[u8]) -> Option<String> {
    String::from_utf8_lossy(output)
        .split_whitespace()
        .find(|token| valid_sha256(token))
        .map(str::to_owned)
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
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn valid_timestamp(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 20
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[10] == b'T'
        && bytes[13] == b':'
        && bytes[16] == b':'
        && bytes[19] == b'Z'
        && bytes.iter().enumerate().all(|(index, byte)| {
            matches!(index, 4 | 7 | 10 | 13 | 16 | 19) || byte.is_ascii_digit()
        })
}

fn require(condition: bool, message: impl Into<String>, errors: &mut Vec<String>) {
    if !condition {
        errors.push(message.into());
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "fixture construction and cleanup failures are unrecoverable test harness defects"
)]
mod tests {
    use super::{
        AccessibilitySnapshot, Artifact, Draft, Footprint, FrameSnapshot, InternalDiagnostic,
        LanguageSnapshot, LifecycleSnapshot, Manifest, ResourceSnapshot, SealRequest, Snapshot,
        artifact, calculate_sha256, derive_process_samples, derive_snapshot, is_v2, load_toml,
        read_limited, run, seal, sha256_from_output, snapshot_source_identity, staging_path,
        valid_git_sha, valid_sha256, valid_slug, valid_timestamp, validate_accessibility,
        validate_artifacts, validate_bundle, validate_fixed_list, validate_frames,
        validate_identity, validate_identity_fields, validate_language, validate_lifecycle,
        validate_resources, validate_seal_request,
    };
    use std::{
        fs,
        path::{Path, PathBuf},
    };

    #[test]
    fn malformed_capture_inputs_fail_closed() {
        let unix_digest = format!("{SHA}  fixture\n");
        assert_eq!(
            sha256_from_output(unix_digest.as_bytes()).as_deref(),
            Some(SHA)
        );
        let certutil_digest =
            format!("SHA256 hash of fixture:\r\n{SHA}\r\nCertUtil: completed\r\n");
        assert_eq!(
            sha256_from_output(certutil_digest.as_bytes()).as_deref(),
            Some(SHA)
        );
        assert!(sha256_from_output(b"not-a-digest").is_none());

        let root = root("rejections");
        if root.exists() {
            fs::remove_dir_all(&root).expect("remove stale rejection fixture");
        }
        fs::create_dir_all(&root).expect("create rejection fixture");

        let empty = root.join("empty");
        fs::write(&empty, b"").expect("write empty hash fixture");
        assert_eq!(
            calculate_sha256(&empty).expect("hash empty regular file"),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );

        let missing = root.join("missing");
        assert!(calculate_sha256(&missing).is_err());
        assert!(artifact(&root, "missing", "missing.json").is_err());
        assert!(staging_path(std::path::Path::new("")).is_err());

        let malformed_identity = root.join("malformed-identity.json");
        fs::write(&malformed_identity, b"{not-json").expect("write malformed identity");
        assert!(snapshot_source_identity(&malformed_identity).is_err());

        let invalid_utf8_toml = root.join("invalid-utf8.toml");
        fs::write(&invalid_utf8_toml, [0xff]).expect("write invalid UTF-8 TOML");
        assert!(load_toml::<Draft>(&invalid_utf8_toml, 16).is_err());
        assert!(load_toml::<Manifest>(&invalid_utf8_toml, 16).is_err());
        assert!(load_toml::<Snapshot>(&invalid_utf8_toml, 16).is_err());

        let malformed_toml = root.join("malformed.toml");
        fs::write(&malformed_toml, b"schema = [").expect("write malformed TOML");
        assert!(load_toml::<Draft>(&malformed_toml, 16).is_err());
        assert!(load_toml::<Manifest>(&malformed_toml, 16).is_err());
        assert!(load_toml::<Snapshot>(&malformed_toml, 16).is_err());

        let missing_bundle = root.join("missing-bundle");
        assert!(!is_v2(&missing_bundle));

        let invalid_utf8_bundle = root.join("invalid-utf8-bundle");
        fs::create_dir_all(&invalid_utf8_bundle).expect("create invalid UTF-8 bundle");
        fs::write(invalid_utf8_bundle.join("manifest.toml"), [0xff])
            .expect("write invalid UTF-8 manifest");
        assert!(!is_v2(&invalid_utf8_bundle));

        let malformed_bundle = root.join("malformed-bundle");
        fs::create_dir_all(&malformed_bundle).expect("create malformed bundle");
        fs::write(malformed_bundle.join("manifest.toml"), b"schema = [")
            .expect("write malformed manifest");
        assert!(!is_v2(&malformed_bundle));

        fs::remove_dir_all(root).expect("remove rejection fixture");
    }

    #[test]
    fn seal_rejects_malformed_sources_and_unpublishable_staging() {
        let (internal_root, internal_request) = write_inputs("malformed-internal", REVISION);
        fs::write(&internal_request.internal, b"{malformed")
            .expect("write malformed internal diagnostic");
        let internal_errors = seal(&internal_request).expect_err("reject malformed internal JSON");
        assert!(
            internal_errors
                .iter()
                .any(|error| error.contains("cannot parse internal diagnostic"))
        );
        fs::remove_dir_all(internal_root).expect("remove malformed internal fixture");

        let (footprint_root, footprint_request) = write_inputs("malformed-footprint", REVISION);
        fs::write(&footprint_request.footprint, b"{malformed").expect("write malformed footprint");
        let footprint_errors =
            seal(&footprint_request).expect_err("reject malformed footprint JSON");
        assert!(
            footprint_errors
                .iter()
                .any(|error| error.contains("cannot parse footprint JSON"))
        );
        fs::remove_dir_all(footprint_root).expect("remove malformed footprint fixture");

        let (staging_root, mut staging_request) = write_inputs("unpublishable-staging", REVISION);
        staging_request.destination = staging_root.join("a".repeat(512));
        let staging_errors = seal(&staging_request).expect_err("reject unpublishable staging path");
        assert!(
            staging_errors
                .iter()
                .any(|error| error.contains("cannot create live dogfood staging directory"))
        );
        fs::remove_dir_all(staging_root).expect("remove staging fixture");
    }

    const SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const REVISION: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn root(label: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target")
            .join(format!("live-dogfood-{label}-{}", std::process::id()))
    }

    fn assert_error(errors: &[String], expected: &str) {
        assert!(
            errors.iter().any(|error| error == expected),
            "missing error {expected:?} in {errors:?}"
        );
    }

    fn manifest_errors(manifest_path: &Path, mutate: impl FnOnce(&mut Manifest)) -> Vec<String> {
        let source = fs::read_to_string(manifest_path).expect("read valid manifest");
        let mut manifest: Manifest = toml::from_str(&source).expect("parse valid manifest");
        mutate(&mut manifest);
        fs::write(
            manifest_path,
            toml::to_string_pretty(&manifest).expect("encode mutated manifest"),
        )
        .expect("write mutated manifest");
        let errors = validate_bundle(manifest_path).expect_err("reject mutated manifest");
        fs::write(manifest_path, source).expect("restore valid manifest");
        errors
    }

    fn internal(revision: &str) -> String {
        format!(
            r#"{{
  "schema":"alpine-studio-internal-diagnostic/v1",
  "workload_id":"fixture-live",
  "alpine_revision":"{revision}",
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
        )
    }

    fn footprint(pid: u32) -> String {
        format!(
            r#"{{"unit":"byte","bytes per unit":1,"samples":[
{{"start_time":{{"wall_time_s":1000.0}},"processes":[{{"pid":{pid},"auxiliary":{{"phys_footprint":100,"phys_footprint_peak":100}}}}],"summary":{{"total":{{"dirty":50}}}}}},
{{"start_time":{{"wall_time_s":1001.0}},"processes":[{{"pid":{pid},"auxiliary":{{"phys_footprint":110,"phys_footprint_peak":110}}}}],"summary":{{"total":{{"dirty":55}}}}}},
{{"start_time":{{"wall_time_s":1002.0}},"processes":[{{"pid":{pid},"auxiliary":{{"phys_footprint":105,"phys_footprint_peak":110}}}}],"summary":{{"total":{{"dirty":52}}}}}},
{{"start_time":{{"wall_time_s":1003.0}},"processes":[{{"pid":{pid},"auxiliary":{{"phys_footprint":108,"phys_footprint_peak":110}}}}],"summary":{{"total":{{"dirty":54}}}}}}
]}}"#
        )
    }

    fn draft() -> String {
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
assumptions = ["fixture-only headless process and sampler"]
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
        )
    }

    fn write_inputs(label: &str, revision: &str) -> (PathBuf, SealRequest) {
        let root = root(label);
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("create fixture root");
        let draft_path = root.join("draft.toml");
        let internal_path = root.join("internal.json");
        let footprint_path = root.join("footprint.json");
        let stdout = root.join("stdout");
        let stderr = root.join("stderr");
        let binary = root.join("binary");
        let sampler = root.join("sampler");
        fs::write(&draft_path, draft()).expect("write draft");
        fs::write(&internal_path, internal(revision)).expect("write internal");
        fs::write(&footprint_path, footprint(42)).expect("write footprint");
        fs::write(&stdout, "fixture stdout\n").expect("write stdout");
        fs::write(&stderr, "").expect("write stderr");
        fs::write(&binary, "fixture binary").expect("write binary");
        fs::write(&sampler, "fixture sampler").expect("write sampler");
        let request = SealRequest {
            draft: draft_path,
            internal: internal_path,
            footprint: footprint_path,
            stdout,
            stderr,
            binary,
            sampler,
            expected_pid: 42,
            requested_duration_ms: 3_000,
            interval_ms: 1_000,
            evidence_scope: "fixture".to_owned(),
            process_start: "fixture-start".to_owned(),
            expected_revision: REVISION.to_owned(),
            expected_captured_at: "2026-08-30T18:00:00Z".to_owned(),
            destination: root.join("bundle"),
        };
        (root, request)
    }

    #[test]
    fn derives_omission_aware_physical_snapshot() {
        let mut internal: InternalDiagnostic =
            serde_json::from_str(&internal(REVISION)).expect("parse internal");
        internal.omissions.push("process-samples".to_owned());
        let valid_footprint: Footprint =
            serde_json::from_str(&footprint(42)).expect("parse footprint");
        let snapshot = derive_snapshot(&internal, &valid_footprint, 42).expect("derive snapshot");
        assert_eq!(snapshot.duration_ms, 3_000);
        assert_eq!(snapshot.samples.len(), 4);
        assert!(
            !snapshot
                .omissions
                .iter()
                .any(|item| item == "process-samples")
        );
        assert!(
            snapshot
                .omissions
                .iter()
                .any(|item| item == "process-gpu-bytes")
        );
        assert!(
            snapshot
                .omissions
                .iter()
                .any(|item| item == "process-alpine-retained-bytes")
        );
        assert!(snapshot.stages.iter().all(|stage| stage.omitted));
        assert_eq!(snapshot.resources.len(), 10);
    }

    #[test]
    fn seal_request_and_window_boundaries_are_exact() {
        let (root, mut request) = write_inputs("request-boundaries", REVISION);

        for duration in [0, 604_800_001] {
            request.requested_duration_ms = duration;
            request.interval_ms = 1;
            let mut errors = Vec::new();
            validate_seal_request(&request, &mut errors);
            assert_error(
                &errors,
                "requested duration must be within one millisecond and seven days",
            );
        }

        request.requested_duration_ms = 3_000;
        for interval in [0, 3_000] {
            request.interval_ms = interval;
            let mut errors = Vec::new();
            validate_seal_request(&request, &mut errors);
            assert_error(
                &errors,
                "sample interval must be positive and shorter than duration",
            );
        }

        request.requested_duration_ms = 4_094;
        request.interval_ms = 1;
        let mut errors = Vec::new();
        validate_seal_request(&request, &mut errors);
        assert!(
            !errors
                .iter()
                .any(|error| error == "requested sample bound must remain within 4 and 4096")
        );

        request.requested_duration_ms = 3_000;
        request.interval_ms = 1_000;
        for process_start in [String::new(), "x".repeat(257)] {
            request.process_start = process_start;
            let mut errors = Vec::new();
            validate_seal_request(&request, &mut errors);
            assert_error(
                &errors,
                "process start identity must contain 1 to 256 bytes",
            );
        }

        request.process_start = "fixture-start".to_owned();
        request.requested_duration_ms = 4_000;
        assert!(seal(&request).is_ok());
        fs::remove_dir_all(root).expect("remove request boundary fixture");
    }

    #[test]
    fn sample_window_tolerance_is_symmetric_and_bounded() {
        assert!(!super::sample_window_matches(0, 3_000, 1_000));
        for actual in [2_000, 3_000, 4_000] {
            assert!(super::sample_window_matches(actual, 3_000, 1_000));
        }
        for actual in [1_999, 4_001] {
            assert!(!super::sample_window_matches(actual, 3_000, 1_000));
        }
    }

    #[test]
    fn manifest_boundaries_reject_each_invalid_axis() {
        let (root, request) = write_inputs("manifest-boundaries", REVISION);
        seal(&request).expect("seal valid manifest fixture");
        let manifest_path = request.destination.join("session.toml");

        for mutate in [
            |manifest: &mut Manifest| {
                manifest.duration_ms = 0;
                manifest.requested_duration_ms = 0;
                manifest.interval_ms = 0;
            },
            |manifest: &mut Manifest| {
                manifest.duration_ms = manifest.requested_duration_ms + manifest.interval_ms + 1;
            },
            |manifest: &mut Manifest| {
                manifest.duration_ms = manifest.requested_duration_ms - manifest.interval_ms - 1;
            },
        ] {
            assert_error(
                &manifest_errors(&manifest_path, mutate),
                "captured duration is outside the requested sample window",
            );
        }

        for mutate in [
            |manifest: &mut Manifest| manifest.interval_ms = 0,
            |manifest: &mut Manifest| {
                manifest.interval_ms = manifest.requested_duration_ms;
            },
        ] {
            assert_error(
                &manifest_errors(&manifest_path, mutate),
                "manifest interval is invalid",
            );
        }

        assert_error(
            &manifest_errors(&manifest_path, |manifest| manifest.process_pid = 0),
            "manifest PID must be positive",
        );
        for process_start in [String::new(), "x".repeat(257)] {
            assert_error(
                &manifest_errors(&manifest_path, |manifest| {
                    manifest.process_start = process_start;
                }),
                "manifest process start identity is invalid",
            );
        }
        assert_error(
            &manifest_errors(&manifest_path, |manifest| manifest.binary_bytes = 0),
            "binary byte count must be positive",
        );
        assert_error(
            &manifest_errors(&manifest_path, |manifest| manifest.sampler_bytes = 0),
            "sampler byte count must be positive",
        );

        fs::remove_dir_all(root).expect("remove manifest boundary fixture");
    }

    #[test]
    fn artifact_paths_require_nonempty_relative_normal_components() {
        let root = root("artifact-paths");
        fs::create_dir_all(&root).expect("create artifact path fixture");
        for file in ["", "../snapshot.toml"] {
            let mut errors = Vec::new();
            validate_artifacts(
                &root,
                &[Artifact {
                    name: "snapshot".to_owned(),
                    file: file.to_owned(),
                    sha256: SHA.to_owned(),
                    bytes: 0,
                }],
                &mut errors,
            );
            assert_error(&errors, "artifact snapshot escapes its bundle");
        }
        fs::remove_dir_all(root).expect("remove artifact path fixture");
    }

    #[test]
    fn snapshot_identity_status_duration_and_omission_bounds_are_exact() {
        let valid_footprint: Footprint =
            serde_json::from_str(&footprint(42)).expect("parse footprint");

        let mut zero_duration: InternalDiagnostic =
            serde_json::from_str(&internal(REVISION)).expect("parse internal");
        zero_duration.duration_ms = 0;
        assert_error(
            &derive_snapshot(&zero_duration, &valid_footprint, 42)
                .expect_err("reject zero application duration"),
            "internal duration must be positive",
        );

        for status in [String::new(), "x".repeat(4_097)] {
            let mut diagnostic: InternalDiagnostic =
                serde_json::from_str(&internal(REVISION)).expect("parse internal");
            diagnostic.status = status;
            assert_error(
                &derive_snapshot(&diagnostic, &valid_footprint, 42)
                    .expect_err("reject status boundary"),
                "internal status is invalid",
            );
        }

        let mut zero_window: Footprint =
            serde_json::from_str(&footprint(42)).expect("parse footprint");
        for sample in &mut zero_window.samples {
            sample.start_time.wall_time_s = 1_000.0;
        }
        let diagnostic: InternalDiagnostic =
            serde_json::from_str(&internal(REVISION)).expect("parse internal");
        assert_error(
            &derive_snapshot(&diagnostic, &zero_window, 42)
                .expect_err("reject zero physical duration"),
            "physical sample duration must be positive",
        );

        let mut excessive_omissions: InternalDiagnostic =
            serde_json::from_str(&internal(REVISION)).expect("parse internal");
        excessive_omissions.omissions = (0..33).map(|index| format!("omission-{index}")).collect();
        assert_error(
            &derive_snapshot(&excessive_omissions, &valid_footprint, 42)
                .expect_err("reject excessive omissions"),
            "omission inventory is invalid",
        );
    }

    #[test]
    fn frame_and_language_validation_discriminate_each_ordering_axis() {
        for frames in [
            FrameSnapshot {
                requested: 2,
                submitted: 2,
                completed: 1,
                presented: 2,
                omitted: 0,
                idle_submissions: 0,
                peak_in_flight: 1,
            },
            FrameSnapshot {
                requested: 2,
                submitted: 1,
                completed: 2,
                presented: 1,
                omitted: 0,
                idle_submissions: 0,
                peak_in_flight: 1,
            },
            FrameSnapshot {
                requested: 1,
                submitted: 2,
                completed: 1,
                presented: 1,
                omitted: 0,
                idle_submissions: 0,
                peak_in_flight: 1,
            },
        ] {
            let mut errors = Vec::new();
            validate_frames(&frames, &mut errors);
            assert_error(&errors, "internal frame counters are out of order");
        }

        for (current, peak, budget) in [(2, 1, 3), (1, 3, 2)] {
            let mut errors = Vec::new();
            validate_language(
                &LanguageSnapshot {
                    requests: 1,
                    responses: Some(1),
                    stale_responses: 0,
                    restarts: 0,
                    current_retained_bytes: current,
                    peak_retained_bytes: peak,
                    budget_bytes: budget,
                },
                &[],
                &mut errors,
            );
            assert_error(&errors, "language bytes exceed peak or budget");
        }

        let mut valid_errors = Vec::new();
        validate_language(
            &LanguageSnapshot {
                requests: 1,
                responses: Some(1),
                stale_responses: 0,
                restarts: 0,
                current_retained_bytes: 1,
                peak_retained_bytes: 2,
                budget_bytes: 3,
            },
            &[],
            &mut valid_errors,
        );
        assert!(valid_errors.is_empty());

        validate_language(
            &LanguageSnapshot {
                requests: 1,
                responses: None,
                stale_responses: 0,
                restarts: 0,
                current_retained_bytes: 1,
                peak_retained_bytes: 2,
                budget_bytes: 3,
            },
            &["language-responses".to_owned()],
            &mut valid_errors,
        );
        assert!(valid_errors.is_empty());
    }

    #[test]
    fn elapsed_sample_range_is_bounded_in_validation_and_projection() {
        let mut footprint: Footprint =
            serde_json::from_str(&footprint(42)).expect("parse footprint");
        footprint.samples[3].start_time.wall_time_s = 605_801.0;
        let mut errors = Vec::new();
        let samples = derive_process_samples(&footprint, 42, &mut errors);
        assert_error(&errors, "footprint sample 3 elapsed time is invalid");
        assert_eq!(samples[3].elapsed_ms, 0);
    }

    #[test]
    fn identity_and_fixed_list_boundaries_fail_independently() {
        let mut invalid_schema: Draft = toml::from_str(&draft()).expect("parse draft");
        invalid_schema.schema = "invalid".to_owned();
        assert_error(
            &validate_identity(&invalid_schema),
            "draft schema must be alpine-studio-dogfood-draft/v2",
        );

        let valid: Draft = toml::from_str(&draft()).expect("parse draft");
        let mut zero_version = valid.identity.clone();
        zero_version.workload_version = 0;
        let mut errors = Vec::new();
        validate_identity_fields(&zero_version, &mut errors);
        assert_error(&errors, "workload version must be positive");

        for value in [String::new(), "x".repeat(4_097)] {
            let mut identity = valid.identity.clone();
            identity.workspace_fixture = value;
            let mut errors = Vec::new();
            validate_identity_fields(&identity, &mut errors);
            assert_error(&errors, "workspace fixture is empty or too long");
        }

        for assumptions in [Vec::new(), vec!["x".to_owned(); 33]] {
            let mut identity = valid.identity.clone();
            identity.assumptions = assumptions;
            let mut errors = Vec::new();
            validate_identity_fields(&identity, &mut errors);
            assert_error(&errors, "assumptions must contain 1 to 32 items");
        }

        for assumption in [String::new(), "x".repeat(4_097)] {
            let mut identity = valid.identity.clone();
            identity.assumptions = vec![assumption];
            let mut errors = Vec::new();
            validate_identity_fields(&identity, &mut errors);
            assert_error(&errors, "assumption is empty or too long");
        }

        for values in [Vec::new(), vec!["launch".to_owned(); 33]] {
            let mut errors = Vec::new();
            validate_fixed_list("coverage", &values, &["launch"], false, &mut errors);
            assert_error(&errors, "coverage must contain 1 to 32 items");
        }
    }

    #[test]
    fn bounded_read_and_identity_syntax_edges_are_exact() {
        let root = root("syntax-boundaries");
        fs::create_dir_all(&root).expect("create syntax fixture");
        let bounded = root.join("bounded");
        fs::write(&bounded, b"abcd").expect("write bounded fixture");
        assert_eq!(
            read_limited(&bounded, 4).expect("accept exact limit"),
            b"abcd"
        );
        assert!(read_limited(&bounded, 3).is_err());

        assert!(valid_slug("fixture-slug.1"));
        for invalid in [String::new(), "x".repeat(129), "INVALID".to_owned()] {
            assert!(!valid_slug(&invalid));
        }

        assert!(valid_git_sha(REVISION));
        for invalid in ["b".repeat(39), "B".repeat(40), "g".repeat(40)] {
            assert!(!valid_git_sha(&invalid));
        }

        assert!(valid_sha256(SHA));
        for invalid in ["a".repeat(63), "A".repeat(64), "g".repeat(64)] {
            assert!(!valid_sha256(&invalid));
        }

        assert!(valid_timestamp("2026-08-30T18:00:00Z"));
        let valid = b"2026-08-30T18:00:00Z";
        let mut invalid_timestamps = vec!["2026-08-30T18:00:00Z0".to_owned()];
        for index in [4, 7, 10, 13, 16, 19] {
            let mut bytes = *valid;
            bytes[index] = b'x';
            invalid_timestamps.push(String::from_utf8(bytes.to_vec()).expect("ASCII timestamp"));
        }
        let mut nondigit = *valid;
        nondigit[0] = b'x';
        invalid_timestamps.push(String::from_utf8(nondigit.to_vec()).expect("ASCII timestamp"));
        for invalid in invalid_timestamps {
            assert!(!valid_timestamp(&invalid));
        }

        fs::remove_dir_all(root).expect("remove syntax fixture");
    }

    #[test]
    fn accessibility_validation_discriminates_bounds_and_exact_omissions() {
        let mut errors = Vec::new();
        validate_accessibility(
            &AccessibilitySnapshot {
                queries: 0,
                actions: 0,
                stale_actions: Some(0),
                retained_nodes: None,
                peak_retained_nodes: None,
            },
            &[],
            &mut errors,
        );
        assert!(
            errors
                .iter()
                .any(|error| error == "missing accessibility tree lacks an omission")
        );

        for (current, peak) in [(2, 1), (272, 272)] {
            let mut errors = Vec::new();
            validate_accessibility(
                &AccessibilitySnapshot {
                    queries: 0,
                    actions: 0,
                    stale_actions: Some(0),
                    retained_nodes: Some(current),
                    peak_retained_nodes: Some(peak),
                },
                &[],
                &mut errors,
            );
            assert!(
                errors
                    .iter()
                    .any(|error| error == "accessibility nodes exceed their bound")
            );
        }

        let mut errors = Vec::new();
        validate_accessibility(
            &AccessibilitySnapshot {
                queries: 0,
                actions: 0,
                stale_actions: Some(0),
                retained_nodes: None,
                peak_retained_nodes: None,
            },
            &["accessibility-tree".to_owned()],
            &mut errors,
        );
        assert!(errors.is_empty());

        validate_accessibility(
            &AccessibilitySnapshot {
                queries: 0,
                actions: 0,
                stale_actions: None,
                retained_nodes: Some(0),
                peak_retained_nodes: Some(0),
            },
            &["accessibility-stale-actions".to_owned()],
            &mut errors,
        );
        assert!(errors.is_empty());
    }

    #[test]
    fn lifecycle_validation_distinguishes_failed_and_invalid_passed_captures() {
        let mut errors = Vec::new();
        validate_lifecycle(
            &LifecycleSnapshot {
                close_requests: 0,
                close_completions: 1,
                clean_shutdown: false,
                post_close_bytes: 1,
                post_close_limit_bytes: 0,
            },
            "passed",
            &mut errors,
        );
        assert!(
            errors
                .iter()
                .any(|error| error == "close completions exceed close requests")
        );

        let mut failed_errors = Vec::new();
        validate_lifecycle(
            &LifecycleSnapshot {
                close_requests: 0,
                close_completions: 0,
                clean_shutdown: false,
                post_close_bytes: 1,
                post_close_limit_bytes: 0,
            },
            "failed",
            &mut failed_errors,
        );
        assert!(failed_errors.is_empty());
    }

    #[test]
    fn resource_validation_discriminates_each_byte_axis_and_exact_omission() {
        for (current, peak, budget) in [
            (Some(1), None, None),
            (None, Some(1), None),
            (None, None, Some(1)),
        ] {
            let mut errors = Vec::new();
            validate_resources(
                &[ResourceSnapshot {
                    name: "font-cache".to_owned(),
                    current_bytes: current,
                    peak_bytes: peak,
                    budget_bytes: budget,
                    omitted: true,
                    omitted_axes: Vec::new(),
                }],
                &["font-cache".to_owned()],
                &mut errors,
            );
            assert!(
                errors
                    .iter()
                    .any(|error| error == "omitted resource font-cache contains bytes")
            );
        }

        let mut omission_errors = Vec::new();
        validate_resources(
            &[ResourceSnapshot {
                name: "font-cache".to_owned(),
                current_bytes: None,
                peak_bytes: None,
                budget_bytes: None,
                omitted: true,
                omitted_axes: Vec::new(),
            }],
            &["font-cache".to_owned()],
            &mut omission_errors,
        );
        assert!(
            !omission_errors
                .iter()
                .any(|error| error == "resource font-cache lacks a matching omission")
        );

        for (current, peak, budget) in [(2, 1, 3), (1, 3, 2)] {
            let mut errors = Vec::new();
            validate_resources(
                &[ResourceSnapshot {
                    name: "layout-cache".to_owned(),
                    current_bytes: Some(current),
                    peak_bytes: Some(peak),
                    budget_bytes: Some(budget),
                    omitted: false,
                    omitted_axes: Vec::new(),
                }],
                &[],
                &mut errors,
            );
            assert!(
                errors
                    .iter()
                    .any(|error| error == "resource layout-cache exceeds peak or budget")
            );
        }
    }

    #[test]
    fn footprint_validation_requires_strictly_increasing_sample_time() {
        let mut footprint: Footprint =
            serde_json::from_str(&footprint(42)).expect("parse footprint");
        footprint.samples[1].start_time.wall_time_s = footprint.samples[0].start_time.wall_time_s;
        let mut errors = Vec::new();
        let _ = derive_process_samples(&footprint, 42, &mut errors);
        assert!(
            errors
                .iter()
                .any(|error| error == "footprint sample 1 wall time is not increasing")
        );
    }

    #[test]
    fn rejects_identity_and_sample_drift() {
        let internal: InternalDiagnostic =
            serde_json::from_str(&internal(REVISION)).expect("parse internal");
        let wrong_pid: Footprint = serde_json::from_str(&footprint(41)).expect("parse footprint");
        let errors = derive_snapshot(&internal, &wrong_pid, 42).expect_err("reject PID drift");
        assert!(errors.iter().any(|error| error.contains("PID differs")));

        let short = footprint(42).replacen(",\n{\"start_time\":{\"wall_time_s\":1003.0}", "", 1);
        assert!(serde_json::from_str::<Footprint>(&short).is_err());
    }

    #[test]
    fn seals_validates_reports_and_refuses_overwrite_or_tamper() {
        let (root, request) = write_inputs("seal", REVISION);
        let message = seal(&request).expect("seal fixture");
        assert!(message.contains("fixture-live-session"));
        let manifest = request.destination.join("session.toml");
        assert!(is_v2(&manifest));
        assert!(
            run("validate-studio-dogfood", &manifest)
                .expect("validate fixture")
                .contains("4 physical samples")
        );
        let report = run("studio-dogfood-report", &manifest).expect("render report");
        assert!(report.contains("GPU process sampling: omitted, unavailable"));
        assert!(
            seal(&request)
                .expect_err("refuse overwrite")
                .iter()
                .any(|error| error.contains("already exists"))
        );

        fs::write(request.destination.join("studio.stdout"), "tampered")
            .expect("tamper retained output");
        assert!(
            run("validate-studio-dogfood", &manifest)
                .expect_err("reject tamper")
                .iter()
                .any(|error| error.contains("SHA-256 differs"))
        );
        fs::remove_dir_all(root).expect("remove fixture root");
    }

    #[test]
    fn rejects_revision_duration_and_scope_mismatch_without_publication() {
        let wrong_revision = "cccccccccccccccccccccccccccccccccccccccc";
        let (root, mut request) = write_inputs("reject", wrong_revision);
        assert!(
            seal(&request)
                .expect_err("reject revision")
                .iter()
                .any(|error| error.contains("expected repository revision"))
        );
        assert!(!request.destination.exists());

        fs::write(&request.internal, internal(REVISION)).expect("restore internal");
        request.requested_duration_ms = 2_000;
        request.interval_ms = 999;
        assert!(
            seal(&request)
                .expect_err("reject duration")
                .iter()
                .any(|error| error.contains("requested duration"))
        );
        request.requested_duration_ms = 3_000;
        request.interval_ms = 1_000;
        request.evidence_scope = "claim".to_owned();
        assert!(
            seal(&request)
                .expect_err("reject scope")
                .iter()
                .any(|error| error.contains("evidence scope"))
        );
        request.evidence_scope = "fixture".to_owned();
        request.expected_pid = 0;
        assert!(
            seal(&request)
                .expect_err("reject zero PID")
                .iter()
                .any(|error| error.contains("PID must be positive"))
        );
        fs::remove_dir_all(root).expect("remove fixture root");
    }
}
