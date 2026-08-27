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
const MAX_PATH_BYTES: usize = 4_096;

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

const fn transition_is_teardown(transition: TraceSequenceTransition) -> bool {
    matches!(transition, TraceSequenceTransition::Teardown)
}

const fn transition_starts_renderer_generation(transition: TraceSequenceTransition) -> bool {
    matches!(
        transition,
        TraceSequenceTransition::FullAdmission | TraceSequenceTransition::FullResynchronization
    )
}

const fn owns_exactly_one_atlas(resource_count: usize) -> bool {
    resource_count == 1
}

fn atlas_pixels_match_dimensions(width: u32, height: u32, pixel_count: usize) -> bool {
    usize::try_from(width).ok().and_then(|width| {
        usize::try_from(height)
            .ok()
            .and_then(|height| width.checked_mul(height))
    }) == Some(pixel_count)
}

fn ownership_is_drained(
    state: BackendState,
    retained_bytes: usize,
    expected_retained_bytes: usize,
    invariants_hold: bool,
) -> bool {
    state == BackendState::Stopped && retained_bytes == expected_retained_bytes && invariants_hold
}

fn require_drained_ownership(
    state: BackendState,
    retained_bytes: usize,
    expected_retained_bytes: usize,
    invariants_hold: bool,
    error: &str,
) -> Result<(), Vec<String>> {
    if ownership_is_drained(
        state,
        retained_bytes,
        expected_retained_bytes,
        invariants_hold,
    ) {
        Ok(())
    } else {
        Err(vec![error.to_owned()])
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "one immutable frame observation keeps every compared evidence axis explicit"
)]
fn frame_observation_error(
    sequence: u64,
    channel_delta: u8,
    tolerance: u8,
    uploaded_bytes: usize,
    expected_uploaded_bytes: usize,
    retained_bytes: usize,
    expected_retained_bytes: usize,
    invariants_hold: bool,
) -> Option<String> {
    if channel_delta > tolerance {
        return Some(format!(
            "trace sequence step {sequence} exceeds CPU oracle tolerance: {channel_delta} > {tolerance}"
        ));
    }
    if uploaded_bytes != expected_uploaded_bytes {
        return Some(format!(
            "trace sequence step {sequence} uploaded {uploaded_bytes} atlas bytes, expected {expected_uploaded_bytes}"
        ));
    }
    if retained_bytes != expected_retained_bytes || !invariants_hold {
        return Some(format!(
            "trace sequence step {sequence} did not reach balanced terminal ownership"
        ));
    }
    None
}

