//! Validates trusted-machine macOS accessibility qualification evidence.

use serde::Deserialize;
use std::{
    collections::BTreeSet,
    fs,
    path::{Component, Path},
    process::Command,
};

const SCHEMA: &str = "alpine-ax-evidence/v1";
const MAX_ARTIFACT_BYTES: u64 = 268_435_456;
const MAX_TREE_NODES: usize = 271;
const MAX_IDENTIFIER_BYTES: usize = 256;
const MAX_LABEL_BYTES: usize = 4_096;
const EVENT_HEADER: &str = "sequence,monotonic_ns,kind,identifier,ax_error";
const TREE_HEADER: &str = "depth,identifier,role,label,focused";
const LATENCY_HEADER: &str = "sequence,operation,start_ns,end_ns,ax_error";
const RESIDENCY_HEADER: &str = "sequence,monotonic_ns,physical_footprint_bytes,private_dirty_bytes";
const REQUIRED_EVENTS: &[&str] = &[
    "focus",
    "value",
    "selection",
    "layout",
    "announcement",
    "hidden",
    "shown",
    "minimized",
    "restored",
    "sleep",
    "wake",
    "destroyed",
    "close",
];
const ALLOWED_EVENTS: &[&str] = &[
    "launch",
    "focus",
    "value",
    "selection",
    "layout",
    "announcement",
    "hidden",
    "shown",
    "minimized",
    "restored",
    "sleep",
    "wake",
    "destroyed",
    "close",
    "stale-control",
];

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(transparent)]
struct EvidenceFlag(bool);

