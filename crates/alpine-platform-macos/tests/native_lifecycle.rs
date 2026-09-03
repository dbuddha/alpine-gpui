//! Native cancellation, callback admission, idle, and teardown qualification.

#[cfg(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    validation::run()
}

#[cfg(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64"))]
mod validation {
    use std::{
        error::Error,
        ffi::OsStr,
        fs::File,
        io::Write,
        path::Path,
        process::{Command, ExitStatus},
        thread,
        time::{Duration, Instant},
    };

    use alpine_core::{LinearRgba, Point, Rect, Size};
    use alpine_platform::PresentationOutcome;
    use alpine_platform_macos::{
        SurfaceDescriptor, SurfaceLifecycle, SurfaceResponse, SurfaceStage, native_validation,
    };
    use alpine_scene::{Primitive, Scene, SceneBuilder, SceneRevision};
    use objc2::rc::autoreleasepool;
    use objc2_foundation::{NSDate, NSRunLoop};

    type TestResult<T = ()> = Result<T, Box<dyn Error>>;

    const OWNER_KINDS: usize = 10;
    const LIFECYCLE_OWNER_COUNTS: [u64; OWNER_KINDS] = [1, 1, 1, 1, 1, 1, 1, 1, 1, 0];
    const QUALIFICATION_WARMUP_ITERATIONS: usize = 512;
    const DIAGNOSTIC_WARMUP_ITERATIONS: usize = 8;
    const SOAK_MIN_SAMPLE_COUNT: usize = 65;
    const SOAK_MAX_SAMPLE_COUNT: usize = 129;
    const SOAK_TAIL_SAMPLE_COUNT: usize = 9;
    const SOAK_MAX_GROWTH_PAGES: u64 = 16;
    const MAX_PRESENTATION_UPLOAD_BYTES: usize = 3 * 8 * 1024 * 1024;
    const LIFECYCLE_ARTIFACT_ENV: &str = "ALPINE_NATIVE_LIFECYCLE_ARTIFACT";
    const LIFECYCLE_RSS_ENV: &str = "ALPINE_NATIVE_LIFECYCLE_CAPTURE_RSS";
    const LIFECYCLE_STAGE_RSS_ENV: &str = "ALPINE_NATIVE_LIFECYCLE_STAGE_RSS";
    const LIFECYCLE_STAGE_SAMPLE_COUNT_ENV: &str = "ALPINE_NATIVE_LIFECYCLE_STAGE_SAMPLE_COUNT";
    const MAX_DIAGNOSTIC_SAMPLE_COUNT: usize = 4_097;
    const REVISION_ENV: &str = "ALPINE_REVISION";
    const METAL_DIAGNOSTIC_ENVS: [&str; 6] = [
        "MTL_DEBUG_LAYER",
        "MTL_DEBUG_LAYER_ERROR_MODE",
        "MTL_SHADER_VALIDATION",
        "MTL_SHADER_VALIDATION_ENABLE_ERROR_REPORTING",
        "MTL_SHADER_VALIDATION_REPORT_TO_STDERR",
        "MTL_SHADER_VALIDATION_ABORT_ON_FAULT",
    ];

    struct OwnerSoakEvidence {
        page_bytes: u64,
        samples: Vec<u64>,
        maximum_bytes: u64,
        tail_minimum_bytes: u64,
        tail_maximum_bytes: u64,
    }

    fn validate_bounded_accounting(snapshot: alpine_platform_macos::SurfaceSnapshot) {
        assert!(snapshot.current_retained_bytes() <= snapshot.peak_retained_bytes());
        assert!(snapshot.current_upload_bytes() <= snapshot.peak_upload_bytes());
        assert!(snapshot.peak_upload_bytes() <= MAX_PRESENTATION_UPLOAD_BYTES);
        assert_eq!(snapshot.frame_slot_capacity(), 3);
        assert!(snapshot.occupied_frame_slots() <= snapshot.frame_slot_capacity());
        assert!(snapshot.submitted_frame_slots() <= snapshot.occupied_frame_slots());
        assert!(snapshot.peak_occupied_frame_slots() <= snapshot.frame_slot_capacity());
    }
    const CHILD_SCENARIO_ENV: &str = "ALPINE_NATIVE_LIFECYCLE_SCENARIO";
    const MISSING_CLOSE_SCENARIO: &str = "missing-close-control";
    const POST_COMMIT_CLOSE_SCENARIO: &str = "post-commit-close";
    const RESIDENT_POLICY_SCENARIO: &str = "resident-policy-controls";

