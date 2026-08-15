//! Process-main-thread callback-drawable presentation validation.

#[cfg(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::{ffi::OsStr, time::Duration};

    use alpine_core::{LinearRgba, Point, Rect, Size};
    use alpine_metal::RenderError;
    use alpine_platform_macos::{SurfaceDescriptor, SurfaceError, native_validation};
    use alpine_scene::{Primitive, SceneBuilder, SceneRevision};

    let descriptor = SurfaceDescriptor::new("Alpine presented frame", 96.0, 64.0, 1.0)?;
    let surface = native_validation::new_surface(&descriptor)?;
    let hosted_direct = match std::env::var_os("ALPINE_PRESENTATION_EVIDENCE_MODE") {
        None => false,
        Some(mode) if mode == OsStr::new("hosted-direct") => true,
        Some(_) => return Err("unsupported presentation evidence mode".into()),
    };
    native_validation::inject_driver_error(&surface, SurfaceError::DriverUnavailable);
    assert_eq!(surface.take_error()?, Some(SurfaceError::DriverUnavailable));
    assert_eq!(surface.take_error()?, None);
    surface.show()?;

    let viewport = Size::new(96.0, 64.0).ok_or("valid viewport")?;
    let bounds = Rect::new(
        Point::new(8.0, 8.0).ok_or("valid origin")?,
        Size::new(80.0, 48.0).ok_or("valid quad size")?,
    );
    let color = LinearRgba::new(0.75, 0.25, 0.125, 1.0).ok_or("valid color")?;
    let clear = LinearRgba::new(0.0, 0.0, 0.0, 1.0).ok_or("valid clear")?;
    let mut builder = SceneBuilder::new(SceneRevision::new(1), viewport);
    builder.push(Primitive::Quad { bounds, color });
    let requested = surface.request_frame(builder.finish(), clear)?;
    assert_eq!(requested.get(), 1);
    if hosted_direct {
        // Hosted runners may expose no compositor-visible occlusion bit. Keep
        // that limitation explicit while exercising the same configuration,
        // epoch, callback-drawable, commit, and direct-present path.
        native_validation::inject_surface_configuration(&surface, 96.0, 64.0, 1.0, 0, true)?;
    }
    let observation_timeout = if hosted_direct {
        Duration::from_secs(2)
    } else {
        Duration::from_secs(5)
    };
    native_validation::run_until_frame_terminal(&surface, observation_timeout);

    let first_error = surface.take_error()?;
    let snapshot = surface.snapshot();
    assert!(snapshot.regular_activation_policy());
    assert!(snapshot.submission_count() >= 1);
    assert_eq!(snapshot.direct_present_count(), snapshot.submission_count());
    assert_eq!(
        snapshot.installed_presented_handler_count(),
        snapshot.submission_count()
    );
    if hosted_direct && snapshot.presented_count() == 0 {
        assert!(first_error.is_none());
        assert_eq!(snapshot.last_presented_time_bits(), 0);
        // The driver owns at most one callback drawable at a time. At the
        // hosted cutoff every completed outcome must be an explicit drop, and
        // exactly one directly presented drawable may still await its handler.
        assert_eq!(snapshot.skipped_count() + 1, snapshot.submission_count());
        assert_eq!(snapshot.failed_count(), 0);
        assert!(snapshot.callback_count() >= 2);
        eprintln!(
            "hosted-direct evidence: {} callback drawables committed and directly presented; Core Animation reported {} dropped outcomes and one drawable remains in flight at the bounded cutoff",
            snapshot.submission_count(),
            snapshot.skipped_count()
        );
        surface.close();
        return Ok(());
    }
    if let Some(error) = first_error {
        return Err(error.into());
    }
    assert_eq!(snapshot.presented_count(), 1);
    assert_ne!(snapshot.last_presented_time_bits(), 0);
    assert_eq!(
        snapshot.skipped_count(),
        snapshot.submission_count() - snapshot.presented_count()
    );
    assert_eq!(snapshot.failed_count(), 0);
    assert!(snapshot.callback_count() >= 2);
    assert!(snapshot.display_link_paused());

    let invalid_viewport = Size::new(95.0, 64.0).ok_or("valid invalid-control viewport")?;
    let invalid_scene = SceneBuilder::new(SceneRevision::new(2), invalid_viewport).finish();
    let invalid_revision = surface.request_frame(invalid_scene, clear)?;
    assert_eq!(invalid_revision.get(), 2);
    native_validation::run_until_frame_terminal(&surface, Duration::from_secs(5));

    let error = surface.take_error()?.ok_or("expected callback failure")?;
    assert!(matches!(
        error,
        SurfaceError::Render(RenderError::Validation(_))
    ));
    let failed = surface.snapshot();
    assert_eq!(failed.submission_count(), snapshot.submission_count());
    assert_eq!(
        failed.installed_presented_handler_count(),
        snapshot.installed_presented_handler_count() + 1
    );
    assert_eq!(
        failed.direct_present_count(),
        snapshot.direct_present_count()
    );
    assert_eq!(failed.presented_count(), 1);
    assert_eq!(
        failed.last_presented_time_bits(),
        snapshot.last_presented_time_bits()
    );
    assert_eq!(failed.failed_count(), 1);
    assert!(failed.display_link_paused());

    let mut recovery_builder = SceneBuilder::new(SceneRevision::new(3), viewport);
    recovery_builder.push(Primitive::Quad { bounds, color });
    let recovery_revision = surface.request_frame(recovery_builder.finish(), clear)?;
    assert_eq!(recovery_revision.get(), 3);
    native_validation::run_until_frame_terminal(&surface, Duration::from_secs(5));

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
    surface.close();
    Ok(())
}

#[cfg(not(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64")))]
fn main() {}
