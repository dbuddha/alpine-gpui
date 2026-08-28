//! Process-main-thread callback-drawable presentation validation.

#[cfg(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    validation::run()
}

#[cfg(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64"))]
mod validation {
    use std::{cell::Cell, error::Error, ffi::OsStr, rc::Rc, time::Duration};

    use alpine_core::{LinearRgba, Point, Rect, Size};
    use alpine_metal::RenderError;
    use alpine_platform::PresentationOutcome;
    use alpine_platform_macos::{
        NativeSurface, SurfaceDescriptor, SurfaceError, SurfaceEvent, SurfaceResponse,
        SurfaceSnapshot, SurfaceWakeAdmission, native_validation,
    };
    use alpine_scene::{Primitive, SceneBuilder, SceneRevision};

    type TestResult<T = ()> = Result<T, Box<dyn Error>>;

    pub(super) fn run() -> TestResult {
        let hosted_direct = hosted_direct_mode()?;
        let descriptor = SurfaceDescriptor::new("Alpine presented frame", 96.0, 64.0, 1.0)?;
        let surface = native_validation::new_surface(&descriptor)?;
        native_validation::inject_driver_error(
            &surface,
            SurfaceError::invariant(alpine_platform_macos::SurfaceOperation::Application),
        );
        assert_eq!(
            surface.take_error()?,
            Some(SurfaceError::invariant(
                alpine_platform_macos::SurfaceOperation::Application
            ))
        );
        assert_eq!(surface.take_error()?, None);
        surface.show()?;

        let viewport = Size::new(96.0, 64.0).ok_or("valid viewport")?;
        let bounds = Rect::new(
            Point::new(8.0, 8.0).ok_or("valid origin")?,
            Size::new(80.0, 48.0).ok_or("valid quad size")?,
        );
        let color = LinearRgba::new(0.75, 0.25, 0.125, 1.0).ok_or("valid color")?;
        let clear = LinearRgba::new(0.0, 0.0, 0.0, 1.0).ok_or("valid clear")?;
        let Some(first) =
            validate_first_frame(&surface, viewport, bounds, color, clear, hosted_direct)?
        else {
            surface.close();
            return Ok(());
        };
        validate_failure_and_recovery(&surface, &first, viewport, bounds, color, clear)?;
        surface.close();
        Ok(())
    }

    fn hosted_direct_mode() -> TestResult<bool> {
        match std::env::var_os("ALPINE_PRESENTATION_EVIDENCE_MODE") {
            None => Ok(false),
            Some(mode) if mode == OsStr::new("hosted-direct") => Ok(true),
            Some(_) => Err("unsupported presentation evidence mode".into()),
        }
    }

    fn validate_first_frame(
        surface: &NativeSurface,
        viewport: Size,
        bounds: Rect,
        color: LinearRgba,
        clear: LinearRgba,
        hosted_direct: bool,
    ) -> TestResult<Option<SurfaceSnapshot>> {
        let scene = scene(1, viewport, bounds, color);
        assert_eq!(surface.request_frame(scene, clear)?.get(), 1);
        if hosted_direct {
            // Hosted runners may expose no compositor-visible occlusion bit.
            native_validation::inject_surface_configuration(surface, 96.0, 64.0, 1.0, 0, true)?;
        }
        let timeout = Duration::from_secs(if hosted_direct { 2 } else { 5 });
        let observed_wakes = Rc::new(Cell::new(0_u64));
        let callback_wakes = Rc::clone(&observed_wakes);
        assert_eq!(surface.waker().wake(), SurfaceWakeAdmission::Scheduled);
        native_validation::run_until_frame_terminal_with_handler(surface, timeout, move |event| {
            if matches!(event, SurfaceEvent::Wake { .. }) {
                callback_wakes.set(callback_wakes.get().saturating_add(1));
            }
            SurfaceResponse::default()
        })?;
        assert_eq!(observed_wakes.get(), 1);

        let first_error = surface.take_error()?;
        let snapshot = surface.snapshot();
        assert!(snapshot.regular_activation_policy());
        assert!(snapshot.submission_count() >= 1);
        assert_eq!(snapshot.frame_slot_capacity(), 3);
        assert!(snapshot.peak_occupied_frame_slots() >= 1);
        assert_eq!(snapshot.direct_present_count(), snapshot.submission_count());
        assert_eq!(
            snapshot.installed_presented_handler_count(),
            snapshot.submission_count()
        );
        assert_eq!(
            snapshot.submission_count(),
            snapshot.presented_count() + snapshot.skipped_count() + snapshot.superseded_count()
        );
        if hosted_direct && snapshot.presented_count() == 0 {
            assert!(first_error.is_none());
            let terminal = snapshot
                .last_terminal()
                .ok_or("hosted-direct missing-presentation terminal evidence")?;
            assert_eq!(terminal.requested_revision().get(), 1);
            assert_eq!(terminal.frame_revision().get(), 1);
            assert_eq!(terminal.outcome(), PresentationOutcome::Failed);
            assert_eq!(terminal.submission_count(), 1);
            assert_eq!(terminal.present_call_count(), 1);
            assert!(terminal.eligible_at_commit());
            assert_eq!(terminal.observed_presentation_time_bits(), 0);
            assert_eq!(terminal.retained_bytes(), 0);
            assert_eq!(snapshot.occupied_frame_slots(), 0);
            assert_eq!(snapshot.submitted_frame_slots(), 0);
            assert_eq!(snapshot.last_presented_time_bits(), 0);
            assert!(snapshot.skipped_count() >= 1);
            assert!(snapshot.failed_count() >= 1);
            assert!(snapshot.callback_count() >= 2);
            assert!(snapshot.display_link_paused());
            eprintln!(
                "hosted-direct evidence: the latest callback drawable was committed, directly presented, reported not presented by Core Animation, and released"
            );
            return Ok(None);
        }
        if let Some(error) = first_error {
            return Err(error.into());
        }
        if hosted_direct {
            assert!(snapshot.presented_count() >= 1);
        } else {
            assert_eq!(snapshot.presented_count(), 1);
        }
        assert_eq!(snapshot.occupied_frame_slots(), 0);
        assert_eq!(snapshot.submitted_frame_slots(), 0);
        assert_ne!(snapshot.last_presented_time_bits(), 0);
        assert_eq!(snapshot.failed_count(), 0);
        assert!(snapshot.callback_count() >= 2);
        assert!(snapshot.display_link_paused());
        Ok(Some(snapshot))
    }

