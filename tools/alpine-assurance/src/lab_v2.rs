use serde::Deserialize;
use std::{collections::BTreeSet, fmt::Write as _, fs, path::Path};

const SCHEMA: &str = "alpine-zed-lab-evidence/v2";
const LAB_REVISION: &str = "13fade6ac4c344a6bf40295544c49971ddfecb96";
const ZED_REVISION: &str = "e17dc4f9d50db73a458b64dcce50ecd4878b98a3";
const ALPINE_REVISION: &str = "1b6d16e6ddc120a7670fc225913dad9908dd482c";
const TRACE_MANIFEST_SHA256: &str =
    "afa696780de42292510c3b19bd60602149455fd921ceefc3f6e7f0dcf00b67d4";
const PATCH_SERIES_SHA256: &str =
    "a507d3ab4dd97cb716c89adeac2378750268bbd160503a72e0f2f029582e303b";
const HOSTED_SET_SHA256: &str = "77e2613f2cf6c63872f3d6b009b1951545854e0fcc1541d838d547d89c18a6e7";
const HOSTED_ARTIFACT_SHA256: &str =
    "a7a6816414c9594dbbacdf311b575c73f78bdb01f0461b92bf551760b05e1b45";
const PHYSICAL_SET_SHA256: &str =
    "d921f82667dd9195e1a8d4d36de1eaf824fc2e47ee9313e752d81bead0e7a2d4";

#[derive(Clone, Copy)]
struct FixtureSpec {
    id: &'static str,
    trace_schema: &'static str,
    trace_path: &'static str,
    scene_trace_sha256: &'static str,
    workload_hash: &'static str,
    pair_id: &'static str,
    pair_kind: &'static str,
    pair_sequence_hash: &'static str,
    pair_step: &'static str,
    pair_steps: &'static str,
    pixel_width: u32,
    pixel_height: u32,
    cpu_sha256: &'static str,
    metal_sha256: &'static str,
    physical_manifest_sha256: &'static str,
    max_channel_delta: u8,
    exact_pixel_equivalence: bool,
    adaptation_clips: u32,
    adaptation_operations: u32,
    adaptation_quads: u32,
    adaptation_glyphs: u32,
    adaptation_resources: u32,
    adaptation_resource_bytes: u64,
    adaptation_atlas_allocations: u32,
}

