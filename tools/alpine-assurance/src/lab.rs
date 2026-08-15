//! Validates accepted evidence composed from the isolated GPL Zed lab.

use crate::calibration::valid_utc_timestamp;
use serde::Deserialize;
use std::{collections::BTreeSet, fmt::Write as _, fs, path::Path};

const SCHEMA: &str = "alpine-zed-lab-evidence/v1";
const PIXEL_FORMAT: &str = "compact-bgra8-premultiplied";
const OFFLINE_SHADER_MODE: &str = "offline-metallib";
const SUPPORTING_SHADER_MODE: &str = "runtime-source-unqualified";

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct LabEvidence {
    schema: String,
    id: String,
    capability_issue: u64,
    requirement_issues: Vec<u64>,
    task_issue: u64,
    comparison_level: String,
    lab_repository: String,
    lab_license: String,
    source_influence: String,
    lab_revision: String,
    zed_revision: String,
    alpine_revision: String,
    scene_trace_sha256: String,
    workload_hash: String,
    patch_series_sha256: String,
    pixel_width: u32,
    pixel_height: u32,
    pixel_format: String,
    pixel_sha256: String,
    timing_performed: bool,
    performance_qualified: bool,
    assumptions: Vec<String>,
    exclusions: Vec<String>,
    hosted: HostedEvidence,
    physical: PhysicalEvidence,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct HostedEvidence {
    run_id: u64,
    run_url: String,
    artifact_id: u64,
    artifact_name: String,
    artifact_url: String,
    archive_sha256: String,
    qualification_sha256: String,
    artifact_expires_at_utc: String,
    retention_days: u16,
    shader_mode: String,
    os_version: String,
    architecture: String,
    cpu_oracle_sha256: String,
    alpine_metal_sha256: String,
    gpui_metal_sha256: String,
    direct_metal_performed: bool,
    coverage_performed: bool,
    coverage_tool_version: String,
    coverage_lines_covered: u64,
    coverage_lines_total: u64,
    coverage_functions_covered: u64,
    coverage_functions_total: u64,
    mutation_performed: bool,
    mutation_tool_version: String,
    mutants_total: u64,
    mutants_caught: u64,
    mutants_unviable: u64,
    mutants_missed: u64,
    mutants_timeout: u64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct PhysicalEvidence {
    observed_at_utc: String,
    hardware_id: String,
    os_version: String,
    architecture: String,
    qualification_sha256: String,
    shader_mode: String,
    cpu_oracle_sha256: String,
    alpine_metal_sha256: String,
    gpui_metal_sha256: String,
    direct_metal_performed: bool,
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
        self.errors
    }
}

pub(crate) fn run(command: &str, manifest: &Path) -> Result<String, Vec<String>> {
    let source = fs::read_to_string(manifest)
        .map_err(|error| vec![format!("cannot read {}: {error}", manifest.display())])?;
    let evidence: LabEvidence = toml::from_str(&source)
        .map_err(|error| vec![format!("cannot parse {}: {error}", manifest.display())])?;
    let errors = validate(&evidence);
    if !errors.is_empty() {
        return Err(errors);
    }

    match command {
        "validate-zed-lab-evidence" => Ok(format!(
            "validated Zed lab evidence {} for task #{} with hosted offline GPUI and physical Direct Metal equivalence",
            evidence.id, evidence.task_issue
        )),
        "zed-lab-evidence-report" => Ok(render_report(&evidence)),
        other => Err(vec![format!(
            "unsupported Zed lab evidence command {other:?}"
        )]),
    }
}

fn validate(evidence: &LabEvidence) -> Vec<String> {
    let mut diagnostics = Diagnostics::default();
    validate_record_identity(evidence, &mut diagnostics);
    validate_pixel_identity(evidence, &mut diagnostics);
    validate_hosted(evidence, &mut diagnostics);
    validate_physical(evidence, &mut diagnostics);
    diagnostics.finish()
}

fn validate_record_identity(evidence: &LabEvidence, diagnostics: &mut Diagnostics) {
    diagnostics.require(
        evidence.schema == SCHEMA,
        format!("schema must be {SCHEMA}"),
    );
    diagnostics.require(
        valid_slug(&evidence.id),
        "evidence id must be a lowercase slug",
    );
    diagnostics.require(
        evidence.capability_issue > 0,
        "capability issue number must be positive",
    );
    diagnostics.require(
        evidence.task_issue > 0,
        "task issue number must be positive",
    );
    let requirements = evidence
        .requirement_issues
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    diagnostics.require(
        !requirements.is_empty(),
        "requirement issues cannot be empty",
    );
    diagnostics.require(
        requirements.len() == evidence.requirement_issues.len(),
        "requirement issue numbers must be unique",
    );
    diagnostics.require(
        requirements.iter().all(|issue| *issue > 0),
        "requirement issue numbers must be positive",
    );
    diagnostics.require(
        evidence.comparison_level == "renderer-only",
        "Zed lab evidence currently supports only renderer-only comparison",
    );
    diagnostics.require(
        evidence.lab_repository == "dbuddha/alpine-zed-lab",
        "lab repository must be the isolated dbuddha/alpine-zed-lab boundary",
    );
    diagnostics.require(
        evidence.lab_license == "GPL-3.0-or-later",
        "lab evidence license must remain GPL-3.0-or-later",
    );
    diagnostics.require(
        evidence.source_influence == "source-level-lab-only",
        "source-level influence must remain isolated to the GPL lab",
    );
    for (name, value) in [
        ("lab", evidence.lab_revision.as_str()),
        ("Zed", evidence.zed_revision.as_str()),
        ("Alpine", evidence.alpine_revision.as_str()),
    ] {
        diagnostics.require(
            valid_git_sha(value),
            format!("{name} revision must be a full lowercase Git SHA"),
        );
    }
}

fn validate_pixel_identity(evidence: &LabEvidence, diagnostics: &mut Diagnostics) {
    for (name, value) in [
        ("scene trace", evidence.scene_trace_sha256.as_str()),
        ("workload", evidence.workload_hash.as_str()),
        ("patch series", evidence.patch_series_sha256.as_str()),
        ("pixel", evidence.pixel_sha256.as_str()),
        ("hosted archive", evidence.hosted.archive_sha256.as_str()),
        (
            "hosted qualification",
            evidence.hosted.qualification_sha256.as_str(),
        ),
        (
            "physical qualification",
            evidence.physical.qualification_sha256.as_str(),
        ),
    ] {
        diagnostics.require(
            valid_sha256(value),
            format!("{name} identity must be a SHA-256 value"),
        );
    }
    diagnostics.require(evidence.pixel_width > 0, "pixel width must be nonzero");
    diagnostics.require(evidence.pixel_height > 0, "pixel height must be nonzero");
    diagnostics.require(
        evidence.pixel_format == PIXEL_FORMAT,
        format!("pixel format must be {PIXEL_FORMAT}"),
    );
    diagnostics.require(
        !evidence.timing_performed,
        "renderer equivalence evidence cannot contain timing",
    );
    diagnostics.require(
        !evidence.performance_qualified,
        "renderer equivalence evidence cannot contain a performance claim",
    );
    diagnostics.require(
        !evidence.assumptions.is_empty(),
        "evidence must disclose assumptions",
    );
    diagnostics.require(
        !evidence.exclusions.is_empty(),
        "evidence must disclose exclusions",
    );
}

fn validate_hosted(evidence: &LabEvidence, diagnostics: &mut Diagnostics) {
    validate_hosted_identity(evidence, diagnostics);
    validate_hosted_coverage(&evidence.hosted, diagnostics);
    validate_hosted_mutation(&evidence.hosted, diagnostics);
}

fn validate_hosted_identity(evidence: &LabEvidence, diagnostics: &mut Diagnostics) {
    let hosted = &evidence.hosted;
    diagnostics.require(hosted.run_id > 0, "hosted run id must be positive");
    diagnostics.require(
        hosted.artifact_id > 0,
        "hosted artifact id must be positive",
    );
    diagnostics.require(
        hosted.run_url
            == format!(
                "https://github.com/{}/actions/runs/{}",
                evidence.lab_repository, hosted.run_id
            ),
        "hosted run URL must match the lab repository and run id",
    );
    diagnostics.require(
        hosted.artifact_name == format!("gpui-oracle-{}", evidence.lab_revision),
        "hosted artifact name must bind the exact lab revision",
    );
    diagnostics.require(
        hosted.artifact_url
            == format!(
                "https://api.github.com/repos/{}/actions/artifacts/{}/zip",
                evidence.lab_repository, hosted.artifact_id
            ),
        "hosted artifact URL must match the lab repository and artifact id",
    );
    diagnostics.require(
        valid_utc_timestamp(&hosted.artifact_expires_at_utc),
        "hosted artifact expiry must be a canonical UTC timestamp",
    );
    diagnostics.require(
        hosted.retention_days == 90,
        "hosted qualification artifacts must be retained for exactly 90 days",
    );
    diagnostics.require(
        hosted.shader_mode == OFFLINE_SHADER_MODE,
        "hosted GPUI evidence must use the offline Metal library",
    );
    diagnostics.require(
        hosted.architecture == "arm64",
        "hosted evidence architecture must be arm64",
    );
    diagnostics.require(
        !hosted.os_version.trim().is_empty(),
        "hosted evidence must identify macOS",
    );
    diagnostics.require(
        hosted.cpu_oracle_sha256 == evidence.pixel_sha256,
        "hosted CPU oracle hash must match the accepted pixel identity",
    );
    diagnostics.require(
        hosted.gpui_metal_sha256 == evidence.pixel_sha256,
        "hosted offline GPUI Metal hash must match the accepted pixel identity",
    );
    diagnostics.require(
        hosted.alpine_metal_sha256 == "not-run",
        "hosted virtual Metal evidence must mark Alpine Direct Metal not-run",
    );
    diagnostics.require(
        !hosted.direct_metal_performed,
        "hosted virtual Metal evidence must not claim Alpine Direct Metal execution",
    );
}

fn validate_hosted_coverage(hosted: &HostedEvidence, diagnostics: &mut Diagnostics) {
    diagnostics.require(
        hosted.coverage_performed,
        "hosted adapter coverage must run",
    );
    diagnostics.require(
        hosted.coverage_tool_version.starts_with("cargo-llvm-cov-"),
        "hosted adapter coverage tool must be cargo-llvm-cov",
    );
    diagnostics.require(
        percentage_at_least(
            hosted.coverage_lines_covered,
            hosted.coverage_lines_total,
            95,
        ),
        "hosted adapter line coverage must meet 95 percent",
    );
    diagnostics.require(
        percentage_at_least(
            hosted.coverage_functions_covered,
            hosted.coverage_functions_total,
            90,
        ),
        "hosted adapter function coverage must meet 90 percent",
    );
}

fn validate_hosted_mutation(hosted: &HostedEvidence, diagnostics: &mut Diagnostics) {
    let classified_mutants = hosted
        .mutants_caught
        .checked_add(hosted.mutants_unviable)
        .and_then(|value| value.checked_add(hosted.mutants_missed))
        .and_then(|value| value.checked_add(hosted.mutants_timeout));
    diagnostics.require(
        hosted.mutation_performed,
        "hosted adapter mutation must run",
    );
    diagnostics.require(
        hosted.mutation_tool_version.starts_with("cargo-mutants-"),
        "hosted adapter mutation tool must be cargo-mutants",
    );
    diagnostics.require(
        classified_mutants == Some(hosted.mutants_total),
        "hosted adapter mutation evidence must classify every mutant",
    );
    diagnostics.require(
        hosted.mutants_total > 0,
        "hosted adapter mutation evidence must contain mutants",
    );
    diagnostics.require(
        hosted.mutants_missed == 0,
        "hosted adapter mutation evidence cannot contain missed mutants",
    );
    diagnostics.require(
        hosted.mutants_timeout == 0,
        "hosted adapter mutation evidence cannot contain timed-out mutants",
    );
}

fn validate_physical(evidence: &LabEvidence, diagnostics: &mut Diagnostics) {
    let physical = &evidence.physical;
    diagnostics.require(
        !physical.observed_at_utc.trim().is_empty(),
        "physical evidence must identify its observation time",
    );
    diagnostics.require(
        physical.observed_at_utc.ends_with('Z'),
        "physical evidence observation time must be UTC",
    );
    diagnostics.require(
        !physical.hardware_id.trim().is_empty(),
        "physical evidence must identify its hardware",
    );
    diagnostics.require(
        physical.architecture == "arm64",
        "physical evidence architecture must be arm64",
    );
    diagnostics.require(
        !physical.os_version.trim().is_empty(),
        "physical evidence must identify macOS",
    );
    diagnostics.require(
        physical.shader_mode == SUPPORTING_SHADER_MODE,
        "physical GPUI evidence must remain labeled runtime-source-unqualified",
    );
    diagnostics.require(
        physical.direct_metal_performed,
        "physical evidence must execute Alpine Direct Metal",
    );
    diagnostics.require(
        physical.cpu_oracle_sha256 == evidence.pixel_sha256,
        "physical CPU oracle hash must match the accepted pixel identity",
    );
    diagnostics.require(
        physical.alpine_metal_sha256 == evidence.pixel_sha256,
        "physical Direct Metal hash must match the accepted pixel identity",
    );
    diagnostics.require(
        physical.gpui_metal_sha256 == evidence.pixel_sha256,
        "physical GPUI Metal hash must match the accepted pixel identity",
    );
}

fn percentage_at_least(covered: u64, total: u64, threshold: u64) -> bool {
    total > 0 && u128::from(covered) * 100 >= u128::from(total) * u128::from(threshold)
}

fn valid_slug(value: &str) -> bool {
    !value.is_empty()
        && value.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
}

fn valid_git_sha(value: &str) -> bool {
    value.len() == 40
        && value
            .chars()
            .all(|character| character.is_ascii_hexdigit() && !character.is_ascii_uppercase())
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .chars()
            .all(|character| character.is_ascii_hexdigit() && !character.is_ascii_uppercase())
}

fn render_report(evidence: &LabEvidence) -> String {
    let mut output = String::from("# Alpine Zed lab evidence report\n\n");
    let _ = write!(
        output,
        "- Evidence: {}\n- Capability: #{}\n- Requirements: {}\n- Task: #{}\n- Lab revision: `{}`\n- Zed revision: `{}`\n- Alpine revision: `{}`\n- Workload: `{}`\n- Accepted compact BGRA8: `{}`\n\n",
        evidence.id,
        evidence.capability_issue,
        evidence
            .requirement_issues
            .iter()
            .map(|issue| format!("#{issue}"))
            .collect::<Vec<_>>()
            .join(", "),
        evidence.task_issue,
        evidence.lab_revision,
        evidence.zed_revision,
        evidence.alpine_revision,
        evidence.workload_hash,
        evidence.pixel_sha256,
    );
    let _ = write!(
        output,
        "Hosted offline evidence: GPUI Metal equals the CPU oracle in {} with artifact {} retained for {} days through {}. Physical evidence: GPUI Metal and Alpine Direct Metal equal the same CPU oracle on {}.\n\nCoverage: {}/{} lines and {}/{} functions. Mutation: {} caught, {} unviable, zero missed, zero timed out.\n\nNo timing or performance claim is present. The runtime-source physical GPUI result is supporting evidence composed with the independent hosted offline-shader result.\n",
        evidence.hosted.run_url,
        evidence.hosted.artifact_id,
        evidence.hosted.retention_days,
        evidence.hosted.artifact_expires_at_utc,
        evidence.physical.hardware_id,
        evidence.hosted.coverage_lines_covered,
        evidence.hosted.coverage_lines_total,
        evidence.hosted.coverage_functions_covered,
        evidence.hosted.coverage_functions_total,
        evidence.hosted.mutants_caught,
        evidence.hosted.mutants_unviable,
    );
    output
}

#[cfg(test)]
mod tests {
    use super::{
        LabEvidence, percentage_at_least, valid_git_sha, valid_sha256, valid_slug, validate,
    };

    fn fixture() -> LabEvidence {
        let parsed = toml::from_str(include_str!(
            "../../../assurance/lab/v1/task-61-solid-quad.toml"
        ));
        assert!(parsed.is_ok(), "fixture must parse");
        parsed.unwrap_or_default()
    }

    #[test]
    fn accepts_composed_hosted_and_physical_evidence() {
        assert!(validate(&fixture()).is_empty());
    }

    #[test]
    fn rejects_divergence_and_performance_claims() {
        let mut evidence = fixture();
        evidence.physical.alpine_metal_sha256 = "0".repeat(64);
        evidence.performance_qualified = true;
        let errors = validate(&evidence).join("\n");
        assert!(errors.contains("physical Direct Metal hash"));
        assert!(errors.contains("cannot contain a performance claim"));
    }

    #[test]
    fn rejects_weak_adapter_assurance() {
        let mut evidence = fixture();
        evidence.hosted.mutants_caught -= 1;
        evidence.hosted.mutants_missed = 1;
        evidence.hosted.coverage_lines_covered = 443;
        let errors = validate(&evidence).join("\n");
        assert!(errors.contains("cannot contain missed mutants"));
        assert!(errors.contains("line coverage must meet 95 percent"));
    }

    #[test]
    fn rejects_source_and_artifact_identity_drift() {
        let mut evidence = fixture();
        evidence.lab_license = "Apache-2.0".to_owned();
        assert!(
            validate(&evidence)
                .join("\n")
                .contains("license must remain GPL-3.0-or-later")
        );

        let mut evidence = fixture();
        evidence.source_influence = "source-level-alpine".to_owned();
        assert!(
            validate(&evidence)
                .join("\n")
                .contains("must remain isolated to the GPL lab")
        );

        let mut evidence = fixture();
        evidence.hosted.run_url = "https://example.invalid/run".to_owned();
        assert!(
            validate(&evidence)
                .join("\n")
                .contains("hosted run URL must match")
        );

        let mut evidence = fixture();
        evidence.hosted.artifact_name = "gpui-oracle-wrong".to_owned();
        assert!(
            validate(&evidence)
                .join("\n")
                .contains("artifact name must bind")
        );

        let mut evidence = fixture();
        evidence.hosted.artifact_url = "https://example.invalid/artifact".to_owned();
        assert!(
            validate(&evidence)
                .join("\n")
                .contains("artifact URL must match")
        );

        let mut evidence = fixture();
        evidence.hosted.retention_days = 7;
        assert!(
            validate(&evidence)
                .join("\n")
                .contains("retained for exactly 90 days")
        );

        let mut evidence = fixture();
        evidence.hosted.artifact_expires_at_utc = "not-a-timestamp".to_owned();
        assert!(
            validate(&evidence)
                .join("\n")
                .contains("expiry must be a canonical UTC timestamp")
        );
    }

    #[test]
    fn rejects_virtual_direct_metal_and_unqualified_shader_labels() {
        let mut evidence = fixture();
        evidence.hosted.direct_metal_performed = true;
        assert!(
            validate(&evidence)
                .join("\n")
                .contains("must not claim Alpine Direct Metal")
        );

        let mut evidence = fixture();
        evidence.hosted.shader_mode = "runtime-source-unqualified".to_owned();
        assert!(
            validate(&evidence)
                .join("\n")
                .contains("must use the offline Metal library")
        );

        let mut evidence = fixture();
        evidence.physical.direct_metal_performed = false;
        assert!(
            validate(&evidence)
                .join("\n")
                .contains("must execute Alpine Direct Metal")
        );

        let mut evidence = fixture();
        evidence.physical.shader_mode = "offline-metallib".to_owned();
        assert!(
            validate(&evidence)
                .join("\n")
                .contains("must remain labeled runtime-source-unqualified")
        );
    }

    #[test]
    fn computes_integer_coverage_thresholds_without_rounding() {
        assert!(percentage_at_least(451, 467, 95));
        assert!(percentage_at_least(55, 60, 90));
        assert!(!percentage_at_least(443, 467, 95));
        assert!(!percentage_at_least(0, 0, 0));
    }

    #[test]
    fn rejects_zero_and_duplicate_issue_or_extent_boundaries() {
        let mut evidence = fixture();
        evidence.capability_issue = 0;
        assert!(
            validate(&evidence)
                .join("\n")
                .contains("capability issue number must be positive")
        );

        let mut evidence = fixture();
        evidence.task_issue = 0;
        assert!(
            validate(&evidence)
                .join("\n")
                .contains("task issue number must be positive")
        );

        let mut evidence = fixture();
        evidence.requirement_issues.clear();
        assert!(
            validate(&evidence)
                .join("\n")
                .contains("requirement issues cannot be empty")
        );

        let mut evidence = fixture();
        evidence.requirement_issues = vec![31, 31];
        assert!(
            validate(&evidence)
                .join("\n")
                .contains("requirement issue numbers must be unique")
        );

        let mut evidence = fixture();
        evidence.requirement_issues = vec![0];
        assert!(
            validate(&evidence)
                .join("\n")
                .contains("requirement issue numbers must be positive")
        );

        let mut evidence = fixture();
        evidence.pixel_width = 0;
        assert!(validate(&evidence).join("\n").contains("pixel width"));

        let mut evidence = fixture();
        evidence.pixel_height = 0;
        assert!(validate(&evidence).join("\n").contains("pixel height"));
    }

    #[test]
    fn rejects_missing_qualification_disclosures() {
        let mut evidence = fixture();
        evidence.timing_performed = true;
        assert!(
            validate(&evidence)
                .join("\n")
                .contains("cannot contain timing")
        );

        let mut evidence = fixture();
        evidence.assumptions.clear();
        assert!(
            validate(&evidence)
                .join("\n")
                .contains("must disclose assumptions")
        );

        let mut evidence = fixture();
        evidence.exclusions.clear();
        assert!(
            validate(&evidence)
                .join("\n")
                .contains("must disclose exclusions")
        );
    }

    #[test]
    fn rejects_hosted_identity_boundaries() {
        let mut evidence = fixture();
        evidence.hosted.run_id = 0;
        assert!(
            validate(&evidence)
                .join("\n")
                .contains("run id must be positive")
        );

        let mut evidence = fixture();
        evidence.hosted.artifact_id = 0;
        assert!(
            validate(&evidence)
                .join("\n")
                .contains("artifact id must be positive")
        );

        let mut evidence = fixture();
        evidence.hosted.architecture = "x86_64".to_owned();
        assert!(
            validate(&evidence)
                .join("\n")
                .contains("architecture must be arm64")
        );

        let mut evidence = fixture();
        evidence.hosted.os_version.clear();
        assert!(
            validate(&evidence)
                .join("\n")
                .contains("hosted evidence must identify macOS")
        );

        let mut evidence = fixture();
        evidence.hosted.cpu_oracle_sha256 = "0".repeat(64);
        assert!(
            validate(&evidence)
                .join("\n")
                .contains("hosted CPU oracle hash")
        );

        let mut evidence = fixture();
        evidence.hosted.gpui_metal_sha256 = "0".repeat(64);
        assert!(
            validate(&evidence)
                .join("\n")
                .contains("hosted offline GPUI Metal hash")
        );

        let mut evidence = fixture();
        evidence.hosted.alpine_metal_sha256 = "0".repeat(64);
        assert!(
            validate(&evidence)
                .join("\n")
                .contains("mark Alpine Direct Metal not-run")
        );
    }

    #[test]
    fn rejects_each_coverage_and_mutation_qualification_break() {
        let mut evidence = fixture();
        evidence.hosted.coverage_performed = false;
        assert!(validate(&evidence).join("\n").contains("coverage must run"));

        let mut evidence = fixture();
        evidence.hosted.coverage_tool_version = "unknown".to_owned();
        assert!(
            validate(&evidence)
                .join("\n")
                .contains("coverage tool must be cargo-llvm-cov")
        );

        let mut evidence = fixture();
        evidence.hosted.coverage_functions_covered = 53;
        assert!(
            validate(&evidence)
                .join("\n")
                .contains("function coverage must meet 90 percent")
        );

        let mut evidence = fixture();
        evidence.hosted.mutation_performed = false;
        assert!(validate(&evidence).join("\n").contains("mutation must run"));

        let mut evidence = fixture();
        evidence.hosted.mutation_tool_version = "unknown".to_owned();
        assert!(
            validate(&evidence)
                .join("\n")
                .contains("mutation tool must be cargo-mutants")
        );

        let mut evidence = fixture();
        evidence.hosted.mutants_total += 1;
        assert!(
            validate(&evidence)
                .join("\n")
                .contains("must classify every mutant")
        );

        let mut evidence = fixture();
        evidence.hosted.mutants_total = 0;
        evidence.hosted.mutants_caught = 0;
        evidence.hosted.mutants_unviable = 0;
        assert!(
            validate(&evidence)
                .join("\n")
                .contains("must contain mutants")
        );

        let mut evidence = fixture();
        evidence.hosted.mutants_timeout = 1;
        evidence.hosted.mutants_caught -= 1;
        assert!(
            validate(&evidence)
                .join("\n")
                .contains("cannot contain timed-out mutants")
        );
    }

    #[test]
    fn rejects_each_physical_identity_break() {
        let mut evidence = fixture();
        evidence.physical.observed_at_utc.clear();
        assert!(
            validate(&evidence)
                .join("\n")
                .contains("must identify its observation time")
        );

        let mut evidence = fixture();
        evidence.physical.observed_at_utc = "2026-08-15T09:24:08+00:00".to_owned();
        assert!(
            validate(&evidence)
                .join("\n")
                .contains("observation time must be UTC")
        );

        let mut evidence = fixture();
        evidence.physical.hardware_id.clear();
        assert!(
            validate(&evidence)
                .join("\n")
                .contains("must identify its hardware")
        );

        let mut evidence = fixture();
        evidence.physical.architecture = "x86_64".to_owned();
        assert!(
            validate(&evidence)
                .join("\n")
                .contains("physical evidence architecture must be arm64")
        );

        let mut evidence = fixture();
        evidence.physical.os_version.clear();
        assert!(
            validate(&evidence)
                .join("\n")
                .contains("physical evidence must identify macOS")
        );

        let mut evidence = fixture();
        evidence.physical.cpu_oracle_sha256 = "0".repeat(64);
        assert!(
            validate(&evidence)
                .join("\n")
                .contains("physical CPU oracle hash")
        );

        let mut evidence = fixture();
        evidence.physical.gpui_metal_sha256 = "0".repeat(64);
        assert!(
            validate(&evidence)
                .join("\n")
                .contains("physical GPUI Metal hash")
        );
    }

    #[test]
    fn identifier_predicates_reject_each_boundary() {
        assert!(valid_slug("task-61-solid-quad"));
        assert!(!valid_slug(""));
        assert!(!valid_slug("Task-61"));
        assert!(!valid_slug("task_61"));

        assert!(valid_git_sha(&"a".repeat(40)));
        assert!(!valid_git_sha(&"a".repeat(39)));
        assert!(!valid_git_sha(&"A".repeat(40)));
        assert!(!valid_git_sha(&"g".repeat(40)));

        assert!(valid_sha256(&"a".repeat(64)));
        assert!(!valid_sha256(&"a".repeat(63)));
        assert!(!valid_sha256(&"A".repeat(64)));
        assert!(!valid_sha256(&"g".repeat(64)));
    }
}
