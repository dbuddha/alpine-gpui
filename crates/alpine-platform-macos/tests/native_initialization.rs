//! Process-main-thread partial-initialization rollback validation.

#[cfg(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64"))]
fn main() -> Result<(), alpine_platform_macos::SurfaceError> {
    alpine_platform_macos::native_validation::validate_initialization_rollback()
}

#[cfg(not(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64")))]
fn main() {}
