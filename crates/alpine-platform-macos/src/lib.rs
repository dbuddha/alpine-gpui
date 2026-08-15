//! Safe Alpine-owned contract for one native macOS Metal presentation surface.
//!
//! The public API contains no `AppKit`, `QuartzCore`, or Metal handles. On Apple
//! Silicon macOS, [`NativeSurface`] owns the native object graph and tears it
//! down in callback-safe order. Other targets retain the same descriptor and
//! error contract but reject native construction.

use core::{error::Error, fmt};
use std::sync::{
    Arc,
    atomic::{AtomicU8, AtomicU64, Ordering},
};

use alpine_core::LinearRgba;
use alpine_metal::{InitializationError, RenderError};
use alpine_platform::{PresentationRevision, TransitionError};
use alpine_scene::Scene;

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod native;
#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
mod unsupported;

/// Non-shipping native lifecycle validation entry points.
#[cfg(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64"))]
#[doc(hidden)]
pub mod native_validation {
    use std::time::Duration;

    use crate::{NativeSurface, SurfaceDescriptor, SurfaceError, native};

    /// Creates one real surface while bypassing only the hosted device baseline.
    ///
    /// # Errors
    ///
    /// Returns the same structured construction errors as the production path.
    pub fn new_surface(descriptor: &SurfaceDescriptor) -> Result<NativeSurface, SurfaceError> {
        native::NativeSurface::new_for_validation(descriptor)
            .map(NativeSurface::from_implementation)
    }

    /// Runs the real AppKit event loop until one frame terminates or timeout.
    pub fn run_until_frame_terminal(surface: &NativeSurface, timeout: Duration) {
        surface.implementation.run_until_frame_terminal(timeout);
    }

    /// Installs one deterministic asynchronous driver failure for contract tests.
    pub fn inject_driver_error(surface: &NativeSurface, error: SurfaceError) {
        surface.implementation.inject_driver_error(error);
    }

    /// Injects every initialization-stage failure and verifies complete rollback.
    ///
    /// # Errors
    ///
    /// Returns a structured native surface error if the validation fixture
    /// cannot construct its successful control surface.
    pub fn validate_initialization_rollback() -> Result<(), SurfaceError> {
        native::validate_initialization_rollback()
    }
}

/// Maximum drawable dimension guaranteed by Alpine's Metal 3 baseline.
pub const MAX_DRAWABLE_DIMENSION: u32 = 16_384;

const SURFACE_LIVE: u8 = 1;
const SURFACE_CLOSING: u8 = 2;
const SURFACE_CLOSED: u8 = 3;

/// Validated logical and physical dimensions for one native surface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceExtent {
    logical_width: f64,
    logical_height: f64,
    scale: f64,
    physical_width: u32,
    physical_height: u32,
}

impl SurfaceExtent {
    /// Validates logical dimensions and their rounded physical extent.
    ///
    /// # Errors
    ///
    /// Returns a structured error for non-finite, non-positive, rounded-zero,
    /// or larger-than-baseline dimensions.
    pub fn new(logical_width: f64, logical_height: f64, scale: f64) -> Result<Self, SurfaceError> {
        validate_positive_finite(logical_width, InvalidDimension::Width)?;
        validate_positive_finite(logical_height, InvalidDimension::Height)?;
        validate_positive_finite(scale, InvalidDimension::Scale)?;

        let physical_width = physical_dimension(logical_width, scale, InvalidDimension::Width)?;
        let physical_height = physical_dimension(logical_height, scale, InvalidDimension::Height)?;

        Ok(Self {
            logical_width,
            logical_height,
            scale,
            physical_width,
            physical_height,
        })
    }

    /// Returns the logical width in `AppKit` points.
    #[must_use]
    pub const fn logical_width(self) -> f64 {
        self.logical_width
    }

    /// Returns the logical height in `AppKit` points.
    #[must_use]
    pub const fn logical_height(self) -> f64 {
        self.logical_height
    }

    /// Returns physical pixels per logical point.
    #[must_use]
    pub const fn scale(self) -> f64 {
        self.scale
    }

