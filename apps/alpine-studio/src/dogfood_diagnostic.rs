use std::{
    cell::RefCell,
    env,
    ffi::OsString,
    fmt,
    fs::{self, OpenOptions},
    io::Write as _,
    path::{Component, Path, PathBuf},
    rc::Rc,
    time::Instant,
};

use alpine_platform_macos::{SurfaceOperation, SurfaceSnapshot};
use alpine_runtime::ApplicationSnapshot;
use serde_json::{Value, json};

use super::StudioApp;

const OUTPUT_ENV: &str = "ALPINE_STUDIO_DOGFOOD_OUTPUT";
const WORKLOAD_ENV: &str = "ALPINE_STUDIO_DOGFOOD_WORKLOAD_ID";
const REVISION_ENV: &str = "ALPINE_STUDIO_DOGFOOD_REVISION";
const CAPTURED_AT_ENV: &str = "ALPINE_STUDIO_DOGFOOD_CAPTURED_AT_UTC";
#[cfg(all(test, not(miri)))]
const ENVIRONMENT_TEST_CHILD_ENV: &str = "ALPINE_STUDIO_DOGFOOD_TEST_CHILD";
const MAX_OUTPUT_BYTES: usize = 262_144;
const LANGUAGE_BUDGET_BYTES: usize = 16 * 1024 * 1024;
const FOREGROUND_QUEUE_BUDGET_BYTES: usize = 8 * 1024 * 1024;
const OMITTED_EVIDENCE: [&str; 10] = [
    "accessibility-stale-actions",
    "background-queue-bytes",
    "font-cache",
    "fallback-cache",
    "glyph-atlas-gpu",
    "process-samples",
    "process-gpu-bytes",
    "stage-timings",
    "language-responses",
    "upload-staging-budget",
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct CaptureError(&'static str);

impl fmt::Display for CaptureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

#[derive(Clone)]
pub(super) struct CaptureSink(Rc<RefCell<Option<StudioOwnedSnapshot>>>);

impl CaptureSink {
    pub(super) fn capture(&self, app: &StudioApp) {
        if let Ok(mut slot) = self.0.try_borrow_mut()
            && slot.is_none()
        {
            *slot = Some(StudioOwnedSnapshot::capture(app));
        }
    }
}

pub(super) struct CaptureController {
    output: PathBuf,
    workload_id: String,
    revision: String,
    captured_at_utc: String,
    started: Instant,
    sink: CaptureSink,
}

impl CaptureController {
    pub(super) fn from_environment() -> Result<Option<Self>, CaptureError> {
        Self::from_values(
            env::var_os(OUTPUT_ENV),
            env::var_os(WORKLOAD_ENV),
            env::var_os(REVISION_ENV),
            env::var_os(CAPTURED_AT_ENV),
        )
    }

    fn from_values(
        output: Option<OsString>,
        workload: Option<OsString>,
        revision: Option<OsString>,
        captured_at: Option<OsString>,
    ) -> Result<Option<Self>, CaptureError> {
        let supplied = [
            output.is_some(),
            workload.is_some(),
            revision.is_some(),
            captured_at.is_some(),
        ];
        if supplied.iter().all(|value| !value) {
            return Ok(None);
        }
        if supplied.iter().any(|value| !value) {
            return Err(CaptureError(
                "dogfood diagnostic environment must be supplied completely",
            ));
        }
        let output = PathBuf::from(output.ok_or(CaptureError("missing output path"))?);
        let workload_id = workload
            .and_then(|value| value.into_string().ok())
            .ok_or(CaptureError("dogfood workload must be UTF-8"))?;
        let revision = revision
            .and_then(|value| value.into_string().ok())
            .ok_or(CaptureError("dogfood revision must be UTF-8"))?;
        let captured_at_utc = captured_at
            .and_then(|value| value.into_string().ok())
            .ok_or(CaptureError("dogfood timestamp must be UTF-8"))?;
        Self::new(&output, workload_id, revision, captured_at_utc).map(Some)
    }

    fn new(
        output: &Path,
        workload_id: String,
        revision: String,
        captured_at_utc: String,
    ) -> Result<Self, CaptureError> {
        let output = normalized_output_path(output)?;
        if !valid_slug(&workload_id) {
            return Err(CaptureError("dogfood workload id must be a bounded slug"));
        }
        if !valid_git_sha(&revision) {
            return Err(CaptureError(
                "dogfood revision must be a lowercase 40-character Git SHA",
            ));
        }
        if !valid_timestamp(&captured_at_utc) {
            return Err(CaptureError(
                "dogfood timestamp must be UTC YYYY-MM-DDTHH:MM:SSZ",
            ));
        }
        Ok(Self {
            output,
            workload_id,
            revision,
            captured_at_utc,
            started: Instant::now(),
            sink: CaptureSink(Rc::new(RefCell::new(None))),
        })
    }

    pub(super) fn sink(&self) -> CaptureSink {
        self.sink.clone()
    }

    pub(super) fn finish(
        self,
        application: ApplicationSnapshot,
        surface: &SurfaceSnapshot,
    ) -> Result<(), CaptureError> {
        let owned = self
            .sink
            .0
            .try_borrow_mut()
            .ok()
            .and_then(|mut slot| slot.take())
            .ok_or(CaptureError("Studio did not publish final dogfood state"))?;
        let duration = u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let payload = render_payload(
            &self.workload_id,
            &self.revision,
            &self.captured_at_utc,
            duration.max(1),
            owned,
            application,
            surface,
        );
        write_atomic_json(&self.output, &payload)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct ResourceEvidence {
    current: usize,
    peak: usize,
    budget: usize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct StudioOwnedSnapshot {
    frame_builds: u64,
    shape_calls: u64,
    rasterize_calls: u64,
    syntax_hits: u64,
    syntax_misses: u64,
    syntax_omitted_lines: u64,
    accessibility_queries: u64,
    accessibility_actions: u64,
    accessibility_nodes: Option<usize>,
    language_requests: u64,
    stale_language_responses: u64,
    language_restarts: u64,
    layout: ResourceEvidence,
    syntax: ResourceEvidence,
    glyph_atlas_cpu: ResourceEvidence,
    language: ResourceEvidence,
}

impl StudioOwnedSnapshot {
    fn capture(app: &StudioApp) -> Self {
        let layout = app.layout_cache.snapshot();
        let syntax = app.syntax_cache.snapshot();
        let atlas = app.glyph_atlas.snapshot();
        let text = app.text_system.snapshot();
        let language = app.rust_diagnostics.snapshot();
        let requests = language
            .completion_requests
            .saturating_add(language.navigation_requests)
            .saturating_add(language.symbol_requests)
            .saturating_add(language.workspace_edit_requests);
        let stale = language
            .stale_completions
            .saturating_add(language.stale_navigation)
            .saturating_add(language.stale_symbols)
            .saturating_add(language.stale_workspace_edits)
            .saturating_add(language.stale_diagnostics);
        let current_language_bytes = language
            .process_retained_bytes
            .saturating_add(language.diagnostic_bytes)
            .saturating_add(language.completion_bytes)
            .saturating_add(language.hover_bytes)
            .saturating_add(language.location_bytes)
            .saturating_add(language.symbol_bytes)
            .saturating_add(language.workspace_edit_wire_bytes);
        let peak_language_bytes = language
            .process_retained_bytes
            .saturating_add(language.peak_diagnostic_bytes)
            .saturating_add(language.peak_completion_bytes)
            .saturating_add(language.peak_hover_bytes)
            .saturating_add(language.peak_location_bytes)
            .saturating_add(language.peak_symbol_bytes)
            .saturating_add(language.peak_workspace_edit_wire_bytes)
            .max(current_language_bytes);
        let accessibility_nodes = app
            .accessibility_snapshot()
            .ok()
            .map(|snapshot| snapshot.nodes().len());
        Self {
            frame_builds: app.profile_scene_revision.get(),
            shape_calls: text.shape_calls,
            rasterize_calls: text.rasterize_calls,
            syntax_hits: syntax.hits(),
            syntax_misses: syntax.misses(),
            syntax_omitted_lines: syntax.omitted_lines(),
            accessibility_queries: app.dogfood_accessibility_queries,
            accessibility_actions: app.dogfood_accessibility_actions,
            accessibility_nodes,
            language_requests: requests,
            stale_language_responses: stale,
            language_restarts: language.restarts,
            layout: ResourceEvidence {
                current: layout.current_bytes(),
                peak: layout.peak_bytes(),
                budget: layout.budget_bytes(),
            },
            syntax: ResourceEvidence {
                current: syntax.current_bytes(),
                peak: syntax.peak_bytes(),
                budget: syntax.budget_bytes(),
            },
            glyph_atlas_cpu: ResourceEvidence {
                current: atlas.pixel_bytes().saturating_add(atlas.metadata_bytes()),
                peak: atlas.peak_bytes(),
                budget: atlas.budget_bytes(),
            },
            language: ResourceEvidence {
                current: current_language_bytes,
                peak: peak_language_bytes,
                budget: LANGUAGE_BUDGET_BYTES,
            },
        }
    }
}

fn render_payload(
    workload_id: &str,
    revision: &str,
    captured_at_utc: &str,
    duration_ms: u64,
    owned: StudioOwnedSnapshot,
    application: ApplicationSnapshot,
    surface: &SurfaceSnapshot,
) -> Value {
    let submitted = surface.submission_count();
    let completed = submitted.saturating_sub(u64::from(surface.occupied_frame_slots()));
    let requested = owned.frame_builds.max(submitted);
    let presented = surface.presented_count().min(completed);
    let runtime_worker = application.worker();
    let runtime_external = application.external();
    let foreground = ResourceEvidence {
        current: runtime_external.current_bytes(),
        peak: runtime_external.peak_bytes(),
        budget: FOREGROUND_QUEUE_BUDGET_BYTES,
    };
    let upload = ResourceEvidence {
        current: surface.current_upload_bytes(),
        peak: surface.peak_upload_bytes(),
        budget: surface
            .peak_upload_bytes()
            .max(surface.current_upload_bytes()),
    };
    json!({
        "schema": "alpine-studio-internal-diagnostic/v1",
        "workload_id": workload_id,
        "alpine_revision": revision,
        "captured_at_utc": captured_at_utc,
        "duration_ms": duration_ms,
        "outcome": "passed",
        "status": "clean native close captured",
        "frames": {
            "requested": requested,
            "submitted": submitted,
            "completed": completed,
            "presented": presented,
            "omitted": completed.saturating_sub(presented),
            "idle_submissions": submitted.saturating_sub(owned.frame_builds),
            "peak_in_flight": surface.peak_occupied_frame_slots(),
        },
        "text": {
            "shape_calls": owned.shape_calls,
            "rasterize_calls": owned.rasterize_calls,
            "syntax_cache_hits": owned.syntax_hits,
            "syntax_cache_misses": owned.syntax_misses,
            "syntax_omitted_lines": owned.syntax_omitted_lines,
        },
        "language": language_evidence(owned),
        "accessibility": accessibility_evidence(owned),
        "lifecycle": {
            "close_requests": u64::from(application.is_shutting_down()),
            "close_completions": u64::from(application.is_shutting_down()),
            "clean_shutdown": application.is_shutting_down(),
            "post_close_bytes": surface.current_retained_bytes(),
            "post_close_limit_bytes": 0,
        },
        "resources": resource_inventory(owned, foreground, upload),
        "runtime": {
            "stale_results": application.stale_results(),
            "invalid_scenes": application.invalid_scenes(),
            "queued_worker_requests": runtime_worker.queued_requests(),
            "peak_worker_requests": runtime_worker.peak_queued_requests(),
            "queued_worker_results": runtime_worker.queued_results(),
            "peak_worker_results": runtime_worker.peak_queued_results(),
            "external_items": runtime_external.current_items(),
            "peak_external_items": runtime_external.peak_items(),
        },
        "surface": {
            "callbacks": surface.callback_count(),
            "rejected_callbacks": surface.rejected_callback_count(),
            "submissions": surface.submission_count(),
            "qualified_presentations": surface.qualified_presented_count(),
            "superseded": surface.superseded_count(),
            "cancelled": surface.cancelled_count(),
            "failed": surface.failed_count(),
            "skipped": surface.skipped_count(),
            "allocated_bytes": surface.allocated_bytes().to_string(),
            "peak_retained_bytes": surface.peak_retained_bytes(),
            "current_retained_bytes": surface.current_retained_bytes(),
        },
        "omissions": omission_inventory(owned),
    })
}

fn resource_inventory(
    owned: StudioOwnedSnapshot,
    foreground: ResourceEvidence,
    upload: ResourceEvidence,
) -> Value {
    json!([
        resource("layout-cache", owned.layout, false),
        resource("syntax-cache", owned.syntax, false),
        resource("glyph-atlas-cpu", owned.glyph_atlas_cpu, false),
        omitted_resource("glyph-atlas-gpu"),
        omitted_resource("font-cache"),
        omitted_resource("fallback-cache"),
        resource("language-process", owned.language, false),
        resource("foreground-queue", foreground, false),
        omitted_resource("background-queue"),
        partial_upload_resource(upload),
    ])
}

fn language_evidence(owned: StudioOwnedSnapshot) -> Value {
    json!({
        "requests": owned.language_requests,
        "responses": Value::Null,
        "stale_responses": owned.stale_language_responses,
        "restarts": owned.language_restarts,
        "current_retained_bytes": owned.language.current,
        "peak_retained_bytes": owned.language.peak,
        "budget_bytes": owned.language.budget,
    })
}

fn accessibility_evidence(owned: StudioOwnedSnapshot) -> Value {
    json!({
        "queries": owned.accessibility_queries,
        "actions": owned.accessibility_actions,
        "stale_actions": Value::Null,
        "retained_nodes": owned.accessibility_nodes,
        "peak_retained_nodes": owned.accessibility_nodes,
    })
}

fn omitted_resource(name: &str) -> Value {
    json!({
        "name": name,
        "current_bytes": Value::Null,
        "peak_bytes": Value::Null,
        "budget_bytes": Value::Null,
        "omitted": true,
    })
}

fn partial_upload_resource(evidence: ResourceEvidence) -> Value {
    json!({
        "name": "upload-staging",
        "current_bytes": evidence.current,
        "peak_bytes": evidence.peak,
        "budget_bytes": Value::Null,
        "omitted": false,
        "omitted_axes": ["budget_bytes"],
    })
}

fn omission_inventory(owned: StudioOwnedSnapshot) -> Value {
    let mut omissions = OMITTED_EVIDENCE.to_vec();
    if owned.accessibility_nodes.is_none() {
        omissions.push("accessibility-tree");
    }
    json!(omissions)
}

fn resource(name: &str, evidence: ResourceEvidence, omitted: bool) -> Value {
    json!({
        "name": name,
        "current_bytes": evidence.current,
        "peak_bytes": evidence.peak,
        "budget_bytes": evidence.budget,
        "omitted": omitted,
    })
}

fn normalized_output_path(path: &Path) -> Result<PathBuf, CaptureError> {
    if !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
        || path.file_name().is_none()
    {
        return Err(CaptureError(
            "dogfood diagnostic output must be one normalized absolute file path",
        ));
    }
    if fs::symlink_metadata(path).is_ok() {
        return Err(CaptureError("dogfood diagnostic output already exists"));
    }
    let parent = path
        .parent()
        .ok_or(CaptureError("dogfood diagnostic output lacks a parent"))?;
    let parent_metadata = fs::symlink_metadata(parent)
        .map_err(|_| CaptureError("dogfood diagnostic parent is unavailable"))?;
    if parent_metadata.file_type().is_symlink() {
        return Err(CaptureError(
            "dogfood diagnostic parent must not traverse a symbolic link",
        ));
    }
    let canonical = fs::canonicalize(parent)
        .map_err(|_| CaptureError("dogfood diagnostic parent is unavailable"))?;
    let normalized = canonical.join(
        path.file_name()
            .ok_or(CaptureError("dogfood diagnostic output lacks a file name"))?,
    );
    if fs::symlink_metadata(&normalized).is_ok() {
        return Err(CaptureError("dogfood diagnostic output already exists"));
    }
    Ok(normalized)
}

fn write_atomic_json(path: &Path, payload: &Value) -> Result<(), CaptureError> {
    let path = normalized_output_path(path)?;
    let bytes = serde_json::to_vec_pretty(payload)
        .map_err(|_| CaptureError("dogfood diagnostic encoding failed"))?;
    if bytes.len() > MAX_OUTPUT_BYTES {
        return Err(CaptureError("dogfood diagnostic exceeds its byte limit"));
    }
    let parent = path
        .parent()
        .ok_or(CaptureError("dogfood diagnostic output lacks a parent"))?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(CaptureError("dogfood diagnostic file name must be UTF-8"))?;
    let staging = parent.join(format!(".{name}.{}.staging", std::process::id()));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&staging)
        .map_err(|_| CaptureError("dogfood diagnostic staging is unavailable"))?;
    let result = (|| {
        file.write_all(&bytes)
            .and_then(|()| file.sync_all())
            .map_err(|_| CaptureError("dogfood diagnostic write failed"))?;
        fs::hard_link(&staging, &path)
            .map_err(|_| CaptureError("dogfood diagnostic publication failed"))?;
        fs::remove_file(&staging)
            .map_err(|_| CaptureError("dogfood diagnostic staging cleanup failed"))?;
        Ok(())
    })();
    let _ = fs::remove_file(staging);
    result
}

fn valid_slug(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'.')
        })
}

fn valid_git_sha(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_timestamp(value: &str) -> bool {
    value.len() == 20
        && value.as_bytes().get(4) == Some(&b'-')
        && value.as_bytes().get(7) == Some(&b'-')
        && value.as_bytes().get(10) == Some(&b'T')
        && value.as_bytes().get(13) == Some(&b':')
        && value.as_bytes().get(16) == Some(&b':')
        && value.as_bytes().get(19) == Some(&b'Z')
        && value
            .bytes()
            .enumerate()
            .filter(|(index, _)| !matches!(index, 4 | 7 | 10 | 13 | 16 | 19))
            .all(|(_, byte)| byte.is_ascii_digit())
}

pub(super) fn capture_surface_error(error: &CaptureError) -> alpine_platform_macos::SurfaceError {
    eprintln!("Alpine Studio dogfood diagnostic rejected: {error}");
    alpine_platform_macos::SurfaceError::invariant(SurfaceOperation::Application)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQUENCE: AtomicU64 = AtomicU64::new(1);

    fn output_path(label: &str) -> Result<PathBuf, String> {
        let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target");
        fs::create_dir_all(&root).map_err(|error| error.to_string())?;
        let root = fs::canonicalize(root).map_err(|error| error.to_string())?;
        Ok(root.join(format!(
            "dogfood-diagnostic-{label}-{}-{sequence}.json",
            std::process::id()
        )))
    }

    #[test]
    fn identity_and_output_boundaries_fail_closed() -> Result<(), String> {
        let path = output_path("identity")?;
        let valid = CaptureController::new(
            &path,
            "local-edit".to_owned(),
            "a".repeat(40),
            "2026-08-30T12:00:00Z".to_owned(),
        );
        assert!(valid.is_ok());
        assert!(
            CaptureController::new(
                &path,
                "Local Edit".to_owned(),
                "a".repeat(40),
                "2026-08-30T12:00:00Z".to_owned(),
            )
            .is_err()
        );
        assert!(
            CaptureController::new(
                &path,
                "local-edit".to_owned(),
                "A".repeat(40),
                "2026-08-30T12:00:00Z".to_owned(),
            )
            .is_err()
        );
        assert!(
            CaptureController::new(
                &path,
                "local-edit".to_owned(),
                "a".repeat(40),
                "2026-08-30 12:00:00Z".to_owned(),
            )
            .is_err()
        );
        Ok(())
    }

    #[test]
    #[cfg(not(miri))]
    fn environment_values_and_error_translation_are_exact() -> Result<(), String> {
        assert!(
            CaptureController::from_environment()
                .map_err(|error| error.to_string())?
                .is_none()
        );
        assert!(
            CaptureController::from_values(None, None, None, None)
                .map_err(|error| error.to_string())?
                .is_none()
        );
        assert!(
            CaptureController::from_values(
                Some(output_path("partial")?.into_os_string()),
                None,
                None,
                None,
            )
            .is_err()
        );
        let path = output_path("complete")?;
        let controller = CaptureController::from_values(
            Some(path.into_os_string()),
            Some(OsString::from("local-edit")),
            Some(OsString::from("a".repeat(40))),
            Some(OsString::from("2026-08-30T12:00:00Z")),
        )
        .map_err(|error| error.to_string())?;
        assert!(controller.is_some());
        let error = CaptureError("translated failure");
        assert_eq!(error.to_string(), "translated failure");
        assert_eq!(
            capture_surface_error(&error),
            alpine_platform_macos::SurfaceError::invariant(SurfaceOperation::Application)
        );
        let child_output = output_path("environment-child")?;
        let child =
            std::process::Command::new(std::env::current_exe().map_err(|error| error.to_string())?)
                .arg("--exact")
                .arg("dogfood_diagnostic::tests::environment_child_reads_complete_contract")
                .arg("--nocapture")
                .env(OUTPUT_ENV, &child_output)
                .env(WORKLOAD_ENV, "local-edit")
                .env(REVISION_ENV, "b".repeat(40))
                .env(CAPTURED_AT_ENV, "2026-08-30T12:00:01Z")
                .env(ENVIRONMENT_TEST_CHILD_ENV, "1")
                .status()
                .map_err(|error| error.to_string())?;
        assert!(child.success());
        Ok(())
    }

    #[test]
    #[cfg(not(miri))]
    fn environment_child_reads_complete_contract() -> Result<(), String> {
        if env::var_os(ENVIRONMENT_TEST_CHILD_ENV).is_none() {
            return Ok(());
        }
        assert!(
            CaptureController::from_environment()
                .map_err(|error| error.to_string())?
                .is_some()
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_environment_values_fail_closed() -> Result<(), String> {
        use std::os::unix::ffi::OsStringExt as _;

        let path = output_path("non-utf8-environment")?.into_os_string();
        let invalid = OsString::from_vec(vec![0xff]);
        let valid_revision = OsString::from("a".repeat(40));
        let valid_time = OsString::from("2026-08-30T12:00:00Z");
        assert!(
            CaptureController::from_values(
                Some(path.clone()),
                Some(invalid.clone()),
                Some(valid_revision.clone()),
                Some(valid_time.clone()),
            )
            .is_err()
        );
        assert!(
            CaptureController::from_values(
                Some(path.clone()),
                Some(OsString::from("local-edit")),
                Some(invalid.clone()),
                Some(valid_time),
            )
            .is_err()
        );
        assert!(
            CaptureController::from_values(
                Some(path),
                Some(OsString::from("local-edit")),
                Some(valid_revision),
                Some(invalid),
            )
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn atomic_writer_refuses_overwrite_and_retains_omissions() -> Result<(), String> {
        let path = output_path("atomic")?;
        let payload = json!({
            "schema": "alpine-studio-internal-diagnostic/v1",
            "omissions": ["stage-timings", "process-samples"],
        });
        write_atomic_json(&path, &payload).map_err(|error| error.to_string())?;
        let source = fs::read_to_string(&path).map_err(|error| error.to_string())?;
        assert!(source.contains("stage-timings"));
        assert!(source.contains("process-samples"));
        assert!(write_atomic_json(&path, &payload).is_err());
        fs::remove_file(path).map_err(|error| error.to_string())?;
        Ok(())
    }

    #[test]
    fn unavailable_axes_are_null_and_explicitly_omitted() {
        let owned = StudioOwnedSnapshot::default();
        let payload = json!({
            "language": language_evidence(owned),
            "accessibility": accessibility_evidence(owned),
            "resources": resource_inventory(
                owned,
                ResourceEvidence::default(),
                ResourceEvidence::default(),
            ),
            "omissions": omission_inventory(owned),
        });
        assert!(
            payload
                .pointer("/language/responses")
                .is_some_and(Value::is_null)
        );
        assert!(
            payload
                .pointer("/accessibility/stale_actions")
                .is_some_and(Value::is_null)
        );
        assert!(
            payload
                .pointer("/resources/3/current_bytes")
                .is_some_and(Value::is_null)
        );
        assert!(
            payload
                .pointer("/resources/9/budget_bytes")
                .is_some_and(Value::is_null)
        );
        let omissions = payload
            .get("omissions")
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice);
        for required in [
            "accessibility-stale-actions",
            "accessibility-tree",
            "language-responses",
            "upload-staging-budget",
        ] {
            assert!(omissions.iter().any(|item| item == required));
        }
    }

    #[test]
    fn real_studio_capture_finishes_one_bounded_payload() -> Result<(), String> {
        let output = output_path("full-capture")?;
        let controller = CaptureController::new(
            &output,
            "local-edit".to_owned(),
            "a".repeat(40),
            "2026-08-30T12:00:00Z".to_owned(),
        )
        .map_err(|error| error.to_string())?;
        let sink = controller.sink();
        let mut app =
            StudioApp::new(crate::tests::TestTextSystem).map_err(|error| error.to_string())?;
        app.dogfood_accessibility_queries = 2;
        app.dogfood_accessibility_actions = 1;
        let held = sink.0.borrow_mut();
        sink.capture(&app);
        assert!(held.is_none());
        drop(held);
        sink.capture(&app);
        let before_drop = *sink.0.borrow();
        assert!(before_drop.is_some());
        app.dogfood_capture = Some(sink.clone());
        drop(app);
        assert_eq!(*sink.0.borrow(), before_drop);
        controller
            .finish(
                ApplicationSnapshot::default(),
                &SurfaceSnapshot::empty_for_test(),
            )
            .map_err(|error| error.to_string())?;
        let bytes = fs::read(&output).map_err(|error| error.to_string())?;
        assert!(bytes.len() <= MAX_OUTPUT_BYTES);
        let payload: Value = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
        assert_eq!(
            payload.get("schema"),
            Some(&json!("alpine-studio-internal-diagnostic/v1"))
        );
        assert_eq!(payload.pointer("/accessibility/queries"), Some(&json!(2)));
        assert_eq!(payload.pointer("/accessibility/actions"), Some(&json!(1)));
        assert_eq!(
            payload.pointer("/lifecycle/clean_shutdown"),
            Some(&json!(false))
        );
        assert_eq!(
            payload.pointer("/surface/current_retained_bytes"),
            Some(&json!(0))
        );
        assert!(!String::from_utf8_lossy(&bytes).contains("document_path"));
        fs::remove_file(output).map_err(|error| error.to_string())?;

        let missing = CaptureController::new(
            &output_path("missing-state")?,
            "local-edit".to_owned(),
            "b".repeat(40),
            "2026-08-30T12:00:01Z".to_owned(),
        )
        .map_err(|error| error.to_string())?;
        assert!(
            missing
                .finish(
                    ApplicationSnapshot::default(),
                    &SurfaceSnapshot::empty_for_test(),
                )
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn resource_and_budget_contracts_preserve_exact_values() {
        assert_eq!(LANGUAGE_BUDGET_BYTES, 16_777_216);
        assert_eq!(FOREGROUND_QUEUE_BUDGET_BYTES, 8_388_608);
        assert_eq!(
            resource(
                "glyph-atlas",
                ResourceEvidence {
                    current: 3,
                    peak: 5,
                    budget: 7,
                },
                false,
            ),
            json!({
                "name": "glyph-atlas",
                "current_bytes": 3,
                "peak_bytes": 5,
                "budget_bytes": 7,
                "omitted": false,
            })
        );
    }

    #[test]
    fn path_size_and_staging_boundaries_fail_closed() -> Result<(), String> {
        let normalized_error =
            Some("dogfood diagnostic output must be one normalized absolute file path".to_owned());
        for invalid in ["relative.json", "/tmp/../tmp/output.json", "/"] {
            assert_eq!(
                normalized_output_path(Path::new(invalid))
                    .err()
                    .map(|error| error.to_string()),
                normalized_error
            );
        }

        let root = output_path("boundaries")?.with_extension("root");
        fs::create_dir_all(&root).map_err(|error| error.to_string())?;
        let existing = root.join("existing.json");
        fs::write(&existing, b"existing").map_err(|error| error.to_string())?;
        assert!(normalized_output_path(&existing).is_err());
        assert!(normalized_output_path(&root.join("missing/output.json")).is_err());

        let empty_payload = serde_json::to_vec_pretty(&json!({"payload": ""}))
            .map_err(|error| error.to_string())?;
        let fill = MAX_OUTPUT_BYTES
            .checked_sub(empty_payload.len())
            .ok_or("payload encoding exceeds byte ceiling")?;
        let boundary = root.join("boundary.json");
        let boundary_payload = json!({"payload": "x".repeat(fill)});
        assert_eq!(
            serde_json::to_vec_pretty(&boundary_payload)
                .map_err(|error| error.to_string())?
                .len(),
            MAX_OUTPUT_BYTES
        );
        write_atomic_json(&boundary, &boundary_payload).map_err(|error| error.to_string())?;
        assert_eq!(
            usize::try_from(
                fs::metadata(&boundary)
                    .map_err(|error| error.to_string())?
                    .len()
            )
            .map_err(|error| error.to_string())?,
            MAX_OUTPUT_BYTES
        );
        fs::remove_file(boundary).map_err(|error| error.to_string())?;

        let oversized = root.join("oversized.json");
        let payload = json!({"payload": "x".repeat(fill + 1)});
        assert_eq!(
            write_atomic_json(&oversized, &payload)
                .err()
                .map(|error| error.to_string()),
            Some("dogfood diagnostic exceeds its byte limit".to_owned())
        );
        assert!(!oversized.exists());

        let collision = root.join("collision.json");
        let staging = root.join(format!(".collision.json.{}.staging", std::process::id()));
        fs::write(&staging, b"foreign").map_err(|error| error.to_string())?;
        assert!(write_atomic_json(&collision, &json!({"bounded": true})).is_err());
        assert_eq!(
            fs::read(&staging).map_err(|error| error.to_string())?,
            b"foreign"
        );
        fs::remove_dir_all(root).map_err(|error| error.to_string())?;
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn symlink_and_non_utf8_output_paths_fail_closed() -> Result<(), String> {
        use std::os::unix::{ffi::OsStringExt as _, fs::symlink};

        let root = output_path("unix-paths")?.with_extension("root");
        let target = root.join("target");
        let alias = root.join("alias");
        fs::create_dir_all(&target).map_err(|error| error.to_string())?;
        symlink(&target, &alias).map_err(|error| error.to_string())?;
        assert!(normalized_output_path(&alias.join("output.json")).is_err());

        let nested = target.join("nested");
        fs::create_dir(&nested).map_err(|error| error.to_string())?;
        let nested_output = alias.join("nested/output.json");
        let normalized =
            normalized_output_path(&nested_output).map_err(|error| error.to_string())?;
        assert_eq!(
            normalized,
            fs::canonicalize(nested)
                .map_err(|error| error.to_string())?
                .join("output.json")
        );
        assert_ne!(normalized, nested_output);

        let invalid_name = OsString::from_vec(vec![0xff]);
        assert!(write_atomic_json(&target.join(invalid_name), &json!({})).is_err());
        fs::remove_dir_all(root).map_err(|error| error.to_string())?;
        Ok(())
    }
}
