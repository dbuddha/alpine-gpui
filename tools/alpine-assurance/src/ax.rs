//! Validates trusted-machine macOS accessibility qualification evidence.

use serde::{Deserialize, de::DeserializeOwned};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path},
    process::Command,
};

const SCHEMA: &str = "alpine-ax-evidence/v1";
const MAX_MANIFEST_BYTES: u64 = 131_072;
const MAX_BINARY_BYTES: u64 = 536_870_912;
const MAX_SCENARIO_BYTES: u64 = 1_048_576;
const MAX_TREE_BYTES: u64 = 2_097_152;
const MAX_EVENT_BYTES: u64 = 16_777_216;
const MAX_LATENCY_BYTES: u64 = 16_777_216;
const MAX_RESIDENCY_BYTES: u64 = 16_777_216;
const MAX_LOG_BYTES: u64 = 8_388_608;
const MAX_INSPECTOR_BYTES: u64 = 67_108_864;
const MAX_CHECKLIST_BYTES: u64 = 1_048_576;
const MAX_DIFF_BYTES: u64 = 16_777_216;
const MAX_TOTAL_ARTIFACT_BYTES: u64 = 1_610_612_736;
const MAX_TREE_NODES: usize = 271;
const MAX_EVENTS: usize = 65_536;
const MAX_LATENCY_SAMPLES: usize = 65_536;
const MAX_RESIDENCY_SAMPLES: usize = 86_400;
const MAX_RECORD_BYTES: usize = 16_384;
const MAX_IDENTIFIER_BYTES: usize = 256;
const MAX_LABEL_BYTES: usize = 4_096;
const MAX_DETAIL_BYTES: usize = 4_096;
const MAX_ENVIRONMENT_BYTES: usize = 4_096;
const MAX_DIAGNOSTICS: usize = 128;
const REQUIRED_EVENTS: &[&str] = &[
    "launch",
    "focus",
    "action",
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
    "stale-control",
    "close",
];
const REQUIRED_LATENCY_OPERATIONS: &[&str] =
    &["query", "action", "notification", "stale-query", "close"];

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
struct AxEvidence {
    schema: String,
    task_issue: u64,
    fixture_only: EvidenceFlag,
    scenario_id: String,
    attestation_id: String,
    repository_revision: String,
    repository_clean: EvidenceFlag,
    started_unix_ns: u64,
    ended_unix_ns: u64,
    studio_pid: u32,
    harness_pid: u32,
    studio_exit_status: i32,
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

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TreeRow {
    sequence: u64,
    depth: u16,
    identifier: String,
    parent_identifier: Option<String>,
    role: String,
    label: String,
    focused: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EventRow {
    sequence: u64,
    monotonic_ns: u64,
    source: String,
    kind: String,
    identifier: String,
    detail: String,
    ax_error: i32,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LatencyRow {
    sequence: u64,
    operation: String,
    identifier: String,
    start_ns: u64,
    end_ns: u64,
    ax_error: i32,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResidencyRow {
    sequence: u64,
    monotonic_ns: u64,
    phase: String,
    process_alive: bool,
    physical_footprint_bytes: u64,
    private_dirty_bytes: u64,
}

#[derive(Default)]
struct Diagnostics {
    errors: Vec<String>,
    omitted: usize,
}

impl Diagnostics {
    fn require(&mut self, condition: bool, message: impl Into<String>) {
        if !condition {
            self.push(message);
        }
    }

    fn push(&mut self, message: impl Into<String>) {
        if self.errors.len() < MAX_DIAGNOSTICS {
            self.errors.push(message.into());
        } else {
            self.omitted = self.omitted.saturating_add(1);
        }
    }

    fn is_empty(&self) -> bool {
        self.errors.is_empty() && self.omitted == 0
    }

    fn finish(mut self) -> Vec<String> {
        self.errors.sort();
        self.errors.dedup();
        if self.omitted > 0 {
            self.errors.push(format!(
                "diagnostic ceiling reached; {} additional errors omitted",
                self.omitted
            ));
        }
        self.errors
    }
}

#[derive(Default)]
struct TreeSummary {
    identifiers: BTreeSet<String>,
}

pub(crate) fn run(command: &str, bundle: &Path) -> Result<String, Vec<String>> {
    let fixture_command = command == "validate-ax-fixture";
    let evidence = load_manifest(bundle)?;
    let mut diagnostics = Diagnostics::default();
    validate_identity(&evidence, fixture_command, &mut diagnostics);
    validate_artifacts(bundle, &evidence, &mut diagnostics);
    if diagnostics.is_empty() {
        validate_contents(bundle, &evidence, &mut diagnostics);
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }

    if fixture_command {
        return Ok(format!(
            "validated task #273 AX fixture at revision {} with {} nodes, {} events, {} latency samples, and {} residency samples; no physical or performance claim",
            evidence.repository_revision,
            evidence.tree_node_count,
            evidence.event_count,
            evidence.latency_sample_count,
            evidence.residency_sample_count
        ));
    }
    if command == "validate-ax-evidence" {
        return Ok(format!(
            "validated task #273 AX evidence structure at revision {} with {} nodes, {} events, {} latency samples, and {} residency samples; human and physical review remain required and no performance claim is admitted",
            evidence.repository_revision,
            evidence.tree_node_count,
            evidence.event_count,
            evidence.latency_sample_count,
            evidence.residency_sample_count
        ));
    }
    Ok(format!(
        "# Alpine physical accessibility evidence report\n\n- Revision: `{}`\n- Scenario: `{}`\n- Attestation: `{}`\n- Hardware: {} ({})\n- macOS build: {}\n- AX tree nodes: {}\n- Observed events: {}\n- Latency samples: {} (descriptive only)\n- Residency samples: {} (finite capture only)\n- Structural validation: passed\n- Human and physical review: still required\n- Performance threshold: inactive\n- Performance claim: none\n",
        evidence.repository_revision,
        evidence.scenario_id,
        evidence.attestation_id,
        evidence.hardware_model,
        evidence.architecture,
        evidence.macos_build,
        evidence.tree_node_count,
        evidence.event_count,
        evidence.latency_sample_count,
        evidence.residency_sample_count
    ))
}

fn load_manifest(bundle: &Path) -> Result<AxEvidence, Vec<String>> {
    let mut diagnostics = Diagnostics::default();
    match fs::symlink_metadata(bundle) {
        Ok(metadata) => {
            diagnostics.require(metadata.is_dir(), "AX bundle must be a directory");
            diagnostics.require(
                !metadata.file_type().is_symlink(),
                "AX bundle must not be a symbolic link",
            );
        }
        Err(error) => diagnostics.push(format!(
            "cannot inspect AX bundle {}: {error}",
            bundle.display()
        )),
    }
    let manifest = Path::new("manifest.toml");
    let source = inspect_relative_file(
        bundle,
        manifest,
        MAX_MANIFEST_BYTES,
        "AX manifest",
        &mut diagnostics,
    )
    .and_then(|_| {
        read_bounded_text(
            &bundle.join(manifest),
            MAX_MANIFEST_BYTES,
            "AX manifest",
            &mut diagnostics,
        )
    });
    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }
    let Some(source) = source else {
        diagnostics.push("AX manifest was not readable");
        return Err(diagnostics.finish());
    };
    toml::from_str(&source).map_err(|error| {
        vec![format!(
            "cannot parse {}: {error}",
            bundle.join(manifest).display()
        )]
    })
}

fn validate_identity(evidence: &AxEvidence, fixture_command: bool, diagnostics: &mut Diagnostics) {
    validate_capture_identity(evidence, fixture_command, diagnostics);
    validate_environment(evidence, diagnostics);
    validate_attestations(evidence, diagnostics);
    validate_sample_counts(evidence, diagnostics);
}

fn validate_capture_identity(
    evidence: &AxEvidence,
    fixture_command: bool,
    diagnostics: &mut Diagnostics,
) {
    diagnostics.require(evidence.schema == SCHEMA, "AX schema must be exact");
    diagnostics.require(
        evidence.task_issue == 273,
        "AX evidence must bind task #273",
    );
    diagnostics.require(
        evidence.fixture_only.is_set() == fixture_command,
        if fixture_command {
            "fixture validation requires fixture_only = true"
        } else {
            "physical AX commands reject fixture-only bundles"
        },
    );
    diagnostics.require(
        valid_slug(&evidence.scenario_id),
        "AX scenario identifier is invalid",
    );
    diagnostics.require(
        valid_slug(&evidence.attestation_id),
        "AX attestation identifier is invalid",
    );
    diagnostics.require(
        valid_hash(&evidence.repository_revision, 40),
        "repository revision must be a full lowercase Git hash",
    );
    diagnostics.require(
        evidence.repository_clean.is_set() == evidence.artifacts.repository_diff.is_none(),
        "clean state must omit a diff and dirty state must retain one",
    );
    diagnostics.require(
        evidence.started_unix_ns > 0 && evidence.ended_unix_ns > evidence.started_unix_ns,
        "capture times must be positive and ordered",
    );
    diagnostics.require(evidence.studio_pid != 0, "Studio PID must be nonzero");
    diagnostics.require(evidence.harness_pid != 0, "harness PID must be nonzero");
    diagnostics.require(
        evidence.studio_pid != evidence.harness_pid,
        "Studio and harness PIDs must differ",
    );
    diagnostics.require(
        evidence.studio_exit_status == 0,
        "Studio must terminate with exit status zero",
    );
}

fn validate_environment(evidence: &AxEvidence, diagnostics: &mut Diagnostics) {
    for (name, value) in [
        ("macOS build", evidence.macos_build.as_str()),
        ("SDK build", evidence.sdk_build.as_str()),
        ("rustc version", evidence.rustc_version.as_str()),
        ("hardware model", evidence.hardware_model.as_str()),
        ("locale", evidence.locale.as_str()),
        ("input source", evidence.input_source.as_str()),
        ("display description", evidence.display_description.as_str()),
    ] {
        diagnostics.require(
            bounded_text(value, MAX_ENVIRONMENT_BYTES),
            format!("{name} is empty, invalid, or oversized"),
        );
    }
    diagnostics.require(
        evidence.architecture == "arm64",
        "physical qualification requires arm64",
    );
    diagnostics.require(
        matches!(evidence.power_source.as_str(), "ac" | "battery"),
        "power source must be ac or battery",
    );
    diagnostics.require(
        matches!(
            evidence.thermal_state.as_str(),
            "nominal" | "fair" | "serious" | "critical"
        ),
        "thermal state is invalid",
    );
}

fn validate_attestations(evidence: &AxEvidence, diagnostics: &mut Diagnostics) {
    diagnostics.require(evidence.ax_trusted.is_set(), "AX trust is required");
    diagnostics.require(
        evidence.actual_sleep_wake.is_set(),
        "actual sleep and wake attestation is required",
    );
    diagnostics.require(
        evidence.voiceover_attested.is_set(),
        "human VoiceOver attestation is required",
    );
    diagnostics.require(
        evidence.inspector_attested.is_set(),
        "Accessibility Inspector attestation is required",
    );
    diagnostics.require(
        evidence.post_close_drain_attested.is_set(),
        "post-close drain attestation is required",
    );
    diagnostics.require(
        !evidence.latency_budget_active.is_set(),
        "AX latency budget must remain inactive before A/A calibration",
    );
    diagnostics.require(
        !evidence.performance_claim.is_set(),
        "physical AX evidence cannot contain a performance claim",
    );
}

fn validate_sample_counts(evidence: &AxEvidence, diagnostics: &mut Diagnostics) {
    diagnostics.require(
        (3..=MAX_TREE_NODES).contains(&evidence.tree_node_count),
        format!("tree node count must be between 3 and {MAX_TREE_NODES}"),
    );
    diagnostics.require(
        (REQUIRED_EVENTS.len()..=MAX_EVENTS).contains(&evidence.event_count),
        format!(
            "event count must be between {} and {MAX_EVENTS}",
            REQUIRED_EVENTS.len()
        ),
    );
    diagnostics.require(
        (REQUIRED_LATENCY_OPERATIONS.len()..=MAX_LATENCY_SAMPLES)
            .contains(&evidence.latency_sample_count),
        format!(
            "latency sample count must be between {} and {MAX_LATENCY_SAMPLES}",
            REQUIRED_LATENCY_OPERATIONS.len()
        ),
    );
    diagnostics.require(
        (4..=MAX_RESIDENCY_SAMPLES).contains(&evidence.residency_sample_count),
        format!("residency sample count must be between 4 and {MAX_RESIDENCY_SAMPLES}"),
    );
}

fn validate_artifacts(bundle: &Path, evidence: &AxEvidence, diagnostics: &mut Diagnostics) {
    let artifacts = &evidence.artifacts;
    let required = [
        ("Studio binary", &artifacts.studio_binary, MAX_BINARY_BYTES),
        (
            "harness binary",
            &artifacts.harness_binary,
            MAX_BINARY_BYTES,
        ),
        ("scenario", &artifacts.scenario, MAX_SCENARIO_BYTES),
        ("AX tree", &artifacts.tree, MAX_TREE_BYTES),
        ("AX events", &artifacts.events, MAX_EVENT_BYTES),
        ("AX latency", &artifacts.latency, MAX_LATENCY_BYTES),
        ("AX residency", &artifacts.residency, MAX_RESIDENCY_BYTES),
        ("Studio stdout", &artifacts.stdout, MAX_LOG_BYTES),
        ("Studio stderr", &artifacts.stderr, MAX_LOG_BYTES),
        (
            "Inspector capture",
            &artifacts.inspector_capture,
            MAX_INSPECTOR_BYTES,
        ),
        (
            "human checklist",
            &artifacts.human_checklist,
            MAX_CHECKLIST_BYTES,
        ),
    ];
    let mut paths = BTreeSet::new();
    let mut total = 0_u64;
    for (name, artifact, maximum) in required {
        diagnostics.require(
            paths.insert(artifact.path.as_str()),
            format!("AX artifact path {:?} is duplicated", artifact.path),
        );
        if let Some(bytes) = validate_artifact(bundle, artifact, maximum, name, diagnostics) {
            total = total.saturating_add(bytes);
        }
    }
    if let Some(diff) = &artifacts.repository_diff {
        diagnostics.require(
            paths.insert(diff.path.as_str()),
            format!("AX artifact path {:?} is duplicated", diff.path),
        );
        if let Some(bytes) =
            validate_artifact(bundle, diff, MAX_DIFF_BYTES, "repository diff", diagnostics)
        {
            total = total.saturating_add(bytes);
        }
    }
    diagnostics.require(
        total <= MAX_TOTAL_ARTIFACT_BYTES,
        format!("AX artifact bundle exceeds {MAX_TOTAL_ARTIFACT_BYTES} total bytes"),
    );
}

fn validate_contents(bundle: &Path, evidence: &AxEvidence, diagnostics: &mut Diagnostics) {
    let tree = read_text_artifact(
        bundle,
        &evidence.artifacts.tree,
        MAX_TREE_BYTES,
        "AX tree",
        diagnostics,
    );
    let events = read_text_artifact(
        bundle,
        &evidence.artifacts.events,
        MAX_EVENT_BYTES,
        "AX events",
        diagnostics,
    );
    let latency = read_text_artifact(
        bundle,
        &evidence.artifacts.latency,
        MAX_LATENCY_BYTES,
        "AX latency",
        diagnostics,
    );
    let residency = read_text_artifact(
        bundle,
        &evidence.artifacts.residency,
        MAX_RESIDENCY_BYTES,
        "AX residency",
        diagnostics,
    );
    let scenario = read_text_artifact(
        bundle,
        &evidence.artifacts.scenario,
        MAX_SCENARIO_BYTES,
        "scenario",
        diagnostics,
    );
    let checklist = read_text_artifact(
        bundle,
        &evidence.artifacts.human_checklist,
        MAX_CHECKLIST_BYTES,
        "human checklist",
        diagnostics,
    );

    if let Some(scenario) = scenario {
        validate_attestation_text(
            &scenario,
            evidence.fixture_only.is_set(),
            "scenario",
            &["Open", "Edit", "Save", "Close"],
            diagnostics,
        );
    }
    if let Some(checklist) = checklist {
        validate_attestation_text(
            &checklist,
            evidence.fixture_only.is_set(),
            "human checklist",
            &[
                "VoiceOver: passed",
                "Accessibility Inspector: passed",
                "Post-close drain: passed",
            ],
            diagnostics,
        );
    }

    let summary = tree.map(|source| validate_tree(&source, evidence.tree_node_count, diagnostics));
    if let (Some(events), Some(summary)) = (events, summary.as_ref()) {
        validate_events(&events, evidence.event_count, summary, diagnostics);
    }
    if let (Some(latency), Some(summary)) = (latency, summary.as_ref()) {
        validate_latency(
            &latency,
            evidence.latency_sample_count,
            summary,
            diagnostics,
        );
    }
    if let Some(residency) = residency {
        validate_residency(&residency, evidence.residency_sample_count, diagnostics);
    }
}

fn validate_attestation_text(
    source: &str,
    fixture_only: bool,
    kind: &str,
    required: &[&str],
    diagnostics: &mut Diagnostics,
) {
    if fixture_only {
        diagnostics.require(
            source.contains("FIXTURE ONLY"),
            format!("fixture {kind} must declare FIXTURE ONLY"),
        );
    } else {
        diagnostics.require(
            !source.contains("FIXTURE ONLY"),
            format!("physical {kind} cannot be fixture-only"),
        );
    }
    for phrase in required {
        diagnostics.require(
            source.contains(phrase),
            format!("{kind} lacks required entry {phrase:?}"),
        );
    }
}

fn validate_tree(source: &str, expected: usize, diagnostics: &mut Diagnostics) -> TreeSummary {
    let mut summary = TreeSummary::default();
    let mut depths = BTreeMap::<String, u16>::new();
    let mut count = 0_usize;
    let mut roots = 0_usize;
    let mut focused = 0_usize;
    let mut has_application = false;
    let mut has_window = false;
    let mut has_editor = false;

    for (index, line) in source.lines().enumerate() {
        if index >= MAX_TREE_NODES {
            diagnostics.push("AX tree exceeds the node ceiling");
            break;
        }
        count = count.saturating_add(1);
        let Some(row) = parse_record::<TreeRow>(line, count, "AX tree", diagnostics) else {
            continue;
        };
        diagnostics.require(
            row.sequence == u64::try_from(count).unwrap_or(u64::MAX),
            "AX tree sequence must be contiguous",
        );
        diagnostics.require(
            valid_identifier(&row.identifier),
            "AX tree identifier is empty, invalid, or oversized",
        );
        diagnostics.require(
            row.role.starts_with("AX") && bounded_text(&row.role, MAX_IDENTIFIER_BYTES),
            "AX tree role is invalid",
        );
        diagnostics.require(
            row.label.len() <= MAX_LABEL_BYTES && !row.label.contains('\0'),
            "AX tree label is invalid or oversized",
        );
        if row.focused {
            focused = focused.saturating_add(1);
        }
        has_application |= row.role == "AXApplication";
        has_window |= row.role == "AXWindow";
        has_editor |= row.role == "AXTextArea";

        match row.parent_identifier.as_deref() {
            None => {
                roots = roots.saturating_add(1);
                diagnostics.require(row.depth == 0, "AX tree root depth must be zero");
                diagnostics.require(
                    row.role == "AXApplication",
                    "AX tree root must have AXApplication role",
                );
            }
            Some(parent) => {
                diagnostics.require(
                    valid_identifier(parent),
                    "AX tree parent identifier is invalid",
                );
                match depths.get(parent) {
                    Some(parent_depth) => diagnostics.require(
                        parent_depth
                            .checked_add(1)
                            .is_some_and(|depth| depth == row.depth),
                        "AX tree child depth must follow its preceding parent",
                    ),
                    None => diagnostics
                        .push("AX tree parent must precede its child and identify a retained node"),
                }
            }
        }
        if summary.identifiers.contains(&row.identifier) {
            diagnostics.push("AX tree identifiers must be unique");
        } else {
            depths.insert(row.identifier.clone(), row.depth);
            summary.identifiers.insert(row.identifier);
        }
    }
    diagnostics.require(count == expected, "AX tree count does not match manifest");
    diagnostics.require(roots == 1, "AX tree must contain exactly one root");
    diagnostics.require(
        focused == 1,
        "AX tree must contain exactly one focused node",
    );
    diagnostics.require(
        has_application && has_window && has_editor,
        "AX tree must contain application, window, and text-area roles",
    );
    summary
}

fn validate_events(
    source: &str,
    expected: usize,
    tree: &TreeSummary,
    diagnostics: &mut Diagnostics,
) {
    let mut count = 0_usize;
    let mut previous_time = 0_u64;
    let mut observed = BTreeSet::<String>::new();
    let mut positions = BTreeMap::<String, usize>::new();
    let mut destroyed_identifier = None::<String>;
    let mut stale_identifier = None::<String>;

    for (index, line) in source.lines().enumerate() {
        if index >= MAX_EVENTS {
            diagnostics.push("AX event stream exceeds the event ceiling");
            break;
        }
        count = count.saturating_add(1);
        let Some(row) = parse_record::<EventRow>(line, count, "AX event", diagnostics) else {
            continue;
        };
        diagnostics.require(
            row.sequence == u64::try_from(count).unwrap_or(u64::MAX),
            "AX event sequence must be contiguous",
        );
        diagnostics.require(
            row.monotonic_ns > previous_time,
            "AX event timestamps must increase",
        );
        previous_time = previous_time.max(row.monotonic_ns);
        diagnostics.require(
            tree.identifiers.contains(&row.identifier),
            "AX event identifier must reference the retained tree",
        );
        diagnostics.require(
            bounded_text(&row.detail, MAX_DETAIL_BYTES),
            "AX event detail is empty, invalid, or oversized",
        );
        if validate_event_contract(&row, diagnostics) {
            observed.insert(row.kind.clone());
            positions.entry(row.kind.clone()).or_insert(count);
            if row.kind == "destroyed" {
                destroyed_identifier = Some(row.identifier.clone());
            } else if row.kind == "stale-control" {
                stale_identifier = Some(row.identifier.clone());
            }
        }
    }
    diagnostics.require(count == expected, "AX event count does not match manifest");
    for required in REQUIRED_EVENTS {
        diagnostics.require(
            observed.contains(*required),
            format!("AX event stream lacks valid {required} evidence"),
        );
    }
    diagnostics.require(
        positions.get("launch") == Some(&1),
        "process launch must be the first AX event",
    );
    diagnostics.require(
        positions.get("close") == Some(&count),
        "process close must be the final AX event",
    );
    require_order(&positions, "hidden", "shown", diagnostics);
    require_order(&positions, "minimized", "restored", diagnostics);
    require_order(&positions, "sleep", "wake", diagnostics);
    require_order(&positions, "destroyed", "stale-control", diagnostics);
    require_order(&positions, "stale-control", "close", diagnostics);
    diagnostics.require(
        destroyed_identifier.is_some() && destroyed_identifier == stale_identifier,
        "stale-element control must query the element reported destroyed",
    );
}

fn validate_event_contract(row: &EventRow, diagnostics: &mut Diagnostics) -> bool {
    let exact = match row.kind.as_str() {
        "launch" => row.source == "process" && row.detail == "process-start" && row.ax_error == 0,
        "focus" => {
            row.source == "ax-observer"
                && row.detail == "AXFocusedUIElementChanged"
                && row.ax_error == 0
        }
        "action" => {
            row.source == "ax-action"
                && matches!(row.detail.as_str(), "AXPress" | "AXConfirm" | "AXShowMenu")
                && row.ax_error == 0
        }
        "value" => {
            row.source == "ax-observer" && row.detail == "AXValueChanged" && row.ax_error == 0
        }
        "selection" => {
            row.source == "ax-observer"
                && row.detail == "AXSelectedTextChanged"
                && row.ax_error == 0
        }
        "layout" => {
            row.source == "ax-observer" && row.detail == "AXLayoutChanged" && row.ax_error == 0
        }
        "announcement" => {
            row.source == "ax-observer"
                && row.detail == "AXAnnouncementRequested"
                && row.ax_error == 0
        }
        "hidden" => {
            row.source == "workspace"
                && row.detail == "NSWorkspaceDidHideApplicationNotification"
                && row.ax_error == 0
        }
        "shown" => {
            row.source == "workspace"
                && row.detail == "NSWorkspaceDidUnhideApplicationNotification"
                && row.ax_error == 0
        }
        "minimized" => {
            row.source == "ax-observer" && row.detail == "AXWindowMiniaturized" && row.ax_error == 0
        }
        "restored" => {
            row.source == "ax-observer"
                && row.detail == "AXWindowDeminiaturized"
                && row.ax_error == 0
        }
        "sleep" => {
            row.source == "workspace"
                && row.detail == "NSWorkspaceWillSleepNotification"
                && row.ax_error == 0
        }
        "wake" => {
            row.source == "workspace"
                && row.detail == "NSWorkspaceDidWakeNotification"
                && row.ax_error == 0
        }
        "destroyed" => {
            row.source == "ax-observer" && row.detail == "AXUIElementDestroyed" && row.ax_error == 0
        }
        "stale-control" => {
            row.source == "ax-query"
                && row.detail == "kAXErrorInvalidUIElement"
                && row.ax_error != 0
        }
        "close" => row.source == "process" && row.detail == "exit:0" && row.ax_error == 0,
        _ => false,
    };
    diagnostics.require(
        exact,
        format!(
            "AX event {:?} has an invalid source, detail, or result contract",
            row.kind
        ),
    );
    exact
}

fn validate_latency(
    source: &str,
    expected: usize,
    tree: &TreeSummary,
    diagnostics: &mut Diagnostics,
) {
    let mut count = 0_usize;
    let mut observed = BTreeSet::<String>::new();
    for (index, line) in source.lines().enumerate() {
        if index >= MAX_LATENCY_SAMPLES {
            diagnostics.push("AX latency stream exceeds the sample ceiling");
            break;
        }
        count = count.saturating_add(1);
        let Some(row) = parse_record::<LatencyRow>(line, count, "AX latency", diagnostics) else {
            continue;
        };
        diagnostics.require(
            row.sequence == u64::try_from(count).unwrap_or(u64::MAX),
            "AX latency sequence must be contiguous",
        );
        diagnostics.require(
            REQUIRED_LATENCY_OPERATIONS.contains(&row.operation.as_str()),
            "AX latency operation is invalid",
        );
        diagnostics.require(
            tree.identifiers.contains(&row.identifier),
            "AX latency identifier must reference the retained tree",
        );
        diagnostics.require(
            row.start_ns > 0 && row.end_ns >= row.start_ns,
            "AX latency interval is invalid or reversed",
        );
        let result_valid = if row.operation == "stale-query" {
            row.ax_error != 0
        } else {
            row.ax_error == 0
        };
        diagnostics.require(
            result_valid,
            "AX latency result does not match its operation",
        );
        if result_valid && REQUIRED_LATENCY_OPERATIONS.contains(&row.operation.as_str()) {
            observed.insert(row.operation);
        }
    }
    diagnostics.require(
        count == expected,
        "AX latency count does not match manifest",
    );
    for required in REQUIRED_LATENCY_OPERATIONS {
        diagnostics.require(
            observed.contains(*required),
            format!("AX latency stream lacks {required} evidence"),
        );
    }
}

fn validate_residency(source: &str, expected: usize, diagnostics: &mut Diagnostics) {
    let mut count = 0_usize;
    let mut previous_time = 0_u64;
    let mut startup_count = 0_usize;
    let mut steady_count = 0_usize;
    let mut post_close_count = 0_usize;
    let mut first_phase = None::<String>;
    let mut last_phase = None::<String>;

    for (index, line) in source.lines().enumerate() {
        if index >= MAX_RESIDENCY_SAMPLES {
            diagnostics.push("AX residency stream exceeds the sample ceiling");
            break;
        }
        count = count.saturating_add(1);
        let Some(row) = parse_record::<ResidencyRow>(line, count, "AX residency", diagnostics)
        else {
            continue;
        };
        diagnostics.require(
            row.sequence == u64::try_from(count).unwrap_or(u64::MAX),
            "AX residency sequence must be contiguous",
        );
        diagnostics.require(
            row.monotonic_ns > previous_time,
            "AX residency timestamps must increase",
        );
        previous_time = previous_time.max(row.monotonic_ns);
        first_phase.get_or_insert_with(|| row.phase.clone());
        last_phase = Some(row.phase.clone());
        match row.phase.as_str() {
            "startup" => {
                startup_count = startup_count.saturating_add(1);
                diagnostics.require(
                    row.process_alive
                        && row.physical_footprint_bytes > 0
                        && row.private_dirty_bytes > 0,
                    "live startup residency must be positive",
                );
            }
            "steady" => {
                steady_count = steady_count.saturating_add(1);
                diagnostics.require(
                    row.process_alive
                        && row.physical_footprint_bytes > 0
                        && row.private_dirty_bytes > 0,
                    "live steady residency must be positive",
                );
            }
            "post-close" => {
                post_close_count = post_close_count.saturating_add(1);
                diagnostics.require(
                    !row.process_alive
                        && row.physical_footprint_bytes == 0
                        && row.private_dirty_bytes == 0,
                    "post-close residency must record a dead process and zero retained bytes",
                );
            }
            _ => diagnostics.push("AX residency phase is invalid"),
        }
    }
    diagnostics.require(
        count == expected,
        "AX residency count does not match manifest",
    );
    diagnostics.require(
        first_phase.as_deref() == Some("startup"),
        "AX residency must begin with startup",
    );
    diagnostics.require(
        startup_count == 1,
        "AX residency requires one startup sample",
    );
    diagnostics.require(
        steady_count >= 2,
        "AX residency requires at least two finite steady samples",
    );
    diagnostics.require(
        post_close_count == 1 && last_phase.as_deref() == Some("post-close"),
        "AX residency requires one final post-close sample",
    );
}

fn require_order(
    positions: &BTreeMap<String, usize>,
    first: &str,
    second: &str,
    diagnostics: &mut Diagnostics,
) {
    diagnostics.require(
        positions
            .get(first)
            .zip(positions.get(second))
            .is_some_and(|(first, second)| first < second),
        format!("AX event {first} must precede {second}"),
    );
}

fn parse_record<T: DeserializeOwned>(
    line: &str,
    line_number: usize,
    kind: &str,
    diagnostics: &mut Diagnostics,
) -> Option<T> {
    if line.is_empty() || line.len() > MAX_RECORD_BYTES {
        diagnostics.push(format!(
            "{kind} line {line_number} is empty or exceeds {MAX_RECORD_BYTES} bytes"
        ));
        return None;
    }
    match serde_json::from_str(line) {
        Ok(record) => Some(record),
        Err(error) => {
            diagnostics.push(format!(
                "{kind} line {line_number} is not exact JSON: {error}"
            ));
            None
        }
    }
}

fn validate_artifact(
    bundle: &Path,
    artifact: &Artifact,
    maximum: u64,
    name: &str,
    diagnostics: &mut Diagnostics,
) -> Option<u64> {
    if !valid_hash(&artifact.sha256, 64) {
        diagnostics.push(format!("{name} {:?} has an invalid SHA-256", artifact.path));
        return None;
    }
    let relative = Path::new(&artifact.path);
    let metadata = inspect_relative_file(bundle, relative, maximum, name, diagnostics)?;
    let path = bundle.join(relative);
    match hash_file(&path) {
        Ok(actual) if actual == artifact.sha256 => Some(metadata.len()),
        Ok(actual) => {
            diagnostics.push(format!(
                "{name} {} hash mismatch: expected {}, got {actual}",
                path.display(),
                artifact.sha256
            ));
            None
        }
        Err(error) => {
            diagnostics.push(error);
            None
        }
    }
}

fn inspect_relative_file(
    bundle: &Path,
    relative: &Path,
    maximum: u64,
    name: &str,
    diagnostics: &mut Diagnostics,
) -> Option<fs::Metadata> {
    if relative.as_os_str().is_empty()
        || relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        diagnostics.push(format!(
            "{name} path {} escapes the AX bundle",
            relative.display()
        ));
        return None;
    }
    let mut current = bundle.to_path_buf();
    let mut metadata = None;
    for component in relative.components() {
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(current_metadata) => {
                if current_metadata.file_type().is_symlink() {
                    diagnostics.push(format!(
                        "{name} path cannot traverse a symbolic link: {}",
                        current.display()
                    ));
                    return None;
                }
                metadata = Some(current_metadata);
            }
            Err(error) => {
                diagnostics.push(format!(
                    "cannot inspect {name} {}: {error}",
                    current.display()
                ));
                return None;
            }
        }
    }
    let Some(metadata) = metadata else {
        diagnostics.push(format!("cannot resolve {name} path"));
        return None;
    };
    diagnostics.require(
        metadata.is_file(),
        format!("{name} {} is not a regular file", current.display()),
    );
    diagnostics.require(
        metadata.len() > 0,
        format!("{name} {} must not be empty", current.display()),
    );
    diagnostics.require(
        metadata.len() <= maximum,
        format!("{name} {} exceeds {maximum} bytes", current.display()),
    );
    if metadata.is_file() && metadata.len() > 0 && metadata.len() <= maximum {
        Some(metadata)
    } else {
        None
    }
}

fn read_text_artifact(
    bundle: &Path,
    artifact: &Artifact,
    maximum: u64,
    name: &str,
    diagnostics: &mut Diagnostics,
) -> Option<String> {
    read_bounded_text(&bundle.join(&artifact.path), maximum, name, diagnostics)
}

fn read_bounded_text(
    path: &Path,
    maximum: u64,
    name: &str,
    diagnostics: &mut Diagnostics,
) -> Option<String> {
    match fs::read(path) {
        Ok(bytes) => {
            if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > maximum {
                diagnostics.push(format!("{name} {} exceeds {maximum} bytes", path.display()));
                return None;
            }
            match String::from_utf8(bytes) {
                Ok(source) => Some(source),
                Err(error) => {
                    diagnostics.push(format!("{name} {} is not UTF-8: {error}", path.display()));
                    None
                }
            }
        }
        Err(error) => {
            diagnostics.push(format!("cannot read {name} {}: {error}", path.display()));
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

fn valid_slug(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_IDENTIFIER_BYTES
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_' | b'.')
        })
}

fn valid_identifier(value: &str) -> bool {
    bounded_text(value, MAX_IDENTIFIER_BYTES)
        && value
            .chars()
            .all(|character| !character.is_control() && character != ',')
}

fn bounded_text(value: &str, maximum: usize) -> bool {
    !value.trim().is_empty() && value.len() <= maximum && !value.contains('\0')
}

#[cfg(test)]
mod tests {
    use super::{
        Artifact, Diagnostics, MAX_DIAGNOSTICS, MAX_TREE_BYTES, TreeSummary, validate_events,
        validate_latency, validate_residency, validate_tree,
    };
    use std::{fmt::Write as _, fs, path::Path};

    type EventFixtureRow = (&'static str, &'static str, &'static str, &'static str, i32);

    fn event_rows() -> Vec<EventFixtureRow> {
        vec![
            ("process", "launch", "application", "process-start", 0),
            (
                "ax-observer",
                "focus",
                "editor",
                "AXFocusedUIElementChanged",
                0,
            ),
            ("ax-action", "action", "editor", "AXPress", 0),
            ("ax-observer", "value", "editor", "AXValueChanged", 0),
            (
                "ax-observer",
                "selection",
                "editor",
                "AXSelectedTextChanged",
                0,
            ),
            ("ax-observer", "layout", "window", "AXLayoutChanged", 0),
            (
                "ax-observer",
                "announcement",
                "application",
                "AXAnnouncementRequested",
                0,
            ),
            (
                "workspace",
                "hidden",
                "application",
                "NSWorkspaceDidHideApplicationNotification",
                0,
            ),
            (
                "workspace",
                "shown",
                "application",
                "NSWorkspaceDidUnhideApplicationNotification",
                0,
            ),
            (
                "ax-observer",
                "minimized",
                "window",
                "AXWindowMiniaturized",
                0,
            ),
            (
                "ax-observer",
                "restored",
                "window",
                "AXWindowDeminiaturized",
                0,
            ),
            (
                "workspace",
                "sleep",
                "application",
                "NSWorkspaceWillSleepNotification",
                0,
            ),
            (
                "workspace",
                "wake",
                "application",
                "NSWorkspaceDidWakeNotification",
                0,
            ),
            (
                "ax-observer",
                "destroyed",
                "editor",
                "AXUIElementDestroyed",
                0,
            ),
            (
                "ax-query",
                "stale-control",
                "editor",
                "kAXErrorInvalidUIElement",
                -25211,
            ),
            ("process", "close", "application", "exit:0", 0),
        ]
    }

    fn assert_error(errors: &[String], expected: &str) {
        assert!(
            errors.iter().any(|error| error.contains(expected)),
            "expected {expected:?} in {errors:#?}"
        );
    }

    fn tree() -> (&'static str, TreeSummary) {
        let source = concat!(
            r#"{"sequence":1,"depth":0,"identifier":"application","parent_identifier":null,"role":"AXApplication","label":"Alpine Studio","focused":false}"#,
            "\n",
            r#"{"sequence":2,"depth":1,"identifier":"window","parent_identifier":"application","role":"AXWindow","label":"Alpine, Studio\\nWindow","focused":false}"#,
            "\n",
            r#"{"sequence":3,"depth":2,"identifier":"editor","parent_identifier":"window","role":"AXTextArea","label":"Editor","focused":true}"#,
            "\n"
        );
        let mut diagnostics = Diagnostics::default();
        let summary = validate_tree(source, 3, &mut diagnostics);
        assert!(diagnostics.finish().is_empty());
        (source, summary)
    }

    #[test]
    fn tree_requires_unique_bounded_hierarchy_and_one_focus() {
        let (valid, _) = tree();
        let invalid = valid
            .replace("\"identifier\":\"editor\"", "\"identifier\":\"window\"")
            .replace(
                "\"parent_identifier\":\"window\"",
                "\"parent_identifier\":\"missing\"",
            )
            .replace("\"focused\":false", "\"focused\":true");
        let mut diagnostics = Diagnostics::default();
        let _ = validate_tree(&invalid, 3, &mut diagnostics);
        let errors = diagnostics.finish();
        assert_error(&errors, "identifiers must be unique");
        assert_error(&errors, "exactly one focused node");
        assert_error(&errors, "precede its child");
    }

    #[test]
    fn events_bind_sources_actions_sleep_wake_and_stale_control() {
        let (_, summary) = tree();
        let rows = event_rows();
        let mut source = String::new();
        for (index, (event_source, kind, identifier, detail, error)) in rows.iter().enumerate() {
            let _ = writeln!(
                source,
                "{{\"sequence\":{},\"monotonic_ns\":{},\"source\":\"{}\",\"kind\":\"{}\",\"identifier\":\"{}\",\"detail\":\"{}\",\"ax_error\":{}}}",
                index + 1,
                index + 100,
                event_source,
                kind,
                identifier,
                detail,
                error
            );
        }
        let mut diagnostics = Diagnostics::default();
        validate_events(&source, rows.len(), &summary, &mut diagnostics);
        assert!(diagnostics.finish().is_empty());

        let invalid = source
            .replace(
                "\"source\":\"workspace\",\"kind\":\"sleep\"",
                "\"source\":\"ax-observer\",\"kind\":\"sleep\"",
            )
            .replace(
                "\"identifier\":\"editor\",\"detail\":\"kAXErrorInvalidUIElement\"",
                "\"identifier\":\"window\",\"detail\":\"kAXErrorInvalidUIElement\"",
            );
        let mut diagnostics = Diagnostics::default();
        validate_events(&invalid, rows.len(), &summary, &mut diagnostics);
        let errors = diagnostics.finish();
        assert_error(&errors, "invalid source");
        assert_error(&errors, "element reported destroyed");
    }

    #[test]
    fn samples_are_bounded_descriptive_and_require_post_close_zero() {
        let (_, summary) = tree();
        let latency = concat!(
            r#"{"sequence":1,"operation":"query","identifier":"editor","start_ns":1,"end_ns":2,"ax_error":0}"#,
            "\n",
            r#"{"sequence":2,"operation":"action","identifier":"editor","start_ns":3,"end_ns":4,"ax_error":0}"#,
            "\n",
            r#"{"sequence":3,"operation":"notification","identifier":"editor","start_ns":5,"end_ns":6,"ax_error":0}"#,
            "\n",
            r#"{"sequence":4,"operation":"stale-query","identifier":"editor","start_ns":7,"end_ns":8,"ax_error":-25211}"#,
            "\n",
            r#"{"sequence":5,"operation":"close","identifier":"application","start_ns":9,"end_ns":10,"ax_error":0}"#,
            "\n"
        );
        let mut diagnostics = Diagnostics::default();
        validate_latency(latency, 5, &summary, &mut diagnostics);
        assert!(diagnostics.finish().is_empty());

        let invalid_latency = latency.replace("\"end_ns\":2", "\"end_ns\":0");
        let mut diagnostics = Diagnostics::default();
        validate_latency(&invalid_latency, 5, &summary, &mut diagnostics);
        assert_error(&diagnostics.finish(), "interval is invalid or reversed");

        let residency = concat!(
            r#"{"sequence":1,"monotonic_ns":1,"phase":"startup","process_alive":true,"physical_footprint_bytes":100,"private_dirty_bytes":50}"#,
            "\n",
            r#"{"sequence":2,"monotonic_ns":2,"phase":"steady","process_alive":true,"physical_footprint_bytes":110,"private_dirty_bytes":55}"#,
            "\n",
            r#"{"sequence":3,"monotonic_ns":3,"phase":"steady","process_alive":true,"physical_footprint_bytes":105,"private_dirty_bytes":54}"#,
            "\n",
            r#"{"sequence":4,"monotonic_ns":4,"phase":"post-close","process_alive":false,"physical_footprint_bytes":0,"private_dirty_bytes":0}"#,
            "\n"
        );
        let mut diagnostics = Diagnostics::default();
        validate_residency(residency, 4, &mut diagnostics);
        assert!(diagnostics.finish().is_empty());

        let invalid_residency = residency.replace(
            "\"process_alive\":false,\"physical_footprint_bytes\":0",
            "\"process_alive\":true,\"physical_footprint_bytes\":1",
        );
        let mut diagnostics = Diagnostics::default();
        validate_residency(&invalid_residency, 4, &mut diagnostics);
        assert_error(
            &diagnostics.finish(),
            "dead process and zero retained bytes",
        );
    }

    #[cfg(unix)]
    #[test]
    fn artifact_paths_reject_symlinked_parent_components() -> Result<(), String> {
        use std::os::unix::fs::symlink;

        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .unwrap_or_else(|| Path::new("."));
        let directory = root.join(format!("target/ax-symlink-test-{}", std::process::id()));
        let target = directory.join("target");
        let link = directory.join("link");
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&target).map_err(|error| error.to_string())?;
        fs::write(target.join("tree.jsonl"), "fixture\n").map_err(|error| error.to_string())?;
        symlink(&target, &link).map_err(|error| error.to_string())?;
        let artifact = Artifact {
            path: "link/tree.jsonl".to_owned(),
            sha256: "0".repeat(64),
        };
        let mut diagnostics = Diagnostics::default();
        let _ = super::validate_artifact(
            &directory,
            &artifact,
            MAX_TREE_BYTES,
            "AX tree",
            &mut diagnostics,
        );
        fs::remove_dir_all(&directory).map_err(|error| error.to_string())?;
        assert_error(&diagnostics.finish(), "cannot traverse a symbolic link");
        Ok(())
    }

    #[test]
    fn diagnostics_are_bounded_under_invalid_input() {
        let source = "not-json\n".repeat(MAX_DIAGNOSTICS + 32);
        let mut diagnostics = Diagnostics::default();
        let _ = validate_tree(&source, 3, &mut diagnostics);
        let errors = diagnostics.finish();
        assert!(errors.len() <= MAX_DIAGNOSTICS + 1);
        assert_error(&errors, "additional errors omitted");
    }
}

#[cfg(test)]
mod mutation_controls {
    use super::{
        AxEvidence, Diagnostics, EventRow, MAX_IDENTIFIER_BYTES, MAX_LABEL_BYTES, MAX_RECORD_BYTES,
        TreeSummary, bounded_text, inspect_relative_file, parse_record, run, valid_hash,
        valid_identifier, valid_slug, validate_attestation_text, validate_event_contract,
        validate_events, validate_identity, validate_latency, validate_residency, validate_tree,
    };
    use std::{
        collections::BTreeMap,
        fmt::Write as _,
        fs,
        path::{Path, PathBuf},
    };

