//! Process-main-thread native surface epoch and eligibility validation.

#[cfg(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    validation::run()
}

#[cfg(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64"))]
mod validation {
    use std::error::Error;

    use alpine_core::{LinearRgba, Size};
    use alpine_platform_macos::{
        NativeSurface, SurfaceDescriptor, SurfaceError, SurfaceSnapshot, native_validation,
    };
    use alpine_scene::{SceneBuilder, SceneRevision};

    type TestResult<T = ()> = Result<T, Box<dyn Error>>;

    pub(super) fn run() -> TestResult {
        let descriptor = SurfaceDescriptor::new("Alpine surface epochs", 96.0, 64.0, 1.0)?;
        let surface = native_validation::new_surface(&descriptor)?;
        let reset = validate_real_resize(&surface);
        request_hidden_frame(&surface)?;
        let restored = validate_visibility_and_zero_size(&surface, &reset)?;
        let scaled = validate_scale_and_rejection(&surface, &restored)?;
        validate_hidden_display_migration(&surface, &scaled)?;
        validate_close(surface);
        Ok(())
    }

    fn validate_real_resize(surface: &NativeSurface) -> SurfaceSnapshot {
        let initial = surface.snapshot();
        assert_eq!(initial.surface_epoch(), 0);
        assert!(initial.is_sized());
        assert!(!initial.is_presentation_visible());
        assert!(initial.display_link_paused());

        native_validation::resize_content(surface, 120.0, 80.0);
        let resized = surface.snapshot();
        assert_eq!(resized.surface_epoch(), initial.surface_epoch() + 1);
        assert_ne!(resized.physical_width(), initial.physical_width());
        assert_ne!(resized.physical_height(), initial.physical_height());
        assert_eq!(resized.physical_width() * 2, resized.physical_height() * 3);
        native_validation::resize_content(surface, 120.0, 80.0);
        assert_eq!(surface.snapshot(), resized);
        native_validation::resize_content(surface, 96.0, 64.0);
        let reset = surface.snapshot();
        assert_eq!(reset.surface_epoch(), resized.surface_epoch() + 1);
        assert_eq!(reset.physical_width() * 2, reset.physical_height() * 3);
        reset
    }

    fn request_hidden_frame(surface: &NativeSurface) -> TestResult {
        let viewport = Size::new(96.0, 64.0).ok_or("valid viewport")?;
        let clear = LinearRgba::new(0.0, 0.0, 0.0, 1.0).ok_or("valid clear")?;
        let scene = SceneBuilder::new(SceneRevision::new(1), viewport).finish();
        assert_eq!(surface.request_frame(scene, clear)?.get(), 1);
        let hidden_dirty = surface.snapshot();
        assert!(hidden_dirty.display_link_paused());
        assert_eq!(hidden_dirty.submission_count(), 0);
        assert_eq!(hidden_dirty.allocated_bytes(), 0);
        Ok(())
    }

    fn validate_visibility_and_zero_size(
        surface: &NativeSurface,
        reset: &SurfaceSnapshot,
    ) -> TestResult<SurfaceSnapshot> {
        native_validation::inject_surface_configuration(surface, 96.0, 64.0, 1.0, 101, true)?;
        let visible = surface.snapshot();
        assert_eq!(visible.surface_epoch(), reset.surface_epoch() + 1);
        assert!(visible.is_sized());
        assert!(visible.is_presentation_visible());
        assert!(!visible.display_link_paused());
        native_validation::inject_surface_configuration(surface, 96.0, 64.0, 1.0, 101, true)?;
        assert_eq!(surface.snapshot(), visible);

        native_validation::inject_surface_configuration(surface, 0.0, 64.0, 1.0, 101, true)?;
        let zero = surface.snapshot();
        assert_eq!(zero.surface_epoch(), visible.surface_epoch() + 1);
        assert!(!zero.is_sized());
        assert!(zero.display_link_paused());
        assert_eq!(zero.submission_count(), visible.submission_count());
        assert_eq!(zero.direct_present_count(), visible.direct_present_count());
        assert_eq!(zero.allocated_bytes(), visible.allocated_bytes());
        assert_eq!(
            zero.current_retained_bytes(),
            visible.current_retained_bytes()
        );

        native_validation::inject_surface_configuration(surface, 96.0, 64.0, 1.0, 101, true)?;
        let restored = surface.snapshot();
        assert_eq!(restored.surface_epoch(), zero.surface_epoch() + 1);
        assert!(restored.is_sized());
        assert!(!restored.display_link_paused());
        Ok(restored)
    }

