//! Native cancellation, callback admission, idle, and teardown qualification.

#[cfg(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    validation::run()
}

#[cfg(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64"))]
mod validation {
    use std::{error::Error, ffi::OsStr, time::Duration};

    use alpine_core::{LinearRgba, Point, Rect, Size};
    use alpine_platform::PresentationOutcome;
    use alpine_platform_macos::{SurfaceDescriptor, SurfaceLifecycle, native_validation};
    use alpine_scene::{Primitive, Scene, SceneBuilder, SceneRevision};

    type TestResult<T = ()> = Result<T, Box<dyn Error>>;

    const OWNER_KINDS: usize = 9;
    const SOAK_ITERATIONS: usize = 32;

    pub(super) fn run() -> TestResult {
        let hosted_direct = match std::env::var_os("ALPINE_PRESENTATION_EVIDENCE_MODE") {
            None => false,
            Some(mode) if mode == OsStr::new("hosted-direct") => true,
            Some(_) => return Err("unsupported presentation evidence mode".into()),
        };
        let (scene, clear) = validation_scene()?;
        validate_visible_clean_idle(hosted_direct)?;
        validate_missing_close_control()?;
        validate_pending_close(scene.clone(), clear)?;
        validate_post_commit_close(scene, clear, hosted_direct)?;
        validate_owner_soak()
    }

    fn validate_missing_close_control() -> TestResult {
        let descriptor = SurfaceDescriptor::new("Alpine missing close control", 32.0, 24.0, 1.0)?;
        let surface = native_validation::new_surface(&descriptor)?;
        surface.show()?;
        let timeout = native_validation::arm_run_timeout(&surface, Duration::from_millis(25));
        assert_eq!(
            surface.run(),
            Err(alpine_platform_macos::SurfaceError::UnexpectedRunLoopExit {
                lifecycle: SurfaceLifecycle::Live,
            })
        );
        assert!(timeout.expired());
        native_validation::close_window(&surface);
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

        native_validation::run_until_frame_terminal(&surface, Duration::from_millis(100));
        let after = surface.snapshot();
        assert_eq!(after.callback_count(), before.callback_count());
        assert_eq!(after.submission_count(), before.submission_count());
        assert_eq!(after.direct_present_count(), before.direct_present_count());
        assert_eq!(after.allocated_bytes(), before.allocated_bytes());
        assert_eq!(after.current_retained_bytes(), 0);
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

        native_validation::close_window(&surface);
        assert_eq!(observer.lifecycle(), SurfaceLifecycle::Closing);
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
        timeout.cancel();
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
        assert_eq!(snapshot.failed_count(), 0);
        assert_eq!(snapshot.current_retained_bytes(), 0);
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

    fn validate_owner_soak() -> TestResult {
        let descriptor = SurfaceDescriptor::new("Alpine owner soak", 32.0, 24.0, 1.0)?;
        for _ in 0..SOAK_ITERATIONS {
            let surface = native_validation::new_surface(&descriptor)?;
            let snapshot = surface.snapshot();
            assert!(snapshot.display_link_paused());
            assert_eq!(snapshot.callback_count(), 0);
            assert_eq!(snapshot.submission_count(), 0);
            assert_eq!(snapshot.allocated_bytes(), 0);
            assert_eq!(snapshot.current_retained_bytes(), 0);
            assert_exact_teardown(native_validation::close_with_owner_evidence(surface)?);
        }
        Ok(())
    }

    fn assert_exact_teardown(evidence: native_validation::NativeOwnerEvidence) {
        assert_eq!(evidence.acquired(), [1; OWNER_KINDS]);
        assert_eq!(evidence.released(), [1; OWNER_KINDS]);
        assert_eq!(evidence.active(), [0; OWNER_KINDS]);
        assert_eq!(evidence.run_loop_registrations(), 1);
        assert_eq!(evidence.link_invalidations(), 1);
        assert_eq!(evidence.delegate_revocations(), 1);
        assert_eq!(evidence.window_closes(), 1);
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
