//! Validates versioned golden-workload and qualification manifests.

use alpine_trace::{
    DecodedTrace, MAX_TRACE_ATLAS_PIXELS, MAX_TRACE_CLIPS, PreparedTraceInput,
    PreparedTraceOperation, PreparedTraceQuad, TraceAtlas, TraceClip, TraceGlyph, TraceInput,
    TraceQuad, TraceViewport,
};
use serde::{Deserialize, de::DeserializeOwned};
use std::{
    collections::BTreeSet,
    fmt::Write as _,
    fs,
    io::ErrorKind,
    path::{Component, Path},
};

const SCENE_SCHEMA: &str = "alpine-scene-trace/v1";
const PREPARED_SCENE_SCHEMA: &str = "alpine-scene-trace/v2";
const JOURNEY_SCHEMA: &str = "alpine-journey/v1";
const QUALIFICATION_SCHEMA: &str = "alpine-qualification/v1";
const SUPPORTED_OPERATIONS: &[&str] = &["solid-quad"];
const PREPARED_OPERATIONS: &[&str] = &["solid-quad", "monochrome-glyph"];
const SUPPORTED_ACTIONS: &[&str] = &[
    "open-project",
    "render-frame",
    "type-text",
    "scroll",
    "select",
    "invoke-command",
    "resize",
    "close-project",
];
const EQUIVALENCE_KINDS: &[&str] = &[
    "semantic",
    "visual",
    "accessibility",
    "lifecycle",
    "resources",
];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SceneTrace {
    schema: String,
    id: String,
    workload_hash: String,
    revision: u64,
    viewport: Viewport,
    clear_color: [f32; 4],
    #[serde(default)]
    resources: Vec<Resource>,
    #[serde(default)]
    clips: Vec<Clip>,
    operations: Vec<Operation>,
    pair: Option<ScenePair>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Viewport {
    width: f32,
    height: f32,
    scale_factor: f32,
    pixel_width: u32,
    pixel_height: u32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Resource {
    id: String,
    kind: String,
    content_hash: String,
    revision: Option<u64>,
    width: Option<u32>,
    height: Option<u32>,
    pixels: Option<Vec<u8>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Clip {
    id: String,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Operation {
    sequence: u64,
    kind: String,
    resource: Option<String>,
    clip: Option<String>,
    x: Option<f32>,
    y: Option<f32>,
    width: Option<f32>,
    height: Option<f32>,
    red: Option<f32>,
    green: Option<f32>,
    blue: Option<f32>,
    alpha: Option<f32>,
    atlas_x: Option<u32>,
    atlas_y: Option<u32>,
    atlas_width: Option<u32>,
    atlas_height: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScenePair {
    id: String,
    kind: String,
    sequence_hash: String,
    step: u32,
    steps: u32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journey {
    schema: String,
    id: String,
    workload_hash: String,
    scene_trace: String,
    actions: Vec<Action>,
    expected_document_hash: String,
    expected_layout_hash: String,
    expected_accessibility_hash: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Action {
    sequence: u64,
    kind: String,
    payload_hash: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Qualification {
    schema: String,
    id: String,
    state: String,
    comparison_level: String,
    scene_trace: String,
    journey: String,
    workload_hash: String,
    base_workload_hash: String,
    candidate_workload_hash: String,
    base_revision: String,
    candidate_revision: String,
    zed_revision: String,
    alpine_revision: String,
    independent_windows: u64,
    environment: Environment,
    #[serde(default)]
    equivalence: Vec<Equivalence>,
    #[serde(default)]
    measurements: Vec<Measurement>,
    assumptions: Vec<String>,
    exclusions: Vec<String>,
    #[serde(default)]
    rejection_reasons: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Environment {
    hardware_id: String,
    os: String,
    toolchain: String,
    power_state: String,
    thermal_state: String,
    qualified: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Equivalence {
    kind: String,
    status: String,
    evidence: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Measurement {
    name: String,
    unit: String,
    sample_count: u64,
    artifact: String,
    artifact_sha256: String,
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

pub(crate) fn run(command: &str, manifest: &Path, root: &Path) -> Result<String, Vec<String>> {
    let qualification: Qualification = load_toml(manifest)?;
    let scene_path = resolve_repository_path(root, &qualification.scene_trace)?;
    let journey_path = resolve_repository_path(root, &qualification.journey)?;
    let scene: SceneTrace = load_toml(&scene_path)?;
    let journey: Journey = load_toml(&journey_path)?;
    let errors = validate(&qualification, &scene, &journey, root);
    if !errors.is_empty() {
        return Err(errors);
    }

    match command {
        "validate-qualification" => Ok(format!(
            "validated qualification {} at state {} with {} measurements",
            qualification.id,
            qualification.state,
            qualification.measurements.len()
        )),
        "qualification-report" => Ok(render_report(&qualification, &scene, &journey)),
        other => Err(vec![format!("unsupported qualification command {other:?}")]),
    }
}

pub(crate) fn run_scene(manifest: &Path, _root: &Path) -> Result<String, Vec<String>> {
    let scene: SceneTrace = load_toml(manifest)?;
    let errors = validate_scene_errors_all(&scene);
    if !errors.is_empty() {
        return Err(errors);
    }
    let decoded = match decode_scene(&scene) {
        Ok(decoded) => decoded,
        Err(error) => return Err(vec![error]),
    };
    let frame = match decoded.validated_frame() {
        Ok(frame) => frame,
        Err(error) => {
            return Err(vec![format!(
                "scene trace frame validation failed: {error}"
            )]);
        }
    };
    let image = match frame.reference_image() {
        Ok(image) => image,
        Err(error) => return Err(vec![format!("scene trace CPU oracle failed: {error}")]),
    };
    Ok(format!(
        "validated scene trace {} at revision {} with {} operations and {}x{} reference pixels",
        scene.id,
        scene.revision,
        scene.operations.len(),
        image.width(),
        image.height()
    ))
}

pub(crate) fn render_scene(
    native: bool,
    manifest: &Path,
    output: &Path,
) -> Result<String, Vec<String>> {
    match fs::remove_file(output) {
        Ok(()) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => {
            return Err(vec![format!(
                "cannot invalidate prior output {}: {error}",
                output.display()
            )]);
        }
    }
    let scene: SceneTrace = load_toml(manifest)?;
    let errors = validate_scene_errors_all(&scene);
    if !errors.is_empty() {
        return Err(errors);
    }
    let decoded = match decode_scene(&scene) {
        Ok(decoded) => decoded,
        Err(error) => return Err(vec![error]),
    };
    let (image, source) = if native {
        let mut backend = match alpine_metal::MetalBackend::new() {
            Ok(backend) => backend,
            Err(error) => {
                return Err(vec![format!("cannot initialize Direct Metal: {error}")]);
            }
        };
        let frame = match backend.render_offscreen(decoded.scene(), decoded.descriptor()) {
            Ok(frame) => frame,
            Err(error) => {
                return Err(vec![format!("Direct Metal trace render failed: {error}")]);
            }
        };
        (frame.image().clone(), "direct-metal")
    } else {
        let frame = match decoded.validated_frame() {
            Ok(frame) => frame,
            Err(error) => {
                return Err(vec![format!(
                    "scene trace frame validation failed: {error}"
                )]);
            }
        };
        let image = match frame.reference_image() {
            Ok(image) => image,
            Err(error) => return Err(vec![format!("scene trace CPU oracle failed: {error}")]),
        };
        (image, "cpu-oracle")
    };
    if let Err(error) = fs::write(output, image.bytes()) {
        return Err(vec![format!("cannot write {}: {error}", output.display())]);
    }
    Ok(format!(
        "rendered scene trace {} through {source} to {} as {}x{} compact BGRA8",
        scene.id,
        output.display(),
        image.width(),
        image.height()
    ))
}

fn load_toml<T: DeserializeOwned>(path: &Path) -> Result<T, Vec<String>> {
    let source = fs::read_to_string(path)
        .map_err(|error| vec![format!("cannot read {}: {error}", path.display())])?;
    toml::from_str(&source)
        .map_err(|error| vec![format!("cannot parse {}: {error}", path.display())])
}

fn resolve_repository_path(root: &Path, value: &str) -> Result<std::path::PathBuf, Vec<String>> {
    let path = Path::new(value);
    if value.is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(vec![format!(
            "qualification artifact path must be a repository-relative normal path: {value}"
        )]);
    }
    Ok(root.join(path))
}

fn validate(
    qualification: &Qualification,
    scene: &SceneTrace,
    journey: &Journey,
    root: &Path,
) -> Vec<String> {
    let mut diagnostics = Diagnostics::default();
    validate_identity(qualification, scene, journey, &mut diagnostics);
    validate_scene(scene, &mut diagnostics);
    validate_journey(qualification, scene, journey, &mut diagnostics);
    validate_equivalence(qualification, root, &mut diagnostics);
    validate_measurements(qualification, root, &mut diagnostics);
    diagnostics.finish()
}

fn validate_identity(
    qualification: &Qualification,
    scene: &SceneTrace,
    journey: &Journey,
    diagnostics: &mut Diagnostics,
) {
    diagnostics.require(
        qualification.schema == QUALIFICATION_SCHEMA,
        format!("qualification schema must be {QUALIFICATION_SCHEMA}"),
    );
    diagnostics.require(
        matches!(scene.schema.as_str(), SCENE_SCHEMA | PREPARED_SCENE_SCHEMA),
        format!("scene schema must be {SCENE_SCHEMA} or {PREPARED_SCENE_SCHEMA}"),
    );
    diagnostics.require(
        journey.schema == JOURNEY_SCHEMA,
        format!("journey schema must be {JOURNEY_SCHEMA}"),
    );
    for (name, value) in [
        ("qualification", qualification.id.as_str()),
        ("scene", scene.id.as_str()),
        ("journey", journey.id.as_str()),
    ] {
        diagnostics.require(
            valid_slug(value),
            format!("{name} identifier must be a non-empty lowercase slug"),
        );
    }
    for (name, value) in [
        (
            "qualification workload",
            qualification.workload_hash.as_str(),
        ),
        ("base workload", qualification.base_workload_hash.as_str()),
        (
            "candidate workload",
            qualification.candidate_workload_hash.as_str(),
        ),
        ("scene workload", scene.workload_hash.as_str()),
        ("journey workload", journey.workload_hash.as_str()),
    ] {
        diagnostics.require(
            valid_sha256(value),
            format!("{name} hash must be 64 lowercase hexadecimal characters"),
        );
    }
    diagnostics.require(
        qualification.workload_hash == qualification.base_workload_hash
            && qualification.workload_hash == qualification.candidate_workload_hash
            && qualification.workload_hash == scene.workload_hash
            && qualification.workload_hash == journey.workload_hash,
        "base, candidate, scene, journey, and qualification workload hashes must match",
    );
    for (name, revision) in [
        ("base", qualification.base_revision.as_str()),
        ("candidate", qualification.candidate_revision.as_str()),
        ("Zed", qualification.zed_revision.as_str()),
        ("Alpine", qualification.alpine_revision.as_str()),
    ] {
        diagnostics.require(
            valid_git_sha(revision),
            format!("{name} revision must be a full lowercase Git SHA"),
        );
    }
    diagnostics.require(
        matches!(
            qualification.comparison_level.as_str(),
            "renderer-only" | "full-zed-path" | "product-journey"
        ),
        format!(
            "unsupported comparison level {}",
            qualification.comparison_level
        ),
    );
}

fn validate_scene(scene: &SceneTrace, diagnostics: &mut Diagnostics) {
    diagnostics.require(scene.revision > 0, "scene revision must be positive");
    diagnostics.require(
        finite_positive(scene.viewport.width)
            && finite_positive(scene.viewport.height)
            && finite_positive(scene.viewport.scale_factor),
        "scene viewport dimensions and scale factor must be finite and positive",
    );

    let resources = validate_scene_resources(scene, diagnostics);
    let clips = validate_scene_clips(scene, diagnostics);
    validate_scene_pair(scene, diagnostics);
    validate_scene_operations(scene, &clips, &resources, diagnostics);
    if diagnostics.errors.is_empty() {
        match decode_scene(scene) {
            Ok(decoded) => {
                if let Err(error) = decoded.validated_frame() {
                    diagnostics
                        .errors
                        .push(format!("scene trace frame validation failed: {error}"));
                }
            }
            Err(error) => diagnostics.errors.push(error),
        }
    }
}

fn validate_scene_resources<'a>(
    scene: &'a SceneTrace,
    diagnostics: &mut Diagnostics,
) -> BTreeSet<&'a str> {
    if scene.schema == SCENE_SCHEMA {
        diagnostics.require(
            scene.resources.is_empty(),
            "solid-quad trace slice does not support resources",
        );
    } else {
        diagnostics.require(
            scene.resources.len() <= 1,
            "prepared scene supports at most one A8 atlas resource",
        );
    }
    let mut resources = BTreeSet::new();
    for resource in &scene.resources {
        diagnostics.require(
            valid_slug(&resource.id),
            format!("invalid resource identifier {}", resource.id),
        );
        diagnostics.require(
            resources.insert(resource.id.as_str()),
            format!("duplicate resource identifier {}", resource.id),
        );
        diagnostics.require(
            valid_slug(&resource.kind),
            format!("resource {} has invalid kind", resource.id),
        );
        diagnostics.require(
            valid_sha256(&resource.content_hash),
            format!("resource {} has invalid content hash", resource.id),
        );
        if scene.schema == PREPARED_SCENE_SCHEMA {
            diagnostics.require(
                resource.kind == "a8-atlas",
                format!("resource {} must be an a8-atlas", resource.id),
            );
            let expected = resource
                .width
                .and_then(|width| resource.height.and_then(|height| width.checked_mul(height)))
                .and_then(|pixels| usize::try_from(pixels).ok());
            diagnostics.require(
                resource.revision.is_some_and(|revision| revision > 0),
                format!("resource {} requires a positive revision", resource.id),
            );
            diagnostics.require(
                resource.width.is_some_and(|width| width > 0)
                    && resource.height.is_some_and(|height| height > 0),
                format!("resource {} requires positive dimensions", resource.id),
            );
            diagnostics.require(
                expected.is_some_and(|pixels| pixels <= MAX_TRACE_ATLAS_PIXELS),
                format!("resource {} exceeds the A8 atlas pixel limit", resource.id),
            );
            diagnostics.require(
                expected == resource.pixels.as_ref().map(Vec::len),
                format!("resource {} has an invalid A8 pixel length", resource.id),
            );
        }
    }
    resources
}

fn validate_scene_clips<'a>(
    scene: &'a SceneTrace,
    diagnostics: &mut Diagnostics,
) -> BTreeSet<&'a str> {
    let mut clips = BTreeSet::new();
    for clip in &scene.clips {
        diagnostics.require(
            valid_slug(&clip.id),
            format!("invalid clip identifier {}", clip.id),
        );
        diagnostics.require(
            clips.insert(clip.id.as_str()),
            format!("duplicate clip identifier {}", clip.id),
        );
        diagnostics.require(
            clip.x.is_finite()
                && clip.y.is_finite()
                && finite_positive(clip.width)
                && finite_positive(clip.height),
            format!("clip {} must contain finite positive geometry", clip.id),
        );
    }
    if scene.schema == SCENE_SCHEMA {
        diagnostics.require(
            scene.clips.len() == 1,
            "solid-quad trace slice requires exactly one viewport clip",
        );
    } else {
        diagnostics.require(
            scene.clips.len() <= MAX_TRACE_CLIPS,
            "prepared scene clip limit exceeded",
        );
    }
    clips
}

fn validate_scene_pair(scene: &SceneTrace, diagnostics: &mut Diagnostics) {
    if scene.schema == SCENE_SCHEMA {
        diagnostics.require(
            scene.pair.is_none(),
            "version 1 scene cannot declare a pair",
        );
        return;
    }
    if let Some(pair) = &scene.pair {
        diagnostics.require(valid_slug(&pair.id), "scene pair identifier must be a slug");
        diagnostics.require(
            matches!(pair.kind.as_str(), "scroll" | "resize"),
            format!("unsupported scene pair kind {}", pair.kind),
        );
        diagnostics.require(
            valid_sha256(&pair.sequence_hash),
            "scene pair sequence hash must be a SHA-256",
        );
        diagnostics.require(pair.steps == 2, "scene pair must contain exactly two steps");
        diagnostics.require(
            pair.step < pair.steps,
            "scene pair step is outside its sequence",
        );
    }
}

fn validate_scene_operations(
    scene: &SceneTrace,
    clips: &BTreeSet<&str>,
    resources: &BTreeSet<&str>,
    diagnostics: &mut Diagnostics,
) {
    diagnostics.require(
        !scene.operations.is_empty(),
        "scene must contain operations",
    );
    for (expected, operation) in scene.operations.iter().enumerate() {
        diagnostics.require(
            operation.sequence == expected as u64,
            format!("scene operation sequence must be contiguous at {expected}"),
        );
        let supported = if scene.schema == SCENE_SCHEMA {
            SUPPORTED_OPERATIONS
        } else {
            PREPARED_OPERATIONS
        };
        diagnostics.require(
            supported.contains(&operation.kind.as_str()),
            format!("unsupported scene operation {}", operation.kind),
        );
        if let Some(clip) = &operation.clip {
            diagnostics.require(
                clips.contains(clip.as_str()),
                format!("operation {expected} references unknown clip {clip}"),
            );
        }
        if operation.kind == "solid-quad" {
            if scene.schema == SCENE_SCHEMA {
                diagnostics.require(
                    operation.clip.is_some(),
                    format!("solid-quad operation {expected} requires a clip"),
                );
            }
            diagnostics.require(
                operation_payload(operation).is_some(),
                format!("solid-quad operation {expected} requires complete bounds and color"),
            );
            diagnostics.require(
                operation.resource.is_none(),
                format!("solid-quad operation {expected} cannot reference a resource"),
            );
            diagnostics.require(
                !operation_has_any_atlas_field(operation),
                format!("solid-quad operation {expected} cannot contain atlas bounds"),
            );
        } else if operation.kind == "monochrome-glyph" {
            diagnostics.require(
                operation_payload(operation).is_some(),
                format!("monochrome-glyph operation {expected} requires complete bounds and color"),
            );
            diagnostics.require(
                operation_atlas_payload(operation).is_some(),
                format!("monochrome-glyph operation {expected} requires complete atlas bounds"),
            );
            diagnostics.require(
                operation
                    .resource
                    .as_deref()
                    .is_some_and(|resource| resources.contains(resource)),
                format!("monochrome-glyph operation {expected} requires a known resource"),
            );
        }
    }
}

fn validate_scene_errors_all(scene: &SceneTrace) -> Vec<String> {
    let mut diagnostics = Diagnostics::default();
    validate_scene(scene, &mut diagnostics);
    diagnostics.finish()
}

fn decode_scene(scene: &SceneTrace) -> Result<DecodedTrace, String> {
    if scene.schema == PREPARED_SCENE_SCHEMA {
        return decode_prepared_scene(scene);
    }
    let clips = scene
        .clips
        .iter()
        .map(|clip| {
            (
                clip.id.as_str(),
                TraceClip {
                    bounds: [clip.x, clip.y, clip.width, clip.height],
                },
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut quads = Vec::new();
    if quads.try_reserve_exact(scene.operations.len()).is_err() {
        return Err("scene trace operation allocation failed".to_owned());
    }
    for operation in &scene.operations {
        if operation.kind != "solid-quad" {
            return Err(format!("unsupported scene operation {}", operation.kind));
        }
        let Some(clip_id) = operation.clip.as_deref() else {
            return Err(format!(
                "solid-quad operation {} requires a clip",
                operation.sequence
            ));
        };
        let Some(clip) = clips.get(clip_id).copied() else {
            return Err(format!(
                "operation {} references unknown clip {clip_id}",
                operation.sequence
            ));
        };
        let Some((bounds, color)) = operation_payload(operation) else {
            return Err(format!(
                "solid-quad operation {} requires complete bounds and color",
                operation.sequence
            ));
        };
        quads.push(TraceQuad {
            sequence: operation.sequence,
            bounds,
            color,
            clip,
        });
    }
    let decoded = TraceInput {
        revision: scene.revision,
        viewport: TraceViewport {
            logical_width: scene.viewport.width,
            logical_height: scene.viewport.height,
            scale_factor: scene.viewport.scale_factor,
            pixel_width: scene.viewport.pixel_width,
            pixel_height: scene.viewport.pixel_height,
            clear_color: scene.clear_color,
        },
        quads,
    }
    .decode();
    match decoded {
        Ok(decoded) => Ok(decoded),
        Err(error) => Err(format!("scene trace semantic decoding failed: {error}")),
    }
}

fn decode_prepared_scene(scene: &SceneTrace) -> Result<DecodedTrace, String> {
    let clips = scene
        .clips
        .iter()
        .enumerate()
        .map(|(index, clip)| (clip.id.as_str(), index))
        .collect::<std::collections::BTreeMap<_, _>>();
    let atlas = decode_trace_atlas(scene.resources.first())?;
    let mut operations = Vec::new();
    operations
        .try_reserve_exact(scene.operations.len())
        .map_err(|_| "prepared scene operation allocation failed".to_owned())?;
    for operation in &scene.operations {
        let clip = operation
            .clip
            .as_deref()
            .map(|clip| {
                clips.get(clip).copied().ok_or_else(|| {
                    format!(
                        "operation {} references unknown clip {clip}",
                        operation.sequence
                    )
                })
            })
            .transpose()?;
        let (bounds, color) = operation_payload(operation).ok_or_else(|| {
            format!(
                "operation {} requires complete bounds and color",
                operation.sequence
            )
        })?;
        match operation.kind.as_str() {
            "solid-quad" => operations.push(PreparedTraceOperation::Quad(PreparedTraceQuad {
                sequence: operation.sequence,
                bounds,
                color,
                clip,
            })),
            "monochrome-glyph" => {
                let expected_resource =
                    scene.resources.first().map(|resource| resource.id.as_str());
                if operation.resource.as_deref() != expected_resource {
                    return Err(format!(
                        "monochrome-glyph operation {} references the wrong resource",
                        operation.sequence
                    ));
                }
                let atlas_bounds = operation_atlas_payload(operation).ok_or_else(|| {
                    format!(
                        "monochrome-glyph operation {} requires complete atlas bounds",
                        operation.sequence
                    )
                })?;
                operations.push(PreparedTraceOperation::Glyph(TraceGlyph {
                    sequence: operation.sequence,
                    bounds,
                    atlas_bounds,
                    color,
                    clip,
                }));
            }
            other => return Err(format!("unsupported scene operation {other}")),
        }
    }
    PreparedTraceInput {
        revision: scene.revision,
        viewport: TraceViewport {
            logical_width: scene.viewport.width,
            logical_height: scene.viewport.height,
            scale_factor: scene.viewport.scale_factor,
            pixel_width: scene.viewport.pixel_width,
            pixel_height: scene.viewport.pixel_height,
            clear_color: scene.clear_color,
        },
        clips: scene
            .clips
            .iter()
            .map(|clip| TraceClip {
                bounds: [clip.x, clip.y, clip.width, clip.height],
            })
            .collect(),
        atlas,
        operations,
    }
    .decode()
    .map_err(|error| format!("scene trace semantic decoding failed: {error}"))
}

fn decode_trace_atlas(resource: Option<&Resource>) -> Result<Option<TraceAtlas>, String> {
    resource
        .map(|resource| {
            Ok(TraceAtlas {
                revision: resource
                    .revision
                    .ok_or_else(|| format!("resource {} is missing a revision", resource.id))?,
                width: resource
                    .width
                    .ok_or_else(|| format!("resource {} is missing a width", resource.id))?,
                height: resource
                    .height
                    .ok_or_else(|| format!("resource {} is missing a height", resource.id))?,
                pixels: resource
                    .pixels
                    .clone()
                    .ok_or_else(|| format!("resource {} is missing A8 pixels", resource.id))?,
            })
        })
        .transpose()
}

fn operation_payload(operation: &Operation) -> Option<([f32; 4], [f32; 4])> {
    Some((
        [
            operation.x?,
            operation.y?,
            operation.width?,
            operation.height?,
        ],
        [
            operation.red?,
            operation.green?,
            operation.blue?,
            operation.alpha?,
        ],
    ))
}

fn operation_atlas_payload(operation: &Operation) -> Option<[u32; 4]> {
    Some([
        operation.atlas_x?,
        operation.atlas_y?,
        operation.atlas_width?,
        operation.atlas_height?,
    ])
}

fn operation_has_any_atlas_field(operation: &Operation) -> bool {
    operation.atlas_x.is_some()
        || operation.atlas_y.is_some()
        || operation.atlas_width.is_some()
        || operation.atlas_height.is_some()
}

fn validate_journey(
    qualification: &Qualification,
    scene: &SceneTrace,
    journey: &Journey,
    diagnostics: &mut Diagnostics,
) {
    diagnostics.require(
        journey.scene_trace == qualification.scene_trace,
        "journey and qualification must reference the same scene trace",
    );
    diagnostics.require(!journey.actions.is_empty(), "journey must contain actions");
    for (expected, action) in journey.actions.iter().enumerate() {
        diagnostics.require(
            action.sequence == expected as u64,
            format!("journey action sequence must be contiguous at {expected}"),
        );
        diagnostics.require(
            SUPPORTED_ACTIONS.contains(&action.kind.as_str()),
            format!("unsupported journey action {}", action.kind),
        );
        if let Some(hash) = &action.payload_hash {
            diagnostics.require(
                valid_sha256(hash),
                format!("journey action {expected} has invalid payload hash"),
            );
        }
    }
    for (name, value) in [
        ("document", journey.expected_document_hash.as_str()),
        ("layout", journey.expected_layout_hash.as_str()),
    ] {
        diagnostics.require(
            valid_sha256(value),
            format!("expected {name} hash must be a SHA-256 value"),
        );
    }
    if let Some(hash) = &journey.expected_accessibility_hash {
        diagnostics.require(
            valid_sha256(hash),
            "expected accessibility hash must be a SHA-256 value",
        );
    }
    if qualification.comparison_level != "renderer-only" {
        diagnostics.require(
            journey
                .expected_accessibility_hash
                .as_deref()
                .is_some_and(valid_sha256),
            "full-path and product journeys require an accessibility hash",
        );
    }
    diagnostics.require(
        scene.workload_hash == journey.workload_hash,
        "journey and scene workload hashes must match",
    );
}

fn validate_equivalence(qualification: &Qualification, root: &Path, diagnostics: &mut Diagnostics) {
    let required = if qualification.comparison_level == "renderer-only" {
        &["semantic", "visual", "lifecycle", "resources"][..]
    } else {
        EQUIVALENCE_KINDS
    };
    let mut observed = BTreeSet::new();
    let mut passed = BTreeSet::new();
    for gate in &qualification.equivalence {
        diagnostics.require(
            EQUIVALENCE_KINDS.contains(&gate.kind.as_str()),
            format!("unsupported equivalence gate {}", gate.kind),
        );
        diagnostics.require(
            observed.insert(gate.kind.as_str()),
            format!("duplicate equivalence gate {}", gate.kind),
        );
        diagnostics.require(
            matches!(gate.status.as_str(), "passed" | "failed"),
            format!("equivalence gate {} has invalid status", gate.kind),
        );
        if gate.status == "passed" {
            passed.insert(gate.kind.as_str());
        } else {
            diagnostics.require(
                qualification.state == "rejected",
                format!("equivalence gate {} did not pass", gate.kind),
            );
        }
        match artifact_reference_path(root, &gate.evidence) {
            Ok(path) => diagnostics.require(
                path.is_file(),
                format!("equivalence gate {} evidence is missing", gate.kind),
            ),
            Err(errors) => diagnostics.errors.extend(errors),
        }
    }

    let stage_needs_equivalence = matches!(
        qualification.state.as_str(),
        "equivalent" | "measured" | "reproduced" | "qualified"
    );
    if stage_needs_equivalence {
        for kind in required {
            diagnostics.require(
                passed.contains(kind),
                format!("state {} requires {kind} equivalence", qualification.state),
            );
        }
    }
    if qualification.state == "loaded" {
        diagnostics.require(
            qualification.equivalence.is_empty(),
            "loaded qualification cannot contain equivalence results",
        );
    }
    if qualification.state == "rejected" {
        diagnostics.require(
            !qualification.rejection_reasons.is_empty(),
            "rejected qualification requires rejection reasons",
        );
    } else {
        diagnostics.require(
            qualification.rejection_reasons.is_empty(),
            "non-rejected qualification cannot contain rejection reasons",
        );
    }
}

fn validate_measurements(
    qualification: &Qualification,
    root: &Path,
    diagnostics: &mut Diagnostics,
) {
    validate_environment(qualification, diagnostics);
    validate_measurement_stage(qualification, diagnostics);
    validate_metric_records(qualification, root, diagnostics);
}

fn validate_environment(qualification: &Qualification, diagnostics: &mut Diagnostics) {
    for (name, value) in [
        ("hardware", qualification.environment.hardware_id.as_str()),
        ("operating system", qualification.environment.os.as_str()),
        ("toolchain", qualification.environment.toolchain.as_str()),
        (
            "power state",
            qualification.environment.power_state.as_str(),
        ),
        (
            "thermal state",
            qualification.environment.thermal_state.as_str(),
        ),
    ] {
        diagnostics.require(
            !value.trim().is_empty(),
            format!("{name} identity is required"),
        );
    }
    diagnostics.require(
        !qualification.assumptions.is_empty() && !qualification.exclusions.is_empty(),
        "qualification must disclose assumptions and exclusions",
    );
}

fn validate_measurement_stage(qualification: &Qualification, diagnostics: &mut Diagnostics) {
    diagnostics.require(
        matches!(
            qualification.state.as_str(),
            "loaded" | "equivalent" | "measured" | "reproduced" | "qualified" | "rejected"
        ),
        format!("unsupported qualification state {}", qualification.state),
    );
    let measured_state = matches!(
        qualification.state.as_str(),
        "measured" | "reproduced" | "qualified"
    );
    if measured_state {
        diagnostics.require(
            qualification.environment.qualified,
            "performance measurement requires a qualified environment",
        );
        diagnostics.require(
            !qualification.measurements.is_empty(),
            format!("state {} requires measurements", qualification.state),
        );
        diagnostics.require(
            qualification.independent_windows > 0,
            "measured qualification requires at least one hardware window",
        );
    } else {
        diagnostics.require(
            qualification.measurements.is_empty(),
            format!(
                "state {} cannot contain performance measurements",
                qualification.state
            ),
        );
    }
    if matches!(qualification.state.as_str(), "loaded" | "equivalent") {
        diagnostics.require(
            qualification.independent_windows == 0,
            format!(
                "state {} cannot contain hardware-window results",
                qualification.state
            ),
        );
    }
    if matches!(qualification.state.as_str(), "reproduced" | "qualified") {
        diagnostics.require(
            qualification.independent_windows >= 3,
            format!(
                "state {} requires three independent hardware windows",
                qualification.state
            ),
        );
    }
}

fn validate_metric_records(
    qualification: &Qualification,
    root: &Path,
    diagnostics: &mut Diagnostics,
) {
    let mut metrics = BTreeSet::new();
    for measurement in &qualification.measurements {
        diagnostics.require(
            valid_slug(&measurement.name),
            format!("invalid measurement name {}", measurement.name),
        );
        diagnostics.require(
            metrics.insert(measurement.name.as_str()),
            format!("duplicate measurement {}", measurement.name),
        );
        diagnostics.require(
            !measurement.unit.trim().is_empty(),
            format!("measurement {} needs a unit", measurement.name),
        );
        diagnostics.require(
            measurement.sample_count >= 2,
            format!(
                "measurement {} needs at least two samples",
                measurement.name
            ),
        );
        diagnostics.require(
            valid_sha256(&measurement.artifact_sha256),
            format!("measurement {} has invalid artifact hash", measurement.name),
        );
        match resolve_repository_path(root, &measurement.artifact) {
            Ok(path) => diagnostics.require(
                path.is_file(),
                format!("measurement {} artifact is missing", measurement.name),
            ),
            Err(errors) => diagnostics.errors.extend(errors),
        }
    }
}

#[cfg(test)]
fn validate_scene_errors(scene: &SceneTrace) -> Vec<String> {
    validate_scene_errors_all(scene)
}

#[cfg(test)]
fn validate_journey_errors(
    qualification: &Qualification,
    scene: &SceneTrace,
    journey: &Journey,
) -> Vec<String> {
    let mut diagnostics = Diagnostics::default();
    validate_journey(qualification, scene, journey, &mut diagnostics);
    diagnostics.finish()
}

#[cfg(test)]
fn validate_environment_errors(qualification: &Qualification) -> Vec<String> {
    let mut diagnostics = Diagnostics::default();
    validate_environment(qualification, &mut diagnostics);
    diagnostics.finish()
}

#[cfg(test)]
fn validate_stage_errors(qualification: &Qualification) -> Vec<String> {
    let mut diagnostics = Diagnostics::default();
    validate_measurement_stage(qualification, &mut diagnostics);
    diagnostics.finish()
}

#[cfg(test)]
fn validate_metric_errors(qualification: &Qualification, root: &Path) -> Vec<String> {
    let mut diagnostics = Diagnostics::default();
    validate_metric_records(qualification, root, &mut diagnostics);
    diagnostics.finish()
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

fn finite_positive(value: f32) -> bool {
    value.is_finite() && value > 0.0
}

fn artifact_reference_path(root: &Path, value: &str) -> Result<std::path::PathBuf, Vec<String>> {
    let path = value.split('#').next().unwrap_or_default();
    resolve_repository_path(root, path)
}

fn render_report(qualification: &Qualification, scene: &SceneTrace, journey: &Journey) -> String {
    let mut output = String::from("# Alpine qualification report\n\n");
    let _ = write!(
        output,
        "- Qualification: {}\n- State: {}\n- Comparison level: {}\n- Scene: {} at revision {}\n- Journey: {}\n- Workload SHA-256: {}\n- Base revision: {}\n- Candidate revision: {}\n- Zed revision: {}\n- Alpine revision: {}\n- Hardware: {}\n- Operating system: {}\n- Toolchain: {}\n- Power state: {}\n- Thermal state: {}\n- Environment qualified: {}\n- Independent windows: {}\n\n",
        qualification.id,
        qualification.state,
        qualification.comparison_level,
        scene.id,
        scene.revision,
        journey.id,
        qualification.workload_hash,
        qualification.base_revision,
        qualification.candidate_revision,
        qualification.zed_revision,
        qualification.alpine_revision,
        qualification.environment.hardware_id,
        qualification.environment.os,
        qualification.environment.toolchain,
        qualification.environment.power_state,
        qualification.environment.thermal_state,
        qualification.environment.qualified,
        qualification.independent_windows,
    );
    output.push_str("## Equivalence\n\n");
    for gate in &qualification.equivalence {
        let _ = writeln!(
            output,
            "- {}: {} ({})",
            gate.kind, gate.status, gate.evidence
        );
    }
    output.push_str("\n## Measurements\n\n");
    if qualification.measurements.is_empty() {
        output.push_str("No performance measurements are claimed.\n");
    } else {
        for measurement in &qualification.measurements {
            let _ = writeln!(
                output,
                "- {}: {} samples in {} ({})",
                measurement.name, measurement.sample_count, measurement.unit, measurement.artifact
            );
        }
    }
    output.push_str("\n## Qualifications\n\n- Assumptions:\n");
    for assumption in &qualification.assumptions {
        let _ = writeln!(output, "  - {assumption}");
    }
    output.push_str("- Exclusions:\n");
    for exclusion in &qualification.exclusions {
        let _ = writeln!(output, "  - {exclusion}");
    }
    if !qualification.rejection_reasons.is_empty() {
        output.push_str("- Rejection reasons:\n");
        for reason in &qualification.rejection_reasons {
            let _ = writeln!(output, "  - {reason}");
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::{
        Journey, Qualification, SceneTrace, decode_scene, finite_positive, load_toml,
        render_report, render_scene, resolve_repository_path, run, run_scene, valid_git_sha,
        valid_sha256, valid_slug, validate, validate_scene_errors,
    };
    use std::{
        fs,
        path::{Path, PathBuf},
    };

    fn repository_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    fn reference_bytes(scene: &SceneTrace) -> Option<Vec<u8>> {
        let decoded = decode_scene(scene).ok()?;
        let frame = decoded.validated_frame().ok()?;
        let image = frame.reference_image().ok()?;
        Some(image.bytes().to_vec())
    }

    #[test]
    fn accepts_valid_qualification_and_renders_scope() {
        let root = repository_root();
        let manifest = root.join("assurance/qualification/v1/valid.toml");
        let validation = run("validate-qualification", &manifest, &root);
        assert!(validation.is_ok(), "{validation:#?}");
        let report = run("qualification-report", &manifest, &root);
        assert!(report.is_ok(), "{report:#?}");
        if let Ok(report) = report {
            assert!(report.contains("Comparison level: renderer-only"));
            assert!(report.contains("Environment qualified: true"));
            assert!(report.contains("Exclusions:"));
        }
    }

    #[test]
    fn scene_commands_validate_and_render_reference_evidence() {
        let root = repository_root();
        let scene = root.join("assurance/qualification/v1/scene.toml");
        assert_eq!(
            run_scene(&scene, &root),
            Ok("validated scene trace solid-quad-editor-surface at revision 1 with 3 operations and 8x4 reference pixels".to_owned())
        );

        let output = root.join("target/qualification-unit-reference.bgra");
        assert!(fs::create_dir_all(root.join("target")).is_ok());
        assert_eq!(
            render_scene(false, &scene, &output),
            Ok(format!(
                "rendered scene trace solid-quad-editor-surface through cpu-oracle to {} as 8x4 compact BGRA8",
                output.display()
            ))
        );
        assert_eq!(fs::read(output).ok().map(|bytes| bytes.len()), Some(128));
    }

    #[test]
    fn rejects_performance_before_correctness() {
        let root = repository_root();
        let manifest = root.join("assurance/qualification/v1/performance-before-correctness.toml");
        let errors = run("validate-qualification", &manifest, &root);
        assert!(errors.is_err());
        if let Err(errors) = errors {
            assert!(
                errors
                    .iter()
                    .any(|error| error.contains("visual did not pass")),
                "{errors:#?}"
            );
        }
    }

    #[test]
    fn rejects_mismatched_and_unsupported_workloads() {
        let root = repository_root();
        for (fixture, expected) in [
            (
                "mismatched-workload.toml",
                "base, candidate, scene, journey, and qualification workload hashes must match",
            ),
            ("unsupported-operation.toml", "unsupported scene operation"),
            (
                "unqualified-environment.toml",
                "performance measurement requires a qualified environment",
            ),
        ] {
            let result = run(
                "validate-qualification",
                &root.join("assurance/qualification/v1").join(fixture),
                &root,
            );
            assert!(result.is_err(), "fixture {fixture} unexpectedly passed");
            if let Err(errors) = result {
                assert!(
                    errors.iter().any(|error| error.contains(expected)),
                    "fixture {fixture}: {errors:#?}"
                );
            }
        }
    }

    #[test]
    fn rejects_insufficient_reproduction() {
        let root = repository_root();
        let manifest = root.join("assurance/qualification/v1/valid.toml");
        let qualification: Result<Qualification, _> = load_toml(&manifest);
        let scene: Result<SceneTrace, _> =
            load_toml(&root.join("assurance/qualification/v1/scene.toml"));
        let journey: Result<Journey, _> =
            load_toml(&root.join("assurance/qualification/v1/journey.toml"));
        assert!(qualification.is_ok() && scene.is_ok() && journey.is_ok());
        if let (Ok(mut qualification), Ok(scene), Ok(journey)) = (qualification, scene, journey) {
            qualification.independent_windows = 2;
            let errors = validate(&qualification, &scene, &journey, &root);
            assert!(
                errors
                    .iter()
                    .any(|error| error.contains("requires three independent hardware windows")),
                "{errors:#?}"
            );
        }
    }

    #[test]
    fn accepts_a_scoped_rejection_without_measurement() {
        let root = repository_root();
        let qualification: Result<Qualification, _> =
            load_toml(&root.join("assurance/qualification/v1/valid.toml"));
        let scene: Result<SceneTrace, _> =
            load_toml(&root.join("assurance/qualification/v1/scene.toml"));
        let journey: Result<Journey, _> =
            load_toml(&root.join("assurance/qualification/v1/journey.toml"));
        assert!(qualification.is_ok() && scene.is_ok() && journey.is_ok());
        if let (Ok(mut qualification), Ok(scene), Ok(journey)) = (qualification, scene, journey) {
            qualification.state = "rejected".to_owned();
            qualification.measurements.clear();
            qualification.independent_windows = 0;
            qualification.environment.qualified = false;
            qualification
                .rejection_reasons
                .push("visual equivalence failed".to_owned());
            if let Some(visual) = qualification
                .equivalence
                .iter_mut()
                .find(|gate| gate.kind == "visual")
            {
                visual.status = "failed".to_owned();
            }
            let errors = validate(&qualification, &scene, &journey, &root);
            assert!(errors.is_empty(), "{errors:#?}");
            let report = render_report(&qualification, &scene, &journey);
            assert!(report.contains("Rejection reasons:"));
            assert!(report.contains("visual equivalence failed"));
        }
    }

    #[test]
    fn hash_contract_is_length_and_alphabet_exact() {
        for length in 0..=65 {
            let value = "a".repeat(length);
            assert_eq!(valid_sha256(&value), length == 64);
        }
        assert!(!valid_sha256(
            "gggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggg"
        ));
        assert!(valid_git_sha("0123456789abcdef0123456789abcdef01234567"));
        assert!(!valid_git_sha("0123456789abcdef0123456789abcdef0123456g"));
    }

    #[test]
    fn repository_paths_reject_each_escape_form() {
        let root = repository_root();
        assert!(resolve_repository_path(&root, "artifact/file.toml").is_ok());
        assert!(resolve_repository_path(&root, "").is_err());
        assert!(resolve_repository_path(&root, "../artifact.toml").is_err());
        let absolute = Path::new(env!("CARGO_MANIFEST_DIR")).join("artifact.toml");
        let absolute = absolute.to_string_lossy();
        assert!(resolve_repository_path(&root, &absolute).is_err());
    }

    #[test]
    fn scene_geometry_rejects_each_invalid_component() {
        let root = repository_root();
        let scene: Result<SceneTrace, _> =
            load_toml(&root.join("assurance/qualification/v1/scene.toml"));
        assert!(scene.is_ok());
        if let Ok(mut scene) = scene {
            scene.revision = 0;
            assert!(!super::validate_scene_errors(&scene).is_empty());
            scene.revision = 1;
            for invalid in [0.0, -1.0, f32::NAN, f32::INFINITY] {
                scene.viewport.width = invalid;
                assert!(!super::validate_scene_errors(&scene).is_empty());
                scene.viewport.width = 4.0;
                scene.viewport.height = invalid;
                assert!(!super::validate_scene_errors(&scene).is_empty());
                scene.viewport.height = 2.0;
                scene.viewport.scale_factor = invalid;
                assert!(!super::validate_scene_errors(&scene).is_empty());
                scene.viewport.scale_factor = 2.0;
            }
            if let Some(clip) = scene.clips.first_mut() {
                clip.x = f32::NAN;
            }
            assert!(!super::validate_scene_errors(&scene).is_empty());
            if let Some(clip) = scene.clips.first_mut() {
                clip.x = 0.0;
                clip.y = f32::INFINITY;
            }
            assert!(!super::validate_scene_errors(&scene).is_empty());
            if let Some(clip) = scene.clips.first_mut() {
                clip.y = 0.0;
                clip.width = 0.0;
            }
            assert!(!super::validate_scene_errors(&scene).is_empty());
            if let Some(clip) = scene.clips.first_mut() {
                clip.width = 4.0;
                clip.height = -1.0;
            }
            assert!(!super::validate_scene_errors(&scene).is_empty());
        }
    }

    #[test]
    fn full_qualification_rejects_semantically_invalid_trace_values() {
        let root = repository_root();
        let qualification: Result<Qualification, _> =
            load_toml(&root.join("assurance/qualification/v1/valid.toml"));
        let scene: Result<SceneTrace, _> =
            load_toml(&root.join("assurance/qualification/v1/scene.toml"));
        let journey: Result<Journey, _> =
            load_toml(&root.join("assurance/qualification/v1/journey.toml"));
        assert!(qualification.is_ok() && scene.is_ok() && journey.is_ok());
        if let (Ok(qualification), Ok(mut scene), Ok(journey)) = (qualification, scene, journey) {
            if let Some(operation) = scene.operations.first_mut() {
                operation.red = Some(2.0);
            }
            let errors = validate(&qualification, &scene, &journey, &root);
            assert!(
                errors
                    .iter()
                    .any(|error| error.contains("semantic decoding failed")),
                "{errors:#?}"
            );
        }
    }

    #[test]
    fn solid_quads_reject_resource_adaptation() {
        let root = repository_root();
        let scene: Result<SceneTrace, _> =
            load_toml(&root.join("assurance/qualification/v1/scene.toml"));
        assert!(scene.is_ok());
        if let Ok(mut scene) = scene {
            scene.resources.push(super::Resource {
                id: "unexpected-resource".to_owned(),
                kind: "image".to_owned(),
                content_hash: "a".repeat(64),
                revision: None,
                width: None,
                height: None,
                pixels: None,
            });
            if let Some(operation) = scene.operations.first_mut() {
                operation.resource = Some("unexpected-resource".to_owned());
            }
            let errors = super::validate_scene_errors(&scene);
            assert!(
                errors
                    .iter()
                    .any(|error| error.contains("cannot reference a resource")),
                "{errors:#?}"
            );
            assert!(
                errors
                    .iter()
                    .any(|error| error.contains("does not support resources")),
                "{errors:#?}"
            );
        }
    }

    #[test]
    fn versioned_trace_schema_rejects_unknown_fields() {
        let root = repository_root();
        let source = std::fs::read_to_string(root.join("assurance/qualification/v1/scene.toml"));
        assert!(source.is_ok());
        if let Ok(source) = source {
            let result = toml::from_str::<SceneTrace>(&format!(
                "{source}\nunexpected_protocol_field = true\n"
            ));
            assert!(result.is_err());
        }
    }

    #[test]
    fn realistic_v2_fixtures_decode_and_render_through_the_cpu_oracle() {
        let root = repository_root();
        for fixture in [
            "clipped-grid.toml",
            "glyph-grid.toml",
            "code-viewport.toml",
            "scroll-before.toml",
            "scroll-after.toml",
            "resize-before.toml",
            "resize-after.toml",
        ] {
            let scene = root.join("assurance/qualification/v2").join(fixture);
            let result = run_scene(&scene, &root);
            assert!(result.is_ok(), "{fixture}: {result:#?}");
        }
    }

    #[test]
    fn realistic_v2_rejects_atlas_resource_and_pair_contract_breaks() {
        let root = repository_root();
        let scene: Result<SceneTrace, _> =
            load_toml(&root.join("assurance/qualification/v2/glyph-grid.toml"));
        assert!(scene.is_ok());
        if let Ok(mut scene) = scene {
            if let Some(resource) = scene.resources.first_mut()
                && let Some(pixels) = &mut resource.pixels
            {
                pixels.pop();
            }
            let errors = validate_scene_errors(&scene);
            assert!(
                errors.iter().any(|error| error.contains("pixel length")),
                "{errors:#?}"
            );
        }

        let paired: Result<SceneTrace, _> =
            load_toml(&root.join("assurance/qualification/v2/scroll-before.toml"));
        assert!(paired.is_ok());
        if let Ok(mut paired) = paired {
            if let Some(pair) = &mut paired.pair {
                pair.step = pair.steps;
            }
            let errors = validate_scene_errors(&paired);
            assert!(
                errors
                    .iter()
                    .any(|error| error.contains("outside its sequence")),
                "{errors:#?}"
            );
        }

        let quad: Result<SceneTrace, _> =
            load_toml(&root.join("assurance/qualification/v2/clipped-grid.toml"));
        assert!(quad.is_ok());
        if let Ok(mut quad) = quad {
            if let Some(operation) = quad.operations.first_mut() {
                operation.atlas_x = Some(0);
            }
            let errors = validate_scene_errors(&quad);
            assert!(
                errors
                    .iter()
                    .any(|error| error.contains("cannot contain atlas bounds")),
                "{errors:#?}"
            );
        }
    }

    #[test]
    fn realistic_v2_clip_identifiers_do_not_change_declared_clip_order() {
        let root = repository_root();
        let scene: Result<SceneTrace, _> =
            load_toml(&root.join("assurance/qualification/v2/clipped-grid.toml"));
        assert!(scene.is_ok());
        if let Ok(mut scene) = scene {
            let expected = reference_bytes(&scene);
            assert!(expected.is_some());
            if scene.clips.len() >= 2 {
                let first = scene.clips[0].id.clone();
                let second = scene.clips[1].id.clone();
                scene.clips[0].id = "z-first-declared-clip".to_owned();
                scene.clips[1].id = "a-second-declared-clip".to_owned();
                for operation in &mut scene.operations {
                    if operation.clip.as_deref() == Some(first.as_str()) {
                        operation.clip = Some("z-first-declared-clip".to_owned());
                    } else if operation.clip.as_deref() == Some(second.as_str()) {
                        operation.clip = Some("a-second-declared-clip".to_owned());
                    }
                }
            }
            assert_eq!(reference_bytes(&scene), expected);
        }
    }

    #[test]
    fn journey_validation_rejects_each_contract_break() {
        let root = repository_root();
        let qualification: Result<Qualification, _> =
            load_toml(&root.join("assurance/qualification/v1/valid.toml"));
        let scene: Result<SceneTrace, _> =
            load_toml(&root.join("assurance/qualification/v1/scene.toml"));
        let journey: Result<Journey, _> =
            load_toml(&root.join("assurance/qualification/v1/journey.toml"));
        assert!(qualification.is_ok() && scene.is_ok() && journey.is_ok());
        if let (Ok(mut qualification), Ok(scene), Ok(mut journey)) = (qualification, scene, journey)
        {
            journey.scene_trace = "wrong.toml".to_owned();
            assert!(!super::validate_journey_errors(&qualification, &scene, &journey).is_empty());
            journey.scene_trace = qualification.scene_trace.clone();
            if let Some(action) = journey.actions.first_mut() {
                action.sequence = 4;
            }
            assert!(!super::validate_journey_errors(&qualification, &scene, &journey).is_empty());
            if let Some(action) = journey.actions.first_mut() {
                action.sequence = 0;
                action.kind = "unsupported-action".to_owned();
            }
            assert!(!super::validate_journey_errors(&qualification, &scene, &journey).is_empty());
            if let Some(action) = journey.actions.first_mut() {
                action.kind = "open-project".to_owned();
            }
            journey.expected_document_hash = "invalid".to_owned();
            assert!(!super::validate_journey_errors(&qualification, &scene, &journey).is_empty());
            journey.expected_document_hash = "b".repeat(64);
            qualification.comparison_level = "full-zed-path".to_owned();
            assert!(!super::validate_journey_errors(&qualification, &scene, &journey).is_empty());
        }
    }

    #[test]
    fn environment_and_stage_reject_each_missing_precondition() {
        let root = repository_root();
        let qualification: Result<Qualification, _> =
            load_toml(&root.join("assurance/qualification/v1/valid.toml"));
        assert!(qualification.is_ok());
        if let Ok(mut qualification) = qualification {
            qualification.environment.hardware_id.clear();
            assert!(!super::validate_environment_errors(&qualification).is_empty());
            qualification.environment.hardware_id = "fixture".to_owned();
            qualification.assumptions.clear();
            assert!(!super::validate_environment_errors(&qualification).is_empty());
            qualification.assumptions.push("assumption".to_owned());
            qualification.exclusions.clear();
            assert!(!super::validate_environment_errors(&qualification).is_empty());
            qualification.exclusions.push("exclusion".to_owned());

            qualification.state = "measured".to_owned();
            qualification.independent_windows = 0;
            assert!(!super::validate_stage_errors(&qualification).is_empty());
            qualification.state = "equivalent".to_owned();
            qualification.measurements.clear();
            qualification.independent_windows = 1;
            assert!(!super::validate_stage_errors(&qualification).is_empty());
        }
    }

    #[test]
    fn metric_validation_rejects_each_invalid_record() {
        let root = repository_root();
        let qualification: Result<Qualification, _> =
            load_toml(&root.join("assurance/qualification/v1/valid.toml"));
        assert!(qualification.is_ok());
        if let Ok(mut qualification) = qualification {
            if let Some(metric) = qualification.measurements.first_mut() {
                metric.name = "INVALID".to_owned();
            }
            assert!(!super::validate_metric_errors(&qualification, &root).is_empty());
            if let Some(metric) = qualification.measurements.first_mut() {
                metric.name = "scene-cpu-time".to_owned();
                metric.unit.clear();
            }
            assert!(!super::validate_metric_errors(&qualification, &root).is_empty());
            if let Some(metric) = qualification.measurements.first_mut() {
                metric.unit = "nanoseconds".to_owned();
                metric.sample_count = 1;
            }
            assert!(!super::validate_metric_errors(&qualification, &root).is_empty());
            if let Some(metric) = qualification.measurements.first_mut() {
                metric.sample_count = 2;
                metric.artifact_sha256 = "invalid".to_owned();
            }
            assert!(!super::validate_metric_errors(&qualification, &root).is_empty());
            if let Some(metric) = qualification.measurements.first_mut() {
                metric.artifact_sha256 = "e".repeat(64);
                metric.artifact = "missing.txt".to_owned();
            }
            assert!(!super::validate_metric_errors(&qualification, &root).is_empty());
        }
    }

    #[test]
    fn primitive_predicates_reject_boundaries() {
        assert!(valid_slug("scene-1"));
        assert!(!valid_slug(""));
        assert!(!valid_slug("Scene"));
        assert!(!valid_slug("scene_name"));
        assert!(finite_positive(1.0));
        assert!(!finite_positive(0.0));
        assert!(!finite_positive(-1.0));
        assert!(!finite_positive(f32::NAN));
        assert!(!finite_positive(f32::INFINITY));
    }
}
