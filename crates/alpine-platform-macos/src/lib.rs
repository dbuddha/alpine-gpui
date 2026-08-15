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
use alpine_metal::{InitializationError, RecoveryClassification, RenderError};
use alpine_platform::{
    AttemptEvidence, PendingCancellationEvidence, PresentationOutcome, PresentationRevision,
    TransitionError,
};
use alpine_scene::Scene;

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod native;
#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
mod unsupported;

/// Non-shipping native lifecycle validation entry points.
#[cfg(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64"))]
#[doc(hidden)]
pub mod native_validation {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        time::Duration,
    };

    use crate::{NativeSurface, SurfaceDescriptor, SurfaceError, native};

    /// Evidence that bounds a production event-loop validation run.
    #[derive(Debug)]
    pub struct RunTimeoutEvidence {
        expired: Arc<AtomicBool>,
        cancelled: Arc<AtomicBool>,
    }

    impl RunTimeoutEvidence {
        /// Returns whether the guard had to stop the application run loop.
        #[must_use]
        pub fn expired(&self) -> bool {
            self.expired.load(Ordering::Acquire)
        }

        /// Disarms the guard after the production run loop exits normally.
        pub fn cancel(&self) {
            self.cancelled.store(true, Ordering::Release);
        }
    }

    /// Validation-only exact ownership and teardown counts for one surface.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct NativeOwnerEvidence {
        acquired: [u64; 9],
        released: [u64; 9],
        active: [u64; 9],
        run_loop_registrations: u64,
        link_invalidations: u64,
        delegate_revocations: u64,
        window_closes: u64,
        release_order_violations: u64,
    }

    impl NativeOwnerEvidence {
        #[allow(
            clippy::too_many_arguments,
            reason = "validation evidence preserves each independent cleanup counter"
        )]
        pub(crate) const fn new(
            acquired: [u64; 9],
            released: [u64; 9],
            active: [u64; 9],
            run_loop_registrations: u64,
            link_invalidations: u64,
            delegate_revocations: u64,
            window_closes: u64,
            release_order_violations: u64,
        ) -> Self {
            Self {
                acquired,
                released,
                active,
                run_loop_registrations,
                link_invalidations,
                delegate_revocations,
                window_closes,
                release_order_violations,
            }
        }

        /// Returns per-kind acquisitions in application-to-display-link order.
        #[must_use]
        pub const fn acquired(self) -> [u64; 9] {
            self.acquired
        }

        /// Returns per-kind releases in application-to-display-link order.
        #[must_use]
        pub const fn released(self) -> [u64; 9] {
            self.released
        }

        /// Returns per-kind owners remaining after close.
        #[must_use]
        pub const fn active(self) -> [u64; 9] {
            self.active
        }

        /// Returns main-run-loop registrations performed by the owner.
        #[must_use]
        pub const fn run_loop_registrations(self) -> u64 {
            self.run_loop_registrations
        }

        /// Returns display-link invalidations performed before release.
        #[must_use]
        pub const fn link_invalidations(self) -> u64 {
            self.link_invalidations
        }

        /// Returns native delegate revocations performed before release.
        #[must_use]
        pub const fn delegate_revocations(self) -> u64 {
            self.delegate_revocations
        }

        /// Returns window-close operations performed before release.
        #[must_use]
        pub const fn window_closes(self) -> u64 {
            self.window_closes
        }

        /// Returns owner releases observed before required cleanup.
        #[must_use]
        pub const fn release_order_violations(self) -> u64 {
            self.release_order_violations
        }
    }

    /// Creates one real surface while bypassing only the hosted device baseline.
    ///
    /// # Errors
    ///
    /// Returns the same structured construction errors as the production path.
    pub fn new_surface(descriptor: &SurfaceDescriptor) -> Result<NativeSurface, SurfaceError> {
        native::NativeSurface::new_for_validation(descriptor)
            .map(NativeSurface::from_implementation)
    }

    /// Creates one real surface whose first committed command deterministically
    /// reports native device loss.
    ///
    /// # Errors
    ///
    /// Returns the same structured construction errors as the validation path.
    pub fn new_surface_with_device_loss(
        descriptor: &SurfaceDescriptor,
    ) -> Result<NativeSurface, SurfaceError> {
        native::NativeSurface::new_for_validation_device_loss(descriptor)
            .map(NativeSurface::from_implementation)
    }

    /// Runs the real `AppKit` event loop until one frame terminates or timeout.
    pub fn run_until_frame_terminal(surface: &NativeSurface, timeout: Duration) {
        surface.implementation.run_until_frame_terminal(timeout);
    }

    /// Arms a bounded guard around a subsequent production `run` call.
    ///
    /// The guard is validation-only. Expiration wakes and stops AppKit without
    /// changing surface lifecycle, causing production `run` to fail closed.
    #[must_use]
    pub fn arm_run_timeout(surface: &NativeSurface, timeout: Duration) -> RunTimeoutEvidence {
        let expired = Arc::new(AtomicBool::new(false));
        let cancelled = Arc::new(AtomicBool::new(false));
        surface.implementation.arm_run_timeout(
            timeout,
            Arc::clone(&expired),
            Arc::clone(&cancelled),
        );
        RunTimeoutEvidence { expired, cancelled }
    }

    /// Installs one deterministic asynchronous driver failure for contract tests.
    pub fn inject_driver_error(surface: &NativeSurface, error: SurfaceError) {
        surface.implementation.inject_driver_error(error);
    }

    /// Schedules one deterministic presented-handler observation immediately
    /// after the next callback drawable commits and receives direct present.
    ///
    /// An optional display identity change advances the native surface epoch
    /// at that exact post-commit boundary.
    ///
    /// # Errors
    ///
    /// Returns a driver error for an invalid time or unavailable callback owner.
    pub fn inject_post_commit_observation(
        surface: &NativeSurface,
        display_identity: Option<usize>,
        presented_time: f64,
    ) -> Result<(), SurfaceError> {
        surface
            .implementation
            .inject_post_commit_observation(display_identity, presented_time)
    }

    /// Revokes the native owner generation at the next post-commit boundary.
    pub fn inject_post_commit_close(surface: &NativeSurface) {
        surface.implementation.inject_post_commit_close();
    }

    /// Exercises the production callback-admission guard after close begins.
    pub fn inject_late_callback(surface: &NativeSurface) {
        surface.implementation.inject_late_callback();
    }

    /// Exercises the production native-configuration callback guard.
    #[must_use]
    pub fn inject_configuration_callback(surface: &NativeSurface) -> bool {
        surface.implementation.inject_configuration_callback()
    }

    /// Applies one deterministic native size, scale, display, and visibility event.
    ///
    /// # Errors
    ///
    /// Returns the same geometry or synchronized-driver errors as a real
    /// `AppKit` notification translated through the native owner.
    pub fn inject_surface_configuration(
        surface: &NativeSurface,
        logical_width: f64,
        logical_height: f64,
        scale: f64,
        display_identity: usize,
        visible: bool,
    ) -> Result<(), SurfaceError> {
        surface.implementation.inject_surface_configuration(
            logical_width,
            logical_height,
            scale,
            display_identity,
            visible,
        )
    }

    /// Resizes the real `AppKit` content area so its delegate must synchronize it.
    pub fn resize_content(surface: &NativeSurface, logical_width: f64, logical_height: f64) {
        surface
            .implementation
            .resize_content(logical_width, logical_height);
    }

    /// Closes the real `AppKit` window through its delegate lifecycle.
    pub fn close_window(surface: &NativeSurface) {
        surface.implementation.close_window();
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

    /// Closes one validation surface and returns exact post-drop owner evidence.
    ///
    /// # Errors
    ///
    /// Returns [`SurfaceError::DriverUnavailable`] if validation ownership
    /// instrumentation is unexpectedly absent.
    pub fn close_with_owner_evidence(
        surface: NativeSurface,
    ) -> Result<NativeOwnerEvidence, SurfaceError> {
        surface.implementation.close_with_owner_evidence()
    }

    #[cfg(test)]
    mod tests {
        use super::NativeOwnerEvidence;

        #[test]
        fn owner_evidence_accessors_preserve_each_independent_counter() {
            let evidence = NativeOwnerEvidence::new(
                [2, 3, 5, 7, 11, 13, 17, 19, 23],
                [29, 31, 37, 41, 43, 47, 53, 59, 61],
                [67, 71, 73, 79, 83, 89, 97, 101, 103],
                107,
                109,
                113,
                127,
                131,
            );

            assert_eq!(evidence.acquired(), [2, 3, 5, 7, 11, 13, 17, 19, 23]);
            assert_eq!(evidence.released(), [29, 31, 37, 41, 43, 47, 53, 59, 61]);
            assert_eq!(evidence.active(), [67, 71, 73, 79, 83, 89, 97, 101, 103]);
            assert_eq!(evidence.run_loop_registrations(), 107);
            assert_eq!(evidence.link_invalidations(), 109);
            assert_eq!(evidence.delegate_revocations(), 113);
            assert_eq!(evidence.window_closes(), 127);
            assert_eq!(evidence.release_order_violations(), 131);
        }
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

/// Validated effective configuration reported by a live native surface.
///
/// Unlike [`SurfaceExtent`], a live window may transiently have a zero-sized
/// drawable while minimized or during native layout. The zero extent is kept
/// as an explicit ineligible state instead of fabricating a render target.
#[cfg(any(test, all(target_os = "macos", target_arch = "aarch64")))]
#[derive(Clone, Copy, Debug, PartialEq)]
struct SurfaceConfiguration {
    logical_width: f64,
    logical_height: f64,
    scale: f64,
    physical_width: u32,
    physical_height: u32,
    display_identity: usize,
    visible: bool,
}

#[cfg(any(test, all(target_os = "macos", target_arch = "aarch64")))]
impl SurfaceConfiguration {
    fn from_extent(extent: SurfaceExtent, display_identity: usize, visible: bool) -> Self {
        Self {
            logical_width: extent.logical_width(),
            logical_height: extent.logical_height(),
            scale: extent.scale(),
            physical_width: extent.physical_width(),
            physical_height: extent.physical_height(),
            display_identity,
            visible,
        }
    }

    fn from_native(
        logical_width: f64,
        logical_height: f64,
        scale: f64,
        display_identity: usize,
        visible: bool,
    ) -> Result<Self, SurfaceError> {
        validate_nonnegative_finite(logical_width, InvalidDimension::Width)?;
        validate_nonnegative_finite(logical_height, InvalidDimension::Height)?;
        validate_positive_finite(scale, InvalidDimension::Scale)?;
        let logical_width = if logical_width == 0.0 {
            0.0
        } else {
            logical_width
        };
        let logical_height = if logical_height == 0.0 {
            0.0
        } else {
            logical_height
        };

        Ok(Self {
            logical_width,
            logical_height,
            scale,
            physical_width: runtime_physical_dimension(
                logical_width,
                scale,
                InvalidDimension::Width,
            )?,
            physical_height: runtime_physical_dimension(
                logical_height,
                scale,
                InvalidDimension::Height,
            )?,
            display_identity,
            visible,
        })
    }

    const fn is_sized(self) -> bool {
        self.physical_width != 0 && self.physical_height != 0
    }

    fn geometry_or_display_differs(self, other: Self) -> bool {
        self.logical_width.to_bits() != other.logical_width.to_bits()
            || self.logical_height.to_bits() != other.logical_height.to_bits()
            || self.scale.to_bits() != other.scale.to_bits()
            || self.display_identity != other.display_identity
    }
}

#[cfg(any(test, all(target_os = "macos", target_arch = "aarch64")))]
const fn presentation_visible(
    window_visible: bool,
    miniaturized: bool,
    occlusion_visible: bool,
) -> bool {
    window_visible && !miniaturized && occlusion_visible
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
    /// Standard sRGB color-space creation.
    ColorSpace,
    /// Metal layer creation and configuration.
    Layer,
    /// Layer-bound display-link creation.
    DisplayLink,
    /// Display-link registration with the main run loop.
    RunLoop,
}

/// Handle-free identity of Alpine's configured standard-dynamic-range path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SdrColorContract {
    /// Linear sRGB shader values and blending stored in `BGRA8Unorm_sRGB`,
    /// then composited by Core Animation in the standard sRGB color space.
    LinearSrgbToBgra8UnormSrgb,
}

/// Handle-free terminal evidence correlated across one native frame attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameTerminalEvidence {
    attempt: AttemptEvidence,
    target_timestamp_bits: u64,
    target_presentation_timestamp_bits: u64,
    observed_presentation_time_bits: u64,
    retained_bytes: usize,
    recovery: Option<RecoveryClassification>,
}

