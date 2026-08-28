//! Dropped presentation, post-commit supersession, and native device-loss validation.

#[cfg(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    validation::run()
}

#[cfg(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64"))]
mod validation {
    use std::{
        error::Error,
        ffi::OsStr,
        time::{Duration, Instant},
    };

    use alpine_core::{LinearRgba, Point, Rect, Size};
    use alpine_metal::{BackendState, RecoveryClassification, RenderError};
    use alpine_platform::PresentationOutcome;
    use alpine_platform_macos::{
        NativeSurface, SurfaceDescriptor, SurfaceError, SurfaceSnapshot, native_validation,
    };
    use alpine_scene::{Primitive, Scene, SceneBuilder, SceneRevision};
    use objc2_foundation::{NSDate, NSRunLoop};

    type TestResult<T = ()> = Result<T, Box<dyn Error>>;

    const MAX_RETRY_ATTEMPTS: u8 = 4;
    const LOGICAL_WIDTH: f64 = 96.0;
    const LOGICAL_HEIGHT: f64 = 64.0;
    const PAUSE_SETTLEMENT: Duration = Duration::from_millis(150);
    const HOSTED_PAUSE_SETTLEMENT: Duration = Duration::from_secs(5);

    pub(super) fn run() -> TestResult {
        let hosted_direct = match std::env::var_os("ALPINE_PRESENTATION_EVIDENCE_MODE") {
            None => false,
            Some(mode) if mode == OsStr::new("hosted-direct") => true,
            Some(_) => return Err("unsupported presentation evidence mode".into()),
        };
        let (scene, clear) = validation_scene()?;
        validate_dropped_presentation(scene.clone(), clear, hosted_direct)?;
        validate_missing_presentation(scene.clone(), clear, hosted_direct)?;
        validate_supersession(scene.clone(), clear, hosted_direct)?;
        validate_device_loss(scene, clear, hosted_direct)
    }

