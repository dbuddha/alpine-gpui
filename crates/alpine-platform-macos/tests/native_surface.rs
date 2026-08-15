//! Process-main-thread native surface smoke test.

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
use alpine_metal::InitializationError;
#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
use alpine_platform_macos::SurfaceError;
use alpine_platform_macos::{NativeSurface, SurfaceDescriptor};
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
use alpine_platform_macos::{SdrColorContract, SurfaceLifecycle};

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn main() -> Result<(), alpine_platform_macos::SurfaceError> {
    let descriptor = SurfaceDescriptor::new("Alpine native surface test", 320.0, 180.0, 2.0)?;
    let surface = match NativeSurface::new(&descriptor) {
        Ok(surface) => surface,
        Err(alpine_platform_macos::SurfaceError::RendererInitialization(
            InitializationError::UnsupportedDevice { .. },
        )) => return Ok(()),
        Err(error) => return Err(error),
    };

    let snapshot = surface.snapshot();
    assert_eq!(snapshot.physical_width(), 640);
    assert_eq!(snapshot.physical_height(), 360);
    assert_eq!(
        snapshot.sdr_color_contract(),
        Some(SdrColorContract::LinearSrgbToBgra8UnormSrgb)
    );
    assert!(!snapshot.extended_dynamic_range());
    assert!(snapshot.framebuffer_only());
    assert!(snapshot.display_sync_enabled());
    assert!(snapshot.allows_next_drawable_timeout());
    assert_eq!(snapshot.maximum_drawable_count(), 3);
    assert!(snapshot.display_link_paused());
    assert!(!snapshot.visible());
    assert_eq!(snapshot.callback_count(), 0);
    assert_eq!(snapshot.submission_count(), 0);
    assert_eq!(snapshot.direct_present_count(), 0);
    assert_eq!(snapshot.presented_count(), 0);
    assert_eq!(snapshot.skipped_count(), 0);
    assert_eq!(snapshot.failed_count(), 0);

    let observer = surface.observer();
    assert_eq!(observer.lifecycle(), SurfaceLifecycle::Live);
    assert_eq!(observer.callback_count(), 0);
    surface.show()?;
    assert!(surface.snapshot().visible());
    surface.close();
    assert_eq!(observer.lifecycle(), SurfaceLifecycle::Closed);
    assert_eq!(observer.callback_count(), 0);
    Ok(())
}

#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
fn main() -> Result<(), SurfaceError> {
    let descriptor = SurfaceDescriptor::new("Alpine native surface test", 320.0, 180.0, 1.0)?;
    assert!(matches!(
        NativeSurface::new(&descriptor),
        Err(SurfaceError::UnsupportedPlatform)
    ));
    Ok(())
}