impl FrameTerminalEvidence {
    #[cfg(any(test, all(target_os = "macos", target_arch = "aarch64")))]
    pub(crate) const fn new(
        attempt: AttemptEvidence,
        target_timestamp_bits: u64,
        target_presentation_timestamp_bits: u64,
        observed_presentation_time_bits: u64,
        retained_bytes: usize,
        recovery: Option<RecoveryClassification>,
    ) -> Self {
        Self {
            attempt,
            target_timestamp_bits,
            target_presentation_timestamp_bits,
            observed_presentation_time_bits,
            retained_bytes,
            recovery,
        }
    }

    /// Returns the monotonic frame-attempt identity.
    #[must_use]
    pub const fn attempt(self) -> u64 {
        self.attempt.attempt()
    }

    /// Returns the newest requested revision when this attempt terminated.
    #[must_use]
    pub const fn requested_revision(self) -> PresentationRevision {
        self.attempt.requested_revision()
    }

    /// Returns the immutable scene revision captured before encoding.
    #[must_use]
    pub const fn frame_revision(self) -> PresentationRevision {
        self.attempt.frame_revision()
    }

    /// Returns the native surface epoch captured before encoding.
    #[must_use]
    pub const fn frame_epoch(self) -> alpine_platform::SurfaceEpoch {
        self.attempt.frame_epoch()
    }