    fn validate_dropped_presentation(
        scene: Scene,
        clear: LinearRgba,
        hosted_direct: bool,
    ) -> TestResult {
        let descriptor = SurfaceDescriptor::new("Alpine dropped presentation", 96.0, 64.0, 1.0)?;
        let surface = native_validation::new_surface(&descriptor)?;
        let _backing_scale = prepare_visible_surface(&surface, hosted_direct)?;
        let baseline = surface.snapshot();

        assert!(!native_validation::post_commit_control_armed(&surface));
        native_validation::inject_post_commit_observation(&surface, None, 0.0)?;
        assert!(native_validation::post_commit_control_armed(&surface));
        assert_eq!(surface.request_frame(scene.clone(), clear)?.get(), 1);
        native_validation::run_until_frame_terminal(&surface, Duration::from_secs(5));
        assert!(!native_validation::post_commit_control_armed(&surface));

        assert_eq!(surface.take_error()?, None);
        let dropped = surface.snapshot();
        let terminal = dropped.last_terminal().ok_or_else(|| {
            format!(
                "dropped-presentation terminal evidence: callbacks={}, submissions={}, presented={}, skipped={}, failed={}, paused={}",
                dropped.callback_count(),
                dropped.submission_count(),
                dropped.presented_count(),
                dropped.skipped_count(),
                dropped.failed_count(),
                dropped.display_link_paused()
            )
        })?;
        let dropped_attempt = terminal.attempt();
        assert!(dropped_attempt >= 1);
        assert_eq!(terminal.requested_revision().get(), 1);
        assert_eq!(terminal.frame_revision().get(), 1);
        assert_eq!(terminal.outcome(), PresentationOutcome::Failed);
        assert_eq!(terminal.submission_count(), 1);
        assert_eq!(terminal.present_call_count(), 1);
        assert!(terminal.eligible_at_commit());
        assert_eq!(terminal.observed_presentation_time_bits(), 0);
        assert_eq!(terminal.retained_bytes(), 0);
        assert_eq!(terminal.recovery(), None);
        let dropped_submissions = dropped.submission_count();
        assert!(dropped_submissions > baseline.submission_count());
        assert_eq!(dropped.direct_present_count(), dropped_submissions);
        assert!(dropped.presented_count() >= baseline.presented_count());
        assert!(dropped.qualified_presented_count() >= baseline.qualified_presented_count());
        // A superseded post-commit attempt is terminal without being presented or
        // skipped. Only the injected dropped terminal contributes to `skipped`,
        // while unrelated hosted callback failures may also contribute to the
        // cumulative surface failure count.
        assert!(dropped.skipped_count() > baseline.skipped_count());
        assert!(dropped.failed_count() > baseline.failed_count());
        if hosted_direct {
            await_display_link_paused(&surface, HOSTED_PAUSE_SETTLEMENT)?;
        } else {
            await_display_link_paused(&surface, PAUSE_SETTLEMENT)?;
        }
        let settled = surface.snapshot();
        if hosted_direct {
            assert_hosted_slot_bound(&settled);
        } else {
            let turn = NSDate::dateWithTimeIntervalSinceNow(PAUSE_SETTLEMENT.as_secs_f64());
            NSRunLoop::mainRunLoop().runUntilDate(&turn);
            let quiescent = surface.snapshot();
            assert_eq!(quiescent.callback_count(), settled.callback_count());
            assert_eq!(quiescent.submission_count(), settled.submission_count());
            assert_eq!(
                quiescent.direct_present_count(),
                settled.direct_present_count()
            );
        }

        assert!(!native_validation::post_commit_control_armed(&surface));
        native_validation::inject_post_commit_observation(&surface, None, 2.0)?;
        assert!(native_validation::post_commit_control_armed(&surface));
        assert_eq!(surface.request_frame(scene, clear)?.get(), 2);
        native_validation::run_until_frame_terminal(&surface, Duration::from_secs(5));
        assert!(!native_validation::post_commit_control_armed(&surface));

        assert_eq!(surface.take_error()?, None);
        let recovered = surface.snapshot();
        let terminal = recovered
            .last_terminal()
            .ok_or("post-drop recovery terminal evidence")?;
        assert!(terminal.attempt() > dropped_attempt);
        assert_eq!(terminal.requested_revision().get(), 2);
        assert_eq!(terminal.frame_revision().get(), 2);
        assert_eq!(terminal.outcome(), PresentationOutcome::Presented);
        assert_eq!(
            terminal.observed_presentation_time_bits(),
            2.0_f64.to_bits()
        );
        assert_eq!(terminal.recovery(), None);
        assert!(recovered.submission_count() > dropped_submissions);
        assert_eq!(
            recovered.direct_present_count(),
            recovered.submission_count()
        );
        assert!(recovered.presented_count() > dropped.presented_count());
        assert!(recovered.qualified_presented_count() > dropped.qualified_presented_count());
        assert!(recovered.skipped_count() >= dropped.skipped_count());
        // Hosted AppKit may contribute an unrelated failed callback while the
        // explicit recovery request wins. Surface failure accounting is
        // cumulative, while the terminal evidence above identifies recovery.
        assert!(recovered.failed_count() >= dropped.failed_count());
        if hosted_direct {
            assert_hosted_slot_bound(&recovered);
        } else {
            assert_eq!(recovered.occupied_frame_slots(), 0);
            assert_eq!(recovered.submitted_frame_slots(), 0);
            assert!(recovered.display_link_paused());
        }
        surface.close();
        Ok(())
    }

    fn await_display_link_paused(surface: &NativeSurface, timeout: Duration) -> TestResult {
        let deadline = Instant::now() + timeout;
        while !surface.snapshot().display_link_paused() && Instant::now() < deadline {
            let turn = NSDate::dateWithTimeIntervalSinceNow(0.005);
            NSRunLoop::mainRunLoop().runUntilDate(&turn);
        }
        let snapshot = surface.snapshot();
        assert!(
            snapshot.display_link_paused(),
            "missing-presentation display link did not pause: {snapshot:?}"
        );
        assert_eq!(snapshot.occupied_frame_slots(), 0);
        assert_eq!(snapshot.submitted_frame_slots(), 0);
        Ok(())
    }

    fn assert_hosted_slot_bound(snapshot: &SurfaceSnapshot) {
        assert!(snapshot.occupied_frame_slots() <= snapshot.frame_slot_capacity());
        assert!(snapshot.submitted_frame_slots() <= snapshot.occupied_frame_slots());
    }

