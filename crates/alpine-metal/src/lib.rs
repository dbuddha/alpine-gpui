//! Alpine-owned Direct Metal backend and its safe portable frame-planning core.
//!
//! The crate validates and lowers immutable scenes, owns an initialized native
//! Metal device and pipeline on Apple Silicon macOS, models the single-frame
//! lifecycle, and renders an independent CPU reference image. Native objects
//! remain behind safe Alpine-owned types.

mod frame;
mod initialization;
mod lifecycle;
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod native;
mod oracle;
#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
mod unsupported;

#[cfg(kani)]
mod proofs;

pub use frame::{
    BGRA_BYTES_PER_PIXEL, LoweredQuad, OffscreenDescriptor, OffscreenError, READBACK_ROW_ALIGNMENT,
    ReadbackLayout, ValidatedFrame,
};
pub use initialization::{
    InitializationError, InitializationStage, MetalBackend, MetalCapabilities, NativeFailure,
};
pub use lifecycle::{
    FrameLifecycle, FrameOutcome, FrameState, LifecycleAction, RendererState, ResourceState,
    TransitionError,
};
pub use oracle::Bgra8Image;
