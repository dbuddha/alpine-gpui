use std::{
    collections::HashMap,
    fmt::Write as _,
    fs,
    io::ErrorKind,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use alpine_metal::{BackendState, MetalBackend};
use alpine_trace::{
    DecodedTrace, TraceSequenceAtlas, TraceSequenceInput, TraceSequenceStep,
    TraceSequenceTransition,
};
use serde::Deserialize;

use crate::qualification;

const SCHEMA: &str = "alpine-scene-trace-sequence/v1";
const EVIDENCE_SCHEMA: &str = "alpine-scene-trace-sequence-evidence/v1";
const MAX_MANIFEST_BYTES: usize = 64 * 1024;
const MAX_PATH_BYTES: usize = 4 * 1024;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SequenceManifest {
    schema: String,
    id: String,
    task: u64,
    comparison_level: String,
    cpu_oracle_channel_tolerance: u8,
    renderer_timing_performed: bool,
    memory_claim_performed: bool,
    steps: Vec<SequenceStep>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SequenceStep {
    sequence: u64,
    transition: String,
    renderer_generation: u64,
    scene: Option<String>,
    workload_hash: Option<String>,
    resource_id: Option<String>,
    resource_revision: Option<u64>,
    atlas_width: Option<u32>,
    atlas_height: Option<u32>,
    content_hash: Option<String>,
    expected_atlas_upload_bytes: usize,
    expected_terminal_retained_bytes: usize,
    expected_cpu_bytes: usize,
}

#[derive(Deserialize)]
struct SceneProjection {
    schema: String,
    workload_hash: String,
    resources: Vec<ResourceProjection>,
}

#[derive(Deserialize)]
struct ResourceProjection {
    id: String,
    kind: String,
    content_hash: String,
    revision: Option<u64>,
    width: Option<u32>,
    height: Option<u32>,
    pixels: Option<Vec<u8>>,
}

struct ValidatedSequence {
    id: String,
    tolerance: u8,
    summary: alpine_trace::TraceSequenceSummary,
    steps: Vec<ValidatedStep>,
}

struct ValidatedStep {
    sequence: u64,
    transition: TraceSequenceTransition,
    renderer_generation: u64,
    expected_atlas_upload_bytes: usize,
    expected_terminal_retained_bytes: usize,
    decoded: Option<Arc<DecodedTrace>>,
    reference: Option<alpine_metal::Bgra8Image>,
}

pub(crate) fn validate(manifest: &Path, root: &Path) -> Result<String, Vec<String>> {
    let validated = load_validated(manifest, root)?;
    Ok(format!(
        "validated trace sequence {} with {} visible steps, {} renderer generations, and {} atlas upload bytes",
        validated.id,
        validated.summary.visible_steps(),
        validated.summary.renderer_generations(),
        validated.summary.atlas_upload_bytes()
    ))
}

#[allow(
    clippy::too_many_lines,
    reason = "ordered renderer ownership, evidence, and terminal drain remain one audited lifecycle transaction"
)]
pub(crate) fn render_native(
    manifest: &Path,
    output: &Path,
    root: &Path,
) -> Result<String, Vec<String>> {
    invalidate_output(output)?;
    let validated = load_validated(manifest, root)?;
    let mut backend: Option<MetalBackend> = None;
    let mut evidence = format!(
        "schema = {EVIDENCE_SCHEMA:?}\nid = {:?}\ncomparison_level = \"renderer-only\"\nrenderer_timing_performed = false\nmemory_claim_performed = false\natlas_allocation_identity_observed = false\natlas_allocation_omission = \"The safe offscreen report exposes exact bytes but not native resource handles or allocation identities.\"\n",
        validated.id
    );

    for step in &validated.steps {
        if step.transition == TraceSequenceTransition::Teardown {
            let mut owned = backend
                .take()
                .ok_or_else(|| vec!["trace sequence teardown has no renderer owner".to_owned()])?;
            owned.shutdown();
            let accounting = owned.accounting();
            if accounting.state() != BackendState::Stopped
                || accounting.current_retained_bytes() != step.expected_terminal_retained_bytes
                || !accounting.invariants_hold()
            {
                return Err(vec![
                    "trace sequence teardown did not drain renderer ownership".to_owned(),
                ]);
            }
            let _ = write!(
                evidence,
                "\n[[steps]]\nsequence = {}\ntransition = \"teardown\"\nlogical_renderer_generation = {}\nterminal_retained_bytes = {}\n",
                step.sequence,
                step.renderer_generation,
                accounting.current_retained_bytes()
            );
            continue;
        }

        if matches!(
            step.transition,
            TraceSequenceTransition::FullAdmission | TraceSequenceTransition::FullResynchronization
        ) {
            if backend.is_some() {
                return Err(vec![
                    "trace sequence attempted renderer construction while an owner was live"
                        .to_owned(),
                ]);
            }
            backend = Some(
                MetalBackend::new()
                    .map_err(|error| vec![format!("cannot initialize Direct Metal: {error}")])?,
            );
        }
        let owner = backend
            .as_mut()
            .ok_or_else(|| vec!["trace sequence visible step has no renderer owner".to_owned()])?;
        let decoded = step
            .decoded
            .as_ref()
            .ok_or_else(|| vec!["trace sequence visible step has no decoded scene".to_owned()])?;
        let reference = step.reference.as_ref().ok_or_else(|| {
            vec!["trace sequence visible step has no CPU oracle image".to_owned()]
        })?;
        let frame = owner
            .render_offscreen(decoded.scene(), decoded.descriptor())
            .map_err(|error| {
                vec![format!(
                    "Direct Metal trace sequence render failed: {error}"
                )]
            })?;
        let report = frame.report();
        let delta = max_channel_delta(reference.bytes(), frame.image().bytes())
            .ok_or_else(|| vec!["trace sequence Metal image length mismatch".to_owned()])?;
        let accounting = owner.accounting();
        if delta > validated.tolerance {
            return Err(vec![format!(
                "trace sequence step {} exceeds CPU oracle tolerance: {delta} > {}",
                step.sequence, validated.tolerance
            )]);
        }
        if report.atlas_upload_bytes != step.expected_atlas_upload_bytes {
            return Err(vec![format!(
                "trace sequence step {} uploaded {} atlas bytes, expected {}",
                step.sequence, report.atlas_upload_bytes, step.expected_atlas_upload_bytes
            )]);
        }
        if accounting.current_retained_bytes() != step.expected_terminal_retained_bytes
            || !accounting.invariants_hold()
        {
            return Err(vec![format!(
                "trace sequence step {} did not reach balanced terminal ownership",
                step.sequence
            )]);
        }
        let _ = write!(
            evidence,
            "\n[[steps]]\nsequence = {}\ntransition = {:?}\nlogical_renderer_generation = {}\nbackend_generation = {}\nsubmission = {}\ncpu_bytes = {}\ngpu_allocated_bytes = {}\natlas_upload_bytes = {}\nterminal_retained_bytes = {}\nmax_cpu_oracle_channel_delta = {}\nsemantic_and_pixel_equivalent = true\n",
            step.sequence,
            transition_name(step.transition),
            step.renderer_generation,
            accounting.generation().get(),
            report.submission,
            reference.bytes().len(),
            report.allocated_bytes,
            report.atlas_upload_bytes,
            accounting.current_retained_bytes(),
            delta
        );
    }
    if let Some(mut owner) = backend {
        owner.shutdown();
        if owner.accounting().current_retained_bytes() != 0 || !owner.accounting().invariants_hold()
        {
            return Err(vec![
                "trace sequence final renderer owner did not drain".to_owned(),
            ]);
        }
    }
    fs::write(output, evidence)
        .map_err(|error| vec![format!("cannot write {}: {error}", output.display())])?;
    Ok(format!(
        "rendered trace sequence {} through Direct Metal to {}",
        validated.id,
        output.display()
    ))
}

