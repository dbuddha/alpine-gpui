//! Validates and reports Alpine's claim-to-evidence graph.

mod ax;
mod calibration;
mod dogfood;
mod lab;
mod lab_v2;
mod onscreen;
mod qualification;

use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

const DEFAULT_REGISTRY: &str = "assurance/evidence.toml";
const EVIDENCE_KINDS: &[&str] = &[
    "tla",
    "kani",
    "unit",
    "property",
    "integration",
    "e2e",
    "loom",
    "miri",
    "fuzz",
    "mutation",
    "coverage",
    "benchmark",
    "native",
];

#[derive(Debug, Deserialize)]
struct Registry {
    schema: String,
    claims: Vec<Claim>,
    evidence: Vec<Evidence>,
}

#[derive(Debug, Deserialize)]
struct Claim {
    id: String,
    mission: String,
    capability_issue: u64,
    aep: String,
    requirement_issue: u64,
    #[serde(rename = "claim_type")]
    category: String,
    risk: String,
    #[serde(default)]
    case_study_findings: Vec<String>,
    kani_applicability: String,
    #[serde(default)]
    tla_properties: Vec<String>,
    platform_scope: Vec<String>,
    required_evidence: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct Evidence {
    id: String,
    claim: String,
    kind: String,
    artifact: String,
    assertion: String,
    scope: String,
    #[serde(default)]
    bounds: Vec<String>,
    #[serde(default)]
    assumptions: Vec<String>,
    #[serde(default)]
    exclusions: Vec<String>,
    companion: Option<String>,
}

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

fn run() -> Result<String, Vec<String>> {
    let mut arguments = env::args().skip(1);
    let command = arguments.next().unwrap_or_else(|| "validate".to_owned());
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
    if command == "record-studio-dogfood" {
        return run_dogfood_record_command(&mut arguments);
    }
    if matches!(
        command.as_str(),
        "validate-ax-fixture" | "validate-ax-evidence" | "ax-evidence-report"
    ) {
        return run_ax_command(&command, &mut arguments);
    }
    if matches!(
        command.as_str(),
        "validate-studio-dogfood" | "studio-dogfood-report"
    ) {
        return run_dogfood_command(&command, &mut arguments);
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
    let mut registry_path = PathBuf::from(DEFAULT_REGISTRY);
    let mut github = false;

    for argument in arguments {
        if argument == "--github" {
            github = true;
        } else {
            registry_path = PathBuf::from(argument);
        }
    }

    let registry = load_registry(&registry_path)?;
    let root = Path::new(".");
    let errors = validate_registry(&registry, root, github);
    if !errors.is_empty() {
        return Err(errors);
    }

    match command.as_str() {
        "validate" => Ok(format!(
            "validated {} claims and {} evidence records",
            registry.claims.len(),
            registry.evidence.len()
        )),
        "report" => Ok(render_report(&registry)),
        other => Err(vec![format!(
            "unknown command {other:?}; expected validate, report, validate-scene-trace, render-scene-reference, render-scene-native, validate-qualification, qualification-report, validate-aa-calibration, aa-calibration-report, validate-zed-lab-evidence, zed-lab-evidence-report, validate-onscreen-sdr, onscreen-sdr-report, validate-ax-fixture, validate-ax-evidence, ax-evidence-report, record-studio-dogfood, validate-studio-dogfood, studio-dogfood-report, or upstream-radar"
        )]),
    }
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
    if lab_v2::is_v2_evidence(path) {
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

fn run_dogfood_command(
    command: &str,
    arguments: &mut impl Iterator<Item = String>,
) -> Result<String, Vec<String>> {
    let Some(path) = arguments.next() else {
        return Err(vec![format!("{command} requires a manifest path")]);
    };
    if arguments.next().is_some() {
        return Err(vec![format!("{command} accepts exactly one manifest path")]);
    }
    dogfood::run(command, Path::new(&path))
}

fn run_dogfood_record_command(
    arguments: &mut impl Iterator<Item = String>,
) -> Result<String, Vec<String>> {
    let Some(draft) = arguments.next() else {
        return Err(vec![
            "record-studio-dogfood requires a draft, snapshot, and destination".to_owned(),
        ]);
    };
    let Some(snapshot) = arguments.next() else {
        return Err(vec![
            "record-studio-dogfood requires a snapshot and destination".to_owned(),
        ]);
    };
    let Some(destination) = arguments.next() else {
        return Err(vec![
            "record-studio-dogfood requires a destination".to_owned(),
        ]);
    };
    if arguments.next().is_some() {
        return Err(vec![
            "record-studio-dogfood accepts exactly three paths".to_owned(),
        ]);
    }
    dogfood::record(
        Path::new(&draft),
        Path::new(&snapshot),
        Path::new(&destination),
    )
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

fn load_registry(path: &Path) -> Result<Registry, Vec<String>> {
    let source = fs::read_to_string(path)
        .map_err(|error| vec![format!("cannot read {}: {error}", path.display())])?;
    toml::from_str(&source)
        .map_err(|error| vec![format!("cannot parse {}: {error}", path.display())])
}

fn validate_registry(registry: &Registry, root: &Path, github: bool) -> Vec<String> {
    let mut diagnostics = Diagnostics::default();
    diagnostics.require(
        registry.schema == "alpine-evidence/v1",
        "schema must be alpine-evidence/v1",
    );

    let claims = validate_claims(registry, root, &mut diagnostics);
    let observed = validate_evidence(registry, root, &claims, &mut diagnostics);
    validate_claim_coverage(registry, &observed, &mut diagnostics);
    validate_kani_inventory(registry, root, &mut diagnostics);
    if github {
        validate_github(registry, &mut diagnostics);
    }

    diagnostics.errors.sort();
    diagnostics.errors
}

fn validate_claims<'a>(
    registry: &'a Registry,
    root: &Path,
    diagnostics: &mut Diagnostics,
) -> BTreeMap<&'a str, &'a Claim> {
    let mut claim_ids = BTreeSet::new();
    let mut claims = BTreeMap::new();
    for claim in &registry.claims {
        diagnostics.require(
            valid_identifier(&claim.id, "AEP-", "-C"),
            format!("invalid claim identifier {}", claim.id),
        );
        validate_claim_classification(claim, root, diagnostics);
        diagnostics.require(
            claim_ids.insert(claim.id.as_str()),
            format!("duplicate claim identifier {}", claim.id),
        );
        diagnostics.require(
            claim.mission.starts_with("MP-")
                && claim.mission[3..].chars().all(|c| c.is_ascii_digit()),
            format!("claim {} has invalid mission identifier", claim.id),
        );
        diagnostics.require(
            claim.capability_issue > 0 && claim.requirement_issue > 0,
            format!("claim {} must link positive issue numbers", claim.id),
        );
        diagnostics.require(
            matches!(claim.risk.as_str(), "low" | "medium" | "high" | "critical"),
            format!("claim {} has unsupported risk {}", claim.id, claim.risk),
        );
        diagnostics.require(
            matches!(
                claim.category.as_str(),
                "functional"
                    | "safety"
                    | "lifecycle"
                    | "concurrency"
                    | "accessibility"
                    | "compatibility"
                    | "performance"
                    | "memory"
                    | "visual"
            ),
            format!("claim {} has unsupported type {}", claim.id, claim.category),
        );
        let aep_path = root.join(&claim.aep);
        diagnostics.require(
            artifact_exists(&aep_path),
            format!("claim {} references missing AEP {}", claim.id, claim.aep),
        );
        if let Ok(source) = fs::read_to_string(&aep_path) {
            diagnostics.require(
                source.contains(&claim.id),
                format!("AEP {} does not declare claim {}", claim.aep, claim.id),
            );
        }
        for kind in &claim.required_evidence {
            diagnostics.require(
                EVIDENCE_KINDS.contains(&kind.as_str()),
                format!("claim {} requires unknown evidence kind {kind}", claim.id),
            );
        }
        claims.insert(claim.id.as_str(), claim);
    }

    claims
}

fn validate_claim_classification(claim: &Claim, root: &Path, diagnostics: &mut Diagnostics) {
    diagnostics.require(
        matches!(
            claim.kani_applicability.as_str(),
            "required" | "supporting" | "not_applicable"
        ),
        format!(
            "claim {} has invalid Kani applicability {}",
            claim.id, claim.kani_applicability
        ),
    );
    diagnostics.require(
        !claim.platform_scope.is_empty()
            && claim.platform_scope.iter().all(|platform| {
                matches!(
                    platform.as_str(),
                    "portable"
                        | "tooling"
                        | "macos-metal"
                        | "linux-vulkan-wayland"
                        | "windows-d3d12-win32"
                )
            }),
        format!("claim {} has invalid or empty platform scope", claim.id),
    );
    if matches!(
        claim.category.as_str(),
        "safety" | "lifecycle" | "concurrency"
    ) {
        diagnostics.require(
            !claim.tla_properties.is_empty(),
            format!("claim {} requires at least one TLA+ property", claim.id),
        );
    }
    if claim.kani_applicability == "required" {
        diagnostics.require(
            claim.required_evidence.iter().any(|kind| kind == "kani"),
            format!("claim {} requires Kani evidence", claim.id),
        );
    }
    for finding in &claim.case_study_findings {
        diagnostics.require(
            finding.starts_with("CS-") && finding_exists(root, finding),
            format!("claim {} references unknown finding {finding}", claim.id),
        );
    }
}

fn validate_evidence<'a>(
    registry: &'a Registry,
    root: &Path,
    claims: &BTreeMap<&str, &Claim>,
    diagnostics: &mut Diagnostics,
) -> BTreeMap<&'a str, BTreeSet<&'a str>> {
    let mut evidence_ids = BTreeSet::new();
    let mut observed: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for evidence in &registry.evidence {
        diagnostics.require(
            valid_identifier(&evidence.id, "EV-", "-"),
            format!("invalid evidence identifier {}", evidence.id),
        );
        diagnostics.require(
            evidence_ids.insert(evidence.id.as_str()),
            format!("duplicate evidence identifier {}", evidence.id),
        );
        diagnostics.require(
            claims.contains_key(evidence.claim.as_str()),
            format!(
                "evidence {} references unknown claim {}",
                evidence.id, evidence.claim
            ),
        );
        diagnostics.require(
            EVIDENCE_KINDS.contains(&evidence.kind.as_str()),
            format!(
                "evidence {} has unknown kind {}",
                evidence.id, evidence.kind
            ),
        );
        diagnostics.require(
            !evidence.assertion.trim().is_empty() && !evidence.scope.trim().is_empty(),
            format!(
                "evidence {} must state its assertion and scope",
                evidence.id
            ),
        );
        diagnostics.require(
            artifact_reference_exists(root, &evidence.artifact),
            format!(
                "evidence {} references missing artifact {}",
                evidence.id, evidence.artifact
            ),
        );
        if matches!(evidence.kind.as_str(), "tla" | "kani" | "loom") {
            diagnostics.require(
                !evidence.bounds.is_empty(),
                format!("formal evidence {} must disclose bounds", evidence.id),
            );
            diagnostics.require(
                !evidence.exclusions.is_empty(),
                format!("formal evidence {} must disclose exclusions", evidence.id),
            );
        }
        if evidence.kind == "kani" {
            diagnostics.require(
                evidence
                    .companion
                    .as_deref()
                    .is_some_and(|path| artifact_reference_exists(root, path)),
                format!(
                    "Kani evidence {} needs an existing dynamic companion",
                    evidence.id
                ),
            );
        }
        if evidence.kind == "tla" {
            diagnostics.require(
                evidence
                    .companion
                    .as_deref()
                    .is_some_and(|path| artifact_reference_exists(root, path)),
                format!(
                    "TLA+ evidence {} needs existing Rust conformance evidence",
                    evidence.id
                ),
            );
        }
        observed
            .entry(evidence.claim.as_str())
            .or_default()
            .insert(evidence.kind.as_str());
    }

    observed
}

fn validate_claim_coverage(
    registry: &Registry,
    observed: &BTreeMap<&str, BTreeSet<&str>>,
    diagnostics: &mut Diagnostics,
) {
    for claim in &registry.claims {
        let kinds = observed.get(claim.id.as_str());
        for required in &claim.required_evidence {
            diagnostics.require(
                kinds.is_some_and(|items| items.contains(required.as_str())),
                format!("claim {} lacks required {required} evidence", claim.id),
            );
        }
        if claim.category == "performance" {
            diagnostics.require(
                kinds.is_some_and(|items| items.contains("benchmark")),
                format!("performance claim {} lacks a benchmark", claim.id),
            );
        }
        for property in &claim.tla_properties {
            diagnostics.require(
                registry.evidence.iter().any(|evidence| {
                    evidence.claim == claim.id
                        && evidence.kind == "tla"
                        && artifact_anchor(&evidence.artifact) == Some(property.as_str())
                }),
                format!(
                    "claim {} lacks TLA+ property evidence for {property}",
                    claim.id
                ),
            );
        }
    }

    let mut capability_acceptance: BTreeMap<u64, BTreeSet<&str>> = BTreeMap::new();
    for claim in &registry.claims {
        if let Some(kinds) = observed.get(claim.id.as_str()) {
            capability_acceptance
                .entry(claim.capability_issue)
                .or_default()
                .extend(kinds);
        }
    }
    for (capability, kinds) in capability_acceptance {
        diagnostics.require(
            kinds.contains("e2e") || kinds.contains("native"),
            format!("capability #{capability} lacks end-to-end or native acceptance evidence"),
        );
    }
}

fn validate_github(registry: &Registry, diagnostics: &mut Diagnostics) {
    let Some(repository) = env::var("GH_REPOSITORY").ok() else {
        diagnostics
            .errors
            .push("--github requires GH_REPOSITORY in owner/repository form".to_owned());
        return;
    };
    let mut seen = BTreeSet::new();
    for claim in &registry.claims {
        if seen.insert((claim.capability_issue, "capability")) {
            validate_issue(
                &repository,
                claim.capability_issue,
                "kind:capability",
                diagnostics,
            );
        }
        if seen.insert((claim.requirement_issue, "requirement")) {
            validate_issue(
                &repository,
                claim.requirement_issue,
                "kind:requirement",
                diagnostics,
            );
            let parent = command_output(
                "gh",
                &[
                    "api",
                    &format!(
                        "repos/{repository}/issues/{}/parent",
                        claim.requirement_issue
                    ),
                    "--jq",
                    ".number",
                ],
            );
            let expected_parent = claim.capability_issue.to_string();
            diagnostics.require(
                parent.as_deref() == Some(expected_parent.as_str()),
                format!(
                    "requirement #{} is not a native child of capability #{}",
                    claim.requirement_issue, claim.capability_issue
                ),
            );
        }
    }
}

fn validate_issue(
    repository: &str,
    number: u64,
    expected_kind: &str,
    diagnostics: &mut Diagnostics,
) {
    let labels = command_output(
        "gh",
        &[
            "issue",
            "view",
            &number.to_string(),
            "--repo",
            repository,
            "--json",
            "labels",
            "--jq",
            ".labels[].name",
        ],
    );
    match labels {
        Some(labels) => {
            diagnostics.require(
                labels.lines().any(|label| label == expected_kind),
                format!("issue #{number} must have {expected_kind}"),
            );
            diagnostics.require(
                labels.lines().any(|label| label == "owner:approved"),
                format!("issue #{number} must have owner:approved"),
            );
        }
        None => diagnostics
            .errors
            .push(format!("cannot retrieve GitHub issue #{number}")),
    }
}

fn command_output(program: &str, arguments: &[&str]) -> Option<String> {
    let output = Command::new(program).args(arguments).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn valid_identifier(value: &str, prefix: &str, separator: &str) -> bool {
    let Some(rest) = value.strip_prefix(prefix) else {
        return false;
    };
    let Some((left, right)) = rest.split_once(separator) else {
        return false;
    };
    !left.is_empty()
        && !right.is_empty()
        && left.chars().all(|character| character.is_ascii_digit())
        && right
            .chars()
            .all(|character| character.is_ascii_uppercase() || character.is_ascii_digit())
}

fn artifact_path(value: &str) -> &str {
    value.split_once('#').map_or(value, |(path, _)| path)
}

fn artifact_anchor(value: &str) -> Option<&str> {
    value.split_once('#').map(|(_, anchor)| anchor)
}

fn artifact_exists(path: &Path) -> bool {
    path.is_file()
}

fn finding_exists(root: &Path, identifier: &str) -> bool {
    let Ok(entries) = fs::read_dir(root.join("docs/case-studies")) else {
        return false;
    };
    entries.flatten().any(|entry| {
        entry
            .path()
            .extension()
            .is_some_and(|extension| extension == "md")
            && fs::read_to_string(entry.path()).is_ok_and(|source| source.contains(identifier))
    })
}

fn artifact_reference_exists(root: &Path, reference: &str) -> bool {
    let path = root.join(artifact_path(reference));
    if !artifact_exists(&path) {
        return false;
    }
    artifact_anchor(reference)
        .is_none_or(|anchor| fs::read_to_string(path).is_ok_and(|source| source.contains(anchor)))
}

fn validate_kani_inventory(registry: &Registry, root: &Path, diagnostics: &mut Diagnostics) {
    let registered: BTreeSet<String> = registry
        .evidence
        .iter()
        .filter(|evidence| evidence.kind == "kani")
        .map(|evidence| evidence.artifact.clone())
        .collect();
    let mut discovered = BTreeSet::new();
    discover_kani_harnesses(&root.join("crates"), root, &mut discovered, diagnostics);
    discover_kani_harnesses(
        &root.join("tools/alpine-trace"),
        root,
        &mut discovered,
        diagnostics,
    );
    for artifact in registered
        .iter()
        .filter(|artifact| artifact_path(artifact).starts_with("apps/"))
    {
        discover_kani_file(&root.join(artifact_path(artifact)), root, &mut discovered);
    }

    for harness in discovered.difference(&registered) {
        diagnostics
            .errors
            .push(format!("Kani harness is not registered: {harness}"));
    }
    for harness in registered.difference(&discovered) {
        diagnostics.errors.push(format!(
            "registered Kani harness was not inventoried: {harness}"
        ));
    }
}

fn discover_kani_harnesses(
    directory: &Path,
    root: &Path,
    harnesses: &mut BTreeSet<String>,
    diagnostics: &mut Diagnostics,
) {
    let Ok(entries) = fs::read_dir(directory) else {
        diagnostics.errors.push(format!(
            "cannot inspect Kani source directory {}",
            directory.display()
        ));
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            discover_kani_harnesses(&path, root, harnesses, diagnostics);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            discover_kani_file(&path, root, harnesses);
        }
    }
}

fn discover_kani_file(path: &Path, root: &Path, harnesses: &mut BTreeSet<String>) {
    let Ok(source) = fs::read_to_string(path) else {
        return;
    };
    let mut expects_harness = false;
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed == "#[kani::proof]" {
            expects_harness = true;
        } else if expects_harness && trimmed.starts_with("fn ") {
            let name = trimmed
                .trim_start_matches("fn ")
                .split('(')
                .next()
                .unwrap_or_default();
            if let Ok(relative) = path.strip_prefix(root) {
                harnesses.insert(format!("{}#{name}", registry_path(relative)));
            }
            expects_harness = false;
        } else if !trimmed.is_empty() && !trimmed.starts_with("///") {
            expects_harness = false;
        }
    }
}

fn registry_path(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn render_report(registry: &Registry) -> String {
    let mut output = String::from("# Alpine assurance report\n\n");
    output.push_str("Formal results are qualified by their recorded scope, bounds, assumptions, and exclusions. Model checked does not mean implementation verified.\n\n");
    for claim in &registry.claims {
        let _ = write!(
            output,
            "## {}\n\n- Mission: {}\n- Case-study findings: {}\n- Capability: #{}\n- AEP: {}\n- Requirement: #{}\n- Type and risk: {} / {}\n- Platform scope: {}\n- Kani applicability: {}\n- TLA+ properties: {}\n- Required evidence: {}\n\n",
            claim.id,
            claim.mission,
            display_list(&claim.case_study_findings),
            claim.capability_issue,
            claim.aep,
            claim.requirement_issue,
            claim.category,
            claim.risk,
            display_list(&claim.platform_scope),
            claim.kani_applicability,
            display_list(&claim.tla_properties),
            claim.required_evidence.join(", ")
        );
        for evidence in registry
            .evidence
            .iter()
            .filter(|item| item.claim == claim.id)
        {
            let _ = write!(
                output,
                "### {} ({})\n\n- Assertion: {}\n- Artifact: {}\n- Scope: {}\n- Bounds: {}\n- Assumptions: {}\n- Exclusions: {}\n",
                evidence.id,
                evidence.kind,
                evidence.assertion,
                evidence.artifact,
                evidence.scope,
                display_list(&evidence.bounds),
                display_list(&evidence.assumptions),
                display_list(&evidence.exclusions)
            );
            if let Some(companion) = &evidence.companion {
                let _ = writeln!(output, "- Companion evidence: {companion}");
            }
            output.push('\n');
        }
    }
    output
}

fn display_list(items: &[String]) -> String {
    if items.is_empty() {
        "none".to_owned()
    } else {
        items.join("; ")
    }
}

#[cfg(test)]
mod tests {
    use super::{
        artifact_anchor, artifact_path, discover_kani_file, load_registry, registry_path,
        render_report, valid_identifier, validate_registry,
    };
    use std::{
        collections::BTreeSet,
        fs,
        path::{Path, PathBuf},
    };

    fn repository_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    #[test]
    fn validates_identifier_shapes() {
        assert!(valid_identifier("AEP-0016-C01", "AEP-", "-C"));
        assert!(valid_identifier("EV-0016-KANI01", "EV-", "-"));
        assert!(!valid_identifier("AEP-16", "AEP-", "-C"));
        assert!(!valid_identifier("EV-0016-lower", "EV-", "-"));
    }

    #[test]
    fn splits_artifact_anchors() {
        assert_eq!(artifact_path("path/file.rs#harness"), "path/file.rs");
        assert_eq!(artifact_anchor("path/file.rs#harness"), Some("harness"));
        assert_eq!(artifact_anchor("path/file.rs"), None);
    }

    #[test]
    fn renders_registry_paths_with_portable_separators() {
        let path = Path::new("crates")
            .join("alpine-core")
            .join("src")
            .join("proofs.rs");
        assert_eq!(registry_path(&path), "crates/alpine-core/src/proofs.rs");
    }

    #[test]
    fn discovers_kani_proofs_across_only_supported_trivia() -> Result<(), Box<dyn std::error::Error>>
    {
        let root = repository_root();
        let path = root.join("target").join(format!(
            "assurance-kani-discovery-{}.rs",
            std::process::id()
        ));
        fs::create_dir_all(path.parent().ok_or("temporary proof parent")?)?;
        fs::write(
            &path,
            "#[kani::proof]\n/// documented proof\nfn documented() {}\n\
             #[kani::proof]\n\nfn separated() {}\n\
             #[kani::proof]\n#[cfg(any())]\nfn attributed_is_not_supported() {}\n",
        )?;

        let mut harnesses = BTreeSet::new();
        discover_kani_file(&path, &root, &mut harnesses);
        fs::remove_file(&path)?;
        let relative = path.strip_prefix(&root)?;
        let prefix = registry_path(relative);
        let expected = BTreeSet::from([
            format!("{prefix}#documented"),
            format!("{prefix}#separated"),
        ]);
        assert_eq!(harnesses, expected);
        Ok(())
    }

    #[test]
    fn validates_and_renders_the_committed_registry() {
        let root = repository_root();
        let registry = load_registry(&root.join("assurance/evidence.toml"));
        assert!(registry.is_ok());
        if let Ok(registry) = registry {
            let errors = validate_registry(&registry, &root, false);
            assert!(errors.is_empty(), "{errors:#?}");
            let report = render_report(&registry);
            assert!(report.contains("AEP-0009-C01"));
            assert!(report.contains("Model checked does not mean implementation verified"));
            assert!(report.contains("EV-0016-KANI03 (kani)"));
        }
    }

    #[test]
    fn rejects_an_unregistered_kani_harness() {
        let root = repository_root();
        let registry = load_registry(&root.join("assurance/evidence.toml"));
        assert!(registry.is_ok());
        if let Ok(mut registry) = registry {
            registry.evidence.retain(|evidence| {
                evidence.artifact
                    != "tools/alpine-trace/src/proofs.rs#bounded_trace_preserves_operation_order_and_values"
            });
            let errors = validate_registry(&registry, &root, false);
            assert!(
                errors
                    .iter()
                    .any(|error| error.contains("Kani harness is not registered")),
                "{errors:#?}"
            );
        }
    }

    #[test]
    fn rejects_each_structural_fixture() {
        let fixtures = [
            (
                "assurance/fixtures/duplicate-claim.toml",
                "duplicate claim identifier AEP-0009-C01",
            ),
            (
                "assurance/fixtures/missing-artifact.toml",
                "references missing artifact missing/evidence.rs",
            ),
            (
                "assurance/fixtures/kani-without-companion.toml",
                "needs an existing dynamic companion",
            ),
            (
                "assurance/fixtures/performance-without-benchmark.toml",
                "performance claim AEP-0009-C01 lacks a benchmark",
            ),
        ];
        let root = repository_root();

        for (fixture, expected) in fixtures {
            let registry = load_registry(&root.join(fixture));
            assert!(registry.is_ok());
            if let Ok(registry) = registry {
                let errors = validate_registry(&registry, &root, false);
                assert!(errors.iter().any(|error| error.contains(expected)));
            }
        }
    }
}