const FIXTURES: [FixtureSpec; 8] = [
    FixtureSpec {
        id: "solid-quad-editor-surface",
        trace_schema: "alpine-scene-trace/v1",
        trace_path: "assurance/qualification/v1/scene.toml",
        scene_trace_sha256: "c59c1d7d27abfb2bd6327db236c41934f2da34c9b82d25a6522421c1d38dbe8e",
        workload_hash: "3dbf181350ed2283ce9daa427af5d674cebcceb4dada5abf2a1c1e1f5c156bda",
        pair_id: "none",
        pair_kind: "none",
        pair_sequence_hash: "none",
        pair_step: "none",
        pair_steps: "none",
        pixel_width: 8,
        pixel_height: 4,
        cpu_sha256: "074cadbffac89c52b3d03f54208ee2cd419828855233d0263941404c1026e8c2",
        metal_sha256: "074cadbffac89c52b3d03f54208ee2cd419828855233d0263941404c1026e8c2",
        physical_manifest_sha256: "06b06cf0dcbd92cc5a688932e7baa192c28ad6054604bb503159513fa233a879",
        max_channel_delta: 0,
        exact_pixel_equivalence: true,
        adaptation_clips: 1,
        adaptation_operations: 3,
        adaptation_quads: 3,
        adaptation_glyphs: 0,
        adaptation_resources: 0,
        adaptation_resource_bytes: 0,
        adaptation_atlas_allocations: 0,
    },
    FixtureSpec {
        id: "clipped-quad-grid",
        trace_schema: "alpine-scene-trace/v2",
        trace_path: "assurance/qualification/v2/clipped-grid.toml",
        scene_trace_sha256: "29f3767206d9327a1055625cffd84c87de916dd4c5a45974350ea96ccaef3a3d",
        workload_hash: "921875c7910b753cd3e1201f3576f126fc75a627e95e2521f7d48d874a189981",
        pair_id: "none",
        pair_kind: "none",
        pair_sequence_hash: "none",
        pair_step: "none",
        pair_steps: "none",
        pixel_width: 16,
        pixel_height: 12,
        cpu_sha256: "5bfa18375e62ee84ad1a390b13c6c7637cd7d983c7c70b146fc22449638ac58e",
        metal_sha256: "40e6dc3c22e97bb2133eab41d510440fd5d00e741b98421230b6ff01894a42b5",
        physical_manifest_sha256: "9b3aa2c7939b7f63a05a16751ef2a812965b22ef88e073691c8a102fb636ae31",
        max_channel_delta: 1,
        exact_pixel_equivalence: false,
        adaptation_clips: 2,
        adaptation_operations: 4,
        adaptation_quads: 4,
        adaptation_glyphs: 0,
        adaptation_resources: 0,
        adaptation_resource_bytes: 0,
        adaptation_atlas_allocations: 0,
    },
    FixtureSpec {
        id: "monochrome-glyph-grid",
        trace_schema: "alpine-scene-trace/v2",
        trace_path: "assurance/qualification/v2/glyph-grid.toml",
        scene_trace_sha256: "7b6da05b921fa542ac56f3c40f06275ed316d194378f83eec1642f649093cbd5",
        workload_hash: "9acc0f250209c54000e308f92050f33bda0f5b426455b93c2f932594220bd93c",
        pair_id: "none",
        pair_kind: "none",
        pair_sequence_hash: "none",
        pair_step: "none",
        pair_steps: "none",
        pixel_width: 16,
        pixel_height: 12,
        cpu_sha256: "7672d2a5a8cb7dc3d6a97136b67ef92ed0373e7d68cd9a39782bd30cb89492d9",
        metal_sha256: "7672d2a5a8cb7dc3d6a97136b67ef92ed0373e7d68cd9a39782bd30cb89492d9",
        physical_manifest_sha256: "fb4e97450d292904ba0e358b32a80154400780076fc1fb2acc023f710fbf98dd",
        max_channel_delta: 0,
        exact_pixel_equivalence: true,
        adaptation_clips: 1,
        adaptation_operations: 4,
        adaptation_quads: 0,
        adaptation_glyphs: 4,
        adaptation_resources: 1,
        adaptation_resource_bytes: 64,
        adaptation_atlas_allocations: 1,
    },
    FixtureSpec {
        id: "realistic-code-viewport",
        trace_schema: "alpine-scene-trace/v2",
        trace_path: "assurance/qualification/v2/code-viewport.toml",
        scene_trace_sha256: "6991f75bf19536543c67e90cfa2b6c974a443ff3aad32a40134113ded70b905a",
        workload_hash: "78ae6f66191128422c880a4ee8f9cb6d8b86f44ea0d5ff7af3832ecd37864efe",
        pair_id: "none",
        pair_kind: "none",
        pair_sequence_hash: "none",
        pair_step: "none",
        pair_steps: "none",
        pixel_width: 64,
        pixel_height: 32,
        cpu_sha256: "c278bc8f5920b6e8428f47225407be18c0a2fa9a81156a7d707e925e5cab5440",
        metal_sha256: "c278bc8f5920b6e8428f47225407be18c0a2fa9a81156a7d707e925e5cab5440",
        physical_manifest_sha256: "fd7b1abb28f601432b228b44a05601901b79f43c5fd647fd7c9d86e6c7d81634",
        max_channel_delta: 0,
        exact_pixel_equivalence: true,
        adaptation_clips: 1,
        adaptation_operations: 11,
        adaptation_quads: 4,
        adaptation_glyphs: 7,
        adaptation_resources: 1,
        adaptation_resource_bytes: 64,
        adaptation_atlas_allocations: 1,
    },
    FixtureSpec {
        id: "code-scroll-before",
        trace_schema: "alpine-scene-trace/v2",
        trace_path: "assurance/qualification/v2/scroll-before.toml",
        scene_trace_sha256: "2bcb50b5038eb585cff8cff44ba348619503fa2fb90514f38824520d772fb4b5",
        workload_hash: "820e00afceab09aacaf126e6ffc6c25b02a9f4a6964c7c8aad43bfed8ad24f99",
        pair_id: "code-scroll",
        pair_kind: "scroll",
        pair_sequence_hash: "2a075e0709f1e3266ea1ec96dd427d5d00f35ee2b117779e59ba7fcffbfce9bf",
        pair_step: "0",
        pair_steps: "2",
        pixel_width: 64,
        pixel_height: 32,
        cpu_sha256: "c278bc8f5920b6e8428f47225407be18c0a2fa9a81156a7d707e925e5cab5440",
        metal_sha256: "c278bc8f5920b6e8428f47225407be18c0a2fa9a81156a7d707e925e5cab5440",
        physical_manifest_sha256: "512476c2c167ea5bb582aefd2a0f7f3a30105261a93faa5cfb2a5aca918d5732",
        max_channel_delta: 0,
        exact_pixel_equivalence: true,
        adaptation_clips: 1,
        adaptation_operations: 11,
        adaptation_quads: 4,
        adaptation_glyphs: 7,
        adaptation_resources: 1,
        adaptation_resource_bytes: 64,
        adaptation_atlas_allocations: 1,
    },
    FixtureSpec {
        id: "code-scroll-after",
        trace_schema: "alpine-scene-trace/v2",
        trace_path: "assurance/qualification/v2/scroll-after.toml",
        scene_trace_sha256: "f03a124648d89f187526c73e05effa210447da5e453f1c383c7d3b0fdc7b3f6e",
        workload_hash: "9c6f52c4e604999675a27e72af032d2c24012155780c6cc43f6838cab8d26f53",
        pair_id: "code-scroll",
        pair_kind: "scroll",
        pair_sequence_hash: "2a075e0709f1e3266ea1ec96dd427d5d00f35ee2b117779e59ba7fcffbfce9bf",
        pair_step: "1",
        pair_steps: "2",
        pixel_width: 64,
        pixel_height: 32,
        cpu_sha256: "3d1e23add7852c81d7ddd3d796dc991c76a689f9b627cc33a169629e1877cab9",
        metal_sha256: "3d1e23add7852c81d7ddd3d796dc991c76a689f9b627cc33a169629e1877cab9",
        physical_manifest_sha256: "c1c1ddf5dad164eac40a5f2eee75d0276dd9b569d80e51ec2a365786b76d1ff1",
        max_channel_delta: 0,
        exact_pixel_equivalence: true,
        adaptation_clips: 1,
        adaptation_operations: 11,
        adaptation_quads: 4,
        adaptation_glyphs: 7,
        adaptation_resources: 1,
        adaptation_resource_bytes: 64,
        adaptation_atlas_allocations: 1,
    },
    FixtureSpec {
        id: "code-resize-before",
        trace_schema: "alpine-scene-trace/v2",
        trace_path: "assurance/qualification/v2/resize-before.toml",
        scene_trace_sha256: "c8b9ebdea9a690515f87ef66b0b52004d85cfd406b81e3f911b0bb5d9b1058bb",
        workload_hash: "d32da3fe63ffc260d6bf3157fb7a85d520beef62d142a2c912bbe44449a5fb8d",
        pair_id: "code-resize",
        pair_kind: "resize",
        pair_sequence_hash: "cedbf3c8a93f46d1236b6b8996cbde6f95f771bac2934598c1b07884fab0822a",
        pair_step: "0",
        pair_steps: "2",
        pixel_width: 48,
        pixel_height: 24,
        cpu_sha256: "9522d26b98233ea439528a124ea29a5be6c10e3149f3dc0d9ac4091f0a631ecf",
        metal_sha256: "9522d26b98233ea439528a124ea29a5be6c10e3149f3dc0d9ac4091f0a631ecf",
        physical_manifest_sha256: "46b1c53aeac814c0549f182a2b8f5e81b976f73eb589e73d396241190fc92f27",
        max_channel_delta: 0,
        exact_pixel_equivalence: true,
        adaptation_clips: 1,
        adaptation_operations: 11,
        adaptation_quads: 4,
        adaptation_glyphs: 7,
        adaptation_resources: 1,
        adaptation_resource_bytes: 64,
        adaptation_atlas_allocations: 1,
    },
    FixtureSpec {
        id: "code-resize-after",
        trace_schema: "alpine-scene-trace/v2",
        trace_path: "assurance/qualification/v2/resize-after.toml",
        scene_trace_sha256: "37fa9af5414093610574563be68d18fe4f325ee1641f5bcee96f83b1540e617b",
        workload_hash: "92c29f19c2c2982a9bbf67b64ec5c6c2b4a307eb9558ca619d32c1476e574aa3",
        pair_id: "code-resize",
        pair_kind: "resize",
        pair_sequence_hash: "cedbf3c8a93f46d1236b6b8996cbde6f95f771bac2934598c1b07884fab0822a",
        pair_step: "1",
        pair_steps: "2",
        pixel_width: 72,
        pixel_height: 36,
        cpu_sha256: "121bc152a3c1eaf4de3fe31c0024647afab7ea07dd6f15c01b51ab378faf0a61",
        metal_sha256: "121bc152a3c1eaf4de3fe31c0024647afab7ea07dd6f15c01b51ab378faf0a61",
        physical_manifest_sha256: "ecb8f026f37b4492e92258b61b2da0b805fc8d3e7336a1df9fc16baed50a3bbd",
        max_channel_delta: 0,
        exact_pixel_equivalence: true,
        adaptation_clips: 1,
        adaptation_operations: 11,
        adaptation_quads: 4,
        adaptation_glyphs: 7,
        adaptation_resources: 1,
        adaptation_resource_bytes: 64,
        adaptation_atlas_allocations: 1,
    },
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "the fail-closed record mirrors independent external evidence flags"
)]
struct Evidence {
    schema: String,
    id: String,
    task: u32,
    claim: String,
    state: String,
    comparison_level: String,
    lab_revision: String,
    zed_revision: String,
    alpine_revision: String,
    trace_manifest_sha256: String,
    patch_series_sha256: String,
    fixture_count: usize,
    cpu_oracle_channel_tolerance: u8,
    adaptation_timing_performed: bool,
    renderer_timing_performed: bool,
    memory_performed: bool,
    performance_qualified: bool,
    hosted: HostedEvidence,
    physical: PhysicalEvidence,
    fixtures: Vec<FixtureEvidence>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HostedEvidence {
    state: String,
    run_id: u64,
    run_url: String,
    run_created_at_utc: String,
    head_branch: String,
    head_sha: String,
    artifact_id: u64,
    artifact_name: String,
    artifact_digest_sha256: String,
    artifact_created_at_utc: String,
    artifact_expires_at_utc: String,
    retention_days: u16,
    set_manifest_path: String,
    set_manifest_sha256: String,
    shader_mode: String,
    direct_metal_performed: bool,
    coverage_performed: bool,
    coverage_lines_covered: u32,
    coverage_lines_total: u32,
    coverage_functions_covered: u32,
    coverage_functions_total: u32,
    mutation_performed: bool,
    mutants_total: u32,
    mutants_caught: u32,
    mutants_unviable: u32,
    mutants_missed: u32,
    mutants_timed_out: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PhysicalEvidence {
    state: String,
    set_manifest_path: String,
    set_manifest_sha256: String,
    generated_at_utc: String,
    model: String,
    chip: String,
    memory_bytes: u64,
    os_version: String,
    os_build: String,
    architecture: String,
    shader_mode: String,
    direct_metal_performed: bool,
    coverage_performed: bool,
    mutation_performed: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "the fail-closed record preserves separate hosted and physical outcomes"
)]
struct FixtureEvidence {
    id: String,
    trace_schema: String,
    trace_path: String,
    scene_trace_sha256: String,
    workload_hash: String,
    pair_id: String,
    pair_kind: String,
    pair_sequence_hash: String,
    pair_step: String,
    pair_steps: String,
    pixel_width: u32,
    pixel_height: u32,
    hosted_cpu_oracle_sha256: String,
    hosted_gpui_metal_sha256: String,
    physical_cpu_oracle_sha256: String,
    physical_alpine_metal_sha256: String,
    physical_gpui_metal_sha256: String,
    physical_manifest_sha256: String,
    hosted_cpu_oracle_max_observed_channel_delta: u8,
    physical_cpu_oracle_max_observed_channel_delta: u8,
    hosted_cpu_oracle_equivalence_within_tolerance: bool,
    physical_cpu_oracle_equivalence_within_tolerance: bool,
    exact_pixel_equivalence: bool,
    exact_metal_equivalence: bool,
    adaptation_clips: u32,
    adaptation_operations: u32,
    adaptation_quads: u32,
    adaptation_glyphs: u32,
    adaptation_resources: u32,
    adaptation_resource_bytes: u64,
    adaptation_atlas_allocations: u32,
}

pub(crate) fn is_v2_evidence(path: &Path) -> bool {
    fs::read_to_string(path).is_ok_and(|source| {
        source
            .lines()
            .any(|line| line.trim() == format!("schema = {SCHEMA:?}"))
    })
}

pub(crate) fn run(command: &str, path: &Path) -> Result<String, Vec<String>> {
    let source = fs::read_to_string(path)
        .map_err(|error| vec![format!("failed to read {}: {error}", path.display())])?;
    let evidence: Evidence = toml::from_str(&source)
        .map_err(|error| vec![format!("failed to parse {}: {error}", path.display())])?;
    let errors = validate(&evidence);
    if !errors.is_empty() {
        return Err(errors);
    }

    match command {
        "validate-zed-lab-evidence" => Ok(format!(
            "validated task #61 with hosted offline GPUI and physical Direct Metal across {} fixtures",
            evidence.fixtures.len()
        )),
        "zed-lab-evidence-report" => Ok(render_report(&evidence)),
        other => Err(vec![format!(
            "unsupported Zed lab evidence command {other:?}"
        )]),
    }
}

fn validate(evidence: &Evidence) -> Vec<String> {
    let mut errors = Vec::new();
    require(
        &mut errors,
        evidence.schema == SCHEMA,
        "schema must remain version 2",
    );
    require(
        &mut errors,
        evidence.id == "task-61-realistic-scenes",
        "id must identify the task #61 realistic-scene record",
    );
    require(&mut errors, evidence.task == 61, "task must equal 61");
    require(
        &mut errors,
        evidence.claim == "AEP-0028-C04",
        "claim must equal AEP-0028-C04",
    );
    require(
        &mut errors,
        evidence.state == "equivalent",
        "state must be equivalent",
    );
    require(
        &mut errors,
        evidence.comparison_level == "renderer-only",
        "comparison level must remain renderer-only",
    );
    require(
        &mut errors,
        evidence.lab_revision == LAB_REVISION,
        "lab revision must match merged main",
    );
    require(
        &mut errors,
        evidence.zed_revision == ZED_REVISION,
        "Zed revision must match the accepted pin",
    );
    require(
        &mut errors,
        evidence.alpine_revision == ALPINE_REVISION,
        "Alpine revision must match the fixture source",
    );
    require(
        &mut errors,
        evidence.trace_manifest_sha256 == TRACE_MANIFEST_SHA256,
        "trace manifest hash must match the lab pin",
    );
    require(
        &mut errors,
        evidence.patch_series_sha256 == PATCH_SERIES_SHA256,
        "patch series hash must match the accepted adapter",
    );
    require(
        &mut errors,
        evidence.fixture_count == FIXTURES.len() && evidence.fixtures.len() == FIXTURES.len(),
        "fixture_count must equal the eight-fixture trace ladder",
    );
    require(
        &mut errors,
        evidence.cpu_oracle_channel_tolerance == 1,
        "CPU oracle channel tolerance must equal one",
    );
    require(
        &mut errors,
        !evidence.adaptation_timing_performed
            && !evidence.renderer_timing_performed
            && !evidence.memory_performed
            && !evidence.performance_qualified,
        "version 2 evidence cannot contain timing, memory, or performance claims",
    );
    validate_hosted(&evidence.hosted, &mut errors);
    validate_physical(&evidence.physical, &mut errors);

    let mut ids = BTreeSet::new();
    for (index, fixture) in evidence.fixtures.iter().enumerate() {
        let Some(spec) = FIXTURES.get(index) else {
            errors.push(format!("unexpected fixture {}", fixture.id));
            continue;
        };
        validate_fixture(fixture, *spec, &mut errors);
        require(
            &mut errors,
            ids.insert(fixture.id.as_str()),
            format!("fixture {} must be unique", fixture.id),
        );
    }
    errors
}

fn validate_hosted(hosted: &HostedEvidence, errors: &mut Vec<String>) {
    validate_hosted_identity(hosted, errors);
    validate_hosted_assurance(hosted, errors);
}

fn validate_hosted_identity(hosted: &HostedEvidence, errors: &mut Vec<String>) {
    require(
        errors,
        hosted.state == "gpui-oracle-equivalent",
        "hosted state must be GPUI-oracle-equivalent",
    );
    require(
        errors,
        hosted.run_id == 32_733_054_956,
        "hosted run id must match merged main",
    );
    require(
        errors,
        hosted.run_url == "https://github.com/dbuddha/alpine-zed-lab/actions/runs/32733054956",
        "hosted run URL must identify the merged-main run",
    );
    require(
        errors,
        hosted.run_created_at_utc == "2026-08-24T13:30:08Z",
        "hosted run creation must match GitHub",
    );
    require(
        errors,
        hosted.head_branch == "main",
        "hosted evidence must run from main",
    );
    require(
        errors,
        hosted.head_sha == LAB_REVISION,
        "hosted head must match the lab revision",
    );
    require(
        errors,
        hosted.artifact_id == 9_523_363_090,
        "hosted artifact id must match GitHub",
    );
    require(
        errors,
        hosted.artifact_name == format!("gpui-oracle-{LAB_REVISION}"),
        "hosted artifact name must bind the lab revision",
    );
    require(
        errors,
        hosted.artifact_digest_sha256 == HOSTED_ARTIFACT_SHA256,
        "hosted artifact digest must match GitHub",
    );
    require(
        errors,
        hosted.artifact_created_at_utc == "2026-08-24T14:04:23Z",
        "hosted artifact creation must match GitHub",
    );
    require(
        errors,
        hosted.artifact_expires_at_utc == "2026-11-22T13:30:08Z",
        "hosted artifact expiry must match GitHub",
    );
    require(
        errors,
        hosted.retention_days == 90,
        "hosted artifact must be retained for exactly 90 days",
    );
    require(
        errors,
        hosted.set_manifest_path == "assurance/lab/v2/source/hosted-qualification-set.toml",
        "hosted set manifest path must remain canonical",
    );
    require(
        errors,
        hosted.set_manifest_sha256 == HOSTED_SET_SHA256,
        "hosted set manifest hash must match",
    );
    require(
        errors,
        hosted.shader_mode == "offline-metallib",
        "hosted shader mode must be offline-metallib",
    );
    require(
        errors,
        !hosted.direct_metal_performed,
        "hosted evidence must not claim Alpine Direct Metal execution",
    );
}

fn validate_hosted_assurance(hosted: &HostedEvidence, errors: &mut Vec<String>) {
    require(
        errors,
        hosted.coverage_performed,
        "hosted evidence must include coverage",
    );
    require(
        errors,
        hosted.coverage_lines_covered == 1_142 && hosted.coverage_lines_total == 1_184,
        "hosted line coverage counts must match the artifact",
    );
    require(
        errors,
        hosted.coverage_functions_covered == 122 && hosted.coverage_functions_total == 134,
        "hosted function coverage counts must match the artifact",
    );
    require(
        errors,
        hosted.mutation_performed,
        "hosted evidence must include mutation analysis",
    );
    require(
        errors,
        hosted.mutants_total == 154
            && hosted.mutants_caught == 139
            && hosted.mutants_unviable == 15
            && hosted.mutants_missed == 0
            && hosted.mutants_timed_out == 0,
        "hosted mutation counts must classify every mutant with none missed or timed out",
    );
}

fn validate_physical(physical: &PhysicalEvidence, errors: &mut Vec<String>) {
    require(
        errors,
        physical.state == "equivalent",
        "physical state must be equivalent",
    );
    require(
        errors,
        physical.set_manifest_path == "assurance/lab/v2/source/physical-qualification-set.toml",
        "physical set manifest path must remain canonical",
    );
    require(
        errors,
        physical.set_manifest_sha256 == PHYSICAL_SET_SHA256,
        "physical set manifest hash must match",
    );
    require(
        errors,
        physical.generated_at_utc == "2026-08-24T13:35:39Z",
        "physical generation time must match",
    );
    require(
        errors,
        physical.model == "Mac16,1",
        "physical model must match the executed machine",
    );
    require(
        errors,
        physical.chip == "Apple M4",
        "physical chip must match the executed machine",
    );
    require(
        errors,
        physical.memory_bytes == 25_769_803_776,
        "physical memory identity must match",
    );
    require(
        errors,
        physical.os_version == "26.6.2" && physical.os_build == "25G83",
        "physical OS identity must match",
    );
    require(
        errors,
        physical.architecture == "arm64",
        "physical architecture must be arm64",
    );
    require(
        errors,
        physical.shader_mode == "runtime-source-unqualified",
        "physical shader mode must remain explicitly unqualified",
    );
    require(
        errors,
        physical.direct_metal_performed,
        "physical evidence must include Alpine Direct Metal",
    );
    require(
        errors,
        !physical.coverage_performed && !physical.mutation_performed,
        "physical evidence cannot inherit hosted assurance work",
    );
}

fn validate_fixture(fixture: &FixtureEvidence, spec: FixtureSpec, errors: &mut Vec<String>) {
    let identity_matches = fixture.id == spec.id
        && fixture.trace_schema == spec.trace_schema
        && fixture.trace_path == spec.trace_path
        && fixture.scene_trace_sha256 == spec.scene_trace_sha256
        && fixture.workload_hash == spec.workload_hash
        && fixture.pair_id == spec.pair_id
        && fixture.pair_kind == spec.pair_kind
        && fixture.pair_sequence_hash == spec.pair_sequence_hash
        && fixture.pair_step == spec.pair_step
        && fixture.pair_steps == spec.pair_steps
        && fixture.pixel_width == spec.pixel_width
        && fixture.pixel_height == spec.pixel_height;
    require(
        errors,
        identity_matches,
        format!(
            "fixture {} identity must match the canonical trace",
            spec.id
        ),
    );

    let outputs_match = fixture.hosted_cpu_oracle_sha256 == spec.cpu_sha256
        && fixture.hosted_gpui_metal_sha256 == spec.metal_sha256
        && fixture.physical_cpu_oracle_sha256 == spec.cpu_sha256
        && fixture.physical_alpine_metal_sha256 == spec.metal_sha256
        && fixture.physical_gpui_metal_sha256 == spec.metal_sha256
        && fixture.physical_manifest_sha256 == spec.physical_manifest_sha256;
    require(
        errors,
        outputs_match,
        format!(
            "fixture {} output identities must match hosted and physical artifacts",
            spec.id
        ),
    );
    require(
        errors,
        fixture.hosted_cpu_oracle_max_observed_channel_delta == spec.max_channel_delta
            && fixture.physical_cpu_oracle_max_observed_channel_delta == spec.max_channel_delta
            && fixture.hosted_cpu_oracle_equivalence_within_tolerance
            && fixture.physical_cpu_oracle_equivalence_within_tolerance,
        format!(
            "fixture {} must remain within the one-channel CPU tolerance",
            spec.id
        ),
    );
    require(
        errors,
        fixture.exact_pixel_equivalence == spec.exact_pixel_equivalence
            && fixture.exact_metal_equivalence,
        format!(
            "fixture {} exact-equivalence declarations must match the artifacts",
            spec.id
        ),
    );
    let adaptation_matches = fixture.adaptation_clips == spec.adaptation_clips
        && fixture.adaptation_operations == spec.adaptation_operations
        && fixture.adaptation_quads == spec.adaptation_quads
        && fixture.adaptation_glyphs == spec.adaptation_glyphs
        && fixture.adaptation_resources == spec.adaptation_resources
        && fixture.adaptation_resource_bytes == spec.adaptation_resource_bytes
        && fixture.adaptation_atlas_allocations == spec.adaptation_atlas_allocations;
    require(
        errors,
        adaptation_matches,
        format!("fixture {} adaptation accounting must match", spec.id),
    );
}

fn render_report(evidence: &Evidence) -> String {
    let mut report = String::from("# Zed GPUI realistic renderer evidence\n\n");
    let _ = writeln!(report, "- Lab revision: `{}`", evidence.lab_revision);
    let _ = writeln!(report, "- Zed revision: `{}`", evidence.zed_revision);
    let _ = writeln!(report, "- Alpine revision: `{}`", evidence.alpine_revision);
    let _ = writeln!(report, "- Hosted run: {}", evidence.hosted.run_url);
    let _ = writeln!(
        report,
        "- Hosted artifact digest: `{}`",
        evidence.hosted.artifact_digest_sha256
    );
    let _ = writeln!(
        report,
        "- Physical set digest: `{}`",
        evidence.physical.set_manifest_sha256
    );
    report.push_str("\n| Fixture | Trace | Size | Exact Metal | CPU max delta | Clips | Ops | Quads | Glyphs | Resource bytes |\n");
    report.push_str("| --- | --- | ---: | --- | ---: | ---: | ---: | ---: | ---: | ---: |\n");
    for fixture in &evidence.fixtures {
        let _ = writeln!(
            report,
            "| `{}` | `{}` | {}x{} | {} | {} | {} | {} | {} | {} | {} |",
            fixture.id,
            fixture.trace_schema,
            fixture.pixel_width,
            fixture.pixel_height,
            fixture.exact_metal_equivalence,
            fixture.physical_cpu_oracle_max_observed_channel_delta,
            fixture.adaptation_clips,
            fixture.adaptation_operations,
            fixture.adaptation_quads,
            fixture.adaptation_glyphs,
            fixture.adaptation_resource_bytes
        );
    }
    report.push_str("\nThe hosted offline-metallib run and physical runtime-source run compose through identical CPU and GPUI output identities. Alpine Direct Metal and GPUI Metal are exactly equal for every fixture. The clipped grid differs from the independent CPU oracle by at most one channel value and all other fixtures are exact.\n\nNo timing, memory, latency, presentation, product, or performance claim is present. Adaptation counts are retained separately from renderer execution.\n");
    report
}

fn require(errors: &mut Vec<String>, condition: bool, message: impl Into<String>) {
    if !condition {
        errors.push(message.into());
    }
}

#[cfg(test)]
mod tests {
    use super::{Evidence, render_report, validate};