    /// Returns the rounded physical width in pixels.
    #[must_use]
    pub const fn physical_width(self) -> u32 {
        self.physical_width
    }

    /// Returns the rounded physical height in pixels.
    #[must_use]
    pub const fn physical_height(self) -> u32 {
        self.physical_height
    }
}

/// Validated creation parameters for one native surface.
#[derive(Clone, Debug, PartialEq)]
pub struct SurfaceDescriptor {
    title: String,
    extent: SurfaceExtent,
}

impl SurfaceDescriptor {
    /// Creates a descriptor after validating all dimensions.
    ///
    /// # Errors
    ///
    /// Returns the same structured dimension errors as [`SurfaceExtent::new`].
    pub fn new(
        title: impl Into<String>,
        logical_width: f64,
        logical_height: f64,
        scale: f64,
    ) -> Result<Self, SurfaceError> {
        Ok(Self {
            title: title.into(),
            extent: SurfaceExtent::new(logical_width, logical_height, scale)?,
        })
    }

    /// Returns the native window title.
    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Returns the validated logical and physical extent.
    #[must_use]
    pub const fn extent(&self) -> SurfaceExtent {
        self.extent
    }
}

/// Dimension rejected during surface validation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvalidDimension {
    /// Logical width.
    Width,
    /// Logical height.
    Height,
    /// Backing scale.
    Scale,
}

/// Stage at which native surface initialization failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SurfaceStage {
    /// `AppKit` main-thread admission.
    MainThread,
    /// Metal device acquisition.
    Device,
    /// Direct Metal backend initialization on the layer's device.
    Renderer,
    /// `AppKit` window creation.
    Window,
    /// `AppKit` content-view creation.
    View,
    /// Metal layer creation and configuration.
    Layer,
    /// Layer-bound display-link creation.
    DisplayLink,
    /// Display-link registration with the main run loop.
    RunLoop,
}

/// Structured failure from descriptor validation or native construction.
#[derive(Clone, Debug, PartialEq)]
pub enum SurfaceError {
    /// A dimension was non-finite or not strictly positive.
    InvalidDimension {
        /// Rejected dimension.
        dimension: InvalidDimension,
        /// Rejected numeric value.
        value: f64,
    },
    /// Rounded physical pixels were zero or exceeded the Metal 3 baseline.
    PhysicalDimensionOutOfRange {
        /// Logical dimension that produced the invalid physical value.
        dimension: InvalidDimension,
        /// Rounded physical value when representable as a finite float.
        value: f64,
    },
    /// The host cannot construct Alpine's first native platform surface.
    UnsupportedPlatform,
    /// A native initialization stage could not complete.
    NativeUnavailable {
        /// Stage that rejected initialization.
        stage: SurfaceStage,
    },
    /// The shared Direct Metal backend could not initialize.
    RendererInitialization(InitializationError),
    /// An immutable callback-drawable frame failed.
    Render(RenderError),
    /// The native owner rejected a portable lifecycle transition.
    Presentation(TransitionError),
    /// The synchronized callback owner is no longer usable.
    DriverUnavailable,
    /// Direct presentation was called but no presented timestamp appeared.
    PresentationNotObserved {
        /// Display-link callbacks spent awaiting correlation.
        callbacks: u16,
    },
    /// Core Animation repeatedly completed drawables without presenting them.
    PresentationsSkipped {
        /// Consecutive dropped presentation attempts.
        attempts: u16,
    },
}