    pub(super) fn run() -> TestResult {
        if let Some(scenario) = std::env::var_os(CHILD_SCENARIO_ENV) {
            return run_child_scenario(&scenario);
        }
        if let Some(stage) = std::env::var_os(LIFECYCLE_STAGE_RSS_ENV) {
            return run_stage_soak(&stage);
        }

        validate_resident_policy_controls()?;
        validate_bounded_child(MISSING_CLOSE_SCENARIO, Duration::from_secs(2))?;
        validate_bounded_child(POST_COMMIT_CLOSE_SCENARIO, Duration::from_secs(8))?;

        let hosted_direct = hosted_direct()?;
        let (scene, clear) = validation_scene()?;
        validate_visible_clean_idle(hosted_direct)?;
        validate_pending_close(scene, clear)?;
        if !residency_capture_enabled()? {
            return Ok(());
        }
        let soak = collect_owner_soak()?;
        let plateau = validate_resident_plateau(&soak);
        write_lifecycle_artifact(&soak, plateau.is_ok())?;
        plateau
    }

    fn residency_capture_enabled() -> TestResult<bool> {
        match std::env::var_os(LIFECYCLE_RSS_ENV) {
            None => Ok(false),
            Some(value) if value == OsStr::new("1") => {
                if std::env::var_os(LIFECYCLE_ARTIFACT_ENV).is_none() {
                    return Err("native lifecycle RSS capture requires an artifact path".into());
                }
                for name in METAL_DIAGNOSTIC_ENVS {
                    if std::env::var_os(name).is_some() {
                        return Err(format!(
                            "native lifecycle RSS capture forbids diagnostic environment {name}"
                        )
                        .into());
                    }
                }
                Ok(true)
            }
            Some(_) => Err("native lifecycle RSS capture must be exactly 1".into()),
        }
    }

    fn hosted_direct() -> TestResult<bool> {
        Ok(
            match std::env::var_os("ALPINE_PRESENTATION_EVIDENCE_MODE") {
                None => false,
                Some(mode) if mode == OsStr::new("hosted-direct") => true,
                Some(_) => return Err("unsupported presentation evidence mode".into()),
            },
        )
    }

    fn run_child_scenario(scenario: &OsStr) -> TestResult {
        if scenario == OsStr::new(MISSING_CLOSE_SCENARIO) {
            return validate_missing_close_control();
        }
        if scenario == OsStr::new(POST_COMMIT_CLOSE_SCENARIO) {
            let (scene, clear) = validation_scene()?;
            return validate_post_commit_close(scene, clear, hosted_direct()?);
        }
        if scenario == OsStr::new(RESIDENT_POLICY_SCENARIO) {
            return validate_resident_policy_controls();
        }
        Err(format!("unsupported native lifecycle child scenario: {scenario:?}").into())
    }

