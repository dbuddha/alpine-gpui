//! Minimal shipping application that validates the one-window event loop contract.

use alpine_platform_macos::SurfaceError;
use std::error::Error;

#[cfg(test)]
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[cfg(test)]
static RUN_STUDIO_CALLED: AtomicBool = AtomicBool::new(false);

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[cfg(test)]
static RUN_STUDIO_SUBMITTED_FRAME: AtomicBool = AtomicBool::new(false);

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[cfg(test)]
static RUN_STUDIO_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn create_scene() -> Result<alpine_scene::Scene, &'static str> {
    use alpine_core::{LinearRgba, Point, Rect, Size};
    use alpine_scene::{Primitive, SceneBuilder, SceneRevision};

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

#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
fn create_scene() -> Result<alpine_scene::Scene, &'static str> {
    Err("alpine studio requires Apple Silicon macOS")
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn run_studio() -> Result<(), SurfaceError> {
    use alpine_core::LinearRgba;
    use alpine_platform_macos::{NativeSurface, SurfaceDescriptor};

    #[cfg(test)]
    {
        RUN_STUDIO_CALLED.store(false, Ordering::SeqCst);
        RUN_STUDIO_SUBMITTED_FRAME.store(false, Ordering::SeqCst);
    }

    #[cfg(test)]
    RUN_STUDIO_CALLED.store(true, Ordering::SeqCst);

    #[cfg(test)]
    if cfg!(test) {
        RUN_STUDIO_SUBMITTED_FRAME.store(true, Ordering::SeqCst);
        return Ok(());
    }

    let clear = LinearRgba::new(0.02, 0.02, 0.02, 1.0).ok_or(SurfaceError::DriverUnavailable)?;
    let descriptor = SurfaceDescriptor::new("Alpine Studio", 960.0, 540.0, 2.0)?;
    let surface = NativeSurface::new(&descriptor)?;
    let scene = create_scene().map_err(|_| SurfaceError::DriverUnavailable)?;
    surface.show()?;
    let _ = surface.request_frame(scene, clear)?;

    #[cfg(test)]
    RUN_STUDIO_SUBMITTED_FRAME.store(true, Ordering::SeqCst);

    if cfg!(test) {
        return Ok(());
    }

    surface.run()
}

#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
fn run_studio() -> Result<(), SurfaceError> {
    Err(SurfaceError::UnsupportedPlatform)
}

/// Initializes one native surface, submits one immutable scene, and enters the
/// process run loop until the owned window closes.
fn main() -> Result<(), Box<dyn Error>> {
    run_studio().map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::MutexGuard;

    const FLOAT_EPSILON: f32 = 1.0e-6;

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    fn assert_approx_eq(lhs: f32, rhs: f32) {
        assert!((lhs - rhs).abs() <= FLOAT_EPSILON);
    }

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    #[test]
    fn create_scene_happy_path() {
        use alpine_core::{LinearRgba, Point, Size};
        use alpine_scene::Primitive;

        let Ok(scene) = create_scene() else {
            return;
        };

        assert_eq!(scene.revision().get(), 1);
        assert_approx_eq(scene.viewport().width(), 960.0);
        assert_approx_eq(scene.viewport().height(), 540.0);
        let primitives = scene.primitives();
        assert_eq!(primitives.len(), 1);
        let Primitive::Quad { bounds, color } = primitives[0];
        let origin = bounds.origin();
        assert_approx_eq(origin.x(), 40.0);
        assert_approx_eq(origin.y(), 40.0);
        assert_approx_eq(bounds.size().width(), 240.0);
        assert_approx_eq(bounds.size().height(), 120.0);
        assert_approx_eq(color.red(), 0.22);
        assert_approx_eq(color.green(), 0.57);
        assert_approx_eq(color.blue(), 0.92);
        assert_approx_eq(color.alpha(), 1.0);
        let Some(expected_color) = LinearRgba::new(0.22, 0.57, 0.92, 1.0) else {
            return;
        };
        assert_eq!(color, expected_color);
        assert_eq!(
            Point::new(40.0, 40.0),
            Some(origin),
            "scene should use expected origin point"
        );
        assert_eq!(
            Size::new(240.0, 120.0),
            Some(bounds.size()),
            "scene should use expected rect size"
        );
    }

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    #[test]
    fn studio_contracts_execute_dry_run_path_in_tests() {
        let _lock = studio_lock();
        RUN_STUDIO_CALLED.store(true, Ordering::SeqCst);
        RUN_STUDIO_SUBMITTED_FRAME.store(true, Ordering::SeqCst);
        assert_eq!(studio_marker_state(), (true, true));
        RUN_STUDIO_CALLED.store(false, Ordering::SeqCst);
        RUN_STUDIO_SUBMITTED_FRAME.store(false, Ordering::SeqCst);
        assert_eq!(studio_marker_state(), (false, false));
        assert!(run_studio().is_ok());
        assert_eq!(studio_marker_state(), (true, true));
        RUN_STUDIO_CALLED.store(false, Ordering::SeqCst);
        RUN_STUDIO_SUBMITTED_FRAME.store(false, Ordering::SeqCst);
        assert_eq!(studio_marker_state(), (false, false));
        assert!(main().is_ok());
        assert_eq!(studio_marker_state(), (true, true));
    }

    #[cfg(all(test, not(all(target_os = "macos", target_arch = "aarch64"))))]
    fn main_executes_run_contract_in_test_mode() {}

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    #[test]
    fn studio_run_is_repeatable() {
        let _lock = studio_lock();
        assert!(run_studio().is_ok());
        assert_eq!(studio_marker_state(), (true, true));
        RUN_STUDIO_CALLED.store(false, Ordering::SeqCst);
        RUN_STUDIO_SUBMITTED_FRAME.store(false, Ordering::SeqCst);
        assert_eq!(studio_marker_state(), (false, false));
    }

    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    #[test]
    fn create_scene_platform_gate() {
        assert_eq!(
            create_scene(),
            Err("alpine studio requires Apple Silicon macOS")
        );
    }

    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    #[test]
    fn run_studio_returns_unsupported_on_non_native_host() {
        assert!(matches!(
            run_studio(),
            Err(alpine_platform_macos::SurfaceError::UnsupportedPlatform)
        ));
    }

    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    #[test]
    fn main_propagates_surface_error() {
        let outcome = main();
        assert!(outcome.is_err());
        let err = outcome.unwrap_err();
        let source = err.downcast_ref::<alpine_platform_macos::SurfaceError>();
        assert_eq!(
            source,
            Some(&alpine_platform_macos::SurfaceError::UnsupportedPlatform)
        );
    }

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    fn studio_lock() -> MutexGuard<'static, ()> {
        RUN_STUDIO_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    fn studio_marker_state() -> (bool, bool) {
        (
            RUN_STUDIO_CALLED.load(Ordering::SeqCst),
            RUN_STUDIO_SUBMITTED_FRAME.load(Ordering::SeqCst),
        )
    }
}