impl fmt::Display for SurfaceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDimension { dimension, value } => {
                write!(formatter, "invalid {dimension:?} dimension {value}")
            }
            Self::PhysicalDimensionOutOfRange { dimension, value } => write!(
                formatter,
                "physical {dimension:?} dimension {value} is outside 1..={MAX_DRAWABLE_DIMENSION}"
            ),
            Self::UnsupportedPlatform => formatter
                .write_str("native Alpine presentation requires Apple Silicon macOS 15 or newer"),
            Self::NativeUnavailable { stage } => {
                write!(formatter, "native surface unavailable at {stage:?} stage")
            }
            Self::RendererInitialization(error) => {
                write!(formatter, "native renderer initialization failed: {error}")
            }
            Self::Render(error) => write!(formatter, "native presentation failed: {error}"),
            Self::Presentation(error) => {
                write!(formatter, "native presentation state failed: {error}")
            }
            Self::DriverUnavailable => {
                formatter.write_str("native presentation driver unavailable")
            }
            Self::PresentationNotObserved { callbacks } => write!(
                formatter,
                "native presentation was not observed after {callbacks} display-link callbacks"
            ),
            Self::PresentationsSkipped { attempts } => write!(
                formatter,
                "Core Animation skipped {attempts} consecutive presentation attempts"
            ),
        }
    }
}

impl Error for SurfaceError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::RendererInitialization(error) => Some(error),
            Self::Render(error) => Some(error),
            Self::Presentation(error) => Some(error),
            Self::InvalidDimension { .. }
            | Self::PhysicalDimensionOutOfRange { .. }
            | Self::UnsupportedPlatform
            | Self::NativeUnavailable { .. }
            | Self::DriverUnavailable
            | Self::PresentationNotObserved { .. }
            | Self::PresentationsSkipped { .. } => None,
        }
    }
}

impl From<InitializationError> for SurfaceError {
    fn from(error: InitializationError) -> Self {
        Self::RendererInitialization(error)
    }
}

impl From<RenderError> for SurfaceError {
    fn from(error: RenderError) -> Self {
        Self::Render(error)
    }
}

impl From<TransitionError> for SurfaceError {
    fn from(error: TransitionError) -> Self {
        Self::Presentation(error)
    }
}

/// Read-only native configuration and pacing evidence.
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent native API policies must remain separately observable"
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SurfaceSnapshot {
    physical_width: u32,
    physical_height: u32,
    framebuffer_only: bool,
    display_sync_enabled: bool,
    allows_next_drawable_timeout: bool,
    maximum_drawable_count: u8,
    regular_activation_policy: bool,
    display_link_paused: bool,
    visible: bool,
    callback_count: u64,
    submission_count: u64,
    direct_present_count: u64,
    installed_presented_handler_count: u64,
    presented_count: u64,
    last_presented_time_bits: u64,
    skipped_count: u64,
    failed_count: u64,
}

impl SurfaceSnapshot {
    /// Returns the configured drawable width in pixels.
    #[must_use]
    pub const fn physical_width(self) -> u32 {
        self.physical_width
    }

    /// Returns the configured drawable height in pixels.
    #[must_use]
    pub const fn physical_height(self) -> u32 {
        self.physical_height
    }

    /// Returns whether textures are restricted to framebuffer use.
    #[must_use]
    pub const fn framebuffer_only(self) -> bool {
        self.framebuffer_only
    }

    /// Returns whether display synchronization is enabled.
    #[must_use]
    pub const fn display_sync_enabled(self) -> bool {
        self.display_sync_enabled
    }

    /// Returns whether drawable acquisition may time out.
    #[must_use]
    pub const fn allows_next_drawable_timeout(self) -> bool {
        self.allows_next_drawable_timeout
    }

    /// Returns the layer's bounded drawable queue size.
    #[must_use]
    pub const fn maximum_drawable_count(self) -> u8 {
        self.maximum_drawable_count
    }

    /// Returns whether the standalone application uses the regular `AppKit` policy.
    #[must_use]
    pub const fn regular_activation_policy(self) -> bool {
        self.regular_activation_policy
    }

    /// Returns whether the layer-bound display link is paused.
    #[must_use]
    pub const fn display_link_paused(self) -> bool {
        self.display_link_paused
    }

    /// Returns whether `AppKit` currently reports the window as visible.
    #[must_use]
    pub const fn visible(self) -> bool {
        self.visible
    }

    /// Returns callbacks admitted by the live owner generation.
    #[must_use]
    pub const fn callback_count(self) -> u64 {
        self.callback_count
    }

    /// Returns callback frames that committed one command buffer.
    #[must_use]
    pub const fn submission_count(self) -> u64 {
        self.submission_count
    }