    fn validate_scale_and_rejection(
        surface: &NativeSurface,
        restored: &SurfaceSnapshot,
    ) -> TestResult<SurfaceSnapshot> {
        native_validation::inject_surface_configuration(surface, 96.0, 64.0, 2.0, 101, true)?;
        let scaled = surface.snapshot();
        assert_eq!(scaled.surface_epoch(), restored.surface_epoch() + 1);
        assert_eq!(scaled.physical_width(), 192);
        assert_eq!(scaled.physical_height(), 128);

        let invalid = native_validation::inject_surface_configuration(
            surface, 16_385.0, 64.0, 1.0, 101, true,
        );
        assert!(matches!(
            invalid,
            Err(SurfaceError::PhysicalDimensionOutOfRange { .. })
        ));
        let rejected = surface.snapshot();
        assert_eq!(rejected.surface_epoch(), scaled.surface_epoch());
        assert_eq!(rejected.physical_width(), scaled.physical_width());
        assert_eq!(rejected.physical_height(), scaled.physical_height());
        assert!(!rejected.is_sized());
        assert!(!rejected.is_presentation_visible());
        assert!(rejected.display_link_paused());
        assert_eq!(rejected.submission_count(), scaled.submission_count());
        assert_eq!(rejected.allocated_bytes(), scaled.allocated_bytes());
        assert!(matches!(
            surface.take_error()?,
            Some(SurfaceError::PhysicalDimensionOutOfRange { .. })
        ));

        native_validation::inject_surface_configuration(surface, 96.0, 64.0, 2.0, 101, true)?;
        let recovered = surface.snapshot();
        assert_eq!(recovered.surface_epoch(), rejected.surface_epoch());
        assert!(recovered.is_sized());
        assert!(recovered.is_presentation_visible());
        assert!(!recovered.display_link_paused());
        Ok(recovered)
    }

    fn validate_hidden_display_migration(
        surface: &NativeSurface,
        recovered: &SurfaceSnapshot,
    ) -> TestResult {
        native_validation::inject_surface_configuration(surface, 96.0, 64.0, 2.0, 101, false)?;
        let hidden = surface.snapshot();
        assert_eq!(hidden.surface_epoch(), recovered.surface_epoch());
        assert!(!hidden.is_presentation_visible());
        assert!(hidden.display_link_paused());

        native_validation::inject_surface_configuration(surface, 96.0, 64.0, 2.0, 202, false)?;
        let migrated = surface.snapshot();
        assert_eq!(migrated.surface_epoch(), hidden.surface_epoch() + 1);
        assert!(migrated.display_link_paused());
        Ok(())
    }

    fn validate_close(surface: NativeSurface) {
        let observer = surface.observer();
        native_validation::close_window(&surface);
        assert_eq!(
            observer.lifecycle(),
            alpine_platform_macos::SurfaceLifecycle::Closing
        );
        let callbacks_after_native_close = observer.callback_count();
        surface.close();
        assert_eq!(
            observer.lifecycle(),
            alpine_platform_macos::SurfaceLifecycle::Closed
        );
        assert_eq!(observer.callback_count(), callbacks_after_native_close);
    }
}

#[cfg(not(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64")))]
fn main() {}