    fn validate_missing_presentation(
        scene: Scene,
        clear: LinearRgba,
        hosted_direct: bool,
    ) -> TestResult {
        let descriptor = SurfaceDescriptor::new("Alpine missing presentation", 96.0, 64.0, 1.0)?;
        let surface = native_validation::new_surface(&descriptor)?;
        let _backing_scale = prepare_visible_surface(&surface, hosted_direct)?;
        let baseline = surface.snapshot();

        assert!(!native_validation::post_commit_control_armed(&surface));
        assert!(!native_validation::post_commit_omission_armed(&surface));
        native_validation::inject_post_commit_omission(&surface);
        assert!(native_validation::post_commit_control_armed(&surface));
        assert!(native_validation::post_commit_omission_armed(&surface));
        assert_eq!(surface.request_frame(scene.clone(), clear)?.get(), 1);
        native_validation::run_until_frame_terminal(&surface, Duration::from_secs(5));
        assert!(!native_validation::post_commit_control_armed(&surface));
        assert!(!native_validation::post_commit_omission_armed(&surface));

        assert_eq!(surface.take_error()?, None);
        let first = surface.snapshot();
        let terminal = first
            .last_terminal()
            .ok_or("missing-presentation terminal evidence")?;
        assert!(terminal.attempt() >= 1);
        assert_eq!(terminal.frame_revision().get(), 1);
        assert_eq!(terminal.outcome(), PresentationOutcome::Failed);
        assert_eq!(terminal.submission_count(), 1);
        assert_eq!(terminal.present_call_count(), 1);
        assert!(terminal.eligible_at_commit());
        assert_eq!(terminal.observed_presentation_time_bits(), 0);
        assert_eq!(terminal.retained_bytes(), 0);
        assert_eq!(terminal.recovery(), None);
        assert!(first.submission_count() > baseline.submission_count());
        assert_eq!(first.direct_present_count(), first.submission_count());
        assert!(first.presented_count() >= baseline.presented_count());
        assert!(first.qualified_presented_count() >= baseline.qualified_presented_count());
        assert!(first.skipped_count() >= baseline.skipped_count());
        assert!(first.failed_count() > baseline.failed_count());
        if hosted_direct {
            await_display_link_paused(&surface, HOSTED_PAUSE_SETTLEMENT)?;
        } else {
            await_display_link_paused(&surface, PAUSE_SETTLEMENT)?;
        }
        assert_eq!(surface.take_error()?, None);
        let settled = surface.snapshot();
        let settled_submissions = settled
            .submission_count()
            .checked_sub(baseline.submission_count())
            .ok_or("missing-presentation submission counter regressed")?;
        let settled_failures = settled
            .failed_count()
            .checked_sub(baseline.failed_count())
            .ok_or("missing-presentation failure counter regressed")?;
        let settled_skipped = settled
            .skipped_count()
            .checked_sub(baseline.skipped_count())
            .ok_or("missing-presentation skipped counter regressed")?;
        assert!(settled_submissions >= 1);
        assert_eq!(settled.direct_present_count(), settled.submission_count());
        assert!(settled_failures >= 1);
        assert!(settled_skipped <= settled_submissions.saturating_sub(1));
        assert!(settled_skipped <= settled_failures);
        if hosted_direct {
            assert_hosted_slot_bound(&settled);
        } else {
            assert_eq!(settled.occupied_frame_slots(), 0);
            assert_eq!(settled.submitted_frame_slots(), 0);
            assert!(settled.display_link_paused());
            let turn = NSDate::dateWithTimeIntervalSinceNow(PAUSE_SETTLEMENT.as_secs_f64());
            NSRunLoop::mainRunLoop().runUntilDate(&turn);
            let quiescent = surface.snapshot();
            assert_eq!(quiescent.callback_count(), settled.callback_count());
            assert_eq!(quiescent.submission_count(), settled.submission_count());
            assert_eq!(
                quiescent.direct_present_count(),
                settled.direct_present_count()
            );
        }

        assert!(!native_validation::post_commit_control_armed(&surface));
        native_validation::inject_post_commit_observation(&surface, None, 2.5)?;
        assert!(native_validation::post_commit_control_armed(&surface));
        let recovery_revision = surface.request_frame(scene, clear)?;
        native_validation::run_until_frame_terminal(&surface, Duration::from_secs(5));
        assert!(!native_validation::post_commit_control_armed(&surface));

        assert_eq!(surface.take_error()?, None);
        let recovered = surface.snapshot();
        let terminal = recovered
            .last_terminal()
            .ok_or("post-omission recovery terminal evidence")?;
        assert_eq!(terminal.requested_revision(), recovery_revision);
        assert_eq!(terminal.frame_revision(), recovery_revision);
        assert_eq!(terminal.outcome(), PresentationOutcome::Presented);
        assert_eq!(
            terminal.observed_presentation_time_bits(),
            2.5_f64.to_bits()
        );
        assert_eq!(terminal.recovery(), None);
        assert!(recovered.submission_count() > settled.submission_count());
        assert_eq!(
            recovered.direct_present_count(),
            recovered.submission_count()
        );
        assert!(recovered.presented_count() > settled.presented_count());
        assert!(recovered.qualified_presented_count() > settled.qualified_presented_count());
        assert!(recovered.skipped_count() >= settled.skipped_count());
        assert!(recovered.failed_count() >= settled.failed_count());
        if hosted_direct {
            assert_hosted_slot_bound(&recovered);
        } else {
            assert_eq!(recovered.occupied_frame_slots(), 0);
            assert_eq!(recovered.submitted_frame_slots(), 0);
            assert!(recovered.display_link_paused());
        }
        surface.close();
        Ok(())
    }