    /// Returns direct callback-drawable presentation calls.
    #[must_use]
    pub const fn direct_present_count(self) -> u64 {
        self.direct_present_count
    }

    /// Returns callback drawables with a registered presented handler.
    #[must_use]
    pub const fn installed_presented_handler_count(self) -> u64 {
        self.installed_presented_handler_count
    }

    /// Returns drawables correlated with a nonzero presented timestamp.
    #[must_use]
    pub const fn presented_count(self) -> u64 {
        self.presented_count
    }

    /// Returns the raw nonzero `f64` bits from the latest observed presentation time.
    #[must_use]
    pub const fn last_presented_time_bits(self) -> u64 {
        self.last_presented_time_bits
    }

    /// Returns drawables whose presented handler reported a dropped frame.
    #[must_use]
    pub const fn skipped_count(self) -> u64 {
        self.skipped_count
    }

    /// Returns callback frame attempts with classified terminal failure.
    #[must_use]
    pub const fn failed_count(self) -> u64 {
        self.failed_count
    }
}

/// Observable lifecycle state that contains no native handle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SurfaceLifecycle {
    /// The native owner has not begun teardown.
    Live,
    /// Callback admission is revoked while native owners are being released.
    Closing,
    /// Native teardown completed.
    Closed,
}

/// Cloneable handle-free evidence retained independently of a native surface.
#[derive(Clone)]
pub struct SurfaceObserver {
    lifecycle: Arc<AtomicU8>,
    callback_count: Arc<AtomicU64>,
}

impl SurfaceObserver {
    pub(crate) fn new(lifecycle: Arc<AtomicU8>, callback_count: Arc<AtomicU64>) -> Self {
        Self {
            lifecycle,
            callback_count,
        }
    }

    /// Returns whether the corresponding native owner is live or closed.
    #[must_use]
    pub fn lifecycle(&self) -> SurfaceLifecycle {
        match self.lifecycle.load(Ordering::Acquire) {
            SURFACE_LIVE => SurfaceLifecycle::Live,
            SURFACE_CLOSING => SurfaceLifecycle::Closing,
            _ => SurfaceLifecycle::Closed,
        }
    }

    /// Returns callbacks admitted while the owner generation was live.
    #[must_use]
    pub fn callback_count(&self) -> u64 {
        self.callback_count.load(Ordering::Acquire)
    }
}

/// Safe owner of one `AppKit` window, Metal layer, and paused display link.
pub struct NativeSurface {
    implementation: implementation::NativeSurface,
}

impl NativeSurface {
    /// Creates the complete native object graph or returns a structured error.
    ///
    /// # Errors
    ///
    /// Returns [`SurfaceError::UnsupportedPlatform`] outside Apple Silicon
    /// macOS. On macOS, creation requires the process main thread and a system
    /// Metal device.
    pub fn new(descriptor: &SurfaceDescriptor) -> Result<Self, SurfaceError> {
        implementation::NativeSurface::new(descriptor).map(Self::from_implementation)
    }

    fn from_implementation(implementation: implementation::NativeSurface) -> Self {
        Self { implementation }
    }

    /// Orders the initialized native window to the front.
    ///
    /// # Errors
    ///
    /// Returns a structured lifecycle or driver error if the surface cannot
    /// enter visible demand-driven presentation.
    pub fn show(&self) -> Result<(), SurfaceError> {
        self.implementation.show()
    }

    /// Replaces pending immutable work and wakes pacing only when eligible.
    ///
    /// # Errors
    ///
    /// Returns a structured lifecycle error if the surface no longer admits
    /// work or its synchronized native driver is unavailable.
    pub fn request_frame(
        &self,
        scene: Scene,
        clear: LinearRgba,
    ) -> Result<PresentationRevision, SurfaceError> {
        self.implementation.request_frame(scene, clear)
    }

    /// Removes the latest asynchronous callback failure, if one exists.
    ///
    /// # Errors
    ///
    /// Returns [`SurfaceError::DriverUnavailable`] if synchronized callback
    /// state cannot be inspected.
    pub fn take_error(&self) -> Result<Option<SurfaceError>, SurfaceError> {
        self.implementation.take_error()
    }