    fn validate_bounded_child(scenario: &str, timeout: Duration) -> TestResult {
        let mut child = Command::new(std::env::current_exe()?)
            .env(CHILD_SCENARIO_ENV, scenario)
            .spawn()?;
        let deadline = Instant::now() + timeout;

        loop {
            if let Some(status) = child.try_wait()? {
                return require_child_success(scenario, status);
            }
            if Instant::now() >= deadline {
                if let Some(status) = child.try_wait()? {
                    return require_child_success(scenario, status);
                }
                child.kill()?;
                let status = child.wait()?;
                return Err(format!(
                    "native lifecycle child {scenario:?} exceeded {timeout:?} and was terminated with {status}"
                )
                .into());
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn require_child_success(scenario: &str, status: ExitStatus) -> TestResult {
        if status.success() {
            Ok(())
        } else {
            Err(format!("native lifecycle child {scenario:?} failed with {status}").into())
        }
    }

    fn validate_missing_close_control() -> TestResult {
        let descriptor = SurfaceDescriptor::new("Alpine missing close control", 32.0, 24.0, 1.0)?;
        let surface = native_validation::new_surface(&descriptor)?;
        surface.show()?;
        let timeout = native_validation::arm_run_timeout(&surface, Duration::from_millis(25));
        assert_eq!(
            surface.run_with_event_handler(|_| SurfaceResponse::default()),
            Err(alpine_platform_macos::SurfaceError::UnexpectedRunLoopExit {
                lifecycle: SurfaceLifecycle::Live,
            })
        );
        assert!(timeout.expired());
        native_validation::close_window(&surface);
        let drain = native_validation::arm_run_loop_drain_marker(&surface);
        assert!(!drain.executed());
        drain_framework_work_until(&drain);
        assert!(drain.executed());
        assert_exact_teardown(native_validation::close_with_owner_evidence(surface)?);
        Ok(())
    }

    fn validate_visible_clean_idle(hosted_direct: bool) -> TestResult {
        let descriptor = SurfaceDescriptor::new("Alpine clean idle", 96.0, 64.0, 1.0)?;
        let surface = native_validation::new_surface(&descriptor)?;
        surface.show()?;
        if hosted_direct || !surface.snapshot().is_presentation_visible() {
            native_validation::inject_surface_configuration(&surface, 96.0, 64.0, 1.0, 0, true)?;
        }
        assert!(native_validation::inject_configuration_callback(&surface));
        let before = surface.snapshot();
        assert!(before.display_link_paused());
        assert_eq!(before.submission_count(), 0);
        assert_eq!(before.allocated_bytes(), 0);
        assert_eq!(before.current_retained_bytes(), 0);
        validate_bounded_accounting(before);

        native_validation::run_until_frame_terminal(&surface, Duration::from_millis(100));
        let after = surface.snapshot();
        assert_eq!(after.callback_count(), before.callback_count());
        assert_eq!(after.submission_count(), before.submission_count());
        assert_eq!(after.direct_present_count(), before.direct_present_count());
        assert_eq!(after.allocated_bytes(), before.allocated_bytes());
        assert_eq!(after.current_retained_bytes(), 0);
        validate_bounded_accounting(after);
        assert!(after.display_link_paused());

        assert_exact_teardown(native_validation::close_with_owner_evidence(surface)?);
        Ok(())
    }

    fn validate_pending_close(scene: Scene, clear: LinearRgba) -> TestResult {
        let descriptor = SurfaceDescriptor::new("Alpine pending close", 96.0, 64.0, 1.0)?;
        let surface = native_validation::new_surface(&descriptor)?;
        let observer = surface.observer();
        assert_eq!(surface.request_frame(scene, clear)?.get(), 1);
        let pending = surface.snapshot();
        assert!(pending.display_link_paused());
        assert_eq!(pending.submission_count(), 0);
        assert_eq!(pending.current_retained_bytes(), 0);
        validate_bounded_accounting(pending);

        native_validation::close_window(&surface);
        assert_eq!(observer.lifecycle(), SurfaceLifecycle::Closing);
        assert_eq!(
            surface.run(),
            Err(alpine_platform_macos::SurfaceError::RunLoopNotRunnable {
                lifecycle: SurfaceLifecycle::Closing,
            })
        );
        let admitted = observer.callback_count();
        let rejected = observer.rejected_callback_count();
        native_validation::inject_late_callback(&surface);
        assert_eq!(observer.callback_count(), admitted);
        assert_eq!(observer.rejected_callback_count(), rejected + 1);
        let closed = surface.snapshot();
        assert_eq!(closed.submission_count(), 0);
        assert_eq!(closed.direct_present_count(), 0);
        assert_eq!(closed.allocated_bytes(), 0);
        assert_eq!(closed.current_retained_bytes(), 0);
        validate_bounded_accounting(closed);
        assert_eq!(closed.pending_cancellation_count(), 1);
        let cancellation = closed
            .last_pending_cancellation()
            .ok_or("pending cancellation evidence")?;
        assert_eq!(cancellation.requested_revision().get(), 1);
        assert_eq!(cancellation.surface_epoch().get(), pending.surface_epoch());
        assert_eq!(cancellation.outcome(), PresentationOutcome::Cancelled);
        assert_exact_teardown(native_validation::close_with_owner_evidence(surface)?);
        assert_eq!(observer.lifecycle(), SurfaceLifecycle::Closed);
        Ok(())
    }

    fn validate_post_commit_close(
        scene: Scene,
        clear: LinearRgba,
        hosted_direct: bool,
    ) -> TestResult {
        let descriptor = SurfaceDescriptor::new("Alpine committed close", 96.0, 64.0, 1.0)?;
        let surface = native_validation::new_surface(&descriptor)?;
        let observer = surface.observer();
        surface.show()?;
        if hosted_direct || !surface.snapshot().is_presentation_visible() {
            native_validation::inject_surface_configuration(&surface, 96.0, 64.0, 1.0, 0, true)?;
        }
        assert_eq!(surface.request_frame(scene, clear)?.get(), 1);
        native_validation::inject_post_commit_close(&surface);
        let timeout = native_validation::arm_run_timeout(&surface, Duration::from_secs(5));
        surface.run()?;
        assert!(!timeout.cancelled());
        timeout.cancel();
        assert!(timeout.cancelled());
        assert!(!timeout.expired());

        assert!(!native_validation::inject_configuration_callback(&surface));
        assert_eq!(surface.take_error()?, None);
        assert_eq!(observer.lifecycle(), SurfaceLifecycle::Closing);
        let snapshot = surface.snapshot();
        let terminal = snapshot
            .last_cancelled()
            .ok_or("cancelled terminal evidence")?;
        assert_eq!(terminal.attempt(), 1);
        assert_eq!(terminal.requested_revision().get(), 1);
        assert_eq!(terminal.frame_revision().get(), 1);
        assert_eq!(terminal.outcome(), PresentationOutcome::Cancelled);
        assert_eq!(terminal.submission_count(), 1);
        assert_eq!(terminal.present_call_count(), 1);
        assert!(terminal.eligible_at_commit());
        assert_ne!(terminal.target_timestamp_bits(), 0);
        assert_ne!(terminal.target_presentation_timestamp_bits(), 0);
        assert_eq!(terminal.observed_presentation_time_bits(), 0);
        assert_eq!(terminal.retained_bytes(), 0);
        assert_eq!(terminal.recovery(), None);
        assert_eq!(snapshot.submission_count(), 1);
        assert_eq!(snapshot.direct_present_count(), 1);
        assert_eq!(snapshot.qualified_presented_count(), 0);
        assert_eq!(snapshot.cancelled_count(), 1);
        assert!(snapshot.peak_upload_bytes() > 0);
        assert_eq!(snapshot.failed_count(), 0);
        assert_eq!(snapshot.current_retained_bytes(), 0);
        validate_bounded_accounting(snapshot);
        assert_eq!(snapshot.occupied_frame_slots(), 0);
        assert_eq!(snapshot.submitted_frame_slots(), 0);
        assert!(snapshot.display_link_paused());

        let admitted = observer.callback_count();
        let rejected = observer.rejected_callback_count();
        native_validation::inject_late_callback(&surface);
        assert_eq!(observer.callback_count(), admitted);
        assert_eq!(observer.rejected_callback_count(), rejected + 1);
        assert_exact_teardown(native_validation::close_with_owner_evidence(surface)?);
        assert_eq!(observer.lifecycle(), SurfaceLifecycle::Closed);
        assert_eq!(observer.callback_count(), admitted);
        Ok(())
    }

    fn collect_owner_soak() -> TestResult<OwnerSoakEvidence> {
        let descriptor = SurfaceDescriptor::new("Alpine owner soak", 32.0, 24.0, 1.0)?;
        for _ in 0..QUALIFICATION_WARMUP_ITERATIONS {
            validate_owner_iteration(&descriptor)?;
        }
        let page_bytes = host_page_bytes()?;
        let mut samples = Vec::with_capacity(SOAK_MAX_SAMPLE_COUNT);
        let _ = resident_bytes()?;
        while samples.len() < SOAK_MAX_SAMPLE_COUNT {
            validate_owner_iteration(&descriptor)?;
            samples.push(resident_bytes()?);
            if resident_sampling_complete(&samples, page_bytes)? {
                break;
            }
        }
        summarize_samples_with_page(samples, page_bytes)
    }

    fn summarize_samples(samples: Vec<u64>) -> TestResult<OwnerSoakEvidence> {
        summarize_samples_with_page(samples, host_page_bytes()?)
    }

    fn summarize_samples_with_page(
        samples: Vec<u64>,
        page_bytes: u64,
    ) -> TestResult<OwnerSoakEvidence> {
        if samples.len() < SOAK_TAIL_SAMPLE_COUNT {
            return Err("RSS summary requires the complete tail window".into());
        }
        let maximum_bytes = samples.iter().copied().max().ok_or("maximum RSS sample")?;
        let tail = &samples[samples.len() - SOAK_TAIL_SAMPLE_COUNT..];
        let tail_minimum_bytes = tail
            .iter()
            .copied()
            .min()
            .ok_or("tail minimum RSS sample")?;
        let tail_maximum_bytes = tail
            .iter()
            .copied()
            .max()
            .ok_or("tail maximum RSS sample")?;
        Ok(OwnerSoakEvidence {
            page_bytes,
            samples,
            maximum_bytes,
            tail_minimum_bytes,
            tail_maximum_bytes,
        })
    }

    fn resident_sampling_complete(samples: &[u64], page_bytes: u64) -> TestResult<bool> {
        if samples.len() < SOAK_MIN_SAMPLE_COUNT {
            return Ok(false);
        }
        if page_bytes == 0 {
            return Err("resident sampling page size must be positive".into());
        }
        let tail = &samples[samples.len() - SOAK_TAIL_SAMPLE_COUNT..];
        let minimum = tail
            .iter()
            .copied()
            .min()
            .ok_or("tail minimum RSS sample")?;
        let maximum = tail
            .iter()
            .copied()
            .max()
            .ok_or("tail maximum RSS sample")?;
        Ok(maximum - minimum <= page_bytes)
    }

    fn validate_resident_policy_controls() -> TestResult {
        const CONTROL_PAGE_BYTES: u64 = 16;
        const CONTROL_BASE_BYTES: u64 = 1_024;

        let immediate = vec![CONTROL_BASE_BYTES; SOAK_MIN_SAMPLE_COUNT];
        assert!(resident_sampling_complete(&immediate, CONTROL_PAGE_BYTES)?);
        assert!(resident_sampling_complete(&immediate, 0).is_err());
        assert!(
            validate_resident_plateau(
                &summarize_samples_with_page(immediate, CONTROL_PAGE_BYTES,)?
            )
            .is_ok()
        );

        let mut delayed = vec![CONTROL_BASE_BYTES; SOAK_MIN_SAMPLE_COUNT - 4];
        delayed.extend([CONTROL_BASE_BYTES + 3 * CONTROL_PAGE_BYTES; 4]);
        assert!(!resident_sampling_complete(&delayed, CONTROL_PAGE_BYTES)?);
        delayed.extend([CONTROL_BASE_BYTES + 3 * CONTROL_PAGE_BYTES; 5]);
        assert!(resident_sampling_complete(&delayed, CONTROL_PAGE_BYTES)?);
        assert!(
            validate_resident_plateau(&summarize_samples_with_page(delayed, CONTROL_PAGE_BYTES,)?)
                .is_ok()
        );

        let mut exact_tail_span = vec![CONTROL_BASE_BYTES; SOAK_MIN_SAMPLE_COUNT];
        for (index, sample) in exact_tail_span[SOAK_MIN_SAMPLE_COUNT - SOAK_TAIL_SAMPLE_COUNT..]
            .iter_mut()
            .enumerate()
        {
            *sample += u64::from(index % 2 == 0) * CONTROL_PAGE_BYTES;
        }
        assert!(resident_sampling_complete(
            &exact_tail_span,
            CONTROL_PAGE_BYTES
        )?);

        let continuing_growth = (0..SOAK_MAX_SAMPLE_COUNT)
            .map(|index| CONTROL_BASE_BYTES + index as u64 * CONTROL_PAGE_BYTES)
            .collect::<Vec<_>>();
        assert!(!resident_sampling_complete(
            &continuing_growth,
            CONTROL_PAGE_BYTES
        )?);
        assert!(
            validate_resident_plateau(&summarize_samples_with_page(
                continuing_growth,
                CONTROL_PAGE_BYTES,
            )?)
            .is_err()
        );

        let oscillating = (0..SOAK_MAX_SAMPLE_COUNT)
            .map(|index| CONTROL_BASE_BYTES + u64::from(index % 2 == 0) * 2 * CONTROL_PAGE_BYTES)
            .collect::<Vec<_>>();
        assert!(!resident_sampling_complete(
            &oscillating,
            CONTROL_PAGE_BYTES
        )?);
        assert!(
            validate_resident_plateau(&summarize_samples_with_page(
                oscillating,
                CONTROL_PAGE_BYTES,
            )?)
            .is_err()
        );

        let mut exact_growth_bound = vec![CONTROL_BASE_BYTES; SOAK_MIN_SAMPLE_COUNT];
        exact_growth_bound[SOAK_MIN_SAMPLE_COUNT - SOAK_TAIL_SAMPLE_COUNT..]
            .fill(CONTROL_BASE_BYTES + SOAK_MAX_GROWTH_PAGES * CONTROL_PAGE_BYTES);
        assert!(
            validate_resident_plateau(&summarize_samples_with_page(
                exact_growth_bound,
                CONTROL_PAGE_BYTES,
            )?)
            .is_ok()
        );

        let mut excessive_growth = vec![CONTROL_BASE_BYTES; SOAK_MIN_SAMPLE_COUNT];
        excessive_growth[SOAK_MIN_SAMPLE_COUNT - SOAK_TAIL_SAMPLE_COUNT..]
            .fill(CONTROL_BASE_BYTES + (SOAK_MAX_GROWTH_PAGES + 1) * CONTROL_PAGE_BYTES);
        assert!(resident_sampling_complete(
            &excessive_growth,
            CONTROL_PAGE_BYTES
        )?);
        assert!(
            validate_resident_plateau(&summarize_samples_with_page(
                excessive_growth,
                CONTROL_PAGE_BYTES,
            )?)
            .is_err()
        );

        assert!(!resident_sampling_complete(
            &vec![CONTROL_BASE_BYTES; SOAK_MIN_SAMPLE_COUNT - 1],
            CONTROL_PAGE_BYTES
        )?);
        let overlong = vec![CONTROL_BASE_BYTES; SOAK_MAX_SAMPLE_COUNT + 1];
        assert!(
            validate_resident_plateau(&summarize_samples_with_page(overlong, CONTROL_PAGE_BYTES,)?)
                .is_err()
        );
        Ok(())
    }

    fn run_stage_soak(value: &OsStr) -> TestResult {
        if !residency_capture_enabled()? {
            return Err("initialization-stage RSS capture requires residency capture".into());
        }
        let stage = parse_stage(value)?;
        let sample_count = diagnostic_sample_count()?;
        let (soak, acquired_owner_kinds) = collect_stage_soak(stage, sample_count)?;
        write_stage_artifact(stage, &soak, acquired_owner_kinds)
    }

    fn diagnostic_sample_count() -> TestResult<usize> {
        let Some(value) = std::env::var_os(LIFECYCLE_STAGE_SAMPLE_COUNT_ENV) else {
            return Ok(SOAK_MIN_SAMPLE_COUNT);
        };
        let value = value
            .to_str()
            .ok_or("initialization-stage sample count must be UTF-8")?
            .parse::<usize>()?;
        if !(SOAK_TAIL_SAMPLE_COUNT..=MAX_DIAGNOSTIC_SAMPLE_COUNT).contains(&value) {
            return Err(format!(
                "initialization-stage sample count must be between {SOAK_TAIL_SAMPLE_COUNT} and {MAX_DIAGNOSTIC_SAMPLE_COUNT}"
            )
            .into());
        }
        Ok(value)
    }

    fn parse_stage(value: &OsStr) -> TestResult<SurfaceStage> {
        for (name, stage) in [
            ("main-thread", SurfaceStage::MainThread),
            ("device", SurfaceStage::Device),
            ("renderer", SurfaceStage::Renderer),
            ("window", SurfaceStage::Window),
            ("view", SurfaceStage::View),
            ("color-space", SurfaceStage::ColorSpace),
            ("layer", SurfaceStage::Layer),
            ("display-link", SurfaceStage::DisplayLink),
            ("run-loop", SurfaceStage::RunLoop),
        ] {
            if value == OsStr::new(name) {
                return Ok(stage);
            }
        }
        Err(format!("unsupported initialization-stage RSS value: {value:?}").into())
    }

    fn stage_name(stage: SurfaceStage) -> &'static str {
        match stage {
            SurfaceStage::MainThread => "main-thread",
            SurfaceStage::Device => "device",
            SurfaceStage::Renderer => "renderer",
            SurfaceStage::Window => "window",
            SurfaceStage::View => "view",
            SurfaceStage::ColorSpace => "color-space",
            SurfaceStage::Layer => "layer",
            SurfaceStage::DisplayLink => "display-link",
            SurfaceStage::RunLoop => "run-loop",
        }
    }

    fn collect_stage_soak(
        stage: SurfaceStage,
        sample_count: usize,
    ) -> TestResult<(OwnerSoakEvidence, usize)> {
        let mut acquired_owner_kinds = None;
        for _ in 0..DIAGNOSTIC_WARMUP_ITERATIONS {
            admit_stage_owner_count(&mut acquired_owner_kinds, validate_stage_iteration(stage)?)?;
        }
        let mut samples = Vec::with_capacity(sample_count);
        let _ = resident_bytes()?;
        for _ in 0..sample_count {
            admit_stage_owner_count(&mut acquired_owner_kinds, validate_stage_iteration(stage)?)?;
            samples.push(resident_bytes()?);
        }
        let acquired_owner_kinds =
            acquired_owner_kinds.ok_or("stage soak requires ownership evidence")?;
        Ok((summarize_samples(samples)?, acquired_owner_kinds))
    }

    fn admit_stage_owner_count(current: &mut Option<usize>, observed: usize) -> TestResult {
        match *current {
            None => *current = Some(observed),
            Some(expected) if expected == observed => {}
            Some(expected) => {
                return Err(format!(
                    "initialization-stage owner count changed from {expected} to {observed}"
                )
                .into());
            }
        }
        Ok(())
    }

    fn validate_stage_iteration(stage: SurfaceStage) -> TestResult<usize> {
        let acquired_owner_kinds = autoreleasepool(|_| -> TestResult<usize> {
            let evidence = native_validation::exercise_initialization_fault(stage)?;
            assert_eq!(evidence.acquired(), evidence.released());
            assert_eq!(evidence.active(), [0; OWNER_KINDS]);
            assert_eq!(evidence.release_order_violations(), 0);
            assert!(
                evidence
                    .acquired()
                    .iter()
                    .all(|count| *count == 0 || *count == 1)
            );
            Ok(evidence
                .acquired()
                .iter()
                .filter(|count| **count == 1)
                .count())
        })?;
        drain_framework_work();
        Ok(acquired_owner_kinds)
    }

    fn validate_owner_iteration(descriptor: &SurfaceDescriptor) -> TestResult {
        autoreleasepool(|_| -> TestResult {
            let surface = native_validation::new_surface(descriptor)?;
            let snapshot = surface.snapshot();
            assert!(snapshot.display_link_paused());
            assert_eq!(snapshot.callback_count(), 0);
            assert_eq!(snapshot.submission_count(), 0);
            assert_eq!(snapshot.allocated_bytes(), 0);
            assert_eq!(snapshot.current_retained_bytes(), 0);
            validate_bounded_accounting(snapshot);
            assert_exact_teardown(native_validation::close_with_owner_evidence(surface)?);
            Ok(())
        })?;
        drain_framework_work();
        Ok(())
    }

    fn drain_framework_work() {
        autoreleasepool(|_| {
            NSRunLoop::mainRunLoop().runUntilDate(&NSDate::dateWithTimeIntervalSinceNow(0.001));
        });
    }

    fn drain_framework_work_until(drain: &native_validation::RunLoopDrainEvidence) {
        let deadline = Instant::now() + Duration::from_millis(250);
        while !drain.executed() && Instant::now() < deadline {
            drain_framework_work();
        }
    }

    fn resident_bytes() -> TestResult<u64> {
        let output = Command::new("/bin/ps")
            .args(["-o", "rss=", "-p"])
            .arg(std::process::id().to_string())
            .output()?;
        if !output.status.success() {
            return Err(format!("resident-byte sampler failed with {}", output.status).into());
        }
        let kibibytes = String::from_utf8(output.stdout)?.trim().parse::<u64>()?;
        kibibytes
            .checked_mul(1024)
            .ok_or_else(|| "resident-byte sample overflow".into())
    }

    fn host_page_bytes() -> TestResult<u64> {
        let output = Command::new("/usr/bin/getconf").arg("PAGESIZE").output()?;
        if !output.status.success() {
            return Err(format!("page-size sampler failed with {}", output.status).into());
        }
        let page_bytes = String::from_utf8(output.stdout)?.trim().parse::<u64>()?;
        if page_bytes == 0 {
            return Err("host page size must be positive".into());
        }
        Ok(page_bytes)
    }

    fn validate_resident_plateau(soak: &OwnerSoakEvidence) -> TestResult {
        if !(SOAK_MIN_SAMPLE_COUNT..=SOAK_MAX_SAMPLE_COUNT).contains(&soak.samples.len()) {
            return Err(format!(
                "native lifecycle soak requires between {SOAK_MIN_SAMPLE_COUNT} and {SOAK_MAX_SAMPLE_COUNT} RSS samples, found {}",
                soak.samples.len()
            )
            .into());
        }
        let initial_bytes = soak.samples[0];
        let allowed_maximum = initial_bytes
            .checked_add(
                soak.page_bytes
                    .checked_mul(SOAK_MAX_GROWTH_PAGES)
                    .ok_or("RSS growth limit overflow")?,
            )
            .ok_or("RSS maximum limit overflow")?;
        if soak.maximum_bytes > allowed_maximum {
            return Err(format!(
                "native lifecycle RSS grew from {initial_bytes} to {} bytes, above {allowed_maximum}",
                soak.maximum_bytes
            )
            .into());
        }
        if soak.tail_maximum_bytes - soak.tail_minimum_bytes > soak.page_bytes {
            return Err(format!(
                "native lifecycle RSS did not reach a one-page terminal span after {} samples: {} to {} bytes with {}-byte pages",
                soak.samples.len(), soak.tail_minimum_bytes, soak.tail_maximum_bytes, soak.page_bytes
            )
            .into());
        }
        Ok(())
    }

    fn write_lifecycle_artifact(
        soak: &OwnerSoakEvidence,
        process_owner_plateau_qualified: bool,
    ) -> TestResult {
        let Some(path) = std::env::var_os(LIFECYCLE_ARTIFACT_ENV) else {
            return Ok(());
        };
        let revision = std::env::var(REVISION_ENV)?;
        if revision.len() != 40 || !revision.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("native lifecycle artifact requires an exact 40-hex revision".into());
        }
        let path = Path::new(&path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut artifact = File::create(path)?;
        writeln!(artifact, "schema_version = 1")?;
        writeln!(artifact, "revision = \"{}\"", revision.to_ascii_lowercase())?;
        writeln!(artifact, "platform = \"macos\"")?;
        writeln!(artifact, "architecture = \"aarch64\"")?;
        writeln!(artifact, "evidence_scope = \"process-owner-soak\"")?;
        writeln!(artifact, "physical_lifecycle_qualified = false")?;
        writeln!(artifact, "metal_api_validation_enabled = false")?;
        writeln!(artifact, "metal_shader_validation_enabled = false")?;
        writeln!(
            artifact,
            "process_owner_plateau_qualified = {process_owner_plateau_qualified}"
        )?;
        writeln!(
            artifact,
            "warmup_iterations = {QUALIFICATION_WARMUP_ITERATIONS}"
        )?;
        writeln!(artifact, "sample_count = {}", soak.samples.len())?;
        writeln!(artifact, "minimum_sample_count = {SOAK_MIN_SAMPLE_COUNT}")?;
        writeln!(artifact, "maximum_sample_count = {SOAK_MAX_SAMPLE_COUNT}")?;
        writeln!(artifact, "terminal_sample_count = {SOAK_TAIL_SAMPLE_COUNT}")?;
        writeln!(artifact, "page_bytes = {}", soak.page_bytes)?;
        writeln!(artifact, "initial_bytes = {}", soak.samples[0])?;
        writeln!(artifact, "maximum_bytes = {}", soak.maximum_bytes)?;
        writeln!(artifact, "tail_minimum_bytes = {}", soak.tail_minimum_bytes)?;
        writeln!(artifact, "tail_maximum_bytes = {}", soak.tail_maximum_bytes)?;
        writeln!(artifact, "maximum_growth_pages = {SOAK_MAX_GROWTH_PAGES}")?;
        writeln!(artifact, "tail_growth_pages = 1")?;
        writeln!(artifact, "owner_kinds = {OWNER_KINDS}")?;
        writeln!(artifact, "acquired_owner_kinds_per_iteration = 9")?;
        writeln!(artifact, "active_owners_after_each_close = 0")?;
        write!(artifact, "rss_samples_bytes = [")?;
        for (index, sample) in soak.samples.iter().enumerate() {
            if index != 0 {
                write!(artifact, ", ")?;
            }
            write!(artifact, "{sample}")?;
        }
        writeln!(artifact, "]")?;
        artifact.flush()?;
        artifact.sync_all()?;
        Ok(())
    }

    fn write_stage_artifact(
        stage: SurfaceStage,
        soak: &OwnerSoakEvidence,
        acquired_owner_kinds: usize,
    ) -> TestResult {
        let path = std::env::var_os(LIFECYCLE_ARTIFACT_ENV)
            .ok_or("initialization-stage RSS capture requires an artifact path")?;
        let revision = std::env::var(REVISION_ENV)?;
        if revision.len() != 40 || !revision.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("native lifecycle artifact requires an exact 40-hex revision".into());
        }
        let path = Path::new(&path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut artifact = File::create(path)?;
        writeln!(artifact, "schema_version = 1")?;
        writeln!(artifact, "revision = \"{}\"", revision.to_ascii_lowercase())?;
        writeln!(artifact, "platform = \"macos\"")?;
        writeln!(artifact, "architecture = \"aarch64\"")?;
        writeln!(artifact, "evidence_scope = \"initialization-stage-soak\"")?;
        writeln!(artifact, "diagnostic_only = true")?;
        writeln!(artifact, "qualification_claim = false")?;
        writeln!(artifact, "stage = \"{}\"", stage_name(stage))?;
        writeln!(
            artifact,
            "warmup_iterations = {DIAGNOSTIC_WARMUP_ITERATIONS}"
        )?;
        writeln!(artifact, "sample_count = {}", soak.samples.len())?;
        writeln!(artifact, "page_bytes = {}", soak.page_bytes)?;
        writeln!(artifact, "initial_bytes = {}", soak.samples[0])?;
        writeln!(artifact, "maximum_bytes = {}", soak.maximum_bytes)?;
        writeln!(artifact, "tail_minimum_bytes = {}", soak.tail_minimum_bytes)?;
        writeln!(artifact, "tail_maximum_bytes = {}", soak.tail_maximum_bytes)?;
        writeln!(artifact, "owner_kinds = {OWNER_KINDS}")?;
        writeln!(
            artifact,
            "acquired_owner_kinds_per_iteration = {acquired_owner_kinds}"
        )?;
        writeln!(artifact, "active_owners_after_each_rollback = 0")?;
        write!(artifact, "rss_samples_bytes = [")?;
        for (index, sample) in soak.samples.iter().enumerate() {
            if index != 0 {
                write!(artifact, ", ")?;
            }
            write!(artifact, "{sample}")?;
        }
        writeln!(artifact, "]")?;
        artifact.flush()?;
        artifact.sync_all()?;
        Ok(())
    }

    fn assert_exact_teardown(evidence: native_validation::NativeOwnerEvidence) {
        assert_eq!(evidence.acquired(), LIFECYCLE_OWNER_COUNTS);
        assert_eq!(evidence.released(), LIFECYCLE_OWNER_COUNTS);
        assert_eq!(evidence.active(), [0; OWNER_KINDS]);
        assert_eq!(evidence.run_loop_registrations(), 1);
        assert_eq!(evidence.link_invalidations(), 1);
        assert_eq!(evidence.delegate_revocations(), 1);
        assert_eq!(evidence.window_closes(), 1);
        assert_eq!(evidence.pasteboard_releases(), 0);
        assert_eq!(evidence.release_order_violations(), 0);
    }

    fn validation_scene() -> TestResult<(Scene, LinearRgba)> {
        let viewport = Size::new(96.0, 64.0).ok_or("valid viewport")?;
        let bounds = Rect::new(
            Point::new(8.0, 8.0).ok_or("valid origin")?,
            Size::new(80.0, 48.0).ok_or("valid quad size")?,
        );
        let color = LinearRgba::new(0.25, 0.5, 0.75, 1.0).ok_or("valid color")?;
        let clear = LinearRgba::new(0.0, 0.0, 0.0, 1.0).ok_or("valid clear")?;
        let mut builder = SceneBuilder::new(SceneRevision::new(1), viewport);
        builder.push(Primitive::Quad { bounds, color });
        Ok((builder.finish(), clear))
    }
}

#[cfg(not(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64")))]
fn main() {}