    const VALID: &str = include_str!("../../../assurance/lab/v2/task-61-realistic-scenes.toml");

    fn errors_for(source: &str) -> Result<Vec<String>, toml::de::Error> {
        let evidence: Evidence = toml::from_str(source)?;
        Ok(validate(&evidence))
    }

    #[test]
    fn accepted_record_validates() -> Result<(), toml::de::Error> {
        assert!(errors_for(VALID)?.is_empty());
        Ok(())
    }

    #[test]
    fn report_preserves_claim_boundary() -> Result<(), toml::de::Error> {
        let evidence: Evidence = toml::from_str(VALID)?;
        let report = render_report(&evidence);
        assert!(report.contains("Exact Metal"));
        assert!(report.contains(
            "No timing, memory, latency, presentation, product, or performance claim is present"
        ));
        Ok(())
    }

    #[test]
    fn rejects_performance_claim() -> Result<(), toml::de::Error> {
        let changed = VALID.replacen(
            "performance_qualified = false",
            "performance_qualified = true",
            1,
        );
        assert!(
            errors_for(&changed)?
                .iter()
                .any(|error| error.contains("cannot contain timing"))
        );
        Ok(())
    }

    #[test]
    fn rejects_short_retention() -> Result<(), toml::de::Error> {
        let changed = VALID.replacen("retention_days = 90", "retention_days = 7", 1);
        assert!(
            errors_for(&changed)?
                .iter()
                .any(|error| error.contains("exactly 90 days"))
        );
        Ok(())
    }

