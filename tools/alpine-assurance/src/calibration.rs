//! Validates raw renderer A/A calibration evidence without making a performance claim.

use serde::{Deserialize, de::DeserializeOwned};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write as _,
    fs,
    path::{Component, Path, PathBuf},
    process::Command,
};

const SCHEMA: &str = "alpine-aa-calibration/v1";
const MINIMUM_RUNS: usize = 20;
const MINIMUM_WINDOWS: usize = 4;
const MAXIMUM_RUNS: usize = 4_096;
const MAXIMUM_PAIRS_PER_RUN: u64 = 1_000_000;
const MAXIMUM_RAW_BYTES: u64 = 67_108_864;
const CSV_HEADER: &str = "run_id,pair_index,order,base,candidate";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Calibration {
    schema: String,
    id: String,
    comparison_level: String,
    workload_hash: String,
    base_renderer: String,
    candidate_renderer: String,
    base_revision: String,
    candidate_revision: String,
    metric: String,
    unit: String,
    direction: String,
    measurement_stage: String,
    clock: String,
    sample_class: String,
    warmup_iterations: u64,
    raw_samples_artifact: String,
    raw_samples_sha256: String,
    assumptions: Vec<String>,
    exclusions: Vec<String>,
    windows: Vec<Window>,
    runs: Vec<Run>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Window {
    id: String,
    lease_id: String,
    environment_kind: String,
    hardware_id: String,
    hardware_model: String,
    gpu: String,
    memory_bytes: u64,
    os_build: String,
    xcode_build: String,
    rustc: String,
    runner_image: String,
    power_state: String,
    thermal_policy: String,
    display_state: String,
    shader_mode: String,
    validation_enabled: bool,
    started_at_utc: String,
    ended_at_utc: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Run {
    id: String,
    window_id: String,
    randomization_seed: String,
    expected_pairs: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Order {
    BaseFirst,
    CandidateFirst,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Sample {
    run_id: String,
    pair_index: u64,
    order: Order,
    base: u64,
    candidate: u64,
}

#[derive(Default)]
struct Diagnostics {
    errors: Vec<String>,
}

impl Diagnostics {
    fn require(&mut self, condition: bool, message: impl Into<String>) {
        if !condition {
            self.errors.push(message.into());
        }
    }

    fn finish(mut self) -> Vec<String> {
        self.errors.sort();
        self.errors.dedup();
        self.errors
    }
}

pub(crate) fn run(command: &str, manifest: &Path, root: &Path) -> Result<String, Vec<String>> {
    let calibration: Calibration = load_toml(manifest)?;
    let artifact = resolve_repository_path(root, &calibration.raw_samples_artifact)?;
    let mut diagnostics = Diagnostics::default();
    validate_identity(&calibration, &mut diagnostics);
    validate_windows(&calibration, &mut diagnostics);
    validate_runs(&calibration, &mut diagnostics);
    validate_artifact_identity(&calibration, root, &artifact, &mut diagnostics);

    let samples = match load_samples(&artifact) {
        Ok(samples) => samples,
        Err(errors) => {
            diagnostics.errors.extend(errors);
            Vec::new()
        }
    };
    validate_samples(&calibration, &samples, &mut diagnostics);

    let errors = diagnostics.finish();
    if !errors.is_empty() {
        return Err(errors);
    }

    match command {
        "validate-aa-calibration" => Ok(format!(
            "validated A/A calibration {} with {} runs, {} windows, and {} pairs; no performance claim",
            calibration.id,
            calibration.runs.len(),
            calibration.windows.len(),
            samples.len()
        )),
        "aa-calibration-report" => Ok(render_report(&calibration, &samples)),
        other => Err(vec![format!(
            "unsupported A/A calibration command {other:?}"
        )]),
    }
}

fn load_toml<T: DeserializeOwned>(path: &Path) -> Result<T, Vec<String>> {
    let source = fs::read_to_string(path)
        .map_err(|error| vec![format!("cannot read {}: {error}", path.display())])?;
    toml::from_str(&source)
        .map_err(|error| vec![format!("cannot parse {}: {error}", path.display())])
}

fn resolve_repository_path(root: &Path, value: &str) -> Result<PathBuf, Vec<String>> {
    let path = Path::new(value);
    if value.is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(vec![format!(
            "calibration artifact path must be a repository-relative normal path: {value}"
        )]);
    }
    Ok(root.join(path))
}

fn validate_identity(calibration: &Calibration, diagnostics: &mut Diagnostics) {
    diagnostics.require(
        calibration.schema == SCHEMA,
        format!("calibration schema must be {SCHEMA}"),
    );
    for (name, value) in [
        ("calibration", calibration.id.as_str()),
        ("base renderer", calibration.base_renderer.as_str()),
        (
            "candidate renderer",
            calibration.candidate_renderer.as_str(),
        ),
        ("metric", calibration.metric.as_str()),
        ("measurement stage", calibration.measurement_stage.as_str()),
    ] {
        diagnostics.require(
            valid_slug(value),
            format!("{name} identifier must be a lowercase slug"),
        );
    }
    diagnostics.require(
        calibration.comparison_level == "renderer-only",
        "A/A calibration currently supports only renderer-only comparisons",
    );
    diagnostics.require(
        valid_sha256(&calibration.workload_hash),
        "calibration workload hash must be a lowercase SHA-256",
    );
    diagnostics.require(
        valid_sha256(&calibration.raw_samples_sha256),
        "raw sample artifact hash must be a lowercase SHA-256",
    );
    diagnostics.require(
        valid_git_sha(&calibration.base_revision) && valid_git_sha(&calibration.candidate_revision),
        "A/A revisions must be full lowercase Git SHAs",
    );
    diagnostics.require(
        calibration.base_renderer == calibration.candidate_renderer,
        "A/A base and candidate renderers must match",
    );
    diagnostics.require(
        calibration.base_revision == calibration.candidate_revision,
        "A/A base and candidate revisions must match",
    );
    diagnostics.require(
        !calibration.unit.trim().is_empty(),
        "calibration metric unit is required",
    );
    diagnostics.require(
        !calibration.clock.trim().is_empty(),
        "calibration measurement clock is required",
    );
    diagnostics.require(
        matches!(calibration.sample_class.as_str(), "cold" | "warm"),
        "calibration sample class must be cold or warm",
    );
    diagnostics.require(
        matches!(
            (calibration.sample_class.as_str(), calibration.warmup_iterations),
            ("cold", 0) | ("warm", 1..)
        ),
        "cold calibration requires zero warmup iterations and warm calibration requires at least one",
    );
    diagnostics.require(
        matches!(
            calibration.direction.as_str(),
            "lower-is-better" | "higher-is-better"
        ),
        "calibration direction must be lower-is-better or higher-is-better",
    );
    diagnostics.require(
        !calibration.assumptions.is_empty() && !calibration.exclusions.is_empty(),
        "calibration must disclose assumptions and exclusions",
    );
    for (kind, values) in [
        ("assumption", calibration.assumptions.as_slice()),
        ("exclusion", calibration.exclusions.as_slice()),
    ] {
        diagnostics.require(
            values.iter().all(|value| !value.trim().is_empty()),
            format!("calibration {kind}s cannot contain empty values"),
        );
    }
}

fn validate_windows(calibration: &Calibration, diagnostics: &mut Diagnostics) {
    diagnostics.require(
        calibration.windows.len() >= MINIMUM_WINDOWS,
        format!("A/A calibration requires at least {MINIMUM_WINDOWS} independent windows"),
    );
    let mut identifiers = BTreeSet::new();
    let mut leases = BTreeSet::new();
    let mut environment_kind = None;
    for window in &calibration.windows {
        diagnostics.require(
            valid_slug(&window.id),
            format!("invalid calibration window identifier {}", window.id),
        );
        diagnostics.require(
            identifiers.insert(window.id.as_str()),
            format!("duplicate calibration window identifier {}", window.id),
        );
        diagnostics.require(
            valid_slug(&window.lease_id),
            format!("window {} has an invalid lease identifier", window.id),
        );
        diagnostics.require(
            leases.insert(window.lease_id.as_str()),
            format!("duplicate lease identity {}", window.lease_id),
        );
        diagnostics.require(
            matches!(
                window.environment_kind.as_str(),
                "leased-physical" | "test-fixture"
            ),
            format!(
                "window {} is not qualified physical or test-fixture evidence",
                window.id
            ),
        );
        match environment_kind {
            None => environment_kind = Some(window.environment_kind.as_str()),
            Some(kind) => diagnostics.require(
                kind == window.environment_kind,
                "calibration windows cannot mix environment kinds",
            ),
        }
        for (name, value) in [
            ("hardware identity", window.hardware_id.as_str()),
            ("hardware model", window.hardware_model.as_str()),
            ("GPU", window.gpu.as_str()),
            ("operating-system build", window.os_build.as_str()),
            ("Xcode build", window.xcode_build.as_str()),
            ("Rust compiler", window.rustc.as_str()),
            ("runner image", window.runner_image.as_str()),
            ("power state", window.power_state.as_str()),
            ("thermal policy", window.thermal_policy.as_str()),
            ("display state", window.display_state.as_str()),
            ("start time", window.started_at_utc.as_str()),
            ("end time", window.ended_at_utc.as_str()),
        ] {
            diagnostics.require(
                !value.trim().is_empty(),
                format!("window {} {name} is required", window.id),
            );
        }
        diagnostics.require(
            window.memory_bytes > 0,
            format!("window {} memory must be positive", window.id),
        );
        diagnostics.require(
            valid_utc_timestamp(&window.started_at_utc)
                && valid_utc_timestamp(&window.ended_at_utc)
                && window.started_at_utc < window.ended_at_utc,
            format!(
                "window {} must have ordered second-resolution UTC timestamps",
                window.id
            ),
        );
        diagnostics.require(
            window.shader_mode == "offline-metallib",
            format!("window {} must use offline-metallib shaders", window.id),
        );
        diagnostics.require(
            !window.validation_enabled,
            format!(
                "window {} cannot measure with validation layers enabled",
                window.id
            ),
        );
    }
}

fn validate_runs(calibration: &Calibration, diagnostics: &mut Diagnostics) {
    diagnostics.require(
        calibration.runs.len() >= MINIMUM_RUNS,
        format!("A/A calibration requires at least {MINIMUM_RUNS} runs"),
    );
    diagnostics.require(
        calibration.runs.len() <= MAXIMUM_RUNS,
        format!("A/A calibration supports at most {MAXIMUM_RUNS} runs"),
    );
    let window_ids: BTreeSet<&str> = calibration
        .windows
        .iter()
        .map(|window| window.id.as_str())
        .collect();
    let mut run_ids = BTreeSet::new();
    let mut seeds = BTreeSet::new();
    let mut observed_windows = BTreeSet::new();
    for run in &calibration.runs {
        diagnostics.require(
            valid_slug(&run.id),
            format!("invalid calibration run identifier {}", run.id),
        );
        diagnostics.require(
            run_ids.insert(run.id.as_str()),
            format!("duplicate calibration run identifier {}", run.id),
        );
        diagnostics.require(
            window_ids.contains(run.window_id.as_str()),
            format!("run {} references unknown window {}", run.id, run.window_id),
        );
        observed_windows.insert(run.window_id.as_str());
        diagnostics.require(
            valid_sha256(&run.randomization_seed),
            format!("run {} randomization seed must be a SHA-256", run.id),
        );
        diagnostics.require(
            seeds.insert(run.randomization_seed.as_str()),
            format!("duplicate randomization seed in run {}", run.id),
        );
        diagnostics.require(
            (2..=MAXIMUM_PAIRS_PER_RUN).contains(&run.expected_pairs),
            format!(
                "run {} expected pair count must be between 2 and {MAXIMUM_PAIRS_PER_RUN}",
                run.id
            ),
        );
    }
    for window_id in window_ids {
        diagnostics.require(
            observed_windows.contains(window_id),
            format!("calibration window {window_id} has no runs"),
        );
    }
}

fn validate_artifact_identity(
    calibration: &Calibration,
    root: &Path,
    artifact: &Path,
    diagnostics: &mut Diagnostics,
) {
    let relative = artifact.strip_prefix(root).unwrap_or(artifact);
    let mut component_path = root.to_path_buf();
    for component in relative.components() {
        component_path.push(component);
        if let Ok(metadata) = fs::symlink_metadata(&component_path) {
            diagnostics.require(
                !metadata.file_type().is_symlink(),
                format!(
                    "raw sample artifact path cannot traverse a symbolic link: {}",
                    component_path.display()
                ),
            );
        }
    }
    let metadata = fs::symlink_metadata(artifact);
    match metadata {
        Ok(metadata) => {
            diagnostics.require(
                metadata.file_type().is_file(),
                "raw sample artifact must be a regular file",
            );
            diagnostics.require(
                !metadata.file_type().is_symlink(),
                "raw sample artifact cannot be a symbolic link",
            );
            diagnostics.require(
                metadata.len() <= MAXIMUM_RAW_BYTES,
                format!("raw sample artifact exceeds {MAXIMUM_RAW_BYTES} bytes"),
            );
        }
        Err(error) => {
            diagnostics
                .errors
                .push(format!("cannot inspect {}: {error}", artifact.display()));
            return;
        }
    }
    match hash_file(artifact) {
        Ok(hash) => diagnostics.require(
            hash == calibration.raw_samples_sha256,
            "raw sample artifact SHA-256 does not match the manifest",
        ),
        Err(error) => diagnostics.errors.push(error),
    }
}

fn hash_file(path: &Path) -> Result<String, String> {
    let attempts: &[(&str, &[&str])] = &[
        ("sha256sum", &[]),
        ("shasum", &["-a", "256"]),
        ("certutil", &["-hashfile"]),
    ];
    for (program, prefix) in attempts {
        let mut command = Command::new(program);
        command.args(*prefix).arg(path);
        if *program == "certutil" {
            command.arg("SHA256");
        }
        let Ok(output) = command.output() else {
            continue;
        };
        if !output.status.success() {
            continue;
        }
        let stdout = String::from_utf8_lossy(&output.stdout).to_ascii_lowercase();
        if let Some(hash) = extract_sha256(&stdout) {
            return Ok(hash);
        }
    }
    Err(format!(
        "cannot calculate SHA-256 for {}; sha256sum, shasum, or certutil is required",
        path.display()
    ))
}

fn extract_sha256(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let compact: String = line
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect();
        compact.as_bytes().windows(64).find_map(|window| {
            let candidate = std::str::from_utf8(window).ok()?;
            valid_sha256(candidate).then(|| candidate.to_owned())
        })
    })
}

fn load_samples(path: &Path) -> Result<Vec<Sample>, Vec<String>> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| vec![format!("cannot inspect {}: {error}", path.display())])?;
    if !valid_raw_artifact_shape(
        metadata.file_type().is_file(),
        metadata.file_type().is_symlink(),
        metadata.len(),
    ) {
        return Err(vec![format!(
            "raw sample artifact must be a regular non-symlink file no larger than {MAXIMUM_RAW_BYTES} bytes"
        )]);
    }
    let source = fs::read_to_string(path)
        .map_err(|error| vec![format!("cannot read {}: {error}", path.display())])?;
    let mut lines = source.lines();
    if lines.next() != Some(CSV_HEADER) {
        return Err(vec![format!("raw sample CSV header must be {CSV_HEADER}")]);
    }
    let mut samples = Vec::new();
    let mut errors = Vec::new();
    for (offset, line) in lines.enumerate() {
        let line_number = offset + 2;
        if line.is_empty() {
            errors.push(format!("raw sample CSV line {line_number} is empty"));
            continue;
        }
        let fields: Vec<&str> = line.split(',').collect();
        if fields.len() != 5 {
            errors.push(format!(
                "raw sample CSV line {line_number} must contain five fields"
            ));
            continue;
        }
        let pair_index = fields[1].parse::<u64>();
        let base = fields[3].parse::<u64>();
        let candidate = fields[4].parse::<u64>();
        let order = match fields[2] {
            "base-first" => Some(Order::BaseFirst),
            "candidate-first" => Some(Order::CandidateFirst),
            _ => None,
        };
        if !valid_slug(fields[0])
            || pair_index.is_err()
            || order.is_none()
            || base.as_ref().is_err()
            || base.as_ref().is_ok_and(|value| *value == 0)
            || candidate.as_ref().is_err()
            || candidate.as_ref().is_ok_and(|value| *value == 0)
        {
            errors.push(format!("raw sample CSV line {line_number} is invalid"));
            continue;
        }
        if let (Ok(pair_index), Some(order), Ok(base), Ok(candidate)) =
            (pair_index, order, base, candidate)
        {
            samples.push(Sample {
                run_id: fields[0].to_owned(),
                pair_index,
                order,
                base,
                candidate,
            });
        }
    }
    if samples.is_empty() {
        errors.push("raw sample CSV must contain samples".to_owned());
    }
    if errors.is_empty() {
        Ok(samples)
    } else {
        errors.sort();
        Err(errors)
    }
}

