//! Process-main-thread SDR presentation-color qualification.

#[cfg(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64"))]
use alpine_platform_macos::{SdrColorContract, SurfaceDescriptor, native_validation};

#[cfg(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64"))]
fn main() -> Result<(), alpine_platform_macos::SurfaceError> {
    let descriptor = SurfaceDescriptor::new("Alpine SDR color", 96.0, 64.0, 2.0)?;
    let surface = native_validation::new_surface(&descriptor)?;

    let snapshot = surface.snapshot();
    assert_eq!(
        snapshot.sdr_color_contract(),
        Some(SdrColorContract::LinearSrgbToBgra8UnormSrgb)
    );
    assert!(!snapshot.extended_dynamic_range());
    assert!(snapshot.framebuffer_only());
    assert_eq!(snapshot.submission_count(), 0);
    assert_eq!(snapshot.current_retained_bytes(), 0);
    assert!(snapshot.peak_retained_bytes() > 0);
    assert!(snapshot.current_upload_bytes() > 0);
    assert!(snapshot.current_upload_bytes() <= snapshot.peak_upload_bytes());
    assert!(snapshot.peak_upload_bytes() <= 3 * 8 * 1024 * 1024);

    surface.close();
    Ok(())
}

#[cfg(not(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64")))]
fn main() {}