    type EventFixtureRow = (&'static str, &'static str, &'static str, &'static str, i32);

    fn fixture_bundle() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assurance/ax/v1/fixture")
    }

    fn load_fixture() -> Result<AxEvidence, String> {
        super::load_manifest(&fixture_bundle()).map_err(|errors| errors.join("; "))
    }

    fn assert_error(errors: &[String], expected: &str) {
        assert!(
            errors.iter().any(|error| error.contains(expected)),
            "expected {expected:?} in {errors:#?}"
        );
    }

    fn identity_errors(evidence: &AxEvidence) -> Vec<String> {
        let mut diagnostics = Diagnostics::default();
        validate_identity(evidence, true, &mut diagnostics);
        diagnostics.finish()
    }

    fn tree_summary(source: &str) -> TreeSummary {
        let mut diagnostics = Diagnostics::default();
        let summary = validate_tree(source, 3, &mut diagnostics);
        assert!(diagnostics.finish().is_empty());
        summary
    }

    fn tree_source() -> &'static str {
        concat!(
            r#"{"sequence":1,"depth":0,"identifier":"application","parent_identifier":null,"role":"AXApplication","label":"Alpine Studio","focused":false}"#,
            "\n",
            r#"{"sequence":2,"depth":1,"identifier":"window","parent_identifier":"application","role":"AXWindow","label":"Window","focused":false}"#,
            "\n",
            r#"{"sequence":3,"depth":2,"identifier":"editor","parent_identifier":"window","role":"AXTextArea","label":"Editor","focused":true}"#,
            "\n"
        )
    }

    fn assert_tree_error(source: &str, expected: &str) {
        let mut diagnostics = Diagnostics::default();
        let _ = validate_tree(source, 3, &mut diagnostics);
        assert_error(&diagnostics.finish(), expected);
    }

    fn event_rows() -> Vec<EventFixtureRow> {
        vec![
            ("process", "launch", "application", "process-start", 0),
            (
                "ax-observer",
                "focus",
                "editor",
                "AXFocusedUIElementChanged",
                0,
            ),
            ("ax-action", "action", "editor", "AXPress", 0),
            ("ax-observer", "value", "editor", "AXValueChanged", 0),
            (
                "ax-observer",
                "selection",
                "editor",
                "AXSelectedTextChanged",
                0,
            ),
            ("ax-observer", "layout", "window", "AXLayoutChanged", 0),
            (
                "ax-observer",
                "announcement",
                "application",
                "AXAnnouncementRequested",
                0,
            ),
            (
                "workspace",
                "hidden",
                "application",
                "NSWorkspaceDidHideApplicationNotification",
                0,
            ),
            (
                "workspace",
                "shown",
                "application",
                "NSWorkspaceDidUnhideApplicationNotification",
                0,
            ),
            (
                "ax-observer",
                "minimized",
                "window",
                "AXWindowMiniaturized",
                0,
            ),
            (
                "ax-observer",
                "restored",
                "window",
                "AXWindowDeminiaturized",
                0,
            ),
            (
                "workspace",
                "sleep",
                "application",
                "NSWorkspaceWillSleepNotification",
                0,
            ),
            (
                "workspace",
                "wake",
                "application",
                "NSWorkspaceDidWakeNotification",
                0,
            ),
            (
                "ax-observer",
                "destroyed",
                "editor",
                "AXUIElementDestroyed",
                0,
            ),
            (
                "ax-query",
                "stale-control",
                "editor",
                "kAXErrorInvalidUIElement",
                -25211,
            ),
            ("process", "close", "application", "exit:0", 0),
        ]
    }

    fn event_record(row: &EventFixtureRow, sequence: u64) -> EventRow {
        EventRow {
            sequence,
            monotonic_ns: sequence.saturating_add(100),
            source: row.0.to_owned(),
            kind: row.1.to_owned(),
            identifier: row.2.to_owned(),
            detail: row.3.to_owned(),
            ax_error: row.4,
        }
    }

    fn event_source() -> String {
        let mut source = String::new();
        for (index, row) in event_rows().iter().enumerate() {
            let record = event_record(row, u64::try_from(index + 1).unwrap_or(u64::MAX));
            let _ = writeln!(
                source,
                "{{\"sequence\":{},\"monotonic_ns\":{},\"source\":\"{}\",\"kind\":\"{}\",\"identifier\":\"{}\",\"detail\":\"{}\",\"ax_error\":{}}}",
                record.sequence,
                record.monotonic_ns,
                record.source,
                record.kind,
                record.identifier,
                record.detail,
                record.ax_error
            );
        }
        source
    }

    fn latency_source() -> &'static str {
        concat!(
            r#"{"sequence":1,"operation":"query","identifier":"editor","start_ns":1,"end_ns":2,"ax_error":0}"#,
            "\n",
            r#"{"sequence":2,"operation":"action","identifier":"editor","start_ns":3,"end_ns":4,"ax_error":0}"#,
            "\n",
            r#"{"sequence":3,"operation":"notification","identifier":"editor","start_ns":5,"end_ns":6,"ax_error":0}"#,
            "\n",
            r#"{"sequence":4,"operation":"stale-query","identifier":"editor","start_ns":7,"end_ns":8,"ax_error":-25211}"#,
            "\n",
            r#"{"sequence":5,"operation":"close","identifier":"application","start_ns":9,"end_ns":10,"ax_error":0}"#,
            "\n"
        )
    }

    fn residency_source() -> &'static str {
        concat!(
            r#"{"sequence":1,"monotonic_ns":1,"phase":"startup","process_alive":true,"physical_footprint_bytes":100,"private_dirty_bytes":50}"#,
            "\n",
            r#"{"sequence":2,"monotonic_ns":2,"phase":"steady","process_alive":true,"physical_footprint_bytes":110,"private_dirty_bytes":55}"#,
            "\n",
            r#"{"sequence":3,"monotonic_ns":3,"phase":"steady","process_alive":true,"physical_footprint_bytes":105,"private_dirty_bytes":54}"#,
            "\n",
            r#"{"sequence":4,"monotonic_ns":4,"phase":"post-close","process_alive":false,"physical_footprint_bytes":0,"private_dirty_bytes":0}"#,
            "\n"
        )
    }

    fn assert_residency_error(source: &str, expected: &str) {
        let mut diagnostics = Diagnostics::default();
        validate_residency(source, source.lines().count(), &mut diagnostics);
        assert_error(&diagnostics.finish(), expected);
    }

    fn test_directory(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
            "../../target/ax-mutation-{name}-{}",
            std::process::id()
        ))
    }

    fn copy_fixture(destination: &Path) -> Result<(), String> {
        let _ = fs::remove_dir_all(destination);
        fs::create_dir_all(destination).map_err(|error| error.to_string())?;
        for entry in fs::read_dir(fixture_bundle()).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            fs::copy(entry.path(), destination.join(entry.file_name()))
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    fn rewrite_attestation(
        directory: &Path,
        name: &str,
        manifest: &mut String,
    ) -> Result<(), String> {
        let path = directory.join(name);
        let old_hash = super::hash_file(&path)?;
        let source = fs::read_to_string(&path).map_err(|error| error.to_string())?;
        fs::write(&path, source.replace("FIXTURE ONLY", "PHYSICAL CAPTURE"))
            .map_err(|error| error.to_string())?;
        let new_hash = super::hash_file(&path)?;
        *manifest = manifest.replace(&old_hash, &new_hash);
        Ok(())
    }

    fn physical_bundle(name: &str) -> Result<PathBuf, String> {
        let directory = test_directory(name);
        copy_fixture(&directory)?;
        let manifest_path = directory.join("manifest.toml");
        let mut manifest = fs::read_to_string(&manifest_path).map_err(|error| error.to_string())?;
        manifest = manifest.replace("fixture_only = true", "fixture_only = false");
        rewrite_attestation(&directory, "scenario.md", &mut manifest)?;
        rewrite_attestation(&directory, "human-checklist.md", &mut manifest)?;
        fs::write(manifest_path, manifest).map_err(|error| error.to_string())?;
        Ok(directory)
    }

    #[test]
    fn physical_commands_have_distinct_outputs() -> Result<(), String> {
        let directory = physical_bundle("commands")?;
        let validation =
            run("validate-ax-evidence", &directory).map_err(|errors| errors.join("; "))?;
        let report = run("ax-evidence-report", &directory).map_err(|errors| errors.join("; "))?;
        let cleanup = fs::remove_dir_all(&directory).map_err(|error| error.to_string());
        assert!(validation.contains("validated task #273 AX evidence structure"));
        assert!(!validation.contains("# Alpine physical accessibility evidence report"));
        assert!(report.starts_with("# Alpine physical accessibility evidence report"));
        cleanup
    }

    #[test]
    fn every_identity_stage_is_mandatory() -> Result<(), String> {
        let valid = load_fixture()?;

        let mut zero_start = valid.clone();
        zero_start.started_unix_ns = 0;
        assert_error(&identity_errors(&zero_start), "capture times");

        let mut unordered = valid.clone();
        unordered.ended_unix_ns = unordered.started_unix_ns;
        assert_error(&identity_errors(&unordered), "capture times");

        let mut environment = valid.clone();
        environment.architecture = "x86_64".to_owned();
        assert_error(&identity_errors(&environment), "requires arm64");

        let mut attestation = valid.clone();
        attestation.ax_trusted.0 = false;
        assert_error(&identity_errors(&attestation), "AX trust is required");

        let mut counts = valid;
        counts.tree_node_count = 2;
        assert_error(&identity_errors(&counts), "tree node count");
        Ok(())
    }

    #[test]
    fn content_and_attestation_validation_cannot_be_skipped() -> Result<(), String> {
        let mut evidence = load_fixture()?;
        evidence.artifacts.scenario.path = "stdout.txt".to_owned();
        let mut diagnostics = Diagnostics::default();
        super::validate_contents(&fixture_bundle(), &evidence, &mut diagnostics);
        assert_error(
            &diagnostics.finish(),
            "fixture scenario must declare FIXTURE ONLY",
        );

        let mut diagnostics = Diagnostics::default();
        validate_attestation_text(
            "Open Edit Save Close",
            true,
            "scenario",
            &["Open", "Edit", "Save", "Close"],
            &mut diagnostics,
        );
        assert_error(&diagnostics.finish(), "must declare FIXTURE ONLY");

        let mut diagnostics = Diagnostics::default();
        validate_attestation_text(
            "FIXTURE ONLY Open Edit Save Close",
            false,
            "scenario",
            &["Open", "Edit", "Save", "Close"],
            &mut diagnostics,
        );
        assert_error(&diagnostics.finish(), "cannot be fixture-only");
        Ok(())
    }

    #[test]
    fn tree_fields_and_required_roles_are_independently_validated() {
        let source = tree_source();
        assert_tree_error(
            &source.replace("\"role\":\"AXWindow\"", "\"role\":\"Window\""),
            "role is invalid",
        );
        let oversized_role = format!("AX{}", "r".repeat(MAX_IDENTIFIER_BYTES));
        assert_tree_error(
            &source.replace(
                "\"role\":\"AXWindow\"",
                &format!("\"role\":\"{oversized_role}\""),
            ),
            "role is invalid",
        );
        assert_tree_error(
            &source.replace("\"label\":\"Editor\"", "\"label\":\"\\u0000\""),
            "label is invalid",
        );
        let oversized_label = "x".repeat(MAX_LABEL_BYTES + 1);
        assert_tree_error(
            &source.replace(
                "\"label\":\"Editor\"",
                &format!("\"label\":\"{oversized_label}\""),
            ),
            "label is invalid",
        );
        for role in ["AXApplication", "AXWindow", "AXTextArea"] {
            assert_tree_error(
                &source.replace(&format!("\"role\":\"{role}\""), "\"role\":\"AXGroup\""),
                "application, window, and text-area roles",
            );
        }
    }

    #[test]
    fn every_event_contract_field_is_independently_required() {
        for (index, row) in event_rows().iter().enumerate() {
            let valid = event_record(row, u64::try_from(index + 1).unwrap_or(u64::MAX));
            let mut diagnostics = Diagnostics::default();
            assert!(validate_event_contract(&valid, &mut diagnostics));
            assert!(diagnostics.finish().is_empty());

            let mut wrong_source = valid.clone();
            wrong_source.source = "wrong-source".to_owned();
            let mut diagnostics = Diagnostics::default();
            assert!(!validate_event_contract(&wrong_source, &mut diagnostics));

            let mut wrong_detail = valid.clone();
            wrong_detail.detail = "wrong-detail".to_owned();
            let mut diagnostics = Diagnostics::default();
            assert!(!validate_event_contract(&wrong_detail, &mut diagnostics));

            let mut wrong_result = valid;
            wrong_result.ax_error = if wrong_result.kind == "stale-control" {
                0
            } else {
                -1
            };
            let mut diagnostics = Diagnostics::default();
            assert!(!validate_event_contract(&wrong_result, &mut diagnostics));
        }
    }

    #[test]
    fn event_time_and_order_boundaries_are_strict() {
        let source = event_source().replace("\"monotonic_ns\":102", "\"monotonic_ns\":101");
        let summary = tree_summary(tree_source());
        let mut diagnostics = Diagnostics::default();
        validate_events(&source, event_rows().len(), &summary, &mut diagnostics);
        assert_error(&diagnostics.finish(), "timestamps must increase");

        let positions = BTreeMap::from([("hidden".to_owned(), 3), ("shown".to_owned(), 3)]);
        let mut diagnostics = Diagnostics::default();
        super::require_order(&positions, "hidden", "shown", &mut diagnostics);
        assert_error(&diagnostics.finish(), "hidden must precede shown");
    }

    #[test]
    fn latency_intervals_and_results_are_independently_required() {
        let summary = tree_summary(tree_source());
        let zero_start = latency_source().replace("\"start_ns\":1", "\"start_ns\":0");
        let mut diagnostics = Diagnostics::default();
        validate_latency(&zero_start, 5, &summary, &mut diagnostics);
        assert_error(&diagnostics.finish(), "interval is invalid or reversed");

        let bad_result = latency_source().replace(
            "\"operation\":\"action\",\"identifier\":\"editor\",\"start_ns\":3,\"end_ns\":4,\"ax_error\":0",
            "\"operation\":\"action\",\"identifier\":\"editor\",\"start_ns\":3,\"end_ns\":4,\"ax_error\":1",
        );
        let mut diagnostics = Diagnostics::default();
        validate_latency(&bad_result, 5, &summary, &mut diagnostics);
        let errors = diagnostics.finish();
        assert_error(&errors, "result does not match");
        assert_error(&errors, "lacks action evidence");
    }

    #[test]
    fn residency_fields_and_final_phase_are_independently_required() {
        let cases = [
            (
                "\"phase\":\"startup\",\"process_alive\":true",
                "\"phase\":\"startup\",\"process_alive\":false",
                "live startup",
            ),
            (
                "\"phase\":\"startup\",\"process_alive\":true,\"physical_footprint_bytes\":100",
                "\"phase\":\"startup\",\"process_alive\":true,\"physical_footprint_bytes\":0",
                "live startup",
            ),
            (
                "\"physical_footprint_bytes\":100,\"private_dirty_bytes\":50",
                "\"physical_footprint_bytes\":100,\"private_dirty_bytes\":0",
                "live startup",
            ),
            (
                "\"phase\":\"steady\",\"process_alive\":true",
                "\"phase\":\"steady\",\"process_alive\":false",
                "live steady",
            ),
            (
                "\"physical_footprint_bytes\":110",
                "\"physical_footprint_bytes\":0",
                "live steady",
            ),
            (
                "\"private_dirty_bytes\":55",
                "\"private_dirty_bytes\":0",
                "live steady",
            ),
            (
                "\"phase\":\"post-close\",\"process_alive\":false",
                "\"phase\":\"post-close\",\"process_alive\":true",
                "dead process",
            ),
            (
                "\"physical_footprint_bytes\":0,\"private_dirty_bytes\":0",
                "\"physical_footprint_bytes\":1,\"private_dirty_bytes\":0",
                "dead process",
            ),
            (
                "\"private_dirty_bytes\":0}",
                "\"private_dirty_bytes\":1}",
                "dead process",
            ),
        ];
        for (from, to, expected) in cases {
            assert_residency_error(&residency_source().replacen(from, to, 1), expected);
        }
        assert_residency_error(
            &residency_source().replace("\"monotonic_ns\":2", "\"monotonic_ns\":1"),
            "timestamps must increase",
        );
        let duplicate_close = residency_source().replace(
            "\"sequence\":3,\"monotonic_ns\":3,\"phase\":\"steady\",\"process_alive\":true,\"physical_footprint_bytes\":105,\"private_dirty_bytes\":54",
            "\"sequence\":3,\"monotonic_ns\":3,\"phase\":\"post-close\",\"process_alive\":false,\"physical_footprint_bytes\":0,\"private_dirty_bytes\":0",
        );
        assert_residency_error(&duplicate_close, "one final post-close sample");
        let nonfinal_close = residency_source()
            .replace("\"phase\":\"steady\"", "\"phase\":\"post-close\"")
            .replacen("\"phase\":\"post-close\"", "\"phase\":\"steady\"", 1);
        assert_residency_error(&nonfinal_close, "one final post-close sample");
    }

    #[test]
    fn record_and_text_predicate_boundaries_are_exact() {
        let mut diagnostics = Diagnostics::default();
        let empty = parse_record::<String>("", 1, "record", &mut diagnostics);
        assert!(empty.is_none());
        assert_error(&diagnostics.finish(), "empty or exceeds");

        let exact = format!("\"{}\"", "a".repeat(MAX_RECORD_BYTES - 2));
        let mut diagnostics = Diagnostics::default();
        assert!(parse_record::<String>(&exact, 1, "record", &mut diagnostics).is_some());
        assert!(diagnostics.finish().is_empty());

        let oversized = format!("\"{}\"", "a".repeat(MAX_RECORD_BYTES - 1));
        let mut diagnostics = Diagnostics::default();
        assert!(parse_record::<String>(&oversized, 1, "record", &mut diagnostics).is_none());
        assert_error(&diagnostics.finish(), "empty or exceeds");

        assert!(valid_hash(&"a".repeat(40), 40));
        assert!(!valid_hash(&"a".repeat(39), 40));
        assert!(!valid_hash(&"g".repeat(40), 40));
        assert!(valid_slug("valid-slug_1.0"));
        assert!(!valid_slug(""));
        assert!(!valid_slug(&"a".repeat(MAX_IDENTIFIER_BYTES + 1)));
        assert!(!valid_slug("UPPER"));
        assert!(valid_identifier("valid identifier"));
        assert!(!valid_identifier(""));
        assert!(!valid_identifier("has,comma"));
        assert!(!valid_identifier("has\ncontrol"));
        assert!(bounded_text("value", 5));
        assert!(!bounded_text(" ", 5));
        assert!(!bounded_text("value!", 5));
        assert!(!bounded_text("nul\0", 5));
    }

    #[test]
    fn file_paths_and_size_boundaries_fail_closed() -> Result<(), String> {
        let directory = test_directory("files");
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
        fs::create_dir(directory.join("subdir")).map_err(|error| error.to_string())?;
        fs::write(directory.join("exact"), b"1234").map_err(|error| error.to_string())?;
        fs::write(directory.join("empty"), b"").map_err(|error| error.to_string())?;
        fs::write(directory.join("oversized"), b"12345").map_err(|error| error.to_string())?;

        for path in [Path::new(""), Path::new("../escape"), directory.as_path()] {
            let mut diagnostics = Diagnostics::default();
            assert!(inspect_relative_file(&directory, path, 4, "file", &mut diagnostics).is_none());
            assert_error(&diagnostics.finish(), "escapes the AX bundle");
        }

        let mut diagnostics = Diagnostics::default();
        assert!(
            inspect_relative_file(&directory, Path::new("exact"), 4, "file", &mut diagnostics)
                .is_some()
        );
        assert!(diagnostics.finish().is_empty());
        for (path, expected) in [
            ("empty", "must not be empty"),
            ("oversized", "exceeds 4 bytes"),
        ] {
            let mut diagnostics = Diagnostics::default();
            assert!(
                inspect_relative_file(&directory, Path::new(path), 4, "file", &mut diagnostics)
                    .is_none()
            );
            assert_error(&diagnostics.finish(), expected);
        }
        let mut diagnostics = Diagnostics::default();
        assert!(
            inspect_relative_file(&directory, Path::new("subdir"), 4, "file", &mut diagnostics,)
                .is_none()
        );
        assert_error(&diagnostics.finish(), "not a regular file");

        let mut diagnostics = Diagnostics::default();
        assert!(
            super::read_bounded_text(&directory.join("exact"), 4, "file", &mut diagnostics)
                .is_some()
        );
        assert!(diagnostics.finish().is_empty());
        let mut diagnostics = Diagnostics::default();
        assert!(
            super::read_bounded_text(&directory.join("oversized"), 4, "file", &mut diagnostics)
                .is_none()
        );
        assert_error(&diagnostics.finish(), "exceeds 4 bytes");
        fs::remove_dir_all(&directory).map_err(|error| error.to_string())
    }
}
