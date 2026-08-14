//! Backend contracts for Alpine GPUI renderers.

use std::error::Error;

use alpine_scene::Scene;

/// Hardware and backend behavior available to a renderer instance.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RendererCapabilities {
    /// Maximum supported two-dimensional texture extent.
    pub max_texture_dimension_2d: u32,
    /// Whether GPU timestamps can be collected without changing correctness.
    pub timestamps: bool,
    /// Whether the backend supports offscreen readback.
    pub offscreen_readback: bool,
}

/// Work observed for one submitted scene.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FrameReport {
    /// Backend submission sequence number.
    pub submission: u64,
    /// Number of scene primitives consumed.
    pub primitives: usize,
    /// Number of encoded draw calls.
    pub draw_calls: usize,
    /// Bytes uploaded during this submission.
    pub uploaded_bytes: usize,
}

/// A renderer with a backend-specific target and error type.
///
/// Associated types keep production dispatch monomorphized while allowing the
/// conformance suite to provide generic renderer tests.
pub trait Renderer {
    /// Backend-specific render target, such as a drawable or offscreen texture.
    type Target;
    /// Structured backend error.
    type Error: Error + Send + Sync + 'static;

    /// Reports runtime capabilities for the selected physical device.
    fn capabilities(&self) -> RendererCapabilities;

    /// Renders an immutable scene into the target.
    ///
    /// # Errors
    ///
    /// Returns the backend error when validation, allocation, encoding, or
    /// submission cannot complete. The caller owns recovery policy.
    fn render(
        &mut self,
        scene: &Scene,
        target: &mut Self::Target,
    ) -> Result<FrameReport, Self::Error>;
}

#[cfg(test)]
mod tests {
    use std::fmt;

    use alpine_scene::{SceneBuilder, SceneRevision};

    use super::{FrameReport, Renderer, RendererCapabilities};

    #[derive(Debug)]
    struct MockError;

    impl fmt::Display for MockError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("mock render failure")
        }
    }

    impl std::error::Error for MockError {}

    #[derive(Default)]
    struct MockRenderer {
        submission: u64,
    }

    impl Renderer for MockRenderer {
        type Error = MockError;
        type Target = Vec<u64>;

        fn capabilities(&self) -> RendererCapabilities {
            RendererCapabilities {
                max_texture_dimension_2d: 16_384,
                timestamps: true,
                offscreen_readback: true,
            }
        }

        fn render(
            &mut self,
            scene: &alpine_scene::Scene,
            target: &mut Self::Target,
        ) -> Result<FrameReport, Self::Error> {
            self.submission += 1;
            target.push(scene.revision().get());
            Ok(FrameReport {
                submission: self.submission,
                primitives: scene.primitives().len(),
                draw_calls: usize::from(!scene.primitives().is_empty()),
                uploaded_bytes: 0,
            })
        }
    }

    #[test]
    fn renderer_contract_exposes_capabilities_and_frame_evidence() -> Result<(), MockError> {
        let viewport = alpine_core::Size::new(64.0, 64.0).ok_or(MockError)?;
        let scene = SceneBuilder::new(SceneRevision::new(9), viewport).finish();
        let mut renderer = MockRenderer::default();
        let mut target = Vec::new();

        assert_eq!(
            renderer.capabilities(),
            RendererCapabilities {
                max_texture_dimension_2d: 16_384,
                timestamps: true,
                offscreen_readback: true,
            }
        );
        assert_eq!(
            renderer.render(&scene, &mut target)?,
            FrameReport {
                submission: 1,
                primitives: 0,
                draw_calls: 0,
                uploaded_bytes: 0,
            }
        );
        assert_eq!(target, [9]);
        Ok(())
    }
}