fn reserve_step_storage(
    count: usize,
) -> Result<(Vec<TraceSequenceStep>, Vec<ValidatedStep>), Vec<String>> {
    let mut semantic = Vec::new();
    semantic
        .try_reserve_exact(count)
        .map_err(|_| vec!["trace sequence step allocation failed".to_owned()])?;
    let mut validated = Vec::new();
    validated
        .try_reserve_exact(count)
        .map_err(|_| vec!["trace sequence step allocation failed".to_owned()])?;
    Ok((semantic, validated))
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
        if transition_is_teardown(step.transition) {
            let mut owned = backend
                .take()
                .ok_or_else(|| vec!["trace sequence teardown has no renderer owner".to_owned()])?;
            owned.shutdown();
            let accounting = owned.accounting();
            require_drained_ownership(
                accounting.state(),
                accounting.current_retained_bytes(),
                step.expected_terminal_retained_bytes,
                accounting.invariants_hold(),
                "trace sequence teardown did not drain renderer ownership",
            )?;
            let _ = write!(
                evidence,
                "\n[[steps]]\nsequence = {}\ntransition = \"teardown\"\nlogical_renderer_generation = {}\nterminal_retained_bytes = {}\n",
                step.sequence,
                step.renderer_generation,
                accounting.current_retained_bytes()
            );
            continue;
        }

        if transition_starts_renderer_generation(step.transition) {
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
        if let Some(error) = frame_observation_error(
            step.sequence,
            delta,
            validated.tolerance,
            report.atlas_upload_bytes,
            step.expected_atlas_upload_bytes,
            accounting.current_retained_bytes(),
            step.expected_terminal_retained_bytes,
            accounting.invariants_hold(),
        ) {
            return Err(vec![error]);
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
        require_drained_ownership(
            owner.accounting().state(),
            owner.accounting().current_retained_bytes(),
            0,
            owner.accounting().invariants_hold(),
            "trace sequence final renderer owner did not drain",
        )?;
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

    let (mut semantic_steps, mut validated_steps) = reserve_step_storage(manifest.steps.len())?;
    let mut decoded_scenes = HashMap::<PathBuf, Arc<DecodedTrace>>::new();
    let mut resource_id: Option<&str> = None;
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
                owns_exactly_one_atlas(projection.resources.len()),
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
                atlas_pixels_match_dimensions(width, height, pixels),
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
    if relative
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
        output[index] = high * 16 + low;
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
#[allow(
    clippy::manual_let_else,
    clippy::panic,
    clippy::unwrap_used,
    reason = "isolated fixture construction and negative-path extraction must fail the owning test immediately"
)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
    };

    use alpine_metal::BackendState;
    use alpine_trace::TraceSequenceTransition;
    use toml::Value;

    use super::{
        MAX_MANIFEST_BYTES, MAX_PATH_BYTES, SequenceManifest, atlas_pixels_match_dimensions,
        frame_observation_error, invalidate_output, load_toml, load_validated, max_channel_delta,
        ownership_is_drained, owns_exactly_one_atlas, parse_sha256, parse_transition,
        render_native, require, reserve_step_storage, resolve_repository_path,
        transition_is_teardown, transition_name, transition_starts_renderer_generation, valid_slug,
        validate,
    };

    const VALID: &str =
        include_str!("../../../assurance/qualification/sequences/atlas-lifecycle-v1.toml");
    const INITIAL: &str =
        include_str!("../../../assurance/qualification/v2/atlas-lifecycle-initial.toml");
    const CONTENT: &str =
        include_str!("../../../assurance/qualification/v2/atlas-lifecycle-content.toml");
    const CAPACITY: &str =
        include_str!("../../../assurance/qualification/v2/atlas-lifecycle-capacity.toml");
    const MANIFEST_RELATIVE: &str = "assurance/qualification/sequences/atlas-lifecycle-v1.toml";
    const INITIAL_RELATIVE: &str = "assurance/qualification/v2/atlas-lifecycle-initial.toml";
    const CONTENT_RELATIVE: &str = "assurance/qualification/v2/atlas-lifecycle-content.toml";
    const CAPACITY_RELATIVE: &str = "assurance/qualification/v2/atlas-lifecycle-capacity.toml";
    static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    struct FixtureRepository {
        root: PathBuf,
    }

    impl FixtureRepository {
        fn new() -> Self {
            loop {
                let identity = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
                let root = std::env::temp_dir().join(format!(
                    "alpine-trace-sequence-{}-{identity}",
                    std::process::id()
                ));
                match fs::create_dir(&root) {
                    Ok(()) => {
                        let fixture = Self { root };
                        fixture.reset();
                        return fixture;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(error) => panic!("cannot create fixture repository: {error}"),
                }
            }
        }

        fn manifest(&self) -> PathBuf {
            self.root.join(MANIFEST_RELATIVE)
        }

        fn reset(&self) {
            self.write(MANIFEST_RELATIVE, VALID);
            self.write(INITIAL_RELATIVE, INITIAL);
            self.write(CONTENT_RELATIVE, CONTENT);
            self.write(CAPACITY_RELATIVE, CAPACITY);
        }

        fn write(&self, relative: &str, source: &str) {
            let path = self.root.join(relative);
            fs::create_dir_all(path.parent().unwrap_or(&self.root))
                .unwrap_or_else(|error| panic!("cannot create fixture parents: {error}"));
            fs::write(&path, source)
                .unwrap_or_else(|error| panic!("cannot write {}: {error}", path.display()));
        }

        fn write_manifest(&self, value: &Value) {
            let source = toml::to_string(value)
                .unwrap_or_else(|error| panic!("cannot serialize manifest: {error}"));
            self.write(MANIFEST_RELATIVE, &source);
        }

        fn rejection(&self) -> Vec<String> {
            match load_validated(&self.manifest(), &self.root) {
                Ok(_) => panic!("fixture unexpectedly passed validation"),
                Err(errors) => errors,
            }
        }
    }

    impl Drop for FixtureRepository {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn manifest_value() -> Value {
        toml::from_str(VALID).unwrap_or_else(|error| panic!("invalid canonical manifest: {error}"))
    }

    fn steps_mut(value: &mut Value) -> &mut Vec<Value> {
        value
            .get_mut("steps")
            .and_then(Value::as_array_mut)
            .unwrap_or_else(|| panic!("canonical manifest has no steps"))
    }

    fn set_step(value: &mut Value, index: usize, key: &str, replacement: Option<Value>) {
        let table = steps_mut(value)[index]
            .as_table_mut()
            .unwrap_or_else(|| panic!("step {index} is not a table"));
        if let Some(replacement) = replacement {
            table.insert(key.to_owned(), replacement);
        } else {
            table.remove(key);
        }
    }

    fn assert_rejects(errors: &[String], expected: &str) {
        assert!(
            errors.iter().any(|error| error.contains(expected)),
            "expected {expected:?} in {errors:?}"
        );
    }

    fn reject_source(repository: &FixtureRepository, source: &str, expected: &str) {
        repository.write(MANIFEST_RELATIVE, source);
        assert_rejects(&repository.rejection(), expected);
    }

    fn reject_step(
        repository: &FixtureRepository,
        index: usize,
        key: &str,
        replacement: Option<Value>,
        expected: &str,
    ) {
        let mut value = manifest_value();
        set_step(&mut value, index, key, replacement);
        repository.write_manifest(&value);
        assert_rejects(&repository.rejection(), expected);
    }

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
        assert_eq!(
            validate(
                &root.join("assurance/qualification/sequences/atlas-lifecycle-v1.toml"),
                root
            ),
            Ok("validated trace sequence editor-atlas-lifecycle with 5 visible steps, 2 renderer generations, and 24 atlas upload bytes".to_owned())
        );
    }

    #[test]
    fn sequence_parser_rejects_unknown_fields_and_invalid_hashes() {
        let unknown = VALID.replacen("task = 353", "task = 353\nunknown = true", 1);
        assert!(toml::from_str::<SequenceManifest>(&unknown).is_err());
        assert_eq!(parse_sha256(&"0".repeat(64)), Ok([0; 32]));
        assert_eq!(
            parse_sha256(&"09af".repeat(16)),
            Ok([
                0x09, 0xaf, 0x09, 0xaf, 0x09, 0xaf, 0x09, 0xaf, 0x09, 0xaf, 0x09, 0xaf, 0x09, 0xaf,
                0x09, 0xaf, 0x09, 0xaf, 0x09, 0xaf, 0x09, 0xaf, 0x09, 0xaf, 0x09, 0xaf, 0x09, 0xaf,
                0x09, 0xaf, 0x09, 0xaf,
            ])
        );
        assert!(parse_sha256(&"A".repeat(64)).is_err());
        assert!(parse_sha256(&"g".repeat(64)).is_err());
        assert!(parse_sha256("00").is_err());
    }

    #[test]
    fn manifest_identity_and_claim_axes_are_independently_rejected() {
        let repository = FixtureRepository::new();
        let cases = [
            (
                "schema = \"alpine-scene-trace-sequence/v1\"",
                "schema = \"wrong\"",
                "schema must be exact",
            ),
            (
                "id = \"editor-atlas-lifecycle\"",
                "id = \"Editor\"",
                "id must be a slug",
            ),
            ("task = 353", "task = 352", "must bind Task #353"),
            (
                "comparison_level = \"renderer-only\"",
                "comparison_level = \"journey\"",
                "comparison level must be renderer-only",
            ),
            (
                "cpu_oracle_channel_tolerance = 1",
                "cpu_oracle_channel_tolerance = 2",
                "tolerance must be at most one",
            ),
            (
                "renderer_timing_performed = false",
                "renderer_timing_performed = true",
                "cannot contain renderer timing",
            ),
            (
                "memory_claim_performed = false",
                "memory_claim_performed = true",
                "cannot contain a memory claim",
            ),
        ];
        for (from, to, expected) in cases {
            reject_source(&repository, &VALID.replacen(from, to, 1), expected);
        }
    }

    #[test]
    fn required_step_fields_and_teardown_omissions_fail_closed() {
        let repository = FixtureRepository::new();
        let string = |value: &str| Some(Value::String(value.to_owned()));
        reject_step(
            &repository,
            0,
            "transition",
            string("unknown"),
            "unsupported",
        );
        reject_step(&repository, 0, "scene", None, "requires a scene");
        reject_step(
            &repository,
            0,
            "workload_hash",
            None,
            "requires a workload hash",
        );
        reject_step(
            &repository,
            0,
            "resource_id",
            None,
            "requires a resource id",
        );
        reject_step(
            &repository,
            0,
            "resource_revision",
            None,
            "requires a resource revision",
        );
        reject_step(
            &repository,
            0,
            "atlas_width",
            None,
            "requires an atlas width",
        );
        reject_step(
            &repository,
            0,
            "atlas_height",
            None,
            "requires an atlas height",
        );
        reject_step(
            &repository,
            0,
            "content_hash",
            None,
            "requires a content hash",
        );
        reject_step(
            &repository,
            0,
            "expected_cpu_bytes",
            Some(Value::Integer(255)),
            "CPU image byte count drifted",
        );
        reject_step(
            &repository,
            4,
            "scene",
            string(INITIAL_RELATIVE),
            "teardown cannot reference a scene",
        );
        reject_step(
            &repository,
            4,
            "expected_cpu_bytes",
            Some(Value::Integer(1)),
            "teardown cannot retain CPU image bytes",
        );
    }

    #[test]
    fn scene_identity_and_each_metadata_axis_are_independently_bound() {
        let repository = FixtureRepository::new();

        let mut identity = manifest_value();
        for index in [0, 1, 2, 3, 5] {
            set_step(
                &mut identity,
                index,
                "resource_id",
                Some(Value::String("other-atlas".to_owned())),
            );
        }
        repository.write_manifest(&identity);
        assert_rejects(&repository.rejection(), "atlas resource identity drifted");

        let mut revisions = manifest_value();
        for (index, revision) in [(0, 11), (1, 11), (2, 12), (3, 13), (5, 13)] {
            set_step(
                &mut revisions,
                index,
                "resource_revision",
                Some(Value::Integer(revision)),
            );
        }
        repository.write_manifest(&revisions);
        assert_rejects(&repository.rejection(), "atlas metadata drifted");

        let mut widths = manifest_value();
        for (index, width, upload) in [(0, 3, 6), (1, 3, 0), (2, 3, 6), (3, 6, 12), (5, 6, 12)] {
            set_step(
                &mut widths,
                index,
                "atlas_width",
                Some(Value::Integer(width)),
            );
            set_step(
                &mut widths,
                index,
                "expected_atlas_upload_bytes",
                Some(Value::Integer(upload)),
            );
        }
        repository.write_manifest(&widths);
        assert_rejects(&repository.rejection(), "atlas metadata drifted");

        let mut heights = manifest_value();
        for (index, height, upload) in [(0, 3, 6), (1, 3, 0), (2, 3, 6), (3, 3, 12), (5, 3, 12)] {
            set_step(
                &mut heights,
                index,
                "atlas_height",
                Some(Value::Integer(height)),
            );
            set_step(
                &mut heights,
                index,
                "expected_atlas_upload_bytes",
                Some(Value::Integer(upload)),
            );
        }
        repository.write_manifest(&heights);
        assert_rejects(&repository.rejection(), "atlas metadata drifted");

        let mut hashes = manifest_value();
        for (index, byte) in [(0, '1'), (1, '1'), (2, '2'), (3, '3'), (5, '3')] {
            set_step(
                &mut hashes,
                index,
                "content_hash",
                Some(Value::String(byte.to_string().repeat(64))),
            );
        }
        repository.write_manifest(&hashes);
        assert_rejects(&repository.rejection(), "atlas metadata drifted");
    }

    #[test]
    fn scene_projection_resource_count_pixels_and_workload_are_checked() {
        let repository = FixtureRepository::new();

        let mut workload = manifest_value();
        set_step(
            &mut workload,
            0,
            "workload_hash",
            Some(Value::String("0".repeat(64))),
        );
        repository.write_manifest(&workload);
        assert_rejects(&repository.rejection(), "workload hash drifted");

        repository.reset();
        let mut no_resource: Value = toml::from_str(INITIAL)
            .unwrap_or_else(|error| panic!("invalid initial fixture: {error}"));
        no_resource
            .as_table_mut()
            .and_then(|table| table.get_mut("resources"))
            .and_then(Value::as_array_mut)
            .unwrap_or_else(|| panic!("initial fixture has no resources"))
            .clear();
        repository.write(
            INITIAL_RELATIVE,
            &toml::to_string(&no_resource)
                .unwrap_or_else(|error| panic!("cannot serialize scene: {error}")),
        );
        assert_rejects(&repository.rejection(), "has no atlas resource");

        repository.reset();
        let mut duplicate: Value = toml::from_str(INITIAL)
            .unwrap_or_else(|error| panic!("invalid initial fixture: {error}"));
        let resources = duplicate
            .as_table_mut()
            .and_then(|table| table.get_mut("resources"))
            .and_then(Value::as_array_mut)
            .unwrap_or_else(|| panic!("initial fixture has no resources"));
        resources.push(resources[0].clone());
        repository.write(
            INITIAL_RELATIVE,
            &toml::to_string(&duplicate)
                .unwrap_or_else(|error| panic!("cannot serialize scene: {error}")),
        );
        assert_rejects(&repository.rejection(), "at most one A8 atlas resource");

        repository.reset();
        let mut partial: Value = toml::from_str(INITIAL)
            .unwrap_or_else(|error| panic!("invalid initial fixture: {error}"));
        partial
            .get_mut("resources")
            .and_then(Value::as_array_mut)
            .and_then(|resources| resources.first_mut())
            .and_then(Value::as_table_mut)
            .and_then(|resource| resource.get_mut("pixels"))
            .and_then(Value::as_array_mut)
            .unwrap_or_else(|| panic!("initial fixture has no pixels"))
            .pop();
        repository.write(
            INITIAL_RELATIVE,
            &toml::to_string(&partial)
                .unwrap_or_else(|error| panic!("cannot serialize scene: {error}")),
        );
        assert_rejects(&repository.rejection(), "invalid A8 pixel length");
    }

    #[test]
    fn path_resolution_is_relative_bounded_symlink_free_and_file_only() {
        let repository = FixtureRepository::new();
        assert_eq!(
            resolve_repository_path(&repository.root, INITIAL_RELATIVE),
            fs::canonicalize(repository.root.join(INITIAL_RELATIVE))
                .map_err(|error| vec![error.to_string()])
        );
        assert_rejects(
            &resolve_repository_path(&repository.root, "/tmp/scene.toml").unwrap_err(),
            "is invalid",
        );
        assert_rejects(
            &resolve_repository_path(&repository.root, "../scene.toml").unwrap_err(),
            "is invalid",
        );
        assert_rejects(
            &resolve_repository_path(&repository.root, "./scene.toml").unwrap_err(),
            "is invalid",
        );
        assert_rejects(
            &resolve_repository_path(&repository.root, &"x".repeat(MAX_PATH_BYTES + 1))
                .unwrap_err(),
            "exceeds the byte limit",
        );
        let boundary = resolve_repository_path(&repository.root, &"x".repeat(MAX_PATH_BYTES));
        assert!(
            boundary
                .as_ref()
                .err()
                .is_some_and(|errors| !errors.iter().any(|error| error.contains("byte limit")))
        );
        assert_rejects(
            &resolve_repository_path(&repository.root, "missing/scene.toml").unwrap_err(),
            "cannot inspect trace sequence path",
        );
        assert_rejects(
            &resolve_repository_path(&repository.root, "assurance").unwrap_err(),
            "is not a file",
        );
        let missing_root = repository.root.join("missing-root");
        assert_rejects(
            &resolve_repository_path(&missing_root, "scene.toml").unwrap_err(),
            "cannot canonicalize",
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;

            let link = repository.root.join("linked");
            symlink(repository.root.join("assurance"), &link)
                .unwrap_or_else(|error| panic!("cannot create symlink fixture: {error}"));
            assert_rejects(
                &resolve_repository_path(&repository.root, "linked/qualification").unwrap_err(),
                "contains a symlink",
            );
        }
    }

    #[test]
    fn toml_loader_distinguishes_io_size_utf8_and_syntax_failures() {
        let repository = FixtureRepository::new();
        let missing = repository.root.join("missing.toml");
        assert_rejects(
            &load_toml::<SequenceManifest>(&missing).unwrap_err(),
            "cannot read",
        );

        let oversized = repository.root.join("oversized.toml");
        fs::write(&oversized, vec![b' '; MAX_MANIFEST_BYTES + 1])
            .unwrap_or_else(|error| panic!("cannot write oversized fixture: {error}"));
        assert_rejects(
            &load_toml::<SequenceManifest>(&oversized).unwrap_err(),
            "exceeds the manifest byte limit",
        );

        let boundary = repository.root.join("boundary.toml");
        fs::write(&boundary, vec![b' '; MAX_MANIFEST_BYTES])
            .unwrap_or_else(|error| panic!("cannot write boundary fixture: {error}"));
        let boundary_error = load_toml::<SequenceManifest>(&boundary).unwrap_err();
        assert!(
            boundary_error
                .iter()
                .all(|error| !error.contains("manifest byte limit"))
        );

        let invalid_utf8 = repository.root.join("invalid-utf8.toml");
        fs::write(&invalid_utf8, [0xff])
            .unwrap_or_else(|error| panic!("cannot write UTF-8 fixture: {error}"));
        assert_rejects(
            &load_toml::<SequenceManifest>(&invalid_utf8).unwrap_err(),
            "is not UTF-8",
        );

        let syntax = repository.root.join("syntax.toml");
        fs::write(&syntax, "[")
            .unwrap_or_else(|error| panic!("cannot write syntax fixture: {error}"));
        assert_rejects(
            &load_toml::<SequenceManifest>(&syntax).unwrap_err(),
            "cannot parse",
        );
    }

    #[test]
    fn transitions_names_deltas_slugs_requirements_and_reservation_are_exact() {
        let transitions = [
            ("full-admission", TraceSequenceTransition::FullAdmission),
            ("compatible-reuse", TraceSequenceTransition::CompatibleReuse),
            (
                "content-replacement",
                TraceSequenceTransition::ContentReplacement,
            ),
            (
                "capacity-replacement",
                TraceSequenceTransition::CapacityReplacement,
            ),
            ("teardown", TraceSequenceTransition::Teardown),
            (
                "full-resynchronization",
                TraceSequenceTransition::FullResynchronization,
            ),
        ];
        for (name, transition) in transitions {
            assert_eq!(parse_transition(name), Ok(transition));
            assert_eq!(transition_name(transition), name);
            assert_eq!(transition_is_teardown(transition), name == "teardown");
            assert_eq!(
                transition_starts_renderer_generation(transition),
                matches!(name, "full-admission" | "full-resynchronization")
            );
        }
        assert_rejects(&parse_transition("replace").unwrap_err(), "unsupported");
        assert!(!owns_exactly_one_atlas(0));
        assert!(owns_exactly_one_atlas(1));
        assert!(!owns_exactly_one_atlas(2));
        assert!(atlas_pixels_match_dimensions(2, 2, 4));
        assert!(!atlas_pixels_match_dimensions(2, 2, 3));

        assert_eq!(max_channel_delta(&[], &[]), Some(0));
        assert_eq!(max_channel_delta(&[1, 9, 3], &[4, 2, 3]), Some(7));
        assert_eq!(max_channel_delta(&[1], &[1, 2]), None);

        let mut errors = Vec::new();
        require(&mut errors, true, "not retained");
        require(&mut errors, false, "retained");
        assert_eq!(errors, vec!["retained".to_owned()]);

        for valid in ["a", "atlas-2", "2-atlas"] {
            assert!(valid_slug(valid));
        }
        for invalid in ["", "Atlas", "atlas_2", "atlas."] {
            assert!(!valid_slug(invalid));
        }

        assert!(reserve_step_storage(2).is_ok());
        let allocation_error = match reserve_step_storage(usize::MAX) {
            Ok(_) => panic!("maximum reservation unexpectedly succeeded"),
            Err(errors) => errors,
        };
        assert_rejects(&allocation_error, "step allocation failed");
    }

    #[test]
    fn renderer_observations_discriminate_every_terminal_contract_axis() {
        assert!(ownership_is_drained(BackendState::Stopped, 0, 0, true));
        for state in [BackendState::Ready, BackendState::DeviceLost] {
            assert!(!ownership_is_drained(state, 0, 0, true));
        }
        assert!(!ownership_is_drained(BackendState::Stopped, 1, 0, true));
        assert!(!ownership_is_drained(BackendState::Stopped, 0, 0, false));
        assert_eq!(
            super::require_drained_ownership(BackendState::Stopped, 0, 0, true, "drain failed"),
            Ok(())
        );
        assert_eq!(
            super::require_drained_ownership(BackendState::Ready, 0, 0, true, "drain failed"),
            Err(vec!["drain failed".to_owned()])
        );

        assert_eq!(frame_observation_error(7, 1, 1, 4, 4, 0, 0, true), None);
        assert_eq!(
            frame_observation_error(7, 2, 1, 4, 4, 0, 0, true),
            Some("trace sequence step 7 exceeds CPU oracle tolerance: 2 > 1".to_owned())
        );
        assert_eq!(
            frame_observation_error(7, 1, 1, 3, 4, 0, 0, true),
            Some("trace sequence step 7 uploaded 3 atlas bytes, expected 4".to_owned())
        );
        assert_eq!(
            frame_observation_error(7, 1, 1, 4, 4, 1, 0, true),
            Some("trace sequence step 7 did not reach balanced terminal ownership".to_owned())
        );
        assert_eq!(
            frame_observation_error(7, 1, 1, 4, 4, 0, 0, false),
            Some("trace sequence step 7 did not reach balanced terminal ownership".to_owned())
        );
    }

    #[test]
    fn native_render_entry_invalidates_stale_output_and_preserves_io_errors() {
        let repository = FixtureRepository::new();
        let output = repository.root.join("evidence.toml");
        fs::write(&output, "stale")
            .unwrap_or_else(|error| panic!("cannot write stale output: {error}"));
        let missing_manifest = repository.root.join("missing.toml");
        assert_rejects(
            &render_native(&missing_manifest, &output, &repository.root).unwrap_err(),
            "cannot read",
        );
        assert!(!output.exists());

        fs::create_dir(&output)
            .unwrap_or_else(|error| panic!("cannot create output directory: {error}"));
        assert_rejects(
            &render_native(&repository.manifest(), &output, &repository.root).unwrap_err(),
            "cannot invalidate prior output",
        );
        assert!(output.is_dir());

        assert_eq!(invalidate_output(&repository.root.join("absent")), Ok(()));
        let removable = repository.root.join("removable");
        fs::write(&removable, "old")
            .unwrap_or_else(|error| panic!("cannot write removable output: {error}"));
        assert_eq!(invalidate_output(&removable), Ok(()));
        assert!(!removable.exists());
    }
}