impl EvidenceFlag {
    const fn is_set(self) -> bool {
        self.0
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Artifact {
    path: String,
    sha256: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ArtifactSet {
    studio_binary: Artifact,
    harness_binary: Artifact,
    scenario: Artifact,
    tree: Artifact,
    events: Artifact,
    latency: Artifact,
    residency: Artifact,
    stdout: Artifact,
    stderr: Artifact,
    inspector_capture: Artifact,
    human_checklist: Artifact,
    repository_diff: Option<Artifact>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AxEvidence {
    schema: String,
    task_issue: u64,
    repository_revision: String,
    repository_clean: EvidenceFlag,
    started_unix_ns: u64,
    ended_unix_ns: u64,
    studio_pid: u32,
    macos_build: String,
    sdk_build: String,
    rustc_version: String,
    hardware_model: String,
    architecture: String,
    locale: String,
    input_source: String,
    display_description: String,
    power_source: String,
    thermal_state: String,
    ax_trusted: EvidenceFlag,
    actual_sleep_wake: EvidenceFlag,
    voiceover_attested: EvidenceFlag,
    inspector_attested: EvidenceFlag,
    post_close_drain_attested: EvidenceFlag,
    latency_budget_active: EvidenceFlag,
    performance_claim: EvidenceFlag,
    tree_node_count: usize,
    event_count: usize,
    latency_sample_count: usize,
    residency_sample_count: usize,
    artifacts: ArtifactSet,
}

pub(crate) fn run(command: &str, bundle: &Path) -> Result<String, Vec<String>> {
    let manifest_path = bundle.join("manifest.toml");
    let source = fs::read_to_string(&manifest_path)
        .map_err(|error| vec![format!("cannot read {}: {error}", manifest_path.display())])?;
    let evidence: AxEvidence = toml::from_str(&source)
        .map_err(|error| vec![format!("cannot parse {}: {error}", manifest_path.display())])?;
    let mut errors = Vec::new();
    validate_identity(&evidence, &mut errors);
    validate_artifacts(bundle, &evidence, &mut errors);
    if errors.is_empty() {
        validate_contents(bundle, &evidence, &mut errors);
    }
    if !errors.is_empty() {
        errors.sort();
        return Err(errors);
    }
    if command == "validate-ax-evidence" {
        return Ok(format!(
            "validated task #273 physical AX bundle at revision {} with {} nodes, {} events, {} latency samples, and {} residency samples; no performance threshold or claim",
            evidence.repository_revision,
            evidence.tree_node_count,
            evidence.event_count,
            evidence.latency_sample_count,
            evidence.residency_sample_count
        ));
    }
    Ok(format!(
        "# Alpine physical accessibility qualification report\n\n- Revision: `{}`\n- Hardware: {} ({})\n- macOS build: {}\n- AX tree nodes: {}\n- Observed events: {}\n- Latency samples: {} (descriptive only)\n- Residency samples: {} (descriptive only)\n- Actual sleep/wake: attested\n- VoiceOver: human-attested\n- Accessibility Inspector: human-attested\n- Post-close drain: attested\n- Performance threshold: inactive\n- Performance claim: none\n",
        evidence.repository_revision,
        evidence.hardware_model,
        evidence.architecture,
        evidence.macos_build,
        evidence.tree_node_count,
        evidence.event_count,
        evidence.latency_sample_count,
        evidence.residency_sample_count
    ))
}

fn validate_identity(evidence: &AxEvidence, errors: &mut Vec<String>) {
    require(evidence.schema == SCHEMA, "AX schema must be exact", errors);
    require(
        evidence.task_issue == 273,
        "AX evidence must bind task #273",
        errors,
    );
    require(
        valid_hash(&evidence.repository_revision, 40),
        "repository revision must be a full lowercase Git hash",
        errors,
    );
    require(
        evidence.repository_clean.is_set() == evidence.artifacts.repository_diff.is_none(),
        "clean state must omit a diff and dirty state must retain one",
        errors,
    );
    require(
        evidence.started_unix_ns > 0 && evidence.ended_unix_ns > evidence.started_unix_ns,
        "capture times must be positive and ordered",
        errors,
    );
    require(
        evidence.studio_pid != 0,
        "Studio PID must be nonzero",
        errors,
    );
    for (name, value) in [
        ("macOS build", evidence.macos_build.as_str()),
        ("SDK build", evidence.sdk_build.as_str()),
        ("rustc version", evidence.rustc_version.as_str()),
        ("hardware model", evidence.hardware_model.as_str()),
        ("locale", evidence.locale.as_str()),
        ("input source", evidence.input_source.as_str()),
        ("display description", evidence.display_description.as_str()),
    ] {
        require(
            !value.trim().is_empty(),
            format!("{name} is required"),
            errors,
        );
    }
    require(
        evidence.architecture == "arm64",
        "physical qualification requires arm64",
        errors,
    );
    require(
        matches!(evidence.power_source.as_str(), "ac" | "battery"),
        "power source must be ac or battery",
        errors,
    );
    require(
        matches!(
            evidence.thermal_state.as_str(),
            "nominal" | "fair" | "serious" | "critical"
        ),
        "thermal state is invalid",
        errors,
    );
    require(evidence.ax_trusted.is_set(), "AX trust is required", errors);
    require(
        evidence.actual_sleep_wake.is_set(),
        "actual sleep and wake attestation is required",
        errors,
    );
    require(
        evidence.voiceover_attested.is_set(),
        "human VoiceOver attestation is required",
        errors,
    );
    require(
        evidence.inspector_attested.is_set(),
        "Accessibility Inspector attestation is required",
        errors,
    );
    require(
        evidence.post_close_drain_attested.is_set(),
        "post-close drain attestation is required",
        errors,
    );
    require(
        !evidence.latency_budget_active.is_set(),
        "AX latency budget must remain inactive before A/A calibration",
        errors,
    );
    require(
        !evidence.performance_claim.is_set(),
        "physical AX evidence cannot contain a performance claim",
        errors,
    );
    require(
        (1..=MAX_TREE_NODES).contains(&evidence.tree_node_count),
        format!("tree node count must be between 1 and {MAX_TREE_NODES}"),
        errors,
    );
    require(
        evidence.event_count > 0,
        "event count must be positive",
        errors,
    );
    require(
        evidence.latency_sample_count > 0,
        "latency sample count must be positive",
        errors,
    );
    require(
        evidence.residency_sample_count >= 3,
        "at least three residency samples are required",
        errors,
    );
}

fn validate_artifacts(bundle: &Path, evidence: &AxEvidence, errors: &mut Vec<String>) {
    let artifacts = &evidence.artifacts;
    for artifact in [
        &artifacts.studio_binary,
        &artifacts.harness_binary,
        &artifacts.scenario,
        &artifacts.tree,
        &artifacts.events,
        &artifacts.latency,
        &artifacts.residency,
        &artifacts.stdout,
        &artifacts.stderr,
        &artifacts.inspector_capture,
        &artifacts.human_checklist,
    ] {
        validate_artifact(bundle, artifact, errors);
    }
    if let Some(diff) = &artifacts.repository_diff {
        validate_artifact(bundle, diff, errors);
    }
}

fn validate_contents(bundle: &Path, evidence: &AxEvidence, errors: &mut Vec<String>) {
    let tree = read_text_artifact(bundle, &evidence.artifacts.tree, errors);
    let events = read_text_artifact(bundle, &evidence.artifacts.events, errors);
    let latency = read_text_artifact(bundle, &evidence.artifacts.latency, errors);
    let residency = read_text_artifact(bundle, &evidence.artifacts.residency, errors);
    if let Some(tree) = tree {
        validate_tree(&tree, evidence.tree_node_count, errors);
    }
    if let Some(events) = events {
        validate_events(&events, evidence.event_count, errors);
    }
    if let Some(latency) = latency {
        validate_latency(&latency, evidence.latency_sample_count, errors);
    }
    if let Some(residency) = residency {
        validate_residency(&residency, evidence.residency_sample_count, errors);
    }
}

fn validate_tree(source: &str, expected: usize, errors: &mut Vec<String>) {
    let mut lines = source.lines();
    require(
        lines.next() == Some(TREE_HEADER),
        "AX tree header drifted",
        errors,
    );
    let mut count = 0_usize;
    let mut focused = 0_usize;
    let mut identifiers = BTreeSet::new();
    for line in lines {
        count = count.saturating_add(1);
        let fields = split_fields(line, 5, "AX tree", errors);
        let Some(fields) = fields else { continue };
        let depth = parse_u64(fields[0], "AX tree depth", errors);
        require(
            depth.is_some_and(|value| value <= 64),
            "AX tree depth exceeds 64",
            errors,
        );
        require(
            !fields[1].is_empty() && fields[1].len() <= MAX_IDENTIFIER_BYTES,
            "AX tree identifier is empty or oversized",
            errors,
        );
        require(
            identifiers.insert(fields[1]),
            "AX tree identifiers must be unique",
            errors,
        );
        require(
            fields[2].starts_with("AX"),
            "AX tree role is invalid",
            errors,
        );
        require(
            fields[3].len() <= MAX_LABEL_BYTES,
            "AX tree label is oversized",
            errors,
        );
        match fields[4] {
            "true" => focused = focused.saturating_add(1),
            "false" => {}
            _ => errors.push("AX tree focused value must be true or false".to_owned()),
        }
    }
    require(
        count == expected,
        "AX tree count does not match manifest",
        errors,
    );
    require(
        count <= MAX_TREE_NODES,
        "AX tree exceeds the node ceiling",
        errors,
    );
    require(
        focused == 1,
        "AX tree must contain exactly one focused node",
        errors,
    );
}

fn validate_events(source: &str, expected: usize, errors: &mut Vec<String>) {
    let mut lines = source.lines();
    require(
        lines.next() == Some(EVENT_HEADER),
        "AX event header drifted",
        errors,
    );
    let mut count = 0_usize;
    let mut previous_time = 0_u64;
    let mut observed = BTreeSet::new();
    for line in lines {
        count = count.saturating_add(1);
        let fields = split_fields(line, 5, "AX event", errors);
        let Some(fields) = fields else { continue };
        require(
            parse_u64(fields[0], "AX event sequence", errors) == u64::try_from(count).ok(),
            "AX event sequence must be contiguous",
            errors,
        );
        let time = parse_u64(fields[1], "AX event timestamp", errors);
        require(
            time.is_some_and(|value| value > previous_time),
            "AX event timestamps must increase",
            errors,
        );
        if let Some(time) = time {
            previous_time = time;
        }
        require(
            ALLOWED_EVENTS.contains(&fields[2]),
            "AX event kind is invalid",
            errors,
        );
        require(
            !fields[3].is_empty() && fields[3].len() <= MAX_IDENTIFIER_BYTES,
            "AX event identifier is empty or oversized",
            errors,
        );
        let result = parse_i32(fields[4], "AX event error", errors);
        if result == Some(0) {
            observed.insert(fields[2]);
        }
    }
    require(
        count == expected,
        "AX event count does not match manifest",
        errors,
    );
    for required in REQUIRED_EVENTS {
        require(
            observed.contains(required),
            format!("AX event stream lacks successful {required} evidence"),
            errors,
        );
    }
}

fn validate_latency(source: &str, expected: usize, errors: &mut Vec<String>) {
    let mut lines = source.lines();
    require(
        lines.next() == Some(LATENCY_HEADER),
        "AX latency header drifted",
        errors,
    );
    let mut count = 0_usize;
    for line in lines {
        count = count.saturating_add(1);
        let fields = split_fields(line, 5, "AX latency", errors);
        let Some(fields) = fields else { continue };
        require(
            parse_u64(fields[0], "AX latency sequence", errors) == u64::try_from(count).ok(),
            "AX latency sequence must be contiguous",
            errors,
        );
        require(
            !fields[1].is_empty(),
            "AX latency operation is required",
            errors,
        );
        let start = parse_u64(fields[2], "AX latency start", errors);
        let end = parse_u64(fields[3], "AX latency end", errors);
        require(
            start.zip(end).is_some_and(|(start, end)| end >= start),
            "AX latency interval is reversed",
            errors,
        );
        let _ = parse_i32(fields[4], "AX latency error", errors);
    }
    require(
        count == expected,
        "AX latency count does not match manifest",
        errors,
    );
}

fn validate_residency(source: &str, expected: usize, errors: &mut Vec<String>) {
    let mut lines = source.lines();
    require(
        lines.next() == Some(RESIDENCY_HEADER),
        "AX residency header drifted",
        errors,
    );
    let mut count = 0_usize;
    let mut previous_time = 0_u64;
    for line in lines {
        count = count.saturating_add(1);
        let fields = split_fields(line, 4, "AX residency", errors);
        let Some(fields) = fields else { continue };
        require(
            parse_u64(fields[0], "AX residency sequence", errors) == u64::try_from(count).ok(),
            "AX residency sequence must be contiguous",
            errors,
        );
        let time = parse_u64(fields[1], "AX residency timestamp", errors);
        require(
            time.is_some_and(|value| value > previous_time),
            "AX residency timestamps must increase",
            errors,
        );
        if let Some(time) = time {
            previous_time = time;
        }
        require(
            parse_u64(fields[2], "physical footprint", errors).is_some_and(|value| value > 0),
            "physical footprint must be positive",
            errors,
        );
        require(
            parse_u64(fields[3], "private dirty", errors).is_some_and(|value| value > 0),
            "private dirty memory must be positive",
            errors,
        );
    }
    require(
        count == expected,
        "AX residency count does not match manifest",
        errors,
    );
}

fn validate_artifact(bundle: &Path, artifact: &Artifact, errors: &mut Vec<String>) {
    if !valid_hash(&artifact.sha256, 64) {
        errors.push(format!(
            "artifact {:?} has an invalid SHA-256",
            artifact.path
        ));
        return;
    }
    let relative = Path::new(&artifact.path);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        errors.push(format!(
            "artifact path {:?} escapes the bundle",
            artifact.path
        ));
        return;
    }
    let path = bundle.join(relative);
    let Ok(metadata) = fs::symlink_metadata(&path) else {
        errors.push(format!("cannot inspect artifact {}", path.display()));
        return;
    };
    require(
        metadata.is_file(),
        format!("artifact {} is not a regular file", path.display()),
        errors,
    );
    require(
        !metadata.file_type().is_symlink(),
        format!("artifact {} must not be a symlink", path.display()),
        errors,
    );
    require(
        metadata.len() <= MAX_ARTIFACT_BYTES,
        format!(
            "artifact {} exceeds {MAX_ARTIFACT_BYTES} bytes",
            path.display()
        ),
        errors,
    );
    match hash_file(&path) {
        Ok(actual) if actual == artifact.sha256 => {}
        Ok(actual) => errors.push(format!(
            "artifact {} hash mismatch: expected {}, got {actual}",
            path.display(),
            artifact.sha256
        )),
        Err(error) => errors.push(error),
    }
}

fn read_text_artifact(
    bundle: &Path,
    artifact: &Artifact,
    errors: &mut Vec<String>,
) -> Option<String> {
    let path = bundle.join(&artifact.path);
    match fs::read_to_string(&path) {
        Ok(source) => Some(source),
        Err(error) => {
            errors.push(format!(
                "cannot read text artifact {}: {error}",
                path.display()
            ));
            None
        }
    }
}

fn split_fields<'a>(
    line: &'a str,
    expected: usize,
    kind: &str,
    errors: &mut Vec<String>,
) -> Option<Vec<&'a str>> {
    let fields = line.split(',').collect::<Vec<_>>();
    if fields.len() == expected {
        Some(fields)
    } else {
        errors.push(format!("{kind} row must contain exactly {expected} fields"));
        None
    }
}

fn parse_u64(value: &str, field: &str, errors: &mut Vec<String>) -> Option<u64> {
    match value.parse::<u64>() {
        Ok(value) => Some(value),
        Err(_) => {
            errors.push(format!("{field} must be an unsigned integer"));
            None
        }
    }
}

fn parse_i32(value: &str, field: &str, errors: &mut Vec<String>) -> Option<i32> {
    match value.parse::<i32>() {
        Ok(value) => Some(value),
        Err(_) => {
            errors.push(format!("{field} must be an integer"));
            None
        }
    }
}

fn hash_file(path: &Path) -> Result<String, String> {
    let output = Command::new("shasum")
        .args(["-a", "256"])
        .arg(path)
        .output()
        .map_err(|error| format!("cannot launch shasum for {}: {error}", path.display()))?;
    if !output.status.success() {
        return Err(format!("shasum failed for {}", path.display()));
    }
    let stdout = String::from_utf8_lossy(&output.stdout).to_ascii_lowercase();
    stdout
        .split_whitespace()
        .find(|word| valid_hash(word, 64))
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("shasum returned no SHA-256 for {}", path.display()))
}