    /// Returns the portable terminal classification.
    #[must_use]
    pub const fn outcome(self) -> PresentationOutcome {
        self.attempt.outcome()
    }

    /// Returns the command commits recorded for this attempt.
    #[must_use]
    pub const fn submission_count(self) -> u8 {
        self.attempt.submission_count()
    }

    /// Returns direct drawable presentation calls recorded for this attempt.
    #[must_use]
    pub const fn present_call_count(self) -> u8 {
        self.attempt.present_call_count()
    }

    /// Returns whether the attempt was current immediately before commit.
    #[must_use]
    pub const fn eligible_at_commit(self) -> bool {
        self.attempt.eligible_at_commit()
    }

    /// Returns the raw `f64` bits of the display-link render target time.
    #[must_use]
    pub const fn target_timestamp_bits(self) -> u64 {
        self.target_timestamp_bits
    }

    /// Returns the raw `f64` bits of the display-link presentation target time.
    #[must_use]
    pub const fn target_presentation_timestamp_bits(self) -> u64 {
        self.target_presentation_timestamp_bits
    }

    /// Returns the raw `f64` bits observed by the drawable presented handler.
    ///
    /// Zero means no physical presentation timestamp was observed.
    #[must_use]
    pub const fn observed_presentation_time_bits(self) -> u64 {
        self.observed_presentation_time_bits
    }

