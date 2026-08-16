//! First shipping Alpine Studio application boundary.

use alpine_core::{LinearRgba, Point, Rect, Size};
use alpine_platform_macos::{SurfaceDescriptor, SurfaceError, SurfaceEvent};
use alpine_runtime::{
    AppContext, AppDelegate, Application, RuntimeError, WindowContext, WorkerConfig,
};
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
    Ok(StudioApp::new()?.scene(SceneRevision::new(1), viewport))
}

/// Opens one native Studio window, requests one frame, and runs until close.
///
/// # Errors
///
/// Returns the structured surface error from scene construction, native
/// initialization, frame admission, or the application run loop.
pub fn run() -> Result<(), RuntimeError> {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        let clear =
            LinearRgba::new(0.02, 0.02, 0.02, 1.0).ok_or(SurfaceError::DriverUnavailable)?;
        let descriptor = SurfaceDescriptor::new(
            "Alpine Studio",
            f64::from(WINDOW_WIDTH),
            f64::from(WINDOW_HEIGHT),
            2.0,
        )?;
        let viewport =
            Size::new(WINDOW_WIDTH, WINDOW_HEIGHT).ok_or(SurfaceError::DriverUnavailable)?;
        Application::new(StudioApp::new()?, viewport, clear, WorkerConfig::default())?
            .run(&descriptor)
    }

    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    {
        Err(RuntimeError::Surface(SurfaceError::UnsupportedPlatform))
    }
}

struct StudioApp {
    bounds: Rect,
    color: LinearRgba,
}

impl StudioApp {
    fn new() -> Result<Self, SurfaceError> {
        let origin = Point::new(40.0, 40.0).ok_or(SurfaceError::DriverUnavailable)?;
        let bounds = Rect::new(
            origin,
            Size::new(240.0, 120.0).ok_or(SurfaceError::DriverUnavailable)?,
        );
        let color =
            LinearRgba::new(0.22, 0.57, 0.92, 1.0).ok_or(SurfaceError::DriverUnavailable)?;
        Ok(Self { bounds, color })
    }

    fn scene(&self, revision: SceneRevision, viewport: Size) -> Scene {
        let mut builder = SceneBuilder::new(revision, viewport);
        builder.push(Primitive::Quad {
            bounds: self.bounds,
            color: self.color,
        });
        builder.finish()
    }
}

impl AppDelegate for StudioApp {
    type WorkerOutput = ();

    fn event(&mut self, _event: &SurfaceEvent, _context: &mut AppContext<'_, ()>) {}

    fn frame(&mut self, context: WindowContext) -> Scene {
        self.scene(context.scene_revision(), context.viewport())
    }
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

        let expected_bounds = Rect::new(
            Point::new(40.0, 40.0).ok_or(SurfaceError::DriverUnavailable)?,
            Size::new(240.0, 120.0).ok_or(SurfaceError::DriverUnavailable)?,
        );
        let expected_color =
            LinearRgba::new(0.22, 0.57, 0.92, 1.0).ok_or(SurfaceError::DriverUnavailable)?;
        assert_eq!(
            scene.primitives(),
            &[Primitive::Quad {
                bounds: expected_bounds,
                color: expected_color,
            }]
        );
        Ok(())
    }

    #[test]
    fn studio_delegate_builds_only_the_dirty_runtime_frame() -> Result<(), RuntimeError> {
        let viewport =
            Size::new(WINDOW_WIDTH, WINDOW_HEIGHT).ok_or(SurfaceError::DriverUnavailable)?;
        let clear =
            LinearRgba::new(0.02, 0.02, 0.02, 1.0).ok_or(SurfaceError::DriverUnavailable)?;
        let mut application =
            Application::new(StudioApp::new()?, viewport, clear, WorkerConfig::default())?;
        let frame = application
            .frame_if_dirty()
            .ok_or(SurfaceError::DriverUnavailable)?;
        assert_eq!(frame.scene(), &initial_scene()?);
        assert!(
            application
                .dispatch(&SurfaceEvent::Wake {
                    timestamp: alpine_platform_macos::EventTimestamp::new(1),
                })
                .is_none()
        );
        Ok(())
    }

    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    #[test]
    fn run_rejects_an_unsupported_host() {
        assert!(matches!(
            run(),
            Err(RuntimeError::Surface(SurfaceError::UnsupportedPlatform))
        ));
    }
}
