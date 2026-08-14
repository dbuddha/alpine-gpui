//! Alpine-owned Direct Metal backend and its safe portable frame-planning core.
//!
//! The current crate implements no native API calls. It validates and lowers an
//! immutable scene, models the single-frame lifecycle, and renders an independent
//! CPU reference image. Native Metal ownership will remain behind these safe types.

mod frame;
mod lifecycle;
mod oracle;

#[cfg(kani)]
mod proofs;

pub use frame::{
    BGRA_BYTES_PER_PIXEL, LoweredQuad, OffscreenDescriptor, OffscreenError, READBACK_ROW_ALIGNMENT,
    ReadbackLayout, ValidatedFrame,
};
pub use lifecycle::{
    FrameLifecycle, FrameOutcome, FrameState, LifecycleAction, RendererState, ResourceState,
    TransitionError,
};
pub use oracle::Bgra8Image;