    #[test]
    fn rejects_missed_mutant() -> Result<(), toml::de::Error> {
        let changed = VALID.replacen("mutants_missed = 0", "mutants_missed = 1", 1);
        assert!(
            errors_for(&changed)?
                .iter()
                .any(|error| error.contains("mutation counts"))
        );
        Ok(())
    }

    #[test]
    fn rejects_fixture_identity_drift() -> Result<(), toml::de::Error> {
        let changed = VALID.replacen("pair_kind = \"scroll\"", "pair_kind = \"resize\"", 1);
        assert!(
            errors_for(&changed)?
                .iter()
                .any(|error| error.contains("canonical trace"))
        );
        Ok(())
    }

    #[test]
    fn rejects_divergent_physical_output() -> Result<(), toml::de::Error> {
        let changed = VALID.replacen(
            "physical_alpine_metal_sha256 = \"074cadbffac89c52b3d03f54208ee2cd419828855233d0263941404c1026e8c2\"",
            "physical_alpine_metal_sha256 = \"0000000000000000000000000000000000000000000000000000000000000000\"",
            1,
        );
        assert!(
            errors_for(&changed)?
                .iter()
                .any(|error| error.contains("output identities"))
        );
        Ok(())
    }

    #[test]
    fn unknown_fields_fail_to_parse() {
        let changed = format!("{VALID}\nunknown_claim = true\n");
        assert!(toml::from_str::<Evidence>(&changed).is_err());
    }
}