#[allow(
    clippy::too_many_lines,
    reason = "manifest projection, independent oracle construction, and semantic admission remain one fail-closed audit transaction"
)]
fn load_validated(manifest: &Path, root: &Path) -> Result<ValidatedSequence, Vec<String>> {
    let manifest: SequenceManifest = load_toml(manifest)?;
    let mut errors = Vec::new();
    require(
        &mut errors,
        manifest.schema == SCHEMA,
        "trace sequence schema must be exact",
    );
    require(
        &mut errors,
        valid_slug(&manifest.id),
        "trace sequence id must be a slug",
    );
    require(
        &mut errors,
        manifest.task == 353,
        "trace sequence must bind Task #353",
    );
    require(
        &mut errors,
        manifest.comparison_level == "renderer-only",
        "trace sequence comparison level must be renderer-only",
    );
    require(
        &mut errors,
        manifest.cpu_oracle_channel_tolerance <= 1,
        "trace sequence CPU oracle tolerance must be at most one channel value",
    );
    require(
        &mut errors,
        !manifest.renderer_timing_performed,
        "trace sequence cannot contain renderer timing",
    );
    require(
        &mut errors,
        !manifest.memory_claim_performed,
        "trace sequence cannot contain a memory claim",
    );

    let mut semantic_steps = Vec::new();
    let mut validated_steps = Vec::new();
    let mut decoded_scenes = HashMap::<PathBuf, Arc<DecodedTrace>>::new();
    let mut resource_id: Option<&str> = None;
    if semantic_steps
        .try_reserve_exact(manifest.steps.len())
        .is_err()
        || validated_steps
            .try_reserve_exact(manifest.steps.len())
            .is_err()
    {
        return Err(vec!["trace sequence step allocation failed".to_owned()]);
    }
    for step in &manifest.steps {
        let transition = parse_transition(&step.transition)?;
        let is_teardown = transition == TraceSequenceTransition::Teardown;
        let mut decoded = None;
        let mut reference = None;
        let mut semantic_atlas = None;
        let mut semantic_workload = None;
        if is_teardown {
            require(
                &mut errors,
                step.scene.is_none(),
                "teardown cannot reference a scene",
            );
            require(
                &mut errors,
                step.expected_cpu_bytes == 0,
                "teardown cannot retain CPU image bytes",
            );
        } else {
            let scene = step
                .scene
                .as_deref()
                .ok_or_else(|| vec![format!("step {} requires a scene", step.sequence)])?;
            let scene_path = resolve_repository_path(root, scene)?;
            let projection: SceneProjection = load_toml(&scene_path)?;
            require(
                &mut errors,
                projection.schema == "alpine-scene-trace/v2",
                format!("step {} must reference a version 2 scene", step.sequence),
            );
            let workload = step
                .workload_hash
                .as_deref()
                .ok_or_else(|| vec![format!("step {} requires a workload hash", step.sequence)])?;
            require(
                &mut errors,
                projection.workload_hash == workload,
                format!(
                    "step {} workload hash drifted from its scene",
                    step.sequence
                ),
            );
            semantic_workload = Some(parse_sha256(workload)?);
            require(
                &mut errors,
                projection.resources.len() == 1,
                format!("step {} must own exactly one atlas resource", step.sequence),
            );
            let resource = projection
                .resources
                .first()
                .ok_or_else(|| vec![format!("step {} has no atlas resource", step.sequence)])?;
            let declared_resource = step
                .resource_id
                .as_deref()
                .ok_or_else(|| vec![format!("step {} requires a resource id", step.sequence)])?;
            if let Some(expected) = resource_id {
                require(
                    &mut errors,
                    expected == declared_resource,
                    format!("step {} changed atlas resource identity", step.sequence),
                );
            } else {
                resource_id = Some(declared_resource);
            }
            require(
                &mut errors,
                resource.id == declared_resource && resource.kind == "a8-atlas",
                format!("step {} atlas resource identity drifted", step.sequence),
            );
            let revision = step.resource_revision.ok_or_else(|| {
                vec![format!(
                    "step {} requires a resource revision",
                    step.sequence
                )]
            })?;
            let width = step
                .atlas_width
                .ok_or_else(|| vec![format!("step {} requires an atlas width", step.sequence)])?;
            let height = step
                .atlas_height
                .ok_or_else(|| vec![format!("step {} requires an atlas height", step.sequence)])?;
            let content_hash = step
                .content_hash
                .as_deref()
                .ok_or_else(|| vec![format!("step {} requires a content hash", step.sequence)])?;
            require(
                &mut errors,
                resource.revision == Some(revision)
                    && resource.width == Some(width)
                    && resource.height == Some(height)
                    && resource.content_hash == content_hash,
                format!(
                    "step {} atlas metadata drifted from its scene",
                    step.sequence
                ),
            );
            let pixels = resource.pixels.as_ref().map_or(0, Vec::len);
            require(
                &mut errors,
                usize::try_from(width).ok().and_then(|width| {
                    usize::try_from(height)
                        .ok()
                        .and_then(|height| width.checked_mul(height))
                }) == Some(pixels),
                format!("step {} atlas pixels are partial", step.sequence),
            );
            semantic_atlas = Some(TraceSequenceAtlas {
                identity: 1,
                revision,
                width,
                height,
                content_hash: parse_sha256(content_hash)?,
            });
            let decoded_scene = if let Some(decoded) = decoded_scenes.get(&scene_path) {
                Arc::clone(decoded)
            } else {
                let decoded = Arc::new(qualification::decode_scene_file(&scene_path)?);
                decoded_scenes.insert(scene_path, Arc::clone(&decoded));
                decoded
            };
            let frame = decoded_scene.validated_frame().map_err(|error| {
                vec![format!(
                    "step {} frame validation failed: {error}",
                    step.sequence
                )]
            })?;
            let image = frame.reference_image().map_err(|error| {
                vec![format!("step {} CPU oracle failed: {error}", step.sequence)]
            })?;
            require(
                &mut errors,
                image.bytes().len() == step.expected_cpu_bytes,
                format!("step {} CPU image byte count drifted", step.sequence),
            );
            decoded = Some(decoded_scene);
            reference = Some(image);
        }
        semantic_steps.push(TraceSequenceStep {
            sequence: step.sequence,
            transition,
            workload_hash: semantic_workload,
            renderer_generation: step.renderer_generation,
            atlas: semantic_atlas,
            expected_atlas_upload_bytes: step.expected_atlas_upload_bytes,
            expected_terminal_retained_bytes: step.expected_terminal_retained_bytes,
        });
        validated_steps.push(ValidatedStep {
            sequence: step.sequence,
            transition,
            renderer_generation: step.renderer_generation,
            expected_atlas_upload_bytes: step.expected_atlas_upload_bytes,
            expected_terminal_retained_bytes: step.expected_terminal_retained_bytes,
            decoded,
            reference,
        });
    }
    if !errors.is_empty() {
        errors.sort();
        return Err(errors);
    }
    let summary = TraceSequenceInput {
        steps: semantic_steps,
    }
    .validate()
    .map_err(|error| {
        vec![format!(
            "trace sequence semantic validation failed: {error}"
        )]
    })?;
    Ok(ValidatedSequence {
        id: manifest.id,
        tolerance: manifest.cpu_oracle_channel_tolerance,
        summary,
        steps: validated_steps,
    })
}