fn valid_raw_artifact_shape(is_file: bool, is_symlink: bool, bytes: u64) -> bool {
    is_file && !is_symlink && bytes <= MAXIMUM_RAW_BYTES
}

fn validate_samples(calibration: &Calibration, samples: &[Sample], diagnostics: &mut Diagnostics) {
    let runs: BTreeMap<&str, &Run> = calibration
        .runs
        .iter()
        .map(|run| (run.id.as_str(), run))
        .collect();
    let mut by_run: BTreeMap<&str, Vec<&Sample>> = BTreeMap::new();
    let mut observed = BTreeSet::new();
    for sample in samples {
        diagnostics.require(
            runs.contains_key(sample.run_id.as_str()),
            format!("sample references unknown run {}", sample.run_id),
        );
        diagnostics.require(
            observed.insert((sample.run_id.as_str(), sample.pair_index)),
            format!(
                "duplicate sample pair {} in run {}",
                sample.pair_index, sample.run_id
            ),
        );
        by_run
            .entry(sample.run_id.as_str())
            .or_default()
            .push(sample);
    }
    for run in &calibration.runs {
        let mut run_samples = by_run.remove(run.id.as_str()).unwrap_or_default();
        run_samples.sort_by_key(|sample| sample.pair_index);
        diagnostics.require(
            run_samples.len() as u64 == run.expected_pairs,
            format!(
                "run {} expected {} pairs but found {}",
                run.id,
                run.expected_pairs,
                run_samples.len()
            ),
        );
        let mut base_first = 0_u64;
        let mut candidate_first = 0_u64;
        for (expected, sample) in run_samples.iter().enumerate() {
            diagnostics.require(
                sample.pair_index == expected as u64,
                format!("run {} pair indices must be contiguous", run.id),
            );
            match sample.order {
                Order::BaseFirst => base_first += 1,
                Order::CandidateFirst => candidate_first += 1,
            }
        }
        diagnostics.require(
            base_first > 0 && candidate_first > 0,
            format!("run {} must contain both execution orders", run.id),
        );
        diagnostics.require(
            base_first.abs_diff(candidate_first) <= 1,
            format!("run {} execution order is not balanced", run.id),
        );
    }
}