fn valid_hash(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn require(condition: bool, message: impl Into<String>, errors: &mut Vec<String>) {
    if !condition {
        errors.push(message.into());
    }
}

#[cfg(test)]
mod tests {
    use super::{validate_events, validate_latency, validate_residency, validate_tree};

    fn assert_error(errors: &[String], expected: &str) {
        assert!(
            errors.iter().any(|error| error.contains(expected)),
            "expected {expected:?} in {errors:#?}"
        );
    }

    #[test]
    fn tree_requires_unique_bounded_identity_and_one_focus() {
        let valid = "depth,identifier,role,label,focused\n0,window,AXGroup,Alpine Studio,false\n1,editor,AXTextArea,Editor,true\n";
        let mut errors = Vec::new();
        validate_tree(valid, 2, &mut errors);
        assert!(errors.is_empty(), "{errors:#?}");

        let invalid = "depth,identifier,role,label,focused\n0,editor,AXGroup,Alpine Studio,true\n1,editor,AXTextArea,Editor,true\n";
        let mut errors = Vec::new();
        validate_tree(invalid, 2, &mut errors);
        assert_error(&errors, "identifiers must be unique");
        assert_error(&errors, "exactly one focused node");
    }

    #[test]
    fn events_require_every_physical_journey_and_monotonic_identity() {
        let kinds = [
            "focus",
            "value",
            "selection",
            "layout",
            "announcement",
            "hidden",
            "shown",
            "minimized",
            "restored",
            "sleep",
            "wake",
            "destroyed",
            "close",
        ];
        let mut valid = String::from("sequence,monotonic_ns,kind,identifier,ax_error\n");
        for (index, kind) in kinds.iter().enumerate() {
            valid.push_str(&format!(
                "{},{},{kind},node-{index},0\n",
                index + 1,
                index + 10
            ));
        }
        let mut errors = Vec::new();
        validate_events(&valid, kinds.len(), &mut errors);
        assert!(errors.is_empty(), "{errors:#?}");

        let invalid = valid.replace("13,22,close", "13,9,close");
        let mut errors = Vec::new();
        validate_events(&invalid, kinds.len(), &mut errors);
        assert_error(&errors, "timestamps must increase");
    }

    #[test]
    fn samples_reject_reversed_time_and_nonpositive_residency() {
        let latency = "sequence,operation,start_ns,end_ns,ax_error\n1,query,20,10,0\n";
        let mut errors = Vec::new();
        validate_latency(latency, 1, &mut errors);
        assert_error(&errors, "interval is reversed");

        let residency = "sequence,monotonic_ns,physical_footprint_bytes,private_dirty_bytes\n1,1,100,50\n2,2,110,55\n3,3,0,60\n";
        let mut errors = Vec::new();
        validate_residency(residency, 3, &mut errors);
        assert_error(&errors, "physical footprint must be positive");
    }
}