fn load_toml<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, Vec<String>> {
    let bytes =
        fs::read(path).map_err(|error| vec![format!("cannot read {}: {error}", path.display())])?;
    if bytes.len() > MAX_MANIFEST_BYTES {
        return Err(vec![format!(
            "{} exceeds the manifest byte limit",
            path.display()
        )]);
    }
    let source = std::str::from_utf8(&bytes)
        .map_err(|_| vec![format!("{} is not UTF-8", path.display())])?;
    toml::from_str(source)
        .map_err(|error| vec![format!("cannot parse {}: {error}", path.display())])
}

fn resolve_repository_path(root: &Path, value: &str) -> Result<PathBuf, Vec<String>> {
    if value.len() > MAX_PATH_BYTES {
        return Err(vec![
            "trace sequence scene path exceeds the byte limit".to_owned(),
        ]);
    }
    let relative = Path::new(value);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(vec![format!(
            "trace sequence scene path {value:?} is invalid"
        )]);
    }
    let canonical_root = fs::canonicalize(root)
        .map_err(|error| vec![format!("cannot canonicalize {}: {error}", root.display())])?;
    let mut candidate = canonical_root.clone();
    for component in relative.components() {
        let Component::Normal(component) = component else {
            return Err(vec![format!(
                "trace sequence scene path {value:?} is invalid"
            )]);
        };
        candidate.push(component);
        let metadata = fs::symlink_metadata(&candidate).map_err(|error| {
            vec![format!(
                "cannot inspect trace sequence path {}: {error}",
                candidate.display()
            )]
        })?;
        if metadata.file_type().is_symlink() {
            return Err(vec![format!(
                "trace sequence scene path {} contains a symlink",
                candidate.display()
            )]);
        }
    }
    let canonical = fs::canonicalize(&candidate).map_err(|error| {
        vec![format!(
            "cannot canonicalize trace sequence path {}: {error}",
            candidate.display()
        )]
    })?;
    if !canonical.starts_with(&canonical_root) || !canonical.is_file() {
        return Err(vec![format!(
            "trace sequence scene path {value:?} is not a file"
        )]);
    }
    Ok(canonical)
}