fn render_report(calibration: &Calibration, samples: &[Sample]) -> String {
    let fixture = calibration
        .windows
        .iter()
        .all(|window| window.environment_kind == "test-fixture");
    let status = if fixture {
        "fixture-only"
    } else {
        "protocol-ready"
    };
    let mut signed_ppm: Vec<i128> = samples.iter().map(relative_delta_ppm).collect();
    let mut absolute_ppm: Vec<i128> = signed_ppm.iter().map(|value| value.abs()).collect();
    signed_ppm.sort_unstable();
    absolute_ppm.sort_unstable();

    let mut output = String::from("# Alpine renderer A/A calibration report\n\n");
    let _ = write!(
        output,
        "- Calibration: {}\n- Status: {}\n- Performance claim: none\n- Comparison level: {}\n- Measurement stage: {}\n- Clock: {}\n- Sample class: {}\n- Warmup iterations: {}\n- Workload SHA-256: {}\n- Renderer: {}\n- Revision: {}\n- Metric: {} ({}, {})\n- Independent windows: {}\n- Calibration runs: {}\n- Paired samples: {}\n- Raw artifact: {}\n- Raw artifact SHA-256: {}\n\n",
        calibration.id,
        status,
        calibration.comparison_level,
        calibration.measurement_stage,
        calibration.clock,
        calibration.sample_class,
        calibration.warmup_iterations,
        calibration.workload_hash,
        calibration.base_renderer,
        calibration.base_revision,
        calibration.metric,
        calibration.unit,
        calibration.direction,
        calibration.windows.len(),
        calibration.runs.len(),
        samples.len(),
        calibration.raw_samples_artifact,
        calibration.raw_samples_sha256,
    );
    output.push_str("## Descriptive paired deltas\n\n");
    output.push_str(
        "Relative delta is `(candidate - base) / base` in integer parts per million. These descriptive values are not a confidence interval, equivalence margin, sample-size decision, or performance result.\n\n",
    );
    let _ = writeln!(
        output,
        "- Signed p50: {} ppm\n- Absolute p95: {} ppm\n- Absolute p99: {} ppm",
        nearest_rank(&signed_ppm, 50),
        nearest_rank(&absolute_ppm, 95),
        nearest_rank(&absolute_ppm, 99),
    );

    output.push_str("\n## Execution order\n\n");
    for order in [Order::BaseFirst, Order::CandidateFirst] {
        let mut values: Vec<i128> = samples
            .iter()
            .filter(|sample| sample.order == order)
            .map(relative_delta_ppm)
            .collect();
        values.sort_unstable();
        let label = match order {
            Order::BaseFirst => "base-first",
            Order::CandidateFirst => "candidate-first",
        };
        let _ = writeln!(
            output,
            "- {label}: {} pairs, signed p50 {} ppm",
            values.len(),
            nearest_rank(&values, 50)
        );
    }

    output.push_str("\n## Hardware windows\n\n");
    for window in &calibration.windows {
        let run_ids: BTreeSet<&str> = calibration
            .runs
            .iter()
            .filter(|run| run.window_id == window.id)
            .map(|run| run.id.as_str())
            .collect();
        let pair_count = samples
            .iter()
            .filter(|sample| run_ids.contains(sample.run_id.as_str()))
            .count();
        let _ = writeln!(
            output,
            "- {}: lease {}, {}, {} runs, {} pairs",
            window.id,
            window.lease_id,
            window.environment_kind,
            run_ids.len(),
            pair_count
        );
    }

    output.push_str("\n## Qualifications\n\n- Assumptions:\n");
    for assumption in &calibration.assumptions {
        let _ = writeln!(output, "  - {assumption}");
    }
    output.push_str("- Exclusions:\n");
    for exclusion in &calibration.exclusions {
        let _ = writeln!(output, "  - {exclusion}");
    }
    output
}