    fn validate_supersession(scene: Scene, clear: LinearRgba, hosted_direct: bool) -> TestResult {
        let descriptor = SurfaceDescriptor::new("Alpine supersession", 96.0, 64.0, 1.0)?;
        let surface = native_validation::new_surface(&descriptor)?;
        let _backing_scale = prepare_visible_surface(&surface, hosted_direct)?;
        assert!(
            native_validation::inject_post_commit_observation(&surface, None, f64::NAN).is_err()
        );
        assert!(native_validation::inject_post_commit_observation(&surface, None, -1.0).is_err());
        assert!(!native_validation::post_commit_control_armed(&surface));
        let before = surface.snapshot();
        native_validation::inject_post_commit_observation(&surface, Some(usize::MAX), 1.25)?;
        assert!(native_validation::post_commit_control_armed(&surface));
        assert_eq!(surface.request_frame(scene, clear)?.get(), 1);
        native_validation::run_until_frame_terminal(&surface, Duration::from_secs(5));
        assert!(!native_validation::post_commit_control_armed(&surface));

        assert_eq!(surface.take_error()?, None);
        let superseded = surface.snapshot();
        let terminal = superseded
            .last_superseded()
            .ok_or("superseded terminal evidence")?;
        assert!(terminal.attempt() >= 1);
        assert_eq!(terminal.requested_revision().get(), 1);
        assert_eq!(terminal.frame_revision().get(), 1);
        // AppKit may deliver a legitimate geometry or display notification
        // after the pre-request snapshot but before callback encoding. The
        // attempt must never move backward, and the injected replacement must
        // still supersede whichever epoch the callback captured.
        assert!(terminal.frame_epoch().get() >= before.surface_epoch());
        assert_eq!(terminal.outcome(), PresentationOutcome::Superseded);
        assert_eq!(terminal.submission_count(), 1);
        assert_eq!(terminal.present_call_count(), 1);
        assert!(terminal.eligible_at_commit());
        assert_ne!(terminal.target_timestamp_bits(), 0);
        assert_ne!(terminal.target_presentation_timestamp_bits(), 0);
        assert_eq!(
            terminal.observed_presentation_time_bits(),
            1.25_f64.to_bits()
        );
        assert_eq!(terminal.retained_bytes(), 0);
        assert_eq!(terminal.recovery(), None);
        assert!(superseded.surface_epoch() > terminal.frame_epoch().get());
        assert!(superseded.submission_count() > before.submission_count());
        assert_eq!(
            superseded.direct_present_count(),
            superseded.submission_count()
        );
        assert!(superseded.presented_count() > before.presented_count());
        assert!(superseded.qualified_presented_count() >= before.qualified_presented_count());
        assert!(superseded.superseded_count() > before.superseded_count());
        // `last_superseded` remains stable after a newer attempt completes.
        // The later snapshot may therefore observe the display link paused by
        // that newer terminal without changing this attempt's exact evidence.

        validate_retry(
            &surface,
            terminal.attempt(),
            terminal.frame_epoch().get().saturating_add(1),
            superseded.submission_count(),
            superseded.presented_count(),
            superseded.qualified_presented_count(),
            superseded.superseded_count(),
            hosted_direct,
        )?;
        surface.close();
        Ok(())
    }