fn parse_transition(value: &str) -> Result<TraceSequenceTransition, Vec<String>> {
    match value {
        "full-admission" => Ok(TraceSequenceTransition::FullAdmission),
        "compatible-reuse" => Ok(TraceSequenceTransition::CompatibleReuse),
        "content-replacement" => Ok(TraceSequenceTransition::ContentReplacement),
        "capacity-replacement" => Ok(TraceSequenceTransition::CapacityReplacement),
        "teardown" => Ok(TraceSequenceTransition::Teardown),
        "full-resynchronization" => Ok(TraceSequenceTransition::FullResynchronization),
        other => Err(vec![format!(
            "unsupported trace sequence transition {other:?}"
        )]),
    }
}

const fn transition_name(value: TraceSequenceTransition) -> &'static str {
    match value {
        TraceSequenceTransition::FullAdmission => "full-admission",
        TraceSequenceTransition::CompatibleReuse => "compatible-reuse",
        TraceSequenceTransition::ContentReplacement => "content-replacement",
        TraceSequenceTransition::CapacityReplacement => "capacity-replacement",
        TraceSequenceTransition::Teardown => "teardown",
        TraceSequenceTransition::FullResynchronization => "full-resynchronization",
    }
}

fn parse_sha256(value: &str) -> Result<[u8; 32], Vec<String>> {
    if value.len() != 64 {
        return Err(vec![format!("invalid SHA-256 identity {value:?}")]);
    }
    let mut output = [0_u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        let high = hex_nibble(pair[0])
            .ok_or_else(|| vec![format!("invalid SHA-256 identity {value:?}")])?;
        let low = hex_nibble(pair[1])
            .ok_or_else(|| vec![format!("invalid SHA-256 identity {value:?}")])?;
        output[index] = (high << 4) | low;
    }
    Ok(output)
}