    /// Returns current layer configuration and pacing evidence.
    #[must_use]
    pub fn snapshot(&self) -> SurfaceSnapshot {
        self.implementation.snapshot()
    }

    /// Returns handle-free lifecycle evidence that remains valid after close.
    #[must_use]
    pub fn observer(&self) -> SurfaceObserver {
        self.implementation.observer()
    }

    /// Consumes and deterministically tears down the native surface.
    pub fn close(self) {}
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
use native as implementation;
#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
use unsupported as implementation;

fn validate_positive_finite(value: f64, dimension: InvalidDimension) -> Result<(), SurfaceError> {
    if value.is_finite() && value > 0.0 {
        Ok(())
    } else {
        Err(SurfaceError::InvalidDimension { dimension, value })
    }
}

fn physical_dimension(
    logical: f64,
    scale: f64,
    dimension: InvalidDimension,
) -> Result<u32, SurfaceError> {
    let value = (logical * scale).round();
    if value.is_finite() && value >= 1.0 && value <= f64::from(MAX_DRAWABLE_DIMENSION) {
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "the finite rounded value is checked inside the complete u32 output range"
        )]
        Ok(value as u32)
    } else {
        Err(SurfaceError::PhysicalDimensionOutOfRange { dimension, value })
    }
}

fn new_observer_state() -> (Arc<AtomicU8>, Arc<AtomicU64>) {
    (
        Arc::new(AtomicU8::new(SURFACE_LIVE)),
        Arc::new(AtomicU64::new(0)),
    )
}

fn begin_close_observer_state(lifecycle: &AtomicU8) {
    lifecycle.store(SURFACE_CLOSING, Ordering::Release);
}

