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
    fn render(
        &mut self,
        scene: &Scene,
        target: &mut Self::Target,
    ) -> Result<FrameReport, Self::Error>;
}
