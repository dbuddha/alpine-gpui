//! Minimal shipping application that validates the one-window event loop contract.

use std::error::Error;

use alpine_core::{LinearRgba, Point, Rect, Size};
use alpine_platform_macos::{NativeSurface, SurfaceDescriptor, SurfaceError};
use alpine_scene::{Primitive, SceneBuilder, SceneRevision};

fn is_alpine_studio_host() -> bool {
    cfg!(all(target_os = "macos", target_arch = "aarch64"))
}

fn create_scene() -> Result<alpine_scene::Scene, &'static str> {
    if !is_alpine_studio_host() {
        return Err("alpine studio requires Apple Silicon macOS");
    }

    let viewport = Size::new(960.0, 540.0).ok_or("alpine studio viewport must be valid")?;
    let mut builder = SceneBuilder::new(SceneRevision::new(1), viewport);
    let origin = Point::new(40.0, 40.0).ok_or("quad origin must be valid")?;
    let bounds = Rect::new(
        origin,
        Size::new(240.0, 120.0).ok_or("quad size must be valid")?,
    );
    builder.push(Primitive::Quad {
        bounds,
        color: LinearRgba::new(0.22, 0.57, 0.92, 1.0)
            .ok_or("studio background tone must be valid")?,
    });
    Ok(builder.finish())
}

fn run_studio() -> Result<(), SurfaceError> {
    if !is_alpine_studio_host() {
        return Err(SurfaceError::UnsupportedPlatform);
    }

    let clear = LinearRgba::new(0.02, 0.02, 0.02, 1.0).ok_or(SurfaceError::DriverUnavailable)?;
    let descriptor = SurfaceDescriptor::new("Alpine Studio", 960.0, 540.0, 2.0)?;
    let surface = NativeSurface::new(&descriptor)?;
    let scene = create_scene().map_err(|_| SurfaceError::DriverUnavailable)?;
    surface.show()?;
    let _ = surface.request_frame(scene, clear)?;
    surface.run()
}

/// Initializes one native surface, submits one immutable scene, and enters the
/// process run loop until the owned window closes.
fn main() -> Result<(), Box<dyn Error>> {
    run_studio().map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_alpine_studio_host_is_target_os_and_arch() {
        assert_eq!(
            is_alpine_studio_host(),
            cfg!(all(target_os = "macos", target_arch = "aarch64"))
        );
    }

    #[test]
    fn create_scene_platform_gate() {
        if is_alpine_studio_host() {
            assert!(create_scene().is_ok());
        } else {
            assert_eq!(
                create_scene(),
                Err("alpine studio requires Apple Silicon macOS")
            );
        }
    }

    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    #[test]
    fn run_studio_returns_unsupported_on_non_native_host() {
        assert!(matches!(
            run_studio(),
            Err(SurfaceError::UnsupportedPlatform)
        ));
    }

    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    #[test]
    fn main_propagates_surface_error() {
        let outcome = main();
        assert!(outcome.is_err());
        let err = outcome.expect_err("run should fail on non-native host");
        let source = err.downcast_ref::<SurfaceError>();
        assert_eq!(source, Some(&SurfaceError::UnsupportedPlatform));
    }
}
