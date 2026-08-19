//! Validates fixed-hardware onscreen SDR compositor evidence.

use serde::Deserialize;
use std::{
    collections::BTreeMap,
    fs,
    path::{Component, Path},
    process::Command,
};

const SCHEMA: &str = "alpine-onscreen-sdr-capture/v1";
const REQUIRED: [(&str, &str); 4] = [
    ("launch", "accepted"),
    ("resize", "accepted"),
    ("display-move", "accepted"),
    ("wrong-transfer", "wrong-transfer"),
];
const ACCEPTED_EXPECTED: [u8; 5] = [0, 118, 188, 225, 255];
const WRONG_EXPECTED: [u8; 5] = [0, 181, 223, 241, 255];

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(transparent)]
struct EvidenceFlag(bool);

impl EvidenceFlag {
    const fn is_set(self) -> bool {
        self.0
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CaptureEvidence {
    schema: String,
    task_issue: u64,
    revision: String,
    stage: String,
    control: String,
    os_build: String,
    hardware_model: String,
    screen_capture_permission: EvidenceFlag,
    display_count: u32,
    window_id: u32,
    display_id: u32,
    backing_scale: f64,
    logical_width: f64,
    logical_height: f64,
    capture_width: u32,
    capture_height: u32,
    target_format: String,
    layer_color_space: String,
    capture_color_space: String,
    capture_image_color_space: String,
    display_profile_name: String,
    extended_dynamic_range: EvidenceFlag,
    scene_revision: u64,
    presented_time_bits: u64,
    scene_file: String,
    scene_sha256: String,
    capture_file: String,
    capture_sha256: String,
    display_profile_file: String,
    display_profile_sha256: String,
    samples: Vec<u8>,
    accepted_expected: Vec<u8>,
    control_expected: Vec<u8>,
    accepted_max_error: u8,
    control_max_error: u8,
    qualified: EvidenceFlag,
    performance_claim: EvidenceFlag,
}

pub(crate) fn run(command: &str, bundle: &Path) -> Result<String, Vec<String>> {
    let mut captures = BTreeMap::new();
    let mut errors = Vec::new();
    for (stage, control) in REQUIRED {
        let path = bundle.join(format!("{stage}.toml"));
        match load_capture(&path) {
            Ok(capture) => {
                validate_capture(bundle, stage, control, &capture, &mut errors);
                captures.insert(stage, capture);
            }
            Err(error) => errors.push(error),
        }
    }
    if errors.is_empty() {
        validate_bundle(&captures, &mut errors);
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let launch = captures
        .get("launch")
        .ok_or_else(|| vec!["launch capture disappeared after validation".to_owned()])?;
    if command == "validate-onscreen-sdr" {
        return Ok(format!(
            "validated task #234 onscreen SDR bundle at revision {} across {} displays with no performance claim",
            launch.revision, launch.display_count
        ));
    }
    Ok(format!(
        "# Alpine onscreen SDR qualification report\n\n- Revision: `{}`\n- Hardware: {}\n- OS build: {}\n- Displays: {}\n- Accepted stages: launch, resize, display move\n- Deliberate wrong-transfer control: rejected\n- Performance claim: none\n",
        launch.revision, launch.hardware_model, launch.os_build, launch.display_count
    ))
}

fn load_capture(path: &Path) -> Result<CaptureEvidence, String> {
    let source = fs::read_to_string(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    toml::from_str(&source).map_err(|error| format!("cannot parse {}: {error}", path.display()))
}

fn validate_capture(
    bundle: &Path,
    expected_stage: &str,
    expected_control: &str,
    capture: &CaptureEvidence,
    errors: &mut Vec<String>,
) {
    validate_identity(expected_stage, expected_control, capture, errors);
    validate_geometry_and_color(capture, errors);
    validate_transfer(expected_control, capture, errors);
    validate_artifact(bundle, &capture.scene_file, &capture.scene_sha256, errors);
    validate_artifact(
        bundle,
        &capture.capture_file,
        &capture.capture_sha256,
        errors,
    );
    validate_artifact(
        bundle,
        &capture.display_profile_file,
        &capture.display_profile_sha256,
        errors,
    );
}

fn validate_identity(
    expected_stage: &str,
    expected_control: &str,
    capture: &CaptureEvidence,
    errors: &mut Vec<String>,
) {
    require(
        capture.schema == SCHEMA,
        "capture schema must be exact",
        errors,
    );
    require(
        capture.task_issue == 234,
        "capture must bind task #234",
        errors,
    );
    require(
        capture.stage == expected_stage,
        format!("{expected_stage} file has mismatched stage"),
        errors,
    );
    require(
        capture.control == expected_control,
        format!("{expected_stage} file has mismatched control"),
        errors,
    );
    require(
        valid_hash(&capture.revision, 40),
        "revision must be a full lowercase Git hash",
        errors,
    );
    require(
        capture.scene_revision != 0,
        "scene revision must be nonzero",
        errors,
    );
    require(!capture.os_build.is_empty(), "OS build is required", errors);
    require(
        !capture.hardware_model.is_empty(),
        "hardware model is required",
        errors,
    );
    require(
        capture.screen_capture_permission.is_set(),
        "Screen Recording permission must be preflighted",
        errors,
    );
    require(
        capture.display_count >= 2,
        "at least two physical displays are required",
        errors,
    );
    require(capture.window_id != 0, "window ID must be nonzero", errors);
    require(
        capture.display_id != 0,
        "display ID must be nonzero",
        errors,
    );
}

fn validate_geometry_and_color(capture: &CaptureEvidence, errors: &mut Vec<String>) {
    require(
        capture.backing_scale.is_finite() && capture.backing_scale > 0.0,
        "backing scale must be finite and positive",
        errors,
    );
    require(
        capture.logical_width.is_finite() && capture.logical_width > 0.0,
        "logical width must be finite and positive",
        errors,
    );
    require(
        capture.logical_height.is_finite() && capture.logical_height > 0.0,
        "logical height must be finite and positive",
        errors,
    );
    require(
        capture.capture_width > 0 && capture.capture_height > 0,
        "capture extent must be nonzero",
        errors,
    );
    require(
        capture.target_format == "BGRA8Unorm_sRGB",
        "target format must be BGRA8Unorm_sRGB",
        errors,
    );
    require(
        capture.layer_color_space == "kCGColorSpaceSRGB",
        "layer color space must be standard sRGB",
        errors,
    );
    require(
        capture.capture_color_space == "kCGColorSpaceSRGB",
        "capture output must be explicit sRGB",
        errors,
    );
    require(
        capture.capture_image_color_space == "kCGColorSpaceSRGB",
        "captured image must report standard sRGB",
        errors,
    );
    require(
        !capture.display_profile_name.is_empty(),
        "display profile name is required",
        errors,
    );
    require(
        !capture.extended_dynamic_range.is_set(),
        "EDR must remain disabled",
        errors,
    );
    require(
        capture.presented_time_bits != 0,
        "nonzero compositor presentation evidence is required",
        errors,
    );
    require(
        capture.samples.len() == 5,
        "five patch samples are required",
        errors,
    );
}

fn validate_transfer(expected_control: &str, capture: &CaptureEvidence, errors: &mut Vec<String>) {
    require(
        capture.accepted_expected == ACCEPTED_EXPECTED,
        "accepted oracle values drifted",
        errors,
    );
    require(
        capture.control_expected == WRONG_EXPECTED,
        "wrong-transfer oracle values drifted",
        errors,
    );
    require(
        capture.qualified.is_set(),
        "capture did not qualify",
        errors,
    );
    require(
        !capture.performance_claim.is_set(),
        "onscreen correctness evidence cannot contain a performance claim",
        errors,
    );
    if expected_control == "accepted" {
        require(
            capture.accepted_max_error <= 12,
            "accepted capture exceeds the 12-byte tolerance",
            errors,
        );
    } else {
        require(
            capture.accepted_max_error >= 30,
            "wrong-transfer control is not discriminating",
            errors,
        );
        require(
            capture.control_max_error <= 12,
            "wrong-transfer capture does not match its deliberate control",
            errors,
        );
    }
}

fn validate_bundle(captures: &BTreeMap<&str, CaptureEvidence>, errors: &mut Vec<String>) {
    let Some(launch) = captures.get("launch") else {
        return;
    };
    for capture in captures.values() {
        require(
            capture.revision == launch.revision,
            "capture revisions must match",
            errors,
        );
        require(
            capture.os_build == launch.os_build,
            "OS builds must match",
            errors,
        );
        require(
            capture.hardware_model == launch.hardware_model,
            "hardware models must match",
            errors,
        );
        require(
            capture.display_count == launch.display_count,
            "display counts must match",
            errors,
        );
    }
    let resize = &captures["resize"];
    let moved = &captures["display-move"];
    let wrong = &captures["wrong-transfer"];
    require(
        launch.scene_revision.checked_add(1) == Some(resize.scene_revision)
            && resize.scene_revision.checked_add(1) == Some(moved.scene_revision)
            && moved.scene_revision.checked_add(1) == Some(wrong.scene_revision),
        "capture scene revisions must be consecutive in stage order",
        errors,
    );
    require(
        launch.display_id == resize.display_id,
        "launch and resize must remain on one display",
        errors,
    );
    require(
        launch.backing_scale.to_bits() == resize.backing_scale.to_bits(),
        "launch and resize backing scales must match",
        errors,
    );
    require(
        launch.logical_width.to_bits() != resize.logical_width.to_bits()
            || launch.logical_height.to_bits() != resize.logical_height.to_bits(),
        "resize must change logical geometry",
        errors,
    );
    require(
        moved.display_id != launch.display_id,
        "display move must change physical display identity",
        errors,
    );
    require(
        moved.backing_scale.to_bits() != launch.backing_scale.to_bits(),
        "display move must change real backing scale",
        errors,
    );
    require(
        wrong.display_id == moved.display_id,
        "wrong control must remain on the moved display",
        errors,
    );
    require(
        wrong.backing_scale.to_bits() == moved.backing_scale.to_bits(),
        "wrong control must preserve moved backing scale",
        errors,
    );
    require(
        launch.scene_sha256 == resize.scene_sha256 && launch.scene_sha256 == moved.scene_sha256,
        "accepted stages must share one normalized scene hash",
        errors,
    );
    require(
        wrong.scene_sha256 != launch.scene_sha256,
        "wrong-transfer scene hash must differ",
        errors,
    );
}

fn validate_artifact(bundle: &Path, relative: &str, expected: &str, errors: &mut Vec<String>) {
    if !valid_hash(expected, 64) {
        errors.push(format!("artifact {relative:?} has an invalid SHA-256"));
        return;
    }
    let path = Path::new(relative);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        errors.push(format!("artifact path {relative:?} escapes the bundle"));
        return;
    }
    let full = bundle.join(path);
    match hash_file(&full) {
        Ok(actual) if actual == expected => {}
        Ok(actual) => errors.push(format!(
            "artifact {} hash mismatch: expected {expected}, got {actual}",
            full.display()
        )),
        Err(error) => errors.push(error),
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
        if let Some(hash) = stdout
            .split(|character: char| !character.is_ascii_hexdigit())
            .find(|word| valid_hash(word, 64))
        {
            return Ok(hash.to_owned());
        }
    }
    Err(format!("cannot calculate SHA-256 for {}", path.display()))
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