    /// Returns Alpine-owned native frame bytes retained after terminal handling.
    #[must_use]
    pub const fn retained_bytes(self) -> usize {
        self.retained_bytes
    }

    /// Returns renderer recovery guidance when rendering caused termination.
    #[must_use]
    pub const fn recovery(self) -> Option<RecoveryClassification> {
        self.recovery
    }
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
    /// The surface run method cannot execute in the requested lifecycle state.
    RunLoopNotRunnable {
        /// Lifecycle state of the surface when run started.
        lifecycle: SurfaceLifecycle,
    },
    /// The application run loop exited before the surface stopped.
    UnexpectedRunLoopExit {
        /// Lifecycle state of the surface after application exit.
        lifecycle: SurfaceLifecycle,
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
            Self::RunLoopNotRunnable { lifecycle } => {
                write!(formatter, "run is not valid while surface is {lifecycle:?}")
            }
            Self::UnexpectedRunLoopExit { lifecycle } => {
                write!(
                    formatter,
                    "application run loop exited before surface closed: lifecycle {lifecycle:?}"
                )
            }
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
            | Self::PresentationsSkipped { .. }
            | Self::RunLoopNotRunnable { .. }
            | Self::UnexpectedRunLoopExit { .. } => None,
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
    surface_epoch: u64,
    sized: bool,
    presentation_visible: bool,
    sdr_color_contract: Option<SdrColorContract>,
    extended_dynamic_range: bool,
    framebuffer_only: bool,
    display_sync_enabled: bool,
    allows_next_drawable_timeout: bool,
    maximum_drawable_count: u8,
    regular_activation_policy: bool,
    display_link_paused: bool,
    visible: bool,
    callback_count: u64,
    rejected_callback_count: u64,
    submission_count: u64,
    direct_present_count: u64,
    installed_presented_handler_count: u64,
    presented_count: u64,
    qualified_presented_count: u64,
    superseded_count: u64,
    cancelled_count: u64,
    pending_cancellation_count: u64,
    last_presented_time_bits: u64,
    skipped_count: u64,
    failed_count: u64,
    allocated_bytes: u128,
    current_retained_bytes: usize,
    last_terminal: Option<FrameTerminalEvidence>,
    last_superseded: Option<FrameTerminalEvidence>,
    last_cancelled: Option<FrameTerminalEvidence>,
    last_pending_cancellation: Option<PendingCancellationEvidence>,
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

    /// Returns the current native size, scale, and display epoch.
    #[must_use]
    pub const fn surface_epoch(self) -> u64 {
        self.surface_epoch
    }

    /// Returns whether the current physical drawable extent is nonzero.
    #[must_use]
    pub const fn is_sized(self) -> bool {
        self.sized
    }

    /// Returns whether presentation is allowed by visibility and occlusion.
    #[must_use]
    pub const fn is_presentation_visible(self) -> bool {
        self.presentation_visible
    }

    /// Returns the recognized SDR transfer, target-format, and color-space identity.
    #[must_use]
    pub const fn sdr_color_contract(self) -> Option<SdrColorContract> {
        self.sdr_color_contract
    }

    /// Returns whether the native layer requests extended-dynamic-range compositing.
    #[must_use]
    pub const fn extended_dynamic_range(self) -> bool {
        self.extended_dynamic_range
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

    /// Returns callbacks rejected after the native owner generation closed.
    #[must_use]
    pub const fn rejected_callback_count(self) -> u64 {
        self.rejected_callback_count
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

    /// Returns attempts that presented and still matched current revision and epoch.
    #[must_use]
    pub const fn qualified_presented_count(self) -> u64 {
        self.qualified_presented_count
    }

    /// Returns committed attempts that terminated after becoming outdated.
    #[must_use]
    pub const fn superseded_count(self) -> u64 {
        self.superseded_count
    }

    /// Returns frame attempts explicitly cancelled before qualification.
    #[must_use]
    pub const fn cancelled_count(self) -> u64 {
        self.cancelled_count
    }

    /// Returns dirty requests cancelled before an attempt token existed.
    #[must_use]
    pub const fn pending_cancellation_count(self) -> u64 {
        self.pending_cancellation_count
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

    /// Returns cumulative native frame-resource bytes allocated by the backend.
    #[must_use]
    pub const fn allocated_bytes(self) -> u128 {
        self.allocated_bytes
    }

    /// Returns native frame-resource bytes still retained after terminal work.
    #[must_use]
    pub const fn current_retained_bytes(self) -> usize {
        self.current_retained_bytes
    }

    /// Returns the most recent attempt's complete handle-free terminal record.
    #[must_use]
    pub const fn last_terminal(self) -> Option<FrameTerminalEvidence> {
        self.last_terminal
    }

    /// Returns the most recent committed attempt rejected as outdated.
    #[must_use]
    pub const fn last_superseded(self) -> Option<FrameTerminalEvidence> {
        self.last_superseded
    }

    /// Returns the most recent explicitly cancelled attempt.
    #[must_use]
    pub const fn last_cancelled(self) -> Option<FrameTerminalEvidence> {
        self.last_cancelled
    }

    /// Returns the most recent request cancelled before frame preparation.
    #[must_use]
    pub const fn last_pending_cancellation(self) -> Option<PendingCancellationEvidence> {
        self.last_pending_cancellation
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
    rejected_callback_count: Arc<AtomicU64>,
}

impl SurfaceObserver {
    pub(crate) fn new(
        lifecycle: Arc<AtomicU8>,
        callback_count: Arc<AtomicU64>,
        rejected_callback_count: Arc<AtomicU64>,
    ) -> Self {
        Self {
            lifecycle,
            callback_count,
            rejected_callback_count,
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

    /// Returns callbacks rejected after callback admission was revoked.
    #[must_use]
    pub fn rejected_callback_count(&self) -> u64 {
        self.rejected_callback_count.load(Ordering::Acquire)
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

    /// Runs the `AppKit` event loop until the surface closes.
    ///
    /// # Errors
    ///
    /// Returns [`SurfaceError::UnsupportedPlatform`] outside Apple Silicon
    /// macOS, [`SurfaceError::RunLoopNotRunnable`] if already closing or
    /// closed, and [`SurfaceError::UnexpectedRunLoopExit`] for unexpected
    /// loop termination while still live.
    pub fn run(&self) -> Result<(), SurfaceError> {
        self.implementation.run()
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

#[cfg(any(test, all(target_os = "macos", target_arch = "aarch64")))]
fn validate_nonnegative_finite(
    value: f64,
    dimension: InvalidDimension,
) -> Result<(), SurfaceError> {
    if value.is_finite() && value >= 0.0 {
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

#[cfg(any(test, all(target_os = "macos", target_arch = "aarch64")))]
fn runtime_physical_dimension(
    logical: f64,
    scale: f64,
    dimension: InvalidDimension,
) -> Result<u32, SurfaceError> {
    let value = (logical * scale).round();
    if value.is_finite() && value >= 0.0 && value <= f64::from(MAX_DRAWABLE_DIMENSION) {
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

fn new_observer_state() -> (Arc<AtomicU8>, Arc<AtomicU64>, Arc<AtomicU64>) {
    (
        Arc::new(AtomicU8::new(SURFACE_LIVE)),
        Arc::new(AtomicU64::new(0)),
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

    fn terminal_attempt(event: alpine_platform::PresentationEvent) -> Option<AttemptEvidence> {
        if let alpine_platform::PresentationEvent::Terminal(attempt) = event {
            Some(attempt)
        } else {
            None
        }
    }

    fn pending_cancellation(
        event: alpine_platform::PresentationEvent,
    ) -> Option<PendingCancellationEvidence> {
        if let alpine_platform::PresentationEvent::PendingCancelled(evidence) = event {
            Some(evidence)
        } else {
            None
        }
    }

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
    fn live_configuration_preserves_zero_size_and_effective_identity() -> Result<(), SurfaceError> {
        let extent = SurfaceExtent::new(100.25, 50.75, 2.0)?;
        let created = SurfaceConfiguration::from_extent(extent, 7, true);
        let zero_width = SurfaceConfiguration::from_native(0.0, 50.0, 2.0, 7, false)?;
        let zero_height = SurfaceConfiguration::from_native(50.0, -0.0, 2.0, 7, false)?;
        let rounded_zero = SurfaceConfiguration::from_native(0.1, 50.0, 1.0, 7, false)?;
        let sized = SurfaceConfiguration::from_native(100.25, 50.75, 2.0, 7, true)?;

        assert_eq!(created, sized);
        assert!(!zero_width.is_sized());
        assert!(!zero_height.is_sized());
        assert_eq!(zero_height.logical_height.to_bits(), 0.0_f64.to_bits());
        assert!(!rounded_zero.is_sized());
        assert_eq!(sized.physical_width, 201);
        assert_eq!(sized.physical_height, 102);
        assert!(sized.is_sized());
        assert!(sized.visible);
        Ok(())
    }

    #[test]
    fn live_configuration_changes_only_for_geometry_scale_or_display() -> Result<(), SurfaceError> {
        let base = SurfaceConfiguration::from_native(100.0, 50.0, 2.0, 7, true)?;
        let hidden = SurfaceConfiguration::from_native(100.0, 50.0, 2.0, 7, false)?;
        let resized = SurfaceConfiguration::from_native(101.0, 50.0, 2.0, 7, true)?;
        let logical_width_only = SurfaceConfiguration::from_native(100.1, 50.0, 1.0, 7, true)?;
        let logical_width_only_changed =
            SurfaceConfiguration::from_native(100.2, 50.0, 1.0, 7, true)?;
        let logical_height_only = SurfaceConfiguration::from_native(100.0, 50.1, 1.0, 7, true)?;
        let logical_height_only_changed =
            SurfaceConfiguration::from_native(100.0, 50.2, 1.0, 7, true)?;
        let scale_only = SurfaceConfiguration::from_native(0.6, 0.6, 1.0, 7, true)?;
        let scale_only_changed = SurfaceConfiguration::from_native(0.6, 0.6, 1.1, 7, true)?;
        let rescaled = SurfaceConfiguration::from_native(100.0, 50.0, 1.0, 7, true)?;
        let migrated = SurfaceConfiguration::from_native(100.0, 50.0, 2.0, 11, true)?;

        assert!(!base.geometry_or_display_differs(base));
        assert!(!base.geometry_or_display_differs(hidden));
        assert!(base.geometry_or_display_differs(resized));
        assert!(logical_width_only.geometry_or_display_differs(logical_width_only_changed));
        assert!(logical_height_only.geometry_or_display_differs(logical_height_only_changed));
        assert!(scale_only.geometry_or_display_differs(scale_only_changed));
        assert!(base.geometry_or_display_differs(rescaled));
        assert!(base.geometry_or_display_differs(migrated));
        Ok(())
    }

    #[test]
    fn live_configuration_rejects_invalid_native_geometry() {
        for (width, height, scale, dimension) in [
            (-1.0, 1.0, 1.0, InvalidDimension::Width),
            (f64::NAN, 1.0, 1.0, InvalidDimension::Width),
            (1.0, -1.0, 1.0, InvalidDimension::Height),
            (1.0, f64::INFINITY, 1.0, InvalidDimension::Height),
            (1.0, 1.0, 0.0, InvalidDimension::Scale),
        ] {
            assert!(matches!(
                SurfaceConfiguration::from_native(width, height, scale, 0, true),
                Err(SurfaceError::InvalidDimension {
                    dimension: observed,
                    ..
                }) if observed == dimension
            ));
        }
        assert!(matches!(
            SurfaceConfiguration::from_native(
                f64::from(MAX_DRAWABLE_DIMENSION) + 1.0,
                1.0,
                1.0,
                0,
                true,
            ),
            Err(SurfaceError::PhysicalDimensionOutOfRange {
                dimension: InvalidDimension::Width,
                ..
            })
        ));
        assert!(matches!(
            SurfaceConfiguration::from_native(
                1.0,
                f64::from(MAX_DRAWABLE_DIMENSION) + 1.0,
                1.0,
                0,
                true,
            ),
            Err(SurfaceError::PhysicalDimensionOutOfRange {
                dimension: InvalidDimension::Height,
                ..
            })
        ));
    }

    #[test]
    fn presentation_visibility_requires_every_native_condition() {
        for window_visible in [false, true] {
            for miniaturized in [false, true] {
                for occlusion_visible in [false, true] {
                    assert_eq!(
                        presentation_visible(window_visible, miniaturized, occlusion_visible),
                        window_visible && !miniaturized && occlusion_visible
                    );
                }
            }
        }
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
        assert_eq!(
            SurfaceError::NativeUnavailable {
                stage: SurfaceStage::ColorSpace,
            }
            .to_string(),
            "native surface unavailable at ColorSpace stage"
        );
        assert_eq!(
            SurfaceError::RunLoopNotRunnable {
                lifecycle: SurfaceLifecycle::Closing,
            }
            .to_string(),
            "run is not valid while surface is Closing"
        );
        assert_eq!(
            SurfaceError::UnexpectedRunLoopExit {
                lifecycle: SurfaceLifecycle::Live,
            }
            .to_string(),
            "application run loop exited before surface closed: lifecycle Live"
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
    #[allow(
        clippy::too_many_lines,
        reason = "one exhaustive fixture keeps every public snapshot accessor discriminating"
    )]
    fn snapshot_accessors_preserve_discriminating_values() -> Result<(), Box<dyn Error>> {
        let mut state = alpine_platform::PresentationState::new();
        state.apply(alpine_platform::PresentationAction::SetSized(true))?;
        state.apply(alpine_platform::PresentationAction::SetVisible(true))?;
        state.apply(alpine_platform::PresentationAction::Invalidate)?;
        state.apply(alpine_platform::PresentationAction::Resume)?;
        let prepared = state.apply(alpine_platform::PresentationAction::Prepare)?;
        let token = state.active_token().ok_or("prepared frame token")?;
        assert_eq!(
            prepared.event(),
            alpine_platform::PresentationEvent::Prepared(token)
        );
        state.apply(alpine_platform::PresentationAction::BeginUpdate(token))?;
        state.apply(alpine_platform::PresentationAction::Submit(token))?;
        state.apply(alpine_platform::PresentationAction::CallPresent(token))?;
        let terminal = state.apply(alpine_platform::PresentationAction::CompletePresentation(
            token,
        ))?;
        assert_eq!(
            terminal_attempt(alpine_platform::PresentationEvent::PacingResumed),
            None
        );
        let attempt = terminal_attempt(terminal.event()).ok_or("terminal attempt evidence")?;
        let terminal = FrameTerminalEvidence::new(
            attempt,
            73,
            79,
            83,
            89,
            Some(RecoveryClassification::RetryFrame),
        );

        state.apply(alpine_platform::PresentationAction::Invalidate)?;
        state.apply(alpine_platform::PresentationAction::Resume)?;
        let failed_prepared = state.apply(alpine_platform::PresentationAction::Prepare)?;
        let failed_token = state.active_token().ok_or("failed frame token")?;
        assert_eq!(
            failed_prepared.event(),
            alpine_platform::PresentationEvent::Prepared(failed_token)
        );
        state.apply(alpine_platform::PresentationAction::BeginUpdate(
            failed_token,
        ))?;
        let failed_transition = state.apply(alpine_platform::PresentationAction::FailActive(
            failed_token,
        ))?;
        let failed_attempt =
            terminal_attempt(failed_transition.event()).ok_or("failed terminal evidence")?;
        let failed_terminal = FrameTerminalEvidence::new(failed_attempt, 97, 101, 0, 103, None);
        assert_eq!(
            pending_cancellation(alpine_platform::PresentationEvent::Stopped),
            None
        );
        let mut pending_state = alpine_platform::PresentationState::new();
        pending_state.apply(alpine_platform::PresentationAction::SetVisible(true))?;
        pending_state.apply(alpine_platform::PresentationAction::SetSized(true))?;
        pending_state.apply(alpine_platform::PresentationAction::Invalidate)?;
        let pending_transition =
            pending_state.apply(alpine_platform::PresentationAction::BeginShutdown)?;
        let pending_cancellation = pending_cancellation(pending_transition.event())
            .ok_or("pending cancellation evidence")?;
        let snapshot = SurfaceSnapshot {
            physical_width: 17,
            physical_height: 19,
            surface_epoch: 21,
            sized: true,
            presentation_visible: true,
            sdr_color_contract: Some(SdrColorContract::LinearSrgbToBgra8UnormSrgb),
            extended_dynamic_range: false,
            framebuffer_only: false,
            display_sync_enabled: false,
            allows_next_drawable_timeout: true,
            maximum_drawable_count: 2,
            regular_activation_policy: false,
            display_link_paused: false,
            visible: true,
            callback_count: 23,
            rejected_callback_count: 24,
            submission_count: 29,
            direct_present_count: 31,
            installed_presented_handler_count: 33,
            presented_count: 37,
            qualified_presented_count: 38,
            superseded_count: 39,
            cancelled_count: 40,
            pending_cancellation_count: 41,
            last_presented_time_bits: 39,
            skipped_count: 41,
            failed_count: 43,
            allocated_bytes: 47,
            current_retained_bytes: 53,
            last_terminal: Some(terminal),
            last_superseded: None,
            last_cancelled: None,
            last_pending_cancellation: None,
        };
        let inverse = SurfaceSnapshot {
            physical_width: 29,
            physical_height: 31,
            surface_epoch: 33,
            sized: false,
            presentation_visible: false,
            sdr_color_contract: None,
            extended_dynamic_range: true,
            framebuffer_only: true,
            display_sync_enabled: true,
            allows_next_drawable_timeout: false,
            maximum_drawable_count: 3,
            regular_activation_policy: true,
            display_link_paused: true,
            visible: false,
            callback_count: 37,
            rejected_callback_count: 39,
            submission_count: 43,
            direct_present_count: 47,
            installed_presented_handler_count: 49,
            presented_count: 53,
            qualified_presented_count: 54,
            superseded_count: 55,
            cancelled_count: 56,
            pending_cancellation_count: 58,
            last_presented_time_bits: 57,
            skipped_count: 59,
            failed_count: 61,
            allocated_bytes: 67,
            current_retained_bytes: 71,
            last_terminal: Some(failed_terminal),
            last_superseded: Some(terminal),
            last_cancelled: Some(failed_terminal),
            last_pending_cancellation: Some(pending_cancellation),
        };

        assert_eq!(snapshot.physical_width(), 17);
        assert_eq!(snapshot.physical_height(), 19);
        assert_eq!(snapshot.surface_epoch(), 21);
        assert!(snapshot.is_sized());
        assert!(snapshot.is_presentation_visible());
        assert_eq!(
            snapshot.sdr_color_contract(),
            Some(SdrColorContract::LinearSrgbToBgra8UnormSrgb)
        );
        assert!(!snapshot.extended_dynamic_range());
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
        assert_eq!(snapshot.qualified_presented_count(), 38);
        assert_eq!(snapshot.superseded_count(), 39);
        assert_eq!(snapshot.last_presented_time_bits(), 39);
        assert_eq!(snapshot.skipped_count(), 41);
        assert_eq!(snapshot.failed_count(), 43);
        assert_eq!(snapshot.allocated_bytes(), 47);
        assert_eq!(snapshot.current_retained_bytes(), 53);
        assert_eq!(snapshot.last_terminal(), Some(terminal));
        assert_eq!(snapshot.last_superseded(), None);
        assert_eq!(terminal.attempt(), 1);
        assert_eq!(terminal.requested_revision().get(), 1);
        assert_eq!(terminal.frame_revision().get(), 1);
        assert_eq!(terminal.frame_epoch().get(), 0);
        assert_eq!(terminal.outcome(), PresentationOutcome::Presented);
        assert_eq!(terminal.submission_count(), 1);
        assert_eq!(terminal.present_call_count(), 1);
        assert!(terminal.eligible_at_commit());
        assert_eq!(terminal.target_timestamp_bits(), 73);
        assert_eq!(terminal.target_presentation_timestamp_bits(), 79);
        assert_eq!(terminal.observed_presentation_time_bits(), 83);
        assert_eq!(terminal.retained_bytes(), 89);
        assert_eq!(
            terminal.recovery(),
            Some(RecoveryClassification::RetryFrame)
        );

        assert_eq!(inverse.physical_width(), 29);
        assert_eq!(inverse.physical_height(), 31);
        assert_eq!(inverse.surface_epoch(), 33);
        assert!(!inverse.is_sized());
        assert!(!inverse.is_presentation_visible());
        assert_eq!(inverse.sdr_color_contract(), None);
        assert!(inverse.extended_dynamic_range());
        assert!(inverse.framebuffer_only());
        assert!(inverse.display_sync_enabled());
        assert!(!inverse.allows_next_drawable_timeout());
        assert_eq!(inverse.maximum_drawable_count(), 3);
        assert!(inverse.regular_activation_policy());
        assert!(inverse.display_link_paused());
        assert!(!inverse.visible());
        assert_eq!(inverse.callback_count(), 37);
        assert_eq!(inverse.rejected_callback_count(), 39);
        assert_eq!(inverse.submission_count(), 43);
        assert_eq!(inverse.direct_present_count(), 47);
        assert_eq!(inverse.installed_presented_handler_count(), 49);
        assert_eq!(inverse.presented_count(), 53);
        assert_eq!(inverse.qualified_presented_count(), 54);
        assert_eq!(inverse.superseded_count(), 55);
        assert_eq!(inverse.cancelled_count(), 56);
        assert_eq!(inverse.pending_cancellation_count(), 58);
        assert_eq!(inverse.last_presented_time_bits(), 57);
        assert_eq!(inverse.skipped_count(), 59);
        assert_eq!(inverse.failed_count(), 61);
        assert_eq!(inverse.allocated_bytes(), 67);
        assert_eq!(inverse.current_retained_bytes(), 71);
        assert_eq!(inverse.last_terminal(), Some(failed_terminal));
        assert_eq!(inverse.last_superseded(), Some(terminal));
        assert_eq!(inverse.last_cancelled(), Some(failed_terminal));
        assert_eq!(
            inverse.last_pending_cancellation(),
            Some(pending_cancellation)
        );
        assert_eq!(failed_terminal.attempt(), 2);
        assert_eq!(failed_terminal.submission_count(), 0);
        assert_eq!(failed_terminal.present_call_count(), 0);
        assert!(!failed_terminal.eligible_at_commit());
        Ok(())
    }

    #[test]
    fn observer_distinguishes_live_closed_and_callback_count() {
        let (lifecycle, callback_count, rejected_callback_count) = new_observer_state();
        callback_count.store(29, Ordering::Release);
        rejected_callback_count.store(31, Ordering::Release);
        let observer = SurfaceObserver::new(
            Arc::clone(&lifecycle),
            callback_count,
            rejected_callback_count,
        );

        assert_eq!(observer.lifecycle(), SurfaceLifecycle::Live);
        assert_eq!(observer.callback_count(), 29);
        assert_eq!(observer.rejected_callback_count(), 31);
        begin_close_observer_state(&lifecycle);
        assert_eq!(observer.lifecycle(), SurfaceLifecycle::Closing);
        finish_close_observer_state(&lifecycle);
        assert_eq!(observer.lifecycle(), SurfaceLifecycle::Closed);
        assert_eq!(observer.callback_count(), 29);
        assert_eq!(observer.rejected_callback_count(), 31);
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
        assert_eq!(surface.run(), Err(SurfaceError::UnsupportedPlatform));
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
