//! First shipping Alpine Studio application boundary.

use alpine_core::{LinearRgba, Point, Rect, Size};
use alpine_platform_macos::SurfaceError;
use alpine_scene::{Primitive, Scene, SceneBuilder, SceneRevision};

const WINDOW_WIDTH: f32 = 960.0;
const WINDOW_HEIGHT: f32 = 540.0;

/// Builds the first immutable Studio scene through public Alpine values.
///
/// # Errors
///
/// Returns [`SurfaceError::DriverUnavailable`] if a compile-time scene value
/// violates an Alpine domain invariant.
pub fn initial_scene() -> Result<Scene, SurfaceError> {
    let viewport = Size::new(WINDOW_WIDTH, WINDOW_HEIGHT).ok_or(SurfaceError::DriverUnavailable)?;
    let origin = Point::new(40.0, 40.0).ok_or(SurfaceError::DriverUnavailable)?;
    let bounds = Rect::new(
        origin,
        Size::new(240.0, 120.0).ok_or(SurfaceError::DriverUnavailable)?,
    );
    let color = LinearRgba::new(0.22, 0.57, 0.92, 1.0).ok_or(SurfaceError::DriverUnavailable)?;
    let mut builder = SceneBuilder::new(SceneRevision::new(1), viewport);
    builder.push(Primitive::Quad { bounds, color });
    Ok(builder.finish())
}

/// Opens one native Studio window, requests one frame, and runs until close.
///
/// # Errors
///
/// Returns the structured surface error from scene construction, native
/// initialization, frame admission, or the application run loop.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
pub fn run() -> Result<(), SurfaceError> {
    use alpine_platform_macos::{NativeSurface, SurfaceDescriptor};

    let clear = LinearRgba::new(0.02, 0.02, 0.02, 1.0).ok_or(SurfaceError::DriverUnavailable)?;
    let descriptor = SurfaceDescriptor::new(
        "Alpine Studio",
        f64::from(WINDOW_WIDTH),
        f64::from(WINDOW_HEIGHT),
        2.0,
    )?;
    let surface = NativeSurface::new(&descriptor)?;
    surface.show()?;
    let _revision = surface.request_frame(initial_scene()?, clear)?;
    surface.run()
}

/// Rejects execution where the first native Studio runtime is unsupported.
///
/// # Errors
///
/// Always returns [`SurfaceError::UnsupportedPlatform`].
#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
pub const fn run() -> Result<(), SurfaceError> {
    Err(SurfaceError::UnsupportedPlatform)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_scene_preserves_the_shipping_visual_contract() -> Result<(), SurfaceError> {
        let scene = initial_scene()?;
        assert_eq!(scene.revision(), SceneRevision::new(1));
        assert_eq!(
            scene.viewport(),
            Size::new(WINDOW_WIDTH, WINDOW_HEIGHT).ok_or(SurfaceError::DriverUnavailable)?
        );

        let [Primitive::Quad { bounds, color }] = scene.primitives() else {
            return Err(SurfaceError::DriverUnavailable);
        };
        assert_eq!(
            bounds.origin(),
            Point::new(40.0, 40.0).ok_or(SurfaceError::DriverUnavailable)?
        );
        assert_eq!(
            bounds.size(),
            Size::new(240.0, 120.0).ok_or(SurfaceError::DriverUnavailable)?
        );
        assert_eq!(
            *color,
            LinearRgba::new(0.22, 0.57, 0.92, 1.0).ok_or(SurfaceError::DriverUnavailable)?
        );
        Ok(())
    }

    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    #[test]
    fn run_rejects_an_unsupported_host() {
        assert_eq!(run(), Err(SurfaceError::UnsupportedPlatform));
    }
}