fn finish_close_observer_state(lifecycle: &AtomicU8) {
    lifecycle.store(SURFACE_CLOSED, Ordering::Release);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extent_rounds_each_physical_dimension() -> Result<(), SurfaceError> {
        let extent = SurfaceExtent::new(100.25, 50.75, 2.0)?;

        assert_eq!(extent.logical_width().to_bits(), 100.25_f64.to_bits());
        assert_eq!(extent.logical_height().to_bits(), 50.75_f64.to_bits());
        assert_eq!(extent.scale().to_bits(), 2.0_f64.to_bits());
        assert_eq!(extent.physical_width(), 201);
        assert_eq!(extent.physical_height(), 102);
        Ok(())
    }

    #[test]
    fn descriptor_preserves_title_and_extent() -> Result<(), SurfaceError> {
        let descriptor = SurfaceDescriptor::new("Alpine", 640.0, 480.0, 2.0)?;

        assert_eq!(descriptor.title(), "Alpine");
        assert_eq!(descriptor.extent().physical_width(), 1_280);
        assert_eq!(descriptor.extent().physical_height(), 960);
        Ok(())
    }

    #[test]
    fn rejects_each_invalid_logical_input() {
        for (width, height, scale, dimension) in [
            (0.0, 1.0, 1.0, InvalidDimension::Width),
            (-1.0, 1.0, 1.0, InvalidDimension::Width),
            (f64::NAN, 1.0, 1.0, InvalidDimension::Width),
            (1.0, 0.0, 1.0, InvalidDimension::Height),
            (1.0, f64::INFINITY, 1.0, InvalidDimension::Height),
            (1.0, 1.0, 0.0, InvalidDimension::Scale),
            (1.0, 1.0, f64::NEG_INFINITY, InvalidDimension::Scale),
        ] {
            assert!(matches!(
                SurfaceExtent::new(width, height, scale),
                Err(SurfaceError::InvalidDimension {
                    dimension: observed,
                    ..
                }) if observed == dimension
            ));
        }
    }

    #[test]
    fn rejects_rounded_zero_and_oversized_physical_dimensions() {
        assert_eq!(
            SurfaceExtent::new(0.1, 1.0, 1.0),
            Err(SurfaceError::PhysicalDimensionOutOfRange {
                dimension: InvalidDimension::Width,
                value: 0.0,
            })
        );
        assert_eq!(
            SurfaceExtent::new(f64::from(MAX_DRAWABLE_DIMENSION) + 1.0, 1.0, 1.0),
            Err(SurfaceError::PhysicalDimensionOutOfRange {
                dimension: InvalidDimension::Width,
                value: f64::from(MAX_DRAWABLE_DIMENSION) + 1.0,
            })
        );
        assert!(matches!(
            SurfaceExtent::new(1.0, f64::MAX, 2.0),
            Err(SurfaceError::PhysicalDimensionOutOfRange {
                dimension: InvalidDimension::Height,
                value,
            }) if value.is_infinite()
        ));
    }

    #[test]
    fn errors_are_stable_and_descriptive() {
        assert_eq!(
            SurfaceError::InvalidDimension {
                dimension: InvalidDimension::Scale,
                value: -1.0,
            }
            .to_string(),
            "invalid Scale dimension -1"
        );
        assert_eq!(
            SurfaceError::PhysicalDimensionOutOfRange {
                dimension: InvalidDimension::Height,
                value: 16_385.0,
            }
            .to_string(),
            "physical Height dimension 16385 is outside 1..=16384"
        );
        assert_eq!(
            SurfaceError::UnsupportedPlatform.to_string(),
            "native Alpine presentation requires Apple Silicon macOS 15 or newer"
        );
        assert_eq!(
            SurfaceError::NativeUnavailable {
                stage: SurfaceStage::DisplayLink,
            }
            .to_string(),
            "native surface unavailable at DisplayLink stage"
        );

        let initialization: SurfaceError = InitializationError::DeviceUnavailable.into();
        let render: SurfaceError =
            RenderError::Validation(alpine_metal::OffscreenError::ZeroPixelExtent).into();
        let mut state = alpine_platform::PresentationState::new();
        let presentation = state
            .apply(alpine_platform::PresentationAction::Prepare)
            .map_err(SurfaceError::from);
        assert_eq!(
            presentation.as_ref().map_err(ToString::to_string),
            Err("native presentation state failed: presentation action Prepare rejected in Running/Idle: ActionDisabled".to_owned())
        );
        assert!(
            presentation
                .as_ref()
                .err()
                .and_then(|error| std::error::Error::source(error))
                .is_some()
        );
        let cases = [
            (
                initialization,
                "native renderer initialization failed: Metal returned no default device",
                true,
            ),
            (
                render,
                "native presentation failed: offscreen validation failed: offscreen target must be non-empty",
                true,
            ),
            (
                SurfaceError::DriverUnavailable,
                "native presentation driver unavailable",
                false,
            ),
            (
                SurfaceError::PresentationNotObserved { callbacks: 17 },
                "native presentation was not observed after 17 display-link callbacks",
                false,
            ),
            (
                SurfaceError::PresentationsSkipped { attempts: 19 },
                "Core Animation skipped 19 consecutive presentation attempts",
                false,
            ),
        ];
        for (error, message, has_source) in cases {
            assert_eq!(error.to_string(), message);
            assert_eq!(std::error::Error::source(&error).is_some(), has_source);
        }
    }

    #[test]
    fn snapshot_accessors_preserve_discriminating_values() {
        let snapshot = SurfaceSnapshot {
            physical_width: 17,
            physical_height: 19,
            framebuffer_only: false,
            display_sync_enabled: false,
            allows_next_drawable_timeout: true,
            maximum_drawable_count: 2,
            regular_activation_policy: false,
            display_link_paused: false,
            visible: true,
            callback_count: 23,
            submission_count: 29,
            direct_present_count: 31,
            installed_presented_handler_count: 33,
            presented_count: 37,
            last_presented_time_bits: 39,
            skipped_count: 41,
            failed_count: 43,
        };
        let inverse = SurfaceSnapshot {
            physical_width: 29,
            physical_height: 31,
            framebuffer_only: true,
            display_sync_enabled: true,
            allows_next_drawable_timeout: false,
            maximum_drawable_count: 3,
            regular_activation_policy: true,
            display_link_paused: true,
            visible: false,
            callback_count: 37,
            submission_count: 43,
            direct_present_count: 47,
            installed_presented_handler_count: 49,
            presented_count: 53,
            last_presented_time_bits: 57,
            skipped_count: 59,
            failed_count: 61,
        };

        assert_eq!(snapshot.physical_width(), 17);
        assert_eq!(snapshot.physical_height(), 19);
        assert!(!snapshot.framebuffer_only());
        assert!(!snapshot.display_sync_enabled());
        assert!(snapshot.allows_next_drawable_timeout());
        assert_eq!(snapshot.maximum_drawable_count(), 2);
        assert!(!snapshot.regular_activation_policy());
        assert!(!snapshot.display_link_paused());
        assert!(snapshot.visible());
        assert_eq!(snapshot.callback_count(), 23);
        assert_eq!(snapshot.submission_count(), 29);
        assert_eq!(snapshot.direct_present_count(), 31);
        assert_eq!(snapshot.installed_presented_handler_count(), 33);
        assert_eq!(snapshot.presented_count(), 37);
        assert_eq!(snapshot.last_presented_time_bits(), 39);
        assert_eq!(snapshot.skipped_count(), 41);
        assert_eq!(snapshot.failed_count(), 43);

        assert_eq!(inverse.physical_width(), 29);
        assert_eq!(inverse.physical_height(), 31);
        assert!(inverse.framebuffer_only());
        assert!(inverse.display_sync_enabled());
        assert!(!inverse.allows_next_drawable_timeout());
        assert_eq!(inverse.maximum_drawable_count(), 3);
        assert!(inverse.regular_activation_policy());
        assert!(inverse.display_link_paused());
        assert!(!inverse.visible());
        assert_eq!(inverse.callback_count(), 37);
        assert_eq!(inverse.submission_count(), 43);
        assert_eq!(inverse.direct_present_count(), 47);
        assert_eq!(inverse.installed_presented_handler_count(), 49);
        assert_eq!(inverse.presented_count(), 53);
        assert_eq!(inverse.last_presented_time_bits(), 57);
        assert_eq!(inverse.skipped_count(), 59);
        assert_eq!(inverse.failed_count(), 61);
    }

    #[test]
    fn observer_distinguishes_live_closed_and_callback_count() {
        let (lifecycle, callback_count) = new_observer_state();
        callback_count.store(29, Ordering::Release);
        let observer = SurfaceObserver::new(Arc::clone(&lifecycle), callback_count);

        assert_eq!(observer.lifecycle(), SurfaceLifecycle::Live);
        assert_eq!(observer.callback_count(), 29);
        begin_close_observer_state(&lifecycle);
        assert_eq!(observer.lifecycle(), SurfaceLifecycle::Closing);
        finish_close_observer_state(&lifecycle);
        assert_eq!(observer.lifecycle(), SurfaceLifecycle::Closed);
        assert_eq!(observer.callback_count(), 29);
    }

    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    #[test]
    fn unsupported_wrapper_methods_preserve_the_safe_contract() -> Result<(), SurfaceError> {
        use alpine_scene::{SceneBuilder, SceneRevision};

        let surface = NativeSurface::from_implementation(unsupported::NativeSurface);
        let viewport = alpine_core::Size::new(1.0, 1.0).ok_or(SurfaceError::DriverUnavailable)?;
        let scene = SceneBuilder::new(SceneRevision::new(1), viewport).finish();
        let clear = LinearRgba::new(0.0, 0.0, 0.0, 1.0).ok_or(SurfaceError::DriverUnavailable)?;

        assert_eq!(surface.show(), Err(SurfaceError::UnsupportedPlatform));
        assert_eq!(
            surface.request_frame(scene, clear),
            Err(SurfaceError::UnsupportedPlatform)
        );
        assert_eq!(surface.take_error(), Err(SurfaceError::UnsupportedPlatform));
        assert_eq!(surface.snapshot().physical_width(), 0);
        assert_eq!(surface.observer().lifecycle(), SurfaceLifecycle::Closed);
        surface.close();
        Ok(())
    }
}