fn relative_delta_ppm(sample: &Sample) -> i128 {
    let base = i128::from(sample.base);
    let candidate = i128::from(sample.candidate);
    (candidate - base) * 1_000_000 / base
}

fn nearest_rank(sorted: &[i128], percentile: usize) -> i128 {
    let rank = percentile
        .saturating_mul(sorted.len())
        .div_ceil(100)
        .saturating_sub(1);
    sorted.get(rank).copied().unwrap_or_default()
}

fn valid_slug(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_git_sha(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(crate) fn valid_utc_timestamp(value: &str) -> bool {
    if value.len() != 20 {
        return false;
    }
    let bytes = value.as_bytes();
    if bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes[19] != b'Z'
        || bytes.iter().enumerate().any(|(index, byte)| {
            !matches!(index, 4 | 7 | 10 | 13 | 16 | 19) && !byte.is_ascii_digit()
        })
    {
        return false;
    }
    let parse = |start: usize, end: usize| {
        std::str::from_utf8(&bytes[start..end])
            .ok()
            .and_then(|part| part.parse::<u32>().ok())
    };
    let (Some(year), Some(month), Some(day), Some(hour), Some(minute), Some(second)) = (
        parse(0, 4),
        parse(5, 7),
        parse(8, 10),
        parse(11, 13),
        parse(14, 16),
        parse(17, 19),
    ) else {
        return false;
    };
    let leap_year = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days_in_month = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap_year => 29,
        2 => 28,
        _ => return false,
    };
    (1..=9999).contains(&year)
        && (1..=days_in_month).contains(&day)
        && hour <= 23
        && minute <= 59
        && second <= 59
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use super::validate_artifact_identity;
    use super::{
        CSV_HEADER, Calibration, MAXIMUM_RAW_BYTES, Order, Sample, extract_sha256, load_samples,
        load_toml, nearest_rank, relative_delta_ppm, render_report, resolve_repository_path, run,
        valid_git_sha, valid_raw_artifact_shape, valid_slug, valid_utc_timestamp,
        validate_identity, validate_runs, validate_samples, validate_windows,
    };
    use std::{
        fs,
        path::{Path, PathBuf},
    };

    fn repository_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    fn fixture() -> Result<Calibration, String> {
        load_toml(&repository_root().join("assurance/calibration/v1/valid.toml"))
            .map_err(|errors| format!("valid calibration fixture failed: {errors:#?}"))
    }

    fn samples() -> Result<Vec<Sample>, String> {
        load_samples(&repository_root().join("assurance/calibration/v1/raw/aa-samples.csv"))
            .map_err(|errors| format!("valid sample fixture failed: {errors:#?}"))
    }

    #[test]
    fn accepts_fixture_and_renders_no_claim() {
        let root = repository_root();
        let manifest = root.join("assurance/calibration/v1/valid.toml");
        let validation = run("validate-aa-calibration", &manifest, &root);
        assert!(validation.is_ok(), "{validation:#?}");
        let report = run("aa-calibration-report", &manifest, &root);
        assert!(report.is_ok(), "{report:#?}");
        if let Ok(report) = report {
            assert!(report.contains("Status: fixture-only"));
            assert!(report.contains("Performance claim: none"));
            assert!(report.contains("Measurement stage: renderer-submit-readback"));
            assert!(report.contains("Sample class: warm"));
            assert!(report.contains("Warmup iterations: 10"));
            assert!(report.contains("Calibration runs: 20"));
            assert!(report.contains("Independent windows: 4"));
            assert!(report.contains("Paired samples: 40"));
            assert!(report.contains("base-first: 20 pairs"));
            assert!(report.contains("candidate-first: 20 pairs"));
            assert!(
                report
                    .contains("window-01: lease fixture-lease-01, test-fixture, 5 runs, 10 pairs")
            );
        }
    }

    #[test]
    fn identity_rejects_non_aa_and_incomplete_disclosures() -> Result<(), String> {
        let mut calibration = fixture()?;
        calibration.candidate_revision = "2".repeat(40);
        calibration.candidate_renderer = "other-renderer".to_owned();
        calibration.assumptions.clear();
        calibration.clock.clear();
        calibration.sample_class = "mixed".to_owned();
        calibration.warmup_iterations = 0;
        calibration.base_revision = "not-a-full-sha".to_owned();
        let mut diagnostics = super::Diagnostics::default();
        validate_identity(&calibration, &mut diagnostics);
        let errors = diagnostics.finish();
        assert!(
            errors
                .iter()
                .any(|error| error.contains("revisions must match"))
        );
        assert!(
            errors
                .iter()
                .any(|error| error.contains("full lowercase Git SHAs"))
        );
        assert!(
            errors
                .iter()
                .any(|error| error.contains("renderers must match"))
        );
        assert!(
            errors
                .iter()
                .any(|error| error.contains("disclose assumptions"))
        );
        assert!(
            errors
                .iter()
                .any(|error| error.contains("clock is required"))
        );
        assert!(errors.iter().any(|error| error.contains("cold or warm")));
        assert!(
            errors
                .iter()
                .any(|error| error.contains("warm calibration"))
        );
        Ok(())
    }

    #[test]
    fn windows_reject_unqualified_modes_duplicates_and_missing_identity() -> Result<(), String> {
        let mut calibration = fixture()?;
        calibration.windows[0].environment_kind = "hosted-virtual".to_owned();
        calibration.windows[0].shader_mode = "runtime-source".to_owned();
        calibration.windows[0].validation_enabled = true;
        calibration.windows[0].hardware_id.clear();
        calibration.windows[0].memory_bytes = 0;
        calibration.windows[0].ended_at_utc = calibration.windows[0].started_at_utc.clone();
        calibration.windows[1].lease_id = calibration.windows[0].lease_id.clone();
        let mut diagnostics = super::Diagnostics::default();
        validate_windows(&calibration, &mut diagnostics);
        let errors = diagnostics.finish();
        for expected in [
            "not qualified physical",
            "offline-metallib",
            "validation layers",
            "hardware identity",
            "memory must be positive",
            "duplicate lease identity",
            "ordered second-resolution UTC timestamps",
        ] {
            assert!(
                errors.iter().any(|error| error.contains(expected)),
                "missing {expected}: {errors:#?}"
            );
        }
        Ok(())
    }

    #[test]
    fn runs_reject_insufficient_unknown_duplicate_and_unbounded_records() -> Result<(), String> {
        let mut calibration = fixture()?;
        calibration.runs.truncate(19);
        calibration.runs[0].window_id = "unknown-window".to_owned();
        calibration.runs[1].id = calibration.runs[0].id.clone();
        calibration.runs[2].randomization_seed = calibration.runs[0].randomization_seed.clone();
        calibration.runs[3].expected_pairs = 1;
        let mut diagnostics = super::Diagnostics::default();
        validate_runs(&calibration, &mut diagnostics);
        let errors = diagnostics.finish();
        for expected in [
            "at least 20 runs",
            "unknown window",
            "duplicate calibration run",
            "duplicate randomization seed",
            "expected pair count",
        ] {
            assert!(
                errors.iter().any(|error| error.contains(expected)),
                "missing {expected}: {errors:#?}"
            );
        }
        Ok(())
    }

    #[test]
    fn samples_reject_unknown_duplicate_gaps_count_and_order_imbalance() -> Result<(), String> {
        let calibration = fixture()?;
        let mut samples = samples()?;
        samples[39].run_id = "unknown-run".to_owned();
        samples[1].pair_index = samples[0].pair_index;
        samples[2].pair_index = 7;
        samples[5].order = Order::BaseFirst;
        let mut diagnostics = super::Diagnostics::default();
        validate_samples(&calibration, &samples, &mut diagnostics);
        let errors = diagnostics.finish();
        for expected in [
            "unknown run",
            "duplicate sample pair",
            "pair indices must be contiguous",
            "execution order",
        ] {
            assert!(
                errors.iter().any(|error| error.contains(expected)),
                "missing {expected}: {errors:#?}"
            );
        }
        Ok(())
    }

    #[test]
    fn parser_rejects_header_and_invalid_fields() {
        let root = repository_root();
        let path = root.join("assurance/calibration/v1/raw/invalid-samples.csv");
        let result = load_samples(&path);
        assert!(
            result.is_err(),
            "invalid sample fixture unexpectedly passed"
        );
        if let Err(errors) = result {
            assert!(
                errors
                    .iter()
                    .any(|error| error.contains("line 2 is invalid"))
            );
        }
    }

    #[test]
    fn parser_rejects_each_invalid_sample_field() -> Result<(), String> {
        let directory = repository_root().join("target/calibration-unit-tests");
        fs::create_dir_all(&directory)
            .map_err(|error| format!("cannot create fixture directory: {error}"))?;
        for (name, line) in [
            ("run-id", "INVALID,0,base-first,1,1"),
            ("pair-index", "run-01,no,base-first,1,1"),
            ("order", "run-01,0,unknown,1,1"),
            ("base-text", "run-01,0,base-first,no,1"),
            ("base-zero", "run-01,0,base-first,0,1"),
            ("candidate-text", "run-01,0,base-first,1,no"),
            ("candidate-zero", "run-01,0,base-first,1,0"),
        ] {
            let path = directory.join(format!("sample-{name}.csv"));
            fs::write(&path, format!("{CSV_HEADER}\n{line}\n"))
                .map_err(|error| format!("cannot write {name} fixture: {error}"))?;
            let result = load_samples(&path);
            fs::remove_file(&path)
                .map_err(|error| format!("cannot remove {name} fixture: {error}"))?;
            let errors = result.err().unwrap_or_default();
            assert!(
                errors
                    .iter()
                    .any(|error| error.contains("line 2 is invalid")),
                "{name} fixture was not rejected precisely: {errors:#?}"
            );
        }
        Ok(())
    }

    #[test]
    fn paths_and_hash_output_are_strict() {
        let root = repository_root();
        assert!(resolve_repository_path(&root, "assurance/raw.csv").is_ok());
        assert!(resolve_repository_path(&root, "").is_err());
        assert!(resolve_repository_path(&root, "/raw.csv").is_err());
        assert!(resolve_repository_path(&root, "../raw.csv").is_err());
        assert_eq!(
            extract_sha256(
                "SHA256 (fixture) = 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
            ),
            Some("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_owned())
        );
        assert_eq!(extract_sha256("not a digest"), None);
        assert!(valid_slug("renderer-01"));
        assert!(!valid_slug(""));
        assert!(!valid_slug("Renderer"));
        assert!(valid_git_sha(&"a".repeat(40)));
        assert!(!valid_git_sha(&"a".repeat(39)));
        assert!(!valid_git_sha(&"g".repeat(40)));
        assert!(valid_raw_artifact_shape(true, false, 0));
        assert!(valid_raw_artifact_shape(true, false, MAXIMUM_RAW_BYTES));
        assert!(!valid_raw_artifact_shape(false, false, 0));
        assert!(!valid_raw_artifact_shape(true, true, 0));
        assert!(!valid_raw_artifact_shape(
            true,
            false,
            MAXIMUM_RAW_BYTES + 1
        ));
    }

    #[cfg(unix)]
    #[test]
    fn artifact_identity_rejects_symlinked_parent_components() -> Result<(), String> {
        use std::os::unix::fs::symlink;

        let root = repository_root();
        let directory = root.join("target/calibration-symlink-tests");
        fs::create_dir_all(&directory)
            .map_err(|error| format!("cannot create symlink test directory: {error}"))?;
        let link = directory.join(format!("raw-{}", std::process::id()));
        if link.exists() {
            fs::remove_file(&link)
                .map_err(|error| format!("cannot clear symlink fixture: {error}"))?;
        }
        symlink(root.join("assurance/calibration/v1/raw"), &link)
            .map_err(|error| format!("cannot create symlink fixture: {error}"))?;
        let artifact = link.join("aa-samples.csv");
        let calibration = fixture()?;
        let mut diagnostics = super::Diagnostics::default();
        validate_artifact_identity(&calibration, &root, &artifact, &mut diagnostics);
        fs::remove_file(&link)
            .map_err(|error| format!("cannot remove symlink fixture: {error}"))?;
        let errors = diagnostics.finish();
        assert!(
            errors
                .iter()
                .any(|error| error.contains("cannot traverse a symbolic link"))
        );
        Ok(())
    }

    #[test]
    fn utc_timestamps_reject_invalid_calendar_and_clock_values() {
        for valid in [
            "2024-02-29T00:00:00Z",
            "2026-08-14T23:59:59Z",
            "2000-02-29T12:30:45Z",
            "2026-02-28T12:30:45Z",
            "2026-04-30T12:30:45Z",
        ] {
            assert!(
                valid_utc_timestamp(valid),
                "rejected valid timestamp {valid}"
            );
        }
        for invalid in [
            "",
            "2026-02-29T00:00:00Z",
            "1900-02-29T00:00:00Z",
            "2026-04-31T00:00:00Z",
            "2026-13-01T00:00:00Z",
            "2026-01-00T00:00:00Z",
            "2026-01-01T24:00:00Z",
            "2026-01-01T00:60:00Z",
            "2026-01-01T00:00:60Z",
            "2026/01-01T00:00:00Z",
            "2026-01/01T00:00:00Z",
            "2026-01-01 00:00:00Z",
            "2026-01-01T00.00:00Z",
            "2026-01-01T00:00.00Z",
            "2026-01-01T00:00:00X",
            "2026-01-01T00:00:0xZ",
            "2026-01-01T00:00:00+00:00",
        ] {
            assert!(
                !valid_utc_timestamp(invalid),
                "accepted invalid timestamp {invalid}"
            );
        }
    }

    #[test]
    fn sample_order_requires_both_variants_even_without_samples() -> Result<(), String> {
        let mut calibration = fixture()?;
        calibration.runs.truncate(1);
        calibration.runs[0].expected_pairs = 1;
        for order in [Order::BaseFirst, Order::CandidateFirst] {
            let mut diagnostics = super::Diagnostics::default();
            let one_order = [Sample {
                run_id: "run-01".to_owned(),
                pair_index: 0,
                order,
                base: 1,
                candidate: 1,
            }];
            validate_samples(&calibration, &one_order, &mut diagnostics);
            let errors = diagnostics.finish();
            assert!(
                errors
                    .iter()
                    .any(|error| error.contains("must contain both execution orders"))
            );
        }
        Ok(())
    }

    #[test]
    fn report_keeps_execution_orders_separate() -> Result<(), String> {
        let calibration = fixture()?;
        let samples = [
            Sample {
                run_id: "run-01".to_owned(),
                pair_index: 0,
                order: Order::BaseFirst,
                base: 100,
                candidate: 101,
            },
            Sample {
                run_id: "run-01".to_owned(),
                pair_index: 1,
                order: Order::CandidateFirst,
                base: 100,
                candidate: 102,
            },
            Sample {
                run_id: "run-02".to_owned(),
                pair_index: 0,
                order: Order::CandidateFirst,
                base: 100,
                candidate: 103,
            },
        ];
        let report = render_report(&calibration, &samples);
        assert!(report.contains("base-first: 1 pairs, signed p50 10000 ppm"));
        assert!(report.contains("candidate-first: 2 pairs, signed p50 20000 ppm"));
        Ok(())
    }

    #[test]
    fn integer_descriptive_statistics_are_deterministic() -> Result<(), String> {
        let sample = Sample {
            run_id: "run-01".to_owned(),
            pair_index: 0,
            order: Order::BaseFirst,
            base: 1_000,
            candidate: 1_010,
        };
        assert_eq!(relative_delta_ppm(&sample), 10_000);
        assert_eq!(nearest_rank(&[-5, -1, 2, 9], 50), -1);
        assert_eq!(nearest_rank(&[-5, -1, 2, 9], 95), 9);
        let report = render_report(&fixture()?, &samples()?);
        assert!(report.contains("not a confidence interval"));
        Ok(())
    }
}