    fn validate_failure_and_recovery(
        surface: &NativeSurface,
        first: &SurfaceSnapshot,
        viewport: Size,
        bounds: Rect,
        color: LinearRgba,
        clear: LinearRgba,
    ) -> TestResult {
        let invalid_viewport = Size::new(95.0, 64.0).ok_or("valid invalid-control viewport")?;
        assert_eq!(
            surface
                .request_frame(scene(2, invalid_viewport, bounds, color), clear)?
                .get(),
            2
        );
        native_validation::run_until_frame_terminal(surface, Duration::from_secs(5));
        let error = surface.take_error()?.ok_or("expected callback failure")?;
        assert!(matches!(
            error,
            SurfaceError::Render(RenderError::Validation(_))
        ));
        let failed = surface.snapshot();
        assert_eq!(failed.submission_count(), first.submission_count());
        assert_eq!(
            failed.installed_presented_handler_count(),
            first.installed_presented_handler_count() + 1
        );
        assert_eq!(failed.direct_present_count(), first.direct_present_count());
        assert_eq!(failed.presented_count(), 1);
        assert_eq!(
            failed.last_presented_time_bits(),
            first.last_presented_time_bits()
        );
        assert_eq!(failed.failed_count(), 1);
        assert!(failed.display_link_paused());

        assert_eq!(
            surface
                .request_frame(scene(3, viewport, bounds, color), clear)?
                .get(),
            3
        );
        native_validation::run_until_frame_terminal(surface, Duration::from_secs(5));
        assert_eq!(surface.take_error()?, None);
        let recovered = surface.snapshot();
        assert!(recovered.submission_count() > failed.submission_count());
        assert_eq!(
            recovered.installed_presented_handler_count(),
            recovered.submission_count() + 1
        );
        assert_eq!(
            recovered.direct_present_count(),
            recovered.submission_count()
        );
        assert_eq!(recovered.presented_count(), 2);
        assert_ne!(recovered.last_presented_time_bits(), 0);
        assert_eq!(
            recovered.skipped_count(),
            recovered.submission_count() - recovered.presented_count()
        );
        assert_eq!(recovered.failed_count(), 1);
        assert!(recovered.display_link_paused());
        assert_eq!(recovered.occupied_frame_slots(), 0);
        assert_eq!(recovered.submitted_frame_slots(), 0);
        Ok(())
    }

    fn scene(
        revision: u64,
        viewport: Size,
        bounds: Rect,
        color: LinearRgba,
    ) -> alpine_scene::Scene {
        let mut builder = SceneBuilder::new(SceneRevision::new(revision), viewport);
        builder.push(Primitive::Quad { bounds, color });
        builder.finish()
    }
}

#[cfg(not(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64")))]
fn main() {}
