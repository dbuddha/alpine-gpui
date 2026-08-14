//! Public contract tests for safe offscreen frame planning and lifecycle behavior.

use std::error::Error;

use alpine_core::{LinearRgba, Point, Rect, Size};
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
use alpine_metal::{
    BackendState, InitializationError, MetalBackend, OffscreenTarget, RecoveryClassification,
};
use alpine_metal::{
    FrameLifecycle, FrameOutcome, LifecycleAction, OffscreenDescriptor, ValidatedFrame,
};
#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
use alpine_metal::{InitializationStage, MetalBackend};
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
use alpine_renderer::Renderer;
use alpine_scene::{Primitive, SceneBuilder, SceneRevision};

fn color(red: f32, green: f32, blue: f32, alpha: f32) -> Result<LinearRgba, &'static str> {
    LinearRgba::new(red, green, blue, alpha).ok_or("valid contract color")
}

fn size(width: f32, height: f32) -> Result<Size, &'static str> {
    Size::new(width, height).ok_or("valid contract size")
}

fn point(x: f32, y: f32) -> Result<Point, &'static str> {
    Point::new(x, y).ok_or("valid contract point")
}

#[test]
fn public_offscreen_contract_renders_reference() -> Result<(), Box<dyn Error>> {
    let mut builder = SceneBuilder::new(SceneRevision::new(19), size(2.0, 2.0)?);
    builder.push(Primitive::Quad {
        bounds: Rect::new(point(0.0, 0.0)?, size(2.0, 2.0)?),
        color: color(0.0, 1.0, 0.0, 1.0)?,
    });
    let descriptor = OffscreenDescriptor::new(4, 4, 2.0, color(0.0, 0.0, 0.0, 0.0)?)?;
    let frame = ValidatedFrame::new(&builder.finish(), descriptor)?;
    let image = frame.reference_image()?;

    assert_eq!(frame.revision(), SceneRevision::new(19));
    assert_eq!(frame.consumed_primitives(), 1);
    assert_eq!(frame.omitted_primitives(), 0);
    assert_eq!(image.width(), 4);
    assert_eq!(image.height(), 4);
    assert_eq!(image.pixel(0, 0), Some([0, 255, 0, 255]));
    assert_eq!(image.pixel(3, 3), Some([0, 255, 0, 255]));
    assert_eq!(image.bytes().len(), 64);
    Ok(())
}

#[test]
fn public_lifecycle_rejects_submission_before_encoding() {
    let mut lifecycle = FrameLifecycle::new();
    let before = lifecycle;
    assert!(lifecycle.apply(LifecycleAction::Submit).is_err());
    assert_eq!(lifecycle, before);
    assert_eq!(lifecycle.outcome(), FrameOutcome::Pending);
    assert_eq!(lifecycle.submit_count(), 0);
    assert!(lifecycle.invariants_hold());
}

#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
#[test]
fn public_backend_rejects_unsupported_targets() {
    let error = MetalBackend::new().err();
    assert_eq!(
        error.map(|failure| failure.stage()),
        Some(InitializationStage::Platform)
    );
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn public_backend_enforces_apple_silicon_baseline() -> Result<(), Box<dyn Error>> {
    match MetalBackend::new() {
        Ok(mut backend) => {
            let capabilities = backend.capabilities();
            assert!(!capabilities.name().is_empty());
            assert_ne!(capabilities.registry_id(), 0);
            assert!(capabilities.supports_metal3());
            assert!(capabilities.has_unified_memory());

            let mut builder = SceneBuilder::new(SceneRevision::new(20), size(1.0, 1.0)?);
            builder.push(Primitive::Quad {
                bounds: Rect::new(point(0.0, 0.0)?, size(1.0, 1.0)?),
                color: color(0.25, 0.5, 1.0, 0.5)?,
            });
            let scene = builder.finish();
            let descriptor = OffscreenDescriptor::new(1, 1, 1.0, color(0.0, 0.0, 0.0, 0.0)?)?;
            let expected = ValidatedFrame::new(&scene, descriptor)?.reference_image()?;
            let completed = backend.render_offscreen(&scene, descriptor)?;
            assert_eq!(completed.report().submission, 1);
            assert_eq!(completed.report().draw_calls, 1);
            assert_eq!(completed.report().omitted_primitives, 0);
            assert!(completed.report().allocated_bytes > 0);
            assert_eq!(
                completed.report().allocated_bytes,
                completed.report().retained_bytes
            );
            assert!(completed.report().readback_bytes >= completed.image().bytes().len());
            for (actual, expected) in completed.image().bytes().iter().zip(expected.bytes()) {
                assert!(actual.abs_diff(*expected) <= 1);
            }

            let mut target = OffscreenTarget::new(descriptor);
            let report = backend.render(&scene, &mut target)?;
            assert_eq!(report.submission, 2);
            assert!(target.image().is_some());
            assert_eq!(target.descriptor(), descriptor);

            let cancellation = backend.cancel_offscreen(&scene, descriptor)?;
            assert_eq!(cancellation.generation().get(), 1);
            assert_eq!(cancellation.primitives(), 1);
            assert_eq!(cancellation.uploaded_bytes_avoided(), 32);
            let accounting = backend.accounting();
            assert_eq!(accounting.completed_frames(), 2);
            assert_eq!(accounting.cancelled_frames(), 1);
            assert_eq!(accounting.submitted_frames(), 2);
            assert_eq!(accounting.current_retained_bytes(), 0);
            assert!(accounting.invariants_hold());

            backend.shutdown();
            assert_eq!(backend.accounting().state(), BackendState::Stopped);
            let stopped = backend
                .render_offscreen(&scene, descriptor)
                .err()
                .ok_or("stopped backend must reject")?;
            assert_eq!(stopped.recovery(), RecoveryClassification::Stopped);
            assert!(backend.accounting().invariants_hold());
        }
        Err(InitializationError::UnsupportedDevice { .. }) => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}