    fn validate_retry(
        surface: &NativeSurface,
        mut prior_attempt: u64,
        mut minimum_retry_epoch: u64,
        mut prior_submissions: u64,
        mut prior_presented: u64,
        baseline_qualified: u64,
        mut expected_superseded: u64,
        hosted_direct: bool,
    ) -> TestResult {
        // Configuration changes may supersede committed work, but a bounded
        // validation run must still make progress once those changes stop.
        for retry in 0_u8..MAX_RETRY_ATTEMPTS {
            let presented_time = 1.5 + f64::from(retry) / 10.0;
            native_validation::inject_post_commit_observation(surface, None, presented_time)?;
            native_validation::run_until_frame_terminal(surface, Duration::from_secs(5));
            assert_eq!(surface.take_error()?, None);
            let recovered = surface.snapshot();
            let terminal = recovered
                .last_terminal()
                .ok_or("recovered terminal evidence")?;
            assert!(terminal.attempt() > prior_attempt);
            assert_eq!(terminal.requested_revision().get(), 1);
            assert_eq!(terminal.frame_revision().get(), 1);
            assert!(terminal.frame_epoch().get() >= minimum_retry_epoch);
            // The terminal record is an immutable observation at completion.
            // A legitimate AppKit notification can advance the live epoch
            // before this later snapshot, but cannot move it backward.
            assert!(recovered.surface_epoch() >= terminal.frame_epoch().get());
            assert_eq!(terminal.submission_count(), 1);
            assert_eq!(terminal.present_call_count(), 1);
            assert!(terminal.eligible_at_commit());
            assert_eq!(terminal.retained_bytes(), 0);
            assert_eq!(terminal.recovery(), None);
            // A callback can commit after the prior terminal snapshot is read
            // but before this retry helper begins. The immutable attempt record
            // above proves new progress even when its submission was already
            // included in the cumulative surface counter.
            assert!(recovered.submission_count() >= prior_submissions);
            assert_eq!(
                recovered.direct_present_count(),
                recovered.submission_count()
            );
            match terminal.outcome() {
                PresentationOutcome::Presented => {
                    assert_eq!(
                        terminal.observed_presentation_time_bits(),
                        presented_time.to_bits()
                    );
                    assert!(recovered.presented_count() > prior_presented);
                    assert!(recovered.qualified_presented_count() > baseline_qualified);
                    assert!(recovered.superseded_count() >= expected_superseded);
                    assert!(recovered.display_link_paused());
                    return Ok(());
                }
                PresentationOutcome::Superseded => {
                    assert_eq!(
                        terminal.observed_presentation_time_bits(),
                        presented_time.to_bits()
                    );
                    expected_superseded += 1;
                    assert!(recovered.presented_count() >= prior_presented);
                    assert!(recovered.qualified_presented_count() >= baseline_qualified);
                    assert!(recovered.superseded_count() >= expected_superseded);
                    assert!(!recovered.display_link_paused());
                    assert!(recovered.surface_epoch() > terminal.frame_epoch().get());
                    prior_attempt = terminal.attempt();
                    minimum_retry_epoch = terminal.frame_epoch().get().saturating_add(1);
                    prior_submissions = recovered.submission_count();
                    prior_presented = recovered.presented_count();
                }
                PresentationOutcome::Failed if hosted_direct => {
                    assert_eq!(terminal.observed_presentation_time_bits(), 0);
                    assert!(recovered.failed_count() >= 1);
                    assert!(recovered.display_link_paused());
                    return Ok(());
                }
                outcome => return Err(format!("unexpected retry outcome: {outcome:?}").into()),
            }
        }
        Err("retry did not qualify after bounded AppKit configuration churn".into())
    }

