//! Post-commit supersession and native device-loss validation.

#[cfg(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    validation::run()
}

#[cfg(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64"))]
mod validation {
    use std::{error::Error, ffi::OsStr, time::Duration};

    use alpine_core::{LinearRgba, Point, Rect, Size};
    use alpine_metal::{BackendState, RecoveryClassification, RenderError};
    use alpine_platform::PresentationOutcome;
    use alpine_platform_macos::{
        NativeSurface, SurfaceDescriptor, SurfaceError, native_validation,
    };
    use alpine_scene::{Primitive, Scene, SceneBuilder, SceneRevision};

    type TestResult<T = ()> = Result<T, Box<dyn Error>>;

    pub(super) fn run() -> TestResult {
        let hosted_direct = match std::env::var_os("ALPINE_PRESENTATION_EVIDENCE_MODE") {
            None => false,
            Some(mode) if mode == OsStr::new("hosted-direct") => true,
            Some(_) => return Err("unsupported presentation evidence mode".into()),
        };
        let (scene, clear) = validation_scene()?;
        validate_supersession(scene.clone(), clear, hosted_direct)?;
        validate_device_loss(scene, clear, hosted_direct)
    }

    fn validate_supersession(scene: Scene, clear: LinearRgba, hosted_direct: bool) -> TestResult {
        let descriptor = SurfaceDescriptor::new("Alpine supersession", 96.0, 64.0, 1.0)?;
        let surface = native_validation::new_surface(&descriptor)?;
        prepare_visible_surface(&surface, hosted_direct)?;
        let before = surface.snapshot();
        assert_eq!(surface.request_frame(scene, clear)?.get(), 1);
        native_validation::inject_post_commit_observation(&surface, Some(usize::MAX), 1.25)?;
        native_validation::run_until_frame_terminal(&surface, Duration::from_secs(5));

        assert_eq!(surface.take_error()?, None);
        let superseded = surface.snapshot();
        let terminal = superseded
            .last_superseded()
            .ok_or("superseded terminal evidence")?;
        assert_eq!(terminal.attempt(), 1);
        assert_eq!(terminal.requested_revision().get(), 1);
        assert_eq!(terminal.frame_revision().get(), 1);
        assert_eq!(terminal.frame_epoch().get(), before.surface_epoch());
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
        assert!(superseded.submission_count() >= 1);
        assert_eq!(
            superseded.direct_present_count(),
            superseded.submission_count()
        );
        assert!(superseded.presented_count() >= 1);
        assert_eq!(superseded.qualified_presented_count(), 0);
        assert_eq!(superseded.superseded_count(), 1);
        assert!(!superseded.display_link_paused());

        validate_retry(&surface, terminal.attempt(), superseded.submission_count())?;
        surface.close();
        Ok(())
    }

    fn validate_retry(
        surface: &NativeSurface,
        superseded_attempt: u64,
        superseded_submissions: u64,
    ) -> TestResult {
        native_validation::inject_post_commit_observation(surface, None, 1.5)?;
        native_validation::run_until_frame_terminal(surface, Duration::from_secs(5));
        assert_eq!(surface.take_error()?, None);
        let recovered = surface.snapshot();
        let terminal = recovered
            .last_terminal()
            .ok_or("recovered terminal evidence")?;
        assert!(terminal.attempt() > superseded_attempt);
        assert_eq!(terminal.requested_revision().get(), 1);
        assert_eq!(terminal.frame_revision().get(), 1);
        assert_eq!(terminal.frame_epoch().get(), recovered.surface_epoch());
        assert_eq!(terminal.outcome(), PresentationOutcome::Presented);
        assert_eq!(terminal.submission_count(), 1);
        assert_eq!(terminal.present_call_count(), 1);
        assert!(terminal.eligible_at_commit());
        assert_eq!(
            terminal.observed_presentation_time_bits(),
            1.5_f64.to_bits()
        );
        assert_eq!(terminal.retained_bytes(), 0);
        assert_eq!(terminal.recovery(), None);
        assert!(recovered.submission_count() > superseded_submissions);
        assert_eq!(
            recovered.direct_present_count(),
            recovered.submission_count()
        );
        assert!(recovered.presented_count() >= 2);
        assert_eq!(recovered.qualified_presented_count(), 1);
        assert_eq!(recovered.superseded_count(), 1);
        assert!(recovered.display_link_paused());
        Ok(())
    }

    fn validate_device_loss(scene: Scene, clear: LinearRgba, hosted_direct: bool) -> TestResult {
        let descriptor = SurfaceDescriptor::new("Alpine device loss", 96.0, 64.0, 1.0)?;
        let surface = native_validation::new_surface_with_device_loss(&descriptor)?;
        prepare_visible_surface(&surface, hosted_direct)?;
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
        native_validation::inject_surface_configuration(&surface, 96.0, 64.0, 1.0, 0, true)?;
        assert!(surface.snapshot().display_link_paused());
        assert_eq!(failed.submission_count(), 1);
        assert_eq!(failed.direct_present_count(), 1);
        assert_eq!(failed.failed_count(), 1);
        assert_eq!(failed.qualified_presented_count(), 0);
        assert_eq!(failed.superseded_count(), 0);
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
        assert_eq!(guarded.submission_count(), 1);
        assert_eq!(guarded.direct_present_count(), 1);
        assert_eq!(guarded.failed_count(), 2);
        assert!(guarded.display_link_paused());
        Ok(())
    }

    fn prepare_visible_surface(surface: &NativeSurface, hosted_direct: bool) -> TestResult {
        surface.show()?;
        if hosted_direct {
            native_validation::inject_surface_configuration(surface, 96.0, 64.0, 1.0, 0, true)?;
        }
        Ok(())
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