const fn hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        _ => None,
    }
}

fn max_channel_delta(left: &[u8], right: &[u8]) -> Option<u8> {
    if left.len() != right.len() {
        return None;
    }
    left.iter()
        .zip(right)
        .map(|(left, right)| left.abs_diff(*right))
        .max()
        .or(Some(0))
}

fn invalidate_output(output: &Path) -> Result<(), Vec<String>> {
    match fs::remove_file(output) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(vec![format!(
            "cannot invalidate prior output {}: {error}",
            output.display()
        )]),
    }
}

fn require(errors: &mut Vec<String>, condition: bool, message: impl Into<String>) {
    if !condition {
        errors.push(message.into());
    }
}

fn valid_slug(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{SequenceManifest, load_validated, parse_sha256};

    const VALID: &str =
        include_str!("../../../assurance/qualification/sequences/atlas-lifecycle-v1.toml");

    #[test]
    fn canonical_sequence_reaches_every_cpu_oracle_without_claiming_performance() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .unwrap_or_else(|| Path::new("."));
        let validated = load_validated(
            &root.join("assurance/qualification/sequences/atlas-lifecycle-v1.toml"),
            root,
        );
        assert!(validated.is_ok());
        if let Ok(validated) = validated {
            assert_eq!(validated.summary.visible_steps(), 5);
            assert_eq!(validated.summary.renderer_generations(), 2);
            assert_eq!(validated.summary.atlas_upload_bytes(), 24);
        }
    }

    #[test]
    fn sequence_parser_rejects_unknown_fields_and_invalid_hashes() {
        let unknown = VALID.replacen("task = 353", "task = 353\nunknown = true", 1);
        assert!(toml::from_str::<SequenceManifest>(&unknown).is_err());
        assert!(parse_sha256(&"0".repeat(64)).is_ok());
        assert!(parse_sha256(&"g".repeat(64)).is_err());
        assert!(parse_sha256("00").is_err());
    }
}