    fn validate_device_loss(scene: Scene, clear: LinearRgba, hosted_direct: bool) -> TestResult {
        let descriptor = SurfaceDescriptor::new("Alpine device loss", 96.0, 64.0, 1.0)?;
        let surface = native_validation::new_surface_with_device_loss(&descriptor)?;
        let backing_scale = prepare_visible_surface(&surface, hosted_direct)?;
        let baseline = surface.snapshot();
        assert_eq!(surface.request_frame(scene.clone(), clear)?.get(), 1);
        native_validation::run_until_frame_terminal(&surface, Duration::from_secs(5));
        let first_error = surface.take_error()?.ok_or("device-loss failure")?;
        let SurfaceError::Render(first_render) = first_error else {
            return Err("device loss must retain the renderer failure".into());
        };
        assert!(matches!(first_render, RenderError::CommandFailed { .. }));
        assert_eq!(
            first_render.recovery(),
            RecoveryClassification::RecreateBackend
        );
        let failed = surface.snapshot();
        let terminal = failed
            .last_terminal()
            .ok_or("device-loss terminal evidence")?;
        assert_eq!(terminal.outcome(), PresentationOutcome::Failed);
        assert_eq!(terminal.submission_count(), 1);
        assert_eq!(terminal.present_call_count(), 1);
        assert!(terminal.eligible_at_commit());
        assert_eq!(terminal.observed_presentation_time_bits(), 0);
        assert_eq!(terminal.retained_bytes(), 0);
        assert_eq!(
            terminal.recovery(),
            Some(RecoveryClassification::RecreateBackend)
        );
        native_validation::inject_surface_configuration(
            &surface,
            LOGICAL_WIDTH,
            LOGICAL_HEIGHT,
            backing_scale,
            0,
            true,
        )?;
        assert!(surface.snapshot().display_link_paused());
        assert!(failed.submission_count() > baseline.submission_count());
        assert_eq!(failed.direct_present_count(), failed.submission_count());
        assert!(failed.failed_count() > baseline.failed_count());
        assert_eq!(
            failed.qualified_presented_count(),
            baseline.qualified_presented_count()
        );
        assert_eq!(failed.superseded_count(), baseline.superseded_count());
        assert!(failed.display_link_paused());

        validate_lost_generation(&surface, scene, clear)?;
        surface.close();
        Ok(())
    }

    fn validate_lost_generation(
        surface: &NativeSurface,
        scene: Scene,
        clear: LinearRgba,
    ) -> TestResult {
        let before = surface.snapshot();
        assert_eq!(surface.request_frame(scene, clear)?.get(), 2);
        native_validation::run_until_frame_terminal(surface, Duration::from_secs(5));
        let rejected = surface.take_error()?.ok_or("lost generation rejection")?;
        assert!(matches!(
            rejected,
            SurfaceError::Render(RenderError::BackendUnavailable {
                state: BackendState::DeviceLost,
                ..
            })
        ));
        let guarded = surface.snapshot();
        let terminal = guarded.last_terminal().ok_or("guarded terminal evidence")?;
        assert_eq!(terminal.outcome(), PresentationOutcome::Failed);
        assert_eq!(terminal.submission_count(), 0);
        assert_eq!(terminal.present_call_count(), 0);
        assert!(!terminal.eligible_at_commit());
        assert_eq!(
            terminal.recovery(),
            Some(RecoveryClassification::RecreateBackend)
        );
        assert_eq!(terminal.retained_bytes(), 0);
        assert_eq!(guarded.submission_count(), before.submission_count());
        assert_eq!(
            guarded.direct_present_count(),
            before.direct_present_count()
        );
        assert_eq!(guarded.failed_count(), before.failed_count() + 1);
        assert!(guarded.display_link_paused());
        Ok(())
    }

    fn prepare_visible_surface(surface: &NativeSurface, hosted_direct: bool) -> TestResult<f64> {
        surface.show()?;
        let snapshot = surface.snapshot();
        let width_scale = f64::from(snapshot.physical_width()) / LOGICAL_WIDTH;
        let height_scale = f64::from(snapshot.physical_height()) / LOGICAL_HEIGHT;
        if !width_scale.is_finite()
            || width_scale <= 0.0
            || width_scale.to_bits() != height_scale.to_bits()
        {
            return Err("native backing scale must be finite, positive, and uniform".into());
        }
        if hosted_direct {
            native_validation::inject_surface_configuration(
                surface,
                LOGICAL_WIDTH,
                LOGICAL_HEIGHT,
                width_scale,
                0,
                true,
            )?;
        }
        Ok(width_scale)
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
