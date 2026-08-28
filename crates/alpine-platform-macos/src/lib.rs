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

use alpine_core::{LinearRgba, Point, Size};
use alpine_metal::{InitializationError, RecoveryClassification, RenderError};
use alpine_platform::{
    AttemptEvidence, PendingCancellationEvidence, PresentationOutcome, PresentationRevision,
    TransitionError,
};
use alpine_scene::Scene;

mod accessibility;
pub use accessibility::*;
mod signpost;
pub use signpost::*;

/// Monotonic sequence assigned at the native event-dispatch boundary.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct EventTimestamp(u64);

impl EventTimestamp {
    /// Creates an identity from one monotonic process-local sequence value.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the process-local sequence value.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Compact platform modifier identity preserved on input events.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Modifiers(u8);

impl Modifiers {
    /// Shift modifier bit.
    pub const SHIFT: u8 = 0x01;
    /// Control modifier bit.
    pub const CONTROL: u8 = 1 << 1;
    /// Option modifier bit.
    pub const OPTION: u8 = 1 << 2;
    /// Command modifier bit.
    pub const COMMAND: u8 = 1 << 3;
    /// Caps Lock modifier bit.
    pub const CAPS_LOCK: u8 = 1 << 4;

    /// Creates a modifier set from Alpine's stable bit vocabulary.
    #[must_use]
    pub const fn from_bits(bits: u8) -> Self {
        Self(bits & 0x1f)
    }

    /// Returns Alpine's stable modifier bits.
    #[must_use]
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// Returns whether every requested modifier bit is set.
    #[must_use]
    pub const fn contains(self, bits: u8) -> bool {
        self.0 & bits == bits
    }
}

/// Keyboard transition represented independently of `AppKit` objects.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyState {
    /// A key became pressed.
    Down,
    /// A key became released.
    Up,
    /// Modifier state changed without text input.
    ModifiersChanged,
}

/// Pointer transition represented independently of `AppKit` objects.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PointerAction {
    /// Pointer position changed.
    Moved,
    /// A pointer button became pressed.
    Down,
    /// A pointer button became released.
    Up,
}

/// Stable pointer-button identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PointerButton {
    /// No button is associated with a movement event.
    None,
    /// Primary pointer button.
    Primary,
    /// Secondary pointer button.
    Secondary,
    /// Middle pointer button.
    Middle,
    /// Additional platform button number.
    Other(u8),
}

/// Scroll gesture lifecycle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScrollPhase {
    /// Gesture began.
    Began,
    /// Gesture remains active.
    Changed,
    /// Gesture ended.
    Ended,
    /// Gesture was cancelled.
    Cancelled,
    /// Device supplied no phase identity.
    None,
}

/// Clipboard operation represented without clipboard contents or handles.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClipboardOperation {
    /// Copy selected content.
    Copy,
    /// Cut selected content.
    Cut,
    /// Paste available content.
    Paste,
}

/// Terminal result of one native plain-text clipboard operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ClipboardEvent {
    /// A copy write completed or failed without mutating application text.
    CopyCompleted(Result<(), ClipboardError>),
    /// A cut write completed or failed before application text may be removed.
    CutCompleted(Result<(), ClipboardError>),
    /// A paste read produced bounded UTF-8 or a structured failure.
    PasteCompleted(Result<ClipboardText, ClipboardError>),
}

impl ClipboardEvent {
    /// Returns the operation whose terminal result this event carries.
    #[must_use]
    pub const fn operation(&self) -> ClipboardOperation {
        match self {
            Self::CopyCompleted(_) => ClipboardOperation::Copy,
            Self::CutCompleted(_) => ClipboardOperation::Cut,
            Self::PasteCompleted(_) => ClipboardOperation::Paste,
        }
    }
}

/// Maximum UTF-8 bytes retained by one Alpine clipboard value.
pub const MAX_CLIPBOARD_TEXT_BYTES: usize = 64 * 1024 * 1024;

/// A structured clipboard boundary failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClipboardError {
    /// The platform did not provide plain UTF-8 text.
    Unavailable,
    /// The owned UTF-8 value exceeded the retained clipboard ceiling.
    TooLarge {
        /// Observed UTF-8 byte length.
        bytes: usize,
        /// Enforced UTF-8 byte ceiling.
        limit: usize,
    },
    /// A paste operation cannot be used as a clipboard write identity.
    InvalidWriteOperation,
    /// The platform rejected a requested clipboard write.
    WriteRejected,
}

impl core::fmt::Display for ClipboardError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Unavailable => formatter.write_str("plain UTF-8 clipboard text is unavailable"),
            Self::TooLarge { bytes, limit } => {
                write!(
                    formatter,
                    "clipboard text has {bytes} bytes; limit is {limit}"
                )
            }
            Self::InvalidWriteOperation => {
                formatter.write_str("paste cannot be returned as a clipboard write")
            }
            Self::WriteRejected => formatter.write_str("the platform rejected the clipboard write"),
        }
    }
}

impl core::error::Error for ClipboardError {}

/// Bounded owned plain text crossing the handle-free clipboard boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClipboardText(Box<str>);

impl ClipboardText {
    /// Validates one owned UTF-8 clipboard value.
    ///
    /// # Errors
    ///
    /// Returns [`ClipboardError::TooLarge`] when the value exceeds
    /// [`MAX_CLIPBOARD_TEXT_BYTES`].
    pub fn new(text: impl Into<Box<str>>) -> Result<Self, ClipboardError> {
        let text = text.into();
        if text.len() > MAX_CLIPBOARD_TEXT_BYTES {
            return Err(ClipboardError::TooLarge {
                bytes: text.len(),
                limit: MAX_CLIPBOARD_TEXT_BYTES,
            });
        }
        Ok(Self(text))
    }

    /// Returns the retained UTF-8 text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consumes the value into its owned UTF-8 text.
    #[must_use]
    pub fn into_inner(self) -> Box<str> {
        self.0
    }
}

/// One bounded plain-text write requested by an application event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClipboardWrite {
    operation: ClipboardOperation,
    text: ClipboardText,
}

impl ClipboardWrite {
    /// Creates a copy or cut write request.
    ///
    /// # Errors
    ///
    /// Returns [`ClipboardError::InvalidWriteOperation`] for paste.
    pub fn new(operation: ClipboardOperation, text: ClipboardText) -> Result<Self, ClipboardError> {
        match operation {
            ClipboardOperation::Copy | ClipboardOperation::Cut => Ok(Self { operation, text }),
            ClipboardOperation::Paste => Err(ClipboardError::InvalidWriteOperation),
        }
    }

    /// Returns the operation that will receive completion identity.
    #[must_use]
    pub const fn operation(&self) -> ClipboardOperation {
        self.operation
    }

    /// Returns the bounded text to write.
    #[must_use]
    pub const fn text(&self) -> &ClipboardText {
        &self.text
    }

    /// Consumes the request into operation and bounded text.
    #[must_use]
    pub fn into_parts(self) -> (ClipboardOperation, ClipboardText) {
        (self.operation, self.text)
    }
}

/// Synchronous disposition for one native close request.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CloseDisposition {
    /// The current event is not a close request.
    #[default]
    NotRequested,
    /// The application accepts the close and irreversible drain may begin.
    Allow,
    /// The application keeps the window and runtime live.
    Cancel,
}

/// Monotonic identity for one native text-input session.
///
/// Epoch zero is invalid. Native focus or lifecycle loss advances the epoch
/// after marked text is discarded and any active composition is cancelled.
/// Consumers must reject IME values whose epoch is not current before mutation.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct InputEpoch(u64);

impl InputEpoch {
    /// First valid epoch for a newly created native surface.
    pub const INITIAL: Self = Self(1);

    /// Creates a non-zero input epoch.
    #[must_use]
    pub const fn new(value: u64) -> Option<Self> {
        if value == 0 { None } else { Some(Self(value)) }
    }

    /// Returns the underlying monotonic identity.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Returns the next distinct epoch, or `None` at the representable limit.
    #[must_use]
    pub const fn checked_next(self) -> Option<Self> {
        match self.0.checked_add(1) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    /// Classifies an event epoch against this current epoch.
    #[must_use]
    pub const fn classify(self, event: Self) -> InputEpochAdmission {
        if event.0 < self.0 {
            InputEpochAdmission::Stale
        } else if event.0 > self.0 {
            InputEpochAdmission::Future
        } else {
            InputEpochAdmission::Current
        }
    }
}

/// Mutation admission for an epoch-tagged native input value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputEpochAdmission {
    /// The event belongs to the active input session.
    Current,
    /// The event belongs to an obsolete input session.
    Stale,
    /// The event arrived before its focus transition established the epoch.
    Future,
}

/// Input-method composition lifecycle and owned text payload.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ImeEvent {
    /// A new composition began.
    Started,
    /// Marked text and its UTF-16 selection changed.
    Updated {
        /// Current marked text.
        text: Box<str>,
        /// Selected UTF-16 start offset inside marked text.
        selected_start_utf16: u32,
        /// Selected UTF-16 length inside marked text.
        selected_length_utf16: u32,
    },
    /// Composition committed owned text.
    Committed(Box<str>),
    /// Composition ended without committed text.
    Cancelled,
}

/// Handle-free event vocabulary crossing the native surface boundary.
#[derive(Clone, Debug, PartialEq)]
pub enum SurfaceEvent {
    /// Synchronous, demand-driven assistive-technology request.
    Accessibility {
        /// Monotonic event sequence.
        timestamp: EventTimestamp,
        /// Bounded handle-free request.
        request: AccessibilityRequest,
    },
    /// Keyboard identity, text, modifiers, and repeat state.
    Keyboard {
        /// Monotonic event sequence.
        timestamp: EventTimestamp,
        /// Press, release, or modifier transition.
        state: KeyState,
        /// Platform-independent physical key code when known.
        physical_key: u16,
        /// Owned logical key or text identity.
        logical_key: Box<str>,
        /// Active modifiers.
        modifiers: Modifiers,
        /// Whether this is a platform repeat.
        repeat: bool,
    },
    /// Pointer position and button transition.
    Pointer {
        /// Monotonic event sequence.
        timestamp: EventTimestamp,
        /// Movement, press, or release.
        action: PointerAction,
        /// Logical surface position.
        position: Point,
        /// Associated button identity.
        button: PointerButton,
        /// Active modifiers.
        modifiers: Modifiers,
    },
    /// Precision-preserving scroll delta and phase.
    Scroll {
        /// Monotonic event sequence.
        timestamp: EventTimestamp,
        /// Horizontal logical delta.
        delta_x: f32,
        /// Vertical logical delta.
        delta_y: f32,
        /// Gesture lifecycle.
        phase: ScrollPhase,
        /// Whether the device reports precise deltas.
        precise: bool,
        /// Active modifiers.
        modifiers: Modifiers,
    },
    /// Key-window focus transition.
    Focus {
        /// Monotonic event sequence.
        timestamp: EventTimestamp,
        /// Epoch active after this focus transition.
        input_epoch: InputEpoch,
        /// Whether the Studio window became focused.
        focused: bool,
    },
    /// Validated logical, scale, and physical extent update.
    Resize {
        /// Monotonic event sequence.
        timestamp: EventTimestamp,
        /// New validated surface extent.
        extent: SurfaceExtent,
    },
    /// Terminal native clipboard result with bounded owned paste text.
    Clipboard {
        /// Monotonic event sequence.
        timestamp: EventTimestamp,
        /// Typed completion that prevents invalid operation/payload pairs.
        event: ClipboardEvent,
    },
    /// Input-method composition transition.
    Ime {
        /// Monotonic event sequence.
        timestamp: EventTimestamp,
        /// Native input session that produced this composition value.
        input_epoch: InputEpoch,
        /// Composition lifecycle and text.
        event: ImeEvent,
    },
    /// Main-loop wake used to publish bounded background results.
    Wake {
        /// Monotonic event sequence.
        timestamp: EventTimestamp,
    },
    /// Owned-window close intent before event admission is revoked.
    CloseRequested {
        /// Monotonic event sequence.
        timestamp: EventTimestamp,
    },
}

impl SurfaceEvent {
    /// Returns the exact timestamp carried by this event.
    #[must_use]
    pub const fn timestamp(&self) -> EventTimestamp {
        match self {
            Self::Accessibility { timestamp, .. }
            | Self::Keyboard { timestamp, .. }
            | Self::Pointer { timestamp, .. }
            | Self::Scroll { timestamp, .. }
            | Self::Focus { timestamp, .. }
            | Self::Resize { timestamp, .. }
            | Self::Clipboard { timestamp, .. }
            | Self::Ime { timestamp, .. }
            | Self::Wake { timestamp }
            | Self::CloseRequested { timestamp } => *timestamp,
        }
    }
}

#[cfg(kani)]
mod input_epoch_verification {
    use super::{InputEpoch, InputEpochAdmission};

    #[cfg_attr(test, mutants::skip)] // The dedicated Kani gate executes this proof.
    #[kani::proof]
    fn checked_epoch_boundaries_never_wrap_or_alias() {
        let value = kani::any::<u64>();
        kani::assume(value != 0);
        kani::cover!(value == 1, "minimum live epoch");
        kani::cover!(value == u64::MAX, "exhausted epoch");
        let Some(epoch) = InputEpoch::new(value) else {
            unreachable!();
        };
        match epoch.checked_next() {
            Some(next) => {
                assert_eq!(next.get(), value + 1);
                assert_eq!(next.classify(epoch), InputEpochAdmission::Stale);
                assert_eq!(epoch.classify(next), InputEpochAdmission::Future);
            }
            None => assert_eq!(value, u64::MAX),
        }
        assert_eq!(epoch.classify(epoch), InputEpochAdmission::Current);
    }
}

#[cfg(test)]
mod input_epoch_tests {
    use super::{InputEpoch, InputEpochAdmission};

    #[test]
    fn epoch_construction_advancement_and_admission_are_exact() {
        assert_eq!(InputEpoch::new(0), None);
        assert_eq!(InputEpoch::new(1), Some(InputEpoch::INITIAL));
        let next = InputEpoch::INITIAL.checked_next();
        assert_eq!(next.map(InputEpoch::get), Some(2));
        assert_eq!(
            InputEpoch::new(u64::MAX).and_then(InputEpoch::checked_next),
            None
        );
        assert_eq!(
            InputEpoch::INITIAL.classify(InputEpoch::INITIAL),
            InputEpochAdmission::Current
        );
        assert_eq!(
            next.map(|epoch| epoch.classify(InputEpoch::INITIAL)),
            Some(InputEpochAdmission::Stale)
        );
        assert_eq!(
            next.map(|epoch| InputEpoch::INITIAL.classify(epoch)),
            Some(InputEpochAdmission::Future)
        );
    }
}

/// One immutable scene and clear value requested by an event handler.
#[derive(Clone, Debug, PartialEq)]
pub struct SurfaceFrame {
    scene: Scene,
    clear: LinearRgba,
}

impl SurfaceFrame {
    /// Creates one handle-free frame request.
    #[must_use]
    pub const fn new(scene: Scene, clear: LinearRgba) -> Self {
        Self { scene, clear }
    }

    /// Returns the immutable scene.
    #[must_use]
    pub const fn scene(&self) -> &Scene {
        &self.scene
    }

    /// Returns the linear clear value.
    #[must_use]
    pub const fn clear(&self) -> LinearRgba {
        self.clear
    }

    /// Consumes the request into its scene and clear value.
    #[must_use]
    pub fn into_parts(self) -> (Scene, LinearRgba) {
        (self.scene, self.clear)
    }
}

/// One bounded, handle-free application response to a native event.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SurfaceResponse {
    frame: Option<SurfaceFrame>,
    clipboard_write: Option<ClipboardWrite>,
    close: CloseDisposition,
    accessibility: Option<AccessibilityResponse>,
}

impl SurfaceResponse {
    /// Creates one response with at most one frame and clipboard write.
    #[must_use]
    pub const fn new(
        frame: Option<SurfaceFrame>,
        clipboard_write: Option<ClipboardWrite>,
        close: CloseDisposition,
    ) -> Self {
        Self {
            frame,
            clipboard_write,
            close,
            accessibility: None,
        }
    }

    /// Creates one response from every independent bounded output channel.
    #[must_use]
    pub const fn from_channels(
        frame: Option<SurfaceFrame>,
        clipboard_write: Option<ClipboardWrite>,
        close: CloseDisposition,
        accessibility: Option<AccessibilityResponse>,
    ) -> Self {
        Self {
            frame,
            clipboard_write,
            close,
            accessibility,
        }
    }

    /// Returns the optional immutable frame.
    #[must_use]
    pub const fn frame(&self) -> Option<&SurfaceFrame> {
        self.frame.as_ref()
    }

    /// Returns the optional bounded clipboard write.
    #[must_use]
    pub const fn clipboard_write(&self) -> Option<&ClipboardWrite> {
        self.clipboard_write.as_ref()
    }

    /// Returns the close disposition for this event.
    #[must_use]
    pub const fn close_disposition(&self) -> CloseDisposition {
        self.close
    }

    /// Returns the optional exact accessibility response.
    #[must_use]
    pub const fn accessibility_response(&self) -> Option<&AccessibilityResponse> {
        self.accessibility.as_ref()
    }

    /// Consumes the response into every independent output channel.
    #[must_use]
    pub fn into_channels(
        self,
    ) -> (
        Option<SurfaceFrame>,
        Option<ClipboardWrite>,
        CloseDisposition,
        Option<AccessibilityResponse>,
    ) {
        (
            self.frame,
            self.clipboard_write,
            self.close,
            self.accessibility,
        )
    }

    /// Consumes the response into its independent output channels.
    #[must_use]
    pub fn into_parts(
        self,
    ) -> (
        Option<SurfaceFrame>,
        Option<ClipboardWrite>,
        CloseDisposition,
    ) {
        (self.frame, self.clipboard_write, self.close)
    }

    /// Consumes the response and returns only its optional frame.
    #[must_use]
    pub fn into_frame(self) -> Option<SurfaceFrame> {
        self.frame
    }
}

impl From<Option<SurfaceFrame>> for SurfaceResponse {
    fn from(frame: Option<SurfaceFrame>) -> Self {
        Self::new(frame, None, CloseDisposition::NotRequested)
    }
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod native;
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod native_accessibility;
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

    use crate::{
        ClipboardError, ClipboardOperation, NativeSurface, SurfaceDescriptor, SurfaceError,
        SurfaceEvent, SurfaceResponse, native,
    };

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

        /// Returns whether expiration has been disarmed or already consumed.
        #[must_use]
        pub fn cancelled(&self) -> bool {
            self.cancelled.load(Ordering::Acquire)
        }

        /// Disarms the guard after the production run loop exits normally.
        pub fn cancel(&self) {
            self.cancelled.store(true, Ordering::Release);
        }
    }

    /// Validation-only close path selected for one production delegate replay.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum CloseReplayScenario {
        /// Invoke close without an installed application handler.
        MissingHandler,
        /// Hold the event-handler borrow while AppKit requests close.
        ReentrantHandler,
        /// Install a handler that explicitly cancels close.
        Cancel,
        /// Install a handler that explicitly allows close.
        Allow,
    }

    /// Handle-free identity for one real `AppKit` screen used by physical validation.
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct ValidationScreenConfiguration {
        /// Stable index within the current `AppKit` screen list.
        pub index: usize,
        /// Process-local `AppKit` screen object identity.
        pub identity: usize,
        /// Backing pixels per logical screen unit.
        pub backing_scale: f64,
        /// Visible screen origin on the `AppKit` x axis.
        pub visible_x: f64,
        /// Visible screen origin on the `AppKit` y axis.
        pub visible_y: f64,
        /// Visible logical screen width.
        pub visible_width: f64,
        /// Visible logical screen height.
        pub visible_height: f64,
    }

    impl ValidationScreenConfiguration {
        #[allow(
            clippy::too_many_arguments,
            reason = "physical screen evidence preserves each independent AppKit value"
        )]
        pub(crate) const fn new(
            index: usize,
            identity: usize,
            backing_scale: f64,
            visible_x: f64,
            visible_y: f64,
            visible_width: f64,
            visible_height: f64,
        ) -> Self {
            Self {
                index,
                identity,
                backing_scale,
                visible_x,
                visible_y,
                visible_width,
                visible_height,
            }
        }
    }

    /// Validation-only exact ownership and teardown counts for one surface.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct NativeOwnerEvidence {
        acquired: [u64; 10],
        released: [u64; 10],
        active: [u64; 10],
        run_loop_registrations: u64,
        link_invalidations: u64,
        delegate_revocations: u64,
        window_closes: u64,
        pasteboard_releases: u64,
        release_order_violations: u64,
    }

    /// One bounded record from the observer-facing AppKit post protocol.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct NativeAccessibilityNotificationRecord {
        pub(crate) kind_index: u8,
        pub(crate) target: u64,
        pub(crate) payload_elements: usize,
        pub(crate) payload_bytes: usize,
        pub(crate) priority: isize,
    }

    impl NativeAccessibilityNotificationRecord {
        pub(crate) const fn new(
            kind_index: u8,
            target: u64,
            payload_elements: usize,
            payload_bytes: usize,
            priority: isize,
        ) -> Self {
            Self {
                kind_index,
                target,
                payload_elements,
                payload_bytes,
                priority,
            }
        }

        /// Returns the kind index in layout, focus, selection, value,
        /// announcement, and destroyed order.
        #[must_use]
        pub const fn kind_index(self) -> u8 {
            self.kind_index
        }

        /// Returns the stable semantic target identity passed to AppKit.
        #[must_use]
        pub const fn target(self) -> u64 {
            self.target
        }

        /// Returns current elements named by a layout payload.
        #[must_use]
        pub const fn payload_elements(self) -> usize {
            self.payload_elements
        }

        /// Returns retained layout-slot or announcement UTF-8 payload bytes.
        #[must_use]
        pub const fn payload_bytes(self) -> usize {
            self.payload_bytes
        }

        /// Returns AppKit announcement priority, or zero for other kinds.
        #[must_use]
        pub const fn priority(self) -> isize {
            self.priority
        }
    }

    /// One handle-free native element observed through production AppKit selectors.
    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct NativeAccessibilityNodeEvidence {
        pub(crate) semantic_id: u64,
        pub(crate) role: Box<str>,
        pub(crate) label: Box<str>,
        pub(crate) identifier: Box<str>,
        pub(crate) focused: bool,
        pub(crate) selected: bool,
        pub(crate) activate_allowed: bool,
        pub(crate) current: bool,
        pub(crate) bounded_screen_frame: bool,
    }

    impl NativeAccessibilityNodeEvidence {
        /// Returns the stable semantic node identity.
        #[must_use]
        pub const fn semantic_id(&self) -> u64 {
            self.semantic_id
        }
        /// Returns the AppKit role exposed by the native element.
        #[must_use]
        pub fn role(&self) -> &str {
            &self.role
        }
        /// Returns the bounded AppKit label.
        #[must_use]
        pub fn label(&self) -> &str {
            &self.label
        }
        /// Returns the generation-bound external identifier.
        #[must_use]
        pub fn identifier(&self) -> &str {
            &self.identifier
        }
        /// Returns whether AppKit observes this element as focused.
        #[must_use]
        pub const fn focused(&self) -> bool {
            self.focused
        }
        /// Returns whether AppKit observes this element as selected.
        #[must_use]
        pub const fn selected(&self) -> bool {
            self.selected
        }
        /// Returns whether AppKit admits the press selector.
        #[must_use]
        pub const fn activate_allowed(&self) -> bool {
            self.activate_allowed
        }
        /// Returns whether the element remains current at observation time.
        #[must_use]
        pub const fn current(&self) -> bool {
            self.current
        }
        /// Returns whether the converted screen frame is finite and nonempty.
        #[must_use]
        pub const fn bounded_screen_frame(&self) -> bool {
            self.bounded_screen_frame
        }
    }

    /// Handle-free evidence from one complete production AppKit semantic query.
    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct NativeAccessibilityTreeEvidence {
        pub(crate) revision: crate::AccessibilityRevision,
        pub(crate) nodes: Box<[NativeAccessibilityNodeEvidence]>,
        pub(crate) focused_nodes: usize,
    }

    impl NativeAccessibilityTreeEvidence {
        /// Returns the exact semantic revision queried through AppKit.
        #[must_use]
        pub const fn revision(&self) -> crate::AccessibilityRevision {
            self.revision
        }
        /// Returns every bounded current native element in semantic order.
        #[must_use]
        pub fn nodes(&self) -> &[NativeAccessibilityNodeEvidence] {
            &self.nodes
        }
        /// Returns the number of elements AppKit reports as focused.
        #[must_use]
        pub const fn focused_nodes(&self) -> usize {
            self.focused_nodes
        }
    }

    /// Handle-free evidence from one named production AppKit press action.
    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct NativeAccessibilityActivationEvidence {
        pub(crate) semantic_id: u64,
        pub(crate) role: Box<str>,
        pub(crate) label: Box<str>,
        pub(crate) identifier: Box<str>,
        pub(crate) selector_allowed: bool,
        pub(crate) accepted: bool,
        pub(crate) current_after_action: bool,
        pub(crate) dispatch_failed: bool,
    }

    impl NativeAccessibilityActivationEvidence {
        /// Returns the activated semantic node identity.
        #[must_use]
        pub const fn semantic_id(&self) -> u64 {
            self.semantic_id
        }
        /// Returns the native role used to select the action target.
        #[must_use]
        pub fn role(&self) -> &str {
            &self.role
        }
        /// Returns the exact native label used to select the action target.
        #[must_use]
        pub fn label(&self) -> &str {
            &self.label
        }
        /// Returns the target's generation-bound external identifier.
        #[must_use]
        pub fn identifier(&self) -> &str {
            &self.identifier
        }
        /// Returns whether AppKit admitted `accessibilityPerformPress`.
        #[must_use]
        pub const fn selector_allowed(&self) -> bool {
            self.selector_allowed
        }
        /// Returns whether the production action authority accepted the press.
        #[must_use]
        pub const fn accepted(&self) -> bool {
            self.accepted
        }
        /// Returns whether the same native instance remains current afterward.
        #[must_use]
        pub const fn current_after_action(&self) -> bool {
            self.current_after_action
        }
        /// Returns whether native dispatch observed a structured callback failure.
        #[must_use]
        pub const fn dispatch_failed(&self) -> bool {
            self.dispatch_failed
        }
    }

    #[cfg(test)]
    mod native_accessibility_evidence_accessor_tests {
        use super::{
            NativeAccessibilityActivationEvidence, NativeAccessibilityNodeEvidence,
            NativeAccessibilityTreeEvidence,
        };

        #[test]
        fn handle_free_accessors_preserve_negative_and_nondefault_evidence() {
            let node = NativeAccessibilityNodeEvidence {
                semantic_id: 37,
                role: Box::from("AXButton"),
                label: Box::from("Action"),
                identifier: Box::from("alpine.ax.1.2.37"),
                focused: false,
                selected: false,
                activate_allowed: false,
                current: false,
                bounded_screen_frame: false,
            };
            assert_eq!(node.semantic_id(), 37);
            assert_eq!(node.role(), "AXButton");
            assert_eq!(node.label(), "Action");
            assert_eq!(node.identifier(), "alpine.ax.1.2.37");
            assert!(!node.focused());
            assert!(!node.selected());
            assert!(!node.activate_allowed());
            assert!(!node.current());
            assert!(!node.bounded_screen_frame());
            let current_node = NativeAccessibilityNodeEvidence {
                current: true,
                ..node.clone()
            };
            assert!(current_node.current());

            let tree = NativeAccessibilityTreeEvidence {
                revision: crate::AccessibilityRevision::new(3, 5),
                nodes: Box::new([]),
                focused_nodes: 2,
            };
            assert_eq!(tree.focused_nodes(), 2);

            let activation = NativeAccessibilityActivationEvidence {
                semantic_id: 37,
                role: Box::from("AXButton"),
                label: Box::from("Action"),
                identifier: Box::from("alpine.ax.1.2.37"),
                selector_allowed: false,
                accepted: false,
                current_after_action: false,
                dispatch_failed: true,
            };
            assert_eq!(activation.semantic_id(), 37);
            assert_eq!(activation.role(), "AXButton");
            assert_eq!(activation.label(), "Action");
            assert_eq!(activation.identifier(), "alpine.ax.1.2.37");
            assert!(!activation.selector_allowed());
            assert!(!activation.accepted());
            assert!(!activation.current_after_action());
            assert!(activation.dispatch_failed());

            let permitted = NativeAccessibilityActivationEvidence {
                semantic_id: 41,
                role: Box::from("AXButton"),
                label: Box::from("Permitted action"),
                identifier: Box::from("alpine.ax.1.2.41"),
                selector_allowed: true,
                accepted: true,
                current_after_action: true,
                dispatch_failed: false,
            };
            assert!(permitted.selector_allowed());
            assert!(permitted.accepted());
            assert!(permitted.current_after_action());
            assert!(!permitted.dispatch_failed());
        }
    }

    /// Handle-free evidence produced by the production AppKit accessibility selectors.
    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct NativeAccessibilityEvidence {
        pub(crate) root_children: usize,
        pub(crate) stable_root_identity: bool,
        pub(crate) role: Box<str>,
        pub(crate) label: Box<str>,
        pub(crate) text_length_utf16: usize,
        pub(crate) selected_text: Box<str>,
        pub(crate) bounded_text: Box<str>,
        pub(crate) selected_range: crate::AccessibilityTextRange,
        pub(crate) line_for_index: usize,
        pub(crate) range_for_line: crate::AccessibilityTextRange,
        pub(crate) range_for_index: crate::AccessibilityTextRange,
        pub(crate) bounded_text_selector_allowed: bool,
        pub(crate) geometry_selector_allowed: bool,
        pub(crate) semantic_tree_valid: bool,
        pub(crate) text_selector_scope_valid: bool,
        pub(crate) accepted_selection: crate::AccessibilityTextRange,
        pub(crate) stale_action_rejected: bool,
        pub(crate) stable_external_identifier: bool,
        pub(crate) bounded_screen_frame: bool,
        pub(crate) activate_selector_allowed: bool,
        pub(crate) accepted_activation: bool,
        pub(crate) revoked_activation_rejected: bool,
        pub(crate) peak_elements: usize,
        pub(crate) created_elements: u64,
        pub(crate) released_elements: u64,
        pub(crate) notification_counts: [u64; 6],
        pub(crate) notification_records: Box<[NativeAccessibilityNotificationRecord]>,
        pub(crate) omitted_notification_records: u64,
        pub(crate) invalid_notification_user_info: u64,
        pub(crate) notification_user_info_controls_valid: bool,
        pub(crate) posts_after_handler_revocation: u64,
        pub(crate) revoke_starts: u64,
        pub(crate) revoke_terminal: bool,
        pub(crate) posted_notification_payload_bytes: usize,
        pub(crate) peak_notification_retained_bytes: usize,
        pub(crate) current_elements_after_revoke: usize,
        pub(crate) retained_slot_bytes_before_revoke: usize,
        pub(crate) retained_slot_bytes_after_revoke: usize,
        pub(crate) late_selector_rejected: bool,
    }

    impl NativeAccessibilityEvidence {
        /// Returns direct children published beneath the surface view.
        #[must_use]
        pub const fn root_children(&self) -> usize {
            self.root_children
        }
        /// Returns whether repeated snapshots preserved the root AppKit object.
        #[must_use]
        pub const fn stable_root_identity(&self) -> bool {
            self.stable_root_identity
        }
        /// Returns the editor's native role.
        #[must_use]
        pub fn role(&self) -> &str {
            &self.role
        }
        /// Returns the editor's native label.
        #[must_use]
        pub fn label(&self) -> &str {
            &self.label
        }
        /// Returns the editor's complete UTF-16 length without document materialization.
        #[must_use]
        pub const fn text_length_utf16(&self) -> usize {
            self.text_length_utf16
        }
        /// Returns the bounded selected text requested through AppKit.
        #[must_use]
        pub fn selected_text(&self) -> &str {
            &self.selected_text
        }
        /// Returns one bounded string obtained through `accessibilityStringForRange:`.
        #[must_use]
        pub fn bounded_text(&self) -> &str {
            &self.bounded_text
        }
        /// Returns the native selected range observed before the write.
        #[must_use]
        pub const fn selected_range(&self) -> crate::AccessibilityTextRange {
            self.selected_range
        }
        /// Returns the line mapped from the validation index.
        #[must_use]
        pub const fn line_for_index(&self) -> usize {
            self.line_for_index
        }
        /// Returns the range mapped from the validation line.
        #[must_use]
        pub const fn range_for_line(&self) -> crate::AccessibilityTextRange {
            self.range_for_line
        }
        /// Returns the grapheme range mapped from the validation index.
        #[must_use]
        pub const fn range_for_index(&self) -> crate::AccessibilityTextRange {
            self.range_for_index
        }
        /// Returns whether the bounded text selector is admitted.
        #[must_use]
        pub const fn bounded_text_selector_allowed(&self) -> bool {
            self.bounded_text_selector_allowed
        }
        /// Returns whether unsupported geometry materialization was rejected.
        #[must_use]
        pub const fn geometry_selector_allowed(&self) -> bool {
            self.geometry_selector_allowed
        }
        /// Returns whether role, value, focus, selection, and parentage selectors agree.
        #[must_use]
        pub const fn semantic_tree_valid(&self) -> bool {
            self.semantic_tree_valid
        }
        /// Returns whether editor text selectors are denied on non-editor elements.
        #[must_use]
        pub const fn text_selector_scope_valid(&self) -> bool {
            self.text_selector_scope_valid
        }
        /// Returns the accepted revision-checked selection.
        #[must_use]
        pub const fn accepted_selection(&self) -> crate::AccessibilityTextRange {
            self.accepted_selection
        }
        /// Returns whether a stale revision action was rejected.
        #[must_use]
        pub const fn stale_action_rejected(&self) -> bool {
            self.stale_action_rejected
        }
        /// Returns whether one unchanged native element preserved a non-empty identifier.
        #[must_use]
        pub const fn stable_external_identifier(&self) -> bool {
            self.stable_external_identifier
        }
        /// Returns whether view-local bounds became a finite non-empty screen rectangle.
        #[must_use]
        pub const fn bounded_screen_frame(&self) -> bool {
            self.bounded_screen_frame
        }
        /// Returns whether AppKit admits press only for an enabled action node.
        #[must_use]
        pub const fn activate_selector_allowed(&self) -> bool {
            self.activate_selector_allowed
        }
        /// Returns whether one current press crossed the bounded action transport.
        #[must_use]
        pub const fn accepted_activation(&self) -> bool {
            self.accepted_activation
        }
        /// Returns whether the retained element rejected press after adapter revocation.
        #[must_use]
        pub const fn revoked_activation_rejected(&self) -> bool {
            self.revoked_activation_rejected
        }
        /// Returns peak retained native element count.
        #[must_use]
        pub const fn peak_elements(&self) -> usize {
            self.peak_elements
        }
        /// Returns native elements created during the journey.
        #[must_use]
        pub const fn created_elements(&self) -> u64 {
            self.created_elements
        }
        /// Returns native elements released by cache reconciliation and revocation.
        #[must_use]
        pub const fn released_elements(&self) -> u64 {
            self.released_elements
        }
        /// Returns completed synchronous AppKit post calls by notification kind.
        ///
        /// These counts prove invocation, not observation by an assistive client.
        #[must_use]
        pub const fn notification_counts(&self) -> [u64; 6] {
            self.notification_counts
        }
        /// Returns the bounded observer-facing AppKit post protocol fixture.
        ///
        /// Records prove post invocation shape, not external delivery.
        #[must_use]
        pub fn notification_records(&self) -> &[NativeAccessibilityNotificationRecord] {
            &self.notification_records
        }
        /// Returns records omitted after the fixed validation evidence ceiling.
        #[must_use]
        pub const fn omitted_notification_records(&self) -> u64 {
            self.omitted_notification_records
        }
        /// Returns constructed payload dictionaries missing required AppKit keys.
        #[must_use]
        pub const fn invalid_notification_user_info(&self) -> u64 {
            self.invalid_notification_user_info
        }
        /// Returns whether positive and malformed payload-key controls discriminate.
        #[must_use]
        pub const fn notification_user_info_controls_valid(&self) -> bool {
            self.notification_user_info_controls_valid
        }
        /// Returns posts attempted after the application handler was revoked.
        #[must_use]
        pub const fn posts_after_handler_revocation(&self) -> u64 {
            self.posts_after_handler_revocation
        }
        /// Returns the number of admitted revoke transitions.
        #[must_use]
        pub const fn revoke_starts(&self) -> u64 {
            self.revoke_starts
        }
        /// Returns whether revoke ended with no handler and no transition in progress.
        #[must_use]
        pub const fn revoke_terminal(&self) -> bool {
            self.revoke_terminal
        }
        /// Returns cumulative bounded payload bytes passed to AppKit posts.
        #[must_use]
        pub const fn posted_notification_payload_bytes(&self) -> usize {
            self.posted_notification_payload_bytes
        }
        /// Returns peak temporary retained slots and announcement bytes.
        #[must_use]
        pub const fn peak_notification_retained_bytes(&self) -> usize {
            self.peak_notification_retained_bytes
        }
        /// Returns retained native elements after callback revocation.
        #[must_use]
        pub const fn current_elements_after_revoke(&self) -> usize {
            self.current_elements_after_revoke
        }
        /// Returns bytes occupied by retained cache slots before revocation.
        #[must_use]
        pub const fn retained_slot_bytes_before_revoke(&self) -> usize {
            self.retained_slot_bytes_before_revoke
        }
        /// Returns bytes occupied by retained cache slots after revocation.
        #[must_use]
        pub const fn retained_slot_bytes_after_revoke(&self) -> usize {
            self.retained_slot_bytes_after_revoke
        }
        /// Returns whether a retained late element failed closed after revocation.
        #[must_use]
        pub const fn late_selector_rejected(&self) -> bool {
            self.late_selector_rejected
        }
    }

    impl NativeOwnerEvidence {
        #[allow(
            clippy::too_many_arguments,
            reason = "validation evidence preserves each independent cleanup counter"
        )]
        pub(crate) const fn new(
            acquired: [u64; 10],
            released: [u64; 10],
            active: [u64; 10],
            run_loop_registrations: u64,
            link_invalidations: u64,
            delegate_revocations: u64,
            window_closes: u64,
            pasteboard_releases: u64,
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
                pasteboard_releases,
                release_order_violations,
            }
        }

        /// Returns per-kind acquisitions in application-to-display-link order.
        #[must_use]
        pub const fn acquired(self) -> [u64; 10] {
            self.acquired
        }

        /// Returns per-kind releases in application-to-display-link order.
        #[must_use]
        pub const fn released(self) -> [u64; 10] {
            self.released
        }

        /// Returns per-kind owners remaining after close.
        #[must_use]
        pub const fn active(self) -> [u64; 10] {
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

        /// Returns unique validation-pasteboard server releases.
        #[must_use]
        pub const fn pasteboard_releases(self) -> u64 {
            self.pasteboard_releases
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

    /// Runs the real `AppKit` event loop until one frame records a terminal
    /// outcome and all pending, active, and completion-owned work drains, or
    /// timeout.
    pub fn run_until_frame_terminal(surface: &NativeSurface, timeout: Duration) {
        surface.implementation.run_until_frame_terminal(timeout);
    }

    /// Runs until one frame terminates while preserving production handler ownership.
    ///
    /// # Errors
    ///
    /// Returns a structured error if the callback handler cannot be installed.
    pub fn run_until_frame_terminal_with_handler<F>(
        surface: &NativeSurface,
        timeout: Duration,
        handler: F,
    ) -> Result<(), SurfaceError>
    where
        F: FnMut(SurfaceEvent) -> SurfaceResponse + 'static,
    {
        surface
            .implementation
            .run_until_frame_terminal_with_handler(timeout, handler)
    }

    /// Last display-link directive observed by native pause qualification.
    #[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
    pub enum PauseDirectiveEvidence {
        /// No callback has been observed.
        #[default]
        Unknown,
        /// The portable transition emitted no native directive.
        None,
        /// The portable transition requested pacing resume.
        Resume,
        /// The portable transition requested pacing pause.
        Pause,
        /// The portable transition invalidated pacing.
        Invalidate,
    }

    /// Last portable display-link state observed after callback processing.
    #[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
    pub enum PausePortableStateEvidence {
        /// No callback has been observed.
        #[default]
        Unknown,
        /// Portable pacing is paused.
        Paused,
        /// Portable pacing is running.
        Running,
        /// Portable pacing is invalid.
        Invalid,
    }

    /// Stage-separated evidence for deferred native pause confirmation.
    #[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
    pub struct PauseConfirmationEvidence {
        requested: u64,
        enqueued: u64,
        executed: u64,
        eligible: u64,
        observed: u64,
        callback_observations: u64,
        last_directive: PauseDirectiveEvidence,
        last_portable_state: PausePortableStateEvidence,
        last_native_paused_before: bool,
        last_native_paused_after: bool,
        last_pending: bool,
        last_active: bool,
    }

    impl PauseConfirmationEvidence {
        pub(crate) const fn new(
            requested: u64,
            enqueued: u64,
            executed: u64,
            eligible: u64,
            observed: u64,
            callback_observations: u64,
            last_directive: PauseDirectiveEvidence,
            last_portable_state: PausePortableStateEvidence,
            last_native_paused_before: bool,
            last_native_paused_after: bool,
            last_pending: bool,
            last_active: bool,
        ) -> Self {
            Self {
                requested,
                enqueued,
                executed,
                eligible,
                observed,
                callback_observations,
                last_directive,
                last_portable_state,
                last_native_paused_before,
                last_native_paused_after,
                last_pending,
                last_active,
            }
        }

        /// Returns confirmation requests admitted after a portable pause.
        #[must_use]
        pub const fn requested(self) -> u64 {
            self.requested
        }

        /// Returns requests copied into the main run loop.
        #[must_use]
        pub const fn enqueued(self) -> u64 {
            self.enqueued
        }

        /// Returns deferred blocks that began execution.
        #[must_use]
        pub const fn executed(self) -> u64 {
            self.executed
        }

        /// Returns executed blocks that still observed portable paused state.
        #[must_use]
        pub const fn eligible(self) -> u64 {
            self.eligible
        }

        /// Returns eligible blocks that observed native paused state.
        #[must_use]
        pub const fn observed(self) -> u64 {
            self.observed
        }

        /// Returns the last callback's portable-to-native directive.
        #[must_use]
        pub const fn last_directive(self) -> PauseDirectiveEvidence {
            self.last_directive
        }

        /// Returns the last callback's post-update portable pacing state.
        #[must_use]
        pub const fn last_portable_state(self) -> PausePortableStateEvidence {
            self.last_portable_state
        }
    }

    /// Returns stage-separated deferred native pause evidence.
    #[must_use]
    pub fn pause_confirmation_evidence(surface: &NativeSurface) -> PauseConfirmationEvidence {
        surface.implementation.pause_confirmation_evidence()
    }

    /// Returns post-callback pause reaffirmations observed by the main run loop.
    #[must_use]
    pub fn pause_confirmation_count(surface: &NativeSurface) -> u64 {
        pause_confirmation_evidence(surface).observed()
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

    /// Schedules a production close request after a bounded validation delay.
    pub fn arm_window_close(surface: &NativeSurface, delay: Duration) {
        surface.implementation.arm_window_close(delay);
    }

    /// Schedules the user-facing AppKit close action after a bounded delay.
    ///
    /// Unlike [`arm_window_close`], this enters `windowShouldClose` before the
    /// production `windowWillClose` teardown boundary. It therefore proves
    /// application close propagation rather than only native owner teardown.
    pub fn arm_user_window_close(surface: &NativeSurface, delay: Duration) {
        surface.implementation.arm_user_window_close(delay);
    }

    /// Schedules a deterministic delegate-mediated close after a bounded delay.
    ///
    /// This calls the production `windowShouldClose` delegate and, only when
    /// allowed, asks `NSWindow` to close so `windowWillClose` performs normal
    /// teardown. It does not depend on close-button state on a headless host.
    pub fn arm_programmatic_window_close(surface: &NativeSurface, delay: Duration) {
        surface.implementation.arm_programmatic_window_close(delay);
    }

    /// Revokes only the private wake pointer while lifecycle remains live.
    ///
    /// This validation fault proves pointer revocation independently rejects
    /// producer admission before any native owner is released.
    pub fn revoke_surface_waker(surface: &NativeSurface) {
        surface.implementation.revoke_waker_for_validation();
    }

    /// Installs one deterministic asynchronous driver failure for contract tests.
    pub fn inject_driver_error(surface: &NativeSurface, error: SurfaceError) {
        surface.implementation.inject_driver_error(error);
    }

    /// Installs one deterministic presented-handler observation on the current
    /// committed drawable, or on the next commit when no drawable is active.
    /// Zero models Apple's terminal dropped/not-presented drawable result.
    ///
    /// An optional display identity change always targets the next commit and
    /// advances the native surface epoch at that exact boundary.
    ///
    /// # Errors
    ///
    /// Returns a driver error for a negative/non-finite time or unavailable
    /// callback owner.
    pub fn inject_post_commit_observation(
        surface: &NativeSurface,
        display_identity: Option<usize>,
        presented_time: f64,
    ) -> Result<(), SurfaceError> {
        surface
            .implementation
            .inject_post_commit_observation(display_identity, presented_time)
    }

    /// Suppresses the next drawable's presentation callback after commit.
    ///
    /// This validation-only fault proves that optional presentation telemetry
    /// cannot retain completed GPU resources or stall later editor frames.
    pub fn inject_post_commit_omission(surface: &NativeSurface) {
        surface.implementation.inject_post_commit_omission();
    }

    /// Returns whether the next commit has an armed presentation omission.
    #[must_use]
    pub fn post_commit_omission_armed(surface: &NativeSurface) -> bool {
        surface.implementation.post_commit_omission_armed()
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

    /// Returns handle-free metadata for every real AppKit screen.
    #[must_use]
    pub fn screen_configurations(surface: &NativeSurface) -> Vec<ValidationScreenConfiguration> {
        surface.implementation.validation_screen_configurations()
    }

    /// Centers the validation window on one real AppKit screen and synchronizes it.
    ///
    /// # Errors
    ///
    /// Returns a driver error when the index is absent or AppKit does not publish
    /// the selected screen after moving the owned window.
    pub fn move_window_to_screen(
        surface: &NativeSurface,
        index: usize,
    ) -> Result<ValidationScreenConfiguration, SurfaceError> {
        surface.implementation.move_window_to_screen(index)
    }

    /// Closes the real `AppKit` window through its delegate lifecycle.
    pub fn close_window(surface: &NativeSurface) {
        surface.implementation.close_window();
    }

    /// Replays handle-free event values through the production delegate seam.
    ///
    /// # Errors
    ///
    /// Returns a structured error if handler ownership or a returned frame
    /// cannot be admitted by the synchronized native driver.
    pub fn replay_surface_events<F>(
        surface: &NativeSurface,
        events: &[SurfaceEvent],
        handler: F,
    ) -> Result<(), SurfaceError>
    where
        F: FnMut(SurfaceEvent) -> SurfaceResponse + 'static,
    {
        surface
            .implementation
            .replay_surface_events(events, handler)
    }

    /// Replays handle-free values through the non-returning AppKit callback seam.
    ///
    /// # Errors
    ///
    /// Returns a structured error if the callback handler cannot be installed.
    pub fn replay_callback_surface_events<F>(
        surface: &NativeSurface,
        events: &[SurfaceEvent],
        handler: F,
    ) -> Result<(), SurfaceError>
    where
        F: FnMut(SurfaceEvent) -> SurfaceResponse + 'static,
    {
        surface
            .implementation
            .replay_callback_surface_events(events, handler)
    }

    /// Exercises the production AppKit keyboard and text-input selectors.
    ///
    /// # Errors
    ///
    /// Returns a structured native or dispatch error when the production
    /// responder path cannot deliver every event.
    pub fn replay_native_input_path<F>(
        surface: &NativeSurface,
        handler: F,
    ) -> Result<(), SurfaceError>
    where
        F: FnMut(SurfaceEvent) -> SurfaceResponse + 'static,
    {
        surface.implementation.replay_native_input_path(handler)
    }

    /// Commits exact text through the production AppKit text-input selector.
    ///
    /// # Errors
    ///
    /// Returns a structured native or dispatch error when the current
    /// responder cannot admit and deliver the text through its live input epoch.
    pub fn commit_native_text<F>(
        surface: &NativeSurface,
        text: &str,
        handler: F,
    ) -> Result<(), SurfaceError>
    where
        F: FnMut(SurfaceEvent) -> SurfaceResponse + 'static,
    {
        surface.implementation.commit_native_text(text, handler)
    }

    /// Sets the pre-run input epoch and focus state for startup publication validation.
    pub fn set_input_focus_state(
        surface: &NativeSurface,
        input_epoch: crate::InputEpoch,
        focused: bool,
    ) {
        surface
            .implementation
            .set_input_focus_state_for_validation(input_epoch, focused);
    }

    /// Replays the production AppKit accessibility selectors against one event handler.
    ///
    /// # Errors
    ///
    /// Returns a structured native error when selector dispatch, response identity,
    /// cache reconciliation, or lifecycle revocation fails closed.
    pub fn replay_native_accessibility_path<F>(
        surface: &NativeSurface,
        handler: F,
    ) -> Result<NativeAccessibilityEvidence, SurfaceError>
    where
        F: FnMut(SurfaceEvent) -> SurfaceResponse + 'static,
    {
        surface
            .implementation
            .replay_native_accessibility_path(handler)
    }

    /// Queries every current production AppKit accessibility element.
    ///
    /// # Errors
    ///
    /// Returns a structured native error when semantic refresh, selector
    /// dispatch, or bounded evidence construction fails closed.
    pub fn inspect_native_accessibility_tree<F>(
        surface: &NativeSurface,
        handler: F,
    ) -> Result<NativeAccessibilityTreeEvidence, SurfaceError>
    where
        F: FnMut(SurfaceEvent) -> SurfaceResponse + 'static,
    {
        surface
            .implementation
            .inspect_native_accessibility_tree(handler)
    }

    /// Presses one exact current native element selected by semantic role and label.
    ///
    /// # Errors
    ///
    /// Returns a structured native error when the exact target is absent,
    /// ambiguous, revoked, disabled, or cannot dispatch through production code.
    pub fn activate_named_native_accessibility_node<F>(
        surface: &NativeSurface,
        role: crate::AccessibilityRole,
        label: &str,
        handler: F,
    ) -> Result<NativeAccessibilityActivationEvidence, SurfaceError>
    where
        F: FnMut(SurfaceEvent) -> SurfaceResponse + 'static,
    {
        surface
            .implementation
            .activate_named_native_accessibility_node(role, label, handler)
    }

    /// Replays one exact Command-C, Command-X, or Command-V input path.
    ///
    /// # Errors
    ///
    /// Returns a structured dispatch error if the response or completion
    /// cannot cross the production delegate boundary.
    pub fn replay_native_clipboard_operation<F>(
        surface: &NativeSurface,
        operation: ClipboardOperation,
        handler: F,
    ) -> Result<(), SurfaceError>
    where
        F: FnMut(SurfaceEvent) -> SurfaceResponse + 'static,
    {
        surface
            .implementation
            .replay_native_clipboard_operation(operation, handler)
    }

    /// Injects one terminal clipboard failure before the next native operation.
    pub fn inject_clipboard_error(surface: &NativeSurface, error: ClipboardError) {
        surface.implementation.inject_clipboard_error(error);
    }

    /// Exercises one `windowShouldClose` scenario and reports admission.
    ///
    /// # Errors
    ///
    /// Returns a structured dispatch error if the selected scenario cannot be
    /// installed or executed.
    pub fn replay_close(
        surface: &NativeSurface,
        scenario: CloseReplayScenario,
    ) -> Result<bool, SurfaceError> {
        surface.implementation.replay_close(scenario)
    }

    /// Exercises `windowShouldClose` with one production application handler.
    ///
    /// # Errors
    ///
    /// Returns a structured dispatch error if the handler cannot be installed
    /// or the production delegate cannot resolve the close request.
    pub fn replay_close_with_handler<F>(
        surface: &NativeSurface,
        handler: F,
    ) -> Result<bool, SurfaceError>
    where
        F: FnMut(SurfaceEvent) -> SurfaceResponse + 'static,
    {
        surface.implementation.replay_close_with_handler(handler)
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

    /// Exercises one initialization-stage failure and returns post-rollback ownership evidence.
    ///
    /// # Errors
    ///
    /// Returns a structured native surface error if the requested stage does
    /// not fail exactly or validation ownership instrumentation is absent.
    pub fn exercise_initialization_fault(
        stage: crate::SurfaceStage,
    ) -> Result<NativeOwnerEvidence, SurfaceError> {
        native::exercise_initialization_fault(stage)
    }

    /// Closes one validation surface and returns exact post-drop owner evidence.
    ///
    /// # Errors
    ///
    /// Returns [`SurfaceError::ValidationFailure`] if validation ownership
    /// instrumentation is unexpectedly absent or inconsistent.
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
                [2, 3, 5, 7, 11, 13, 17, 19, 23, 29],
                [31, 37, 41, 43, 47, 53, 59, 61, 67, 71],
                [73, 79, 83, 89, 97, 101, 103, 107, 109, 113],
                127,
                131,
                137,
                139,
                149,
                151,
            );

            assert_eq!(evidence.acquired(), [2, 3, 5, 7, 11, 13, 17, 19, 23, 29]);
            assert_eq!(
                evidence.released(),
                [31, 37, 41, 43, 47, 53, 59, 61, 67, 71]
            );
            assert_eq!(
                evidence.active(),
                [73, 79, 83, 89, 97, 101, 103, 107, 109, 113]
            );
            assert_eq!(evidence.run_loop_registrations(), 127);
            assert_eq!(evidence.link_invalidations(), 131);
            assert_eq!(evidence.delegate_revocations(), 137);
            assert_eq!(evidence.window_closes(), 139);
            assert_eq!(evidence.pasteboard_releases(), 149);
            assert_eq!(evidence.release_order_violations(), 151);
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

    /// Converts the validated logical extent to Alpine scene coordinates.
    #[must_use]
    #[allow(
        clippy::cast_possible_truncation,
        reason = "drawable dimensions are bounded to values exactly representable by f32"
    )]
    pub fn logical_size(self) -> Option<Size> {
        Size::new(self.logical_width as f32, self.logical_height as f32)
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

/// Handle-free operation identity for native ownership and invariant failures.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SurfaceOperation {
    /// Alpine Studio or runtime application-state construction.
    Application,
    /// Frame admission, submission, completion, or presentation state.
    Presentation,
    /// Synchronization of window, layer, scale, or display configuration.
    NativeConfiguration,
    /// Installation or ownership of the native event callback.
    EventHandler,
    /// Keyboard, pointer, scroll, focus, IME, or responder dispatch.
    Input,
    /// Native clipboard request or response ownership.
    Clipboard,
    /// Native accessibility request, tree, notification, or action dispatch.
    Accessibility,
    /// `AppKit` application and run-loop admission or termination.
    RunLoop,
    /// Native-only fault injection or qualification instrumentation.
    Validation,
}

/// Handle-free identity of Alpine's configured standard-dynamic-range path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SdrColorContract {
    /// Linear sRGB shader values and blending stored in `BGRA8Unorm_sRGB`,
    /// then composited by Core Animation in the standard sRGB color space.
    LinearSrgbToBgra8UnormSrgb,
}

/// Handle-free process-monotonic latency evidence for one event-driven frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameLatencyEvidence {
    event_timestamp: EventTimestamp,
    event_handler_ns: u64,
    frame_queue_ns: Option<u64>,
    submission_ns: Option<u64>,
    event_to_gpu_terminal_observed_ns: Option<u64>,
    event_to_presented_handler_ns: Option<u64>,
    event_to_terminal_record_ns: u64,
}

impl FrameLatencyEvidence {
    #[cfg(any(test, all(target_os = "macos", target_arch = "aarch64")))]
    pub(crate) const fn new(
        event_timestamp: EventTimestamp,
        event_handler_ns: u64,
        frame_queue_ns: Option<u64>,
        submission_ns: Option<u64>,
        event_to_gpu_terminal_observed_ns: Option<u64>,
        event_to_presented_handler_ns: Option<u64>,
        event_to_terminal_record_ns: u64,
    ) -> Self {
        Self {
            event_timestamp,
            event_handler_ns,
            frame_queue_ns,
            submission_ns,
            event_to_gpu_terminal_observed_ns,
            event_to_presented_handler_ns,
            event_to_terminal_record_ns,
        }
    }

    /// Returns the native event sequence that produced the admitted frame.
    #[must_use]
    pub const fn event_timestamp(self) -> EventTimestamp {
        self.event_timestamp
    }

    /// Returns nanoseconds spent in application event handling and scene construction.
    #[must_use]
    pub const fn event_handler_ns(self) -> u64 {
        self.event_handler_ns
    }

    /// Returns nanoseconds from frame admission to its display-link submission attempt.
    #[must_use]
    pub const fn frame_queue_ns(self) -> Option<u64> {
        self.frame_queue_ns
    }

    /// Returns nanoseconds spent validating, uploading, encoding, committing, and presenting.
    #[must_use]
    pub const fn submission_ns(self) -> Option<u64> {
        self.submission_ns
    }

    /// Returns nanoseconds from event receipt to main-thread observation of GPU termination.
    ///
    /// This is an upper bound because the non-blocking display callback observes an
    /// already-published Metal completion instead of timestamping inside that handler.
    #[must_use]
    pub const fn event_to_gpu_terminal_observed_ns(self) -> Option<u64> {
        self.event_to_gpu_terminal_observed_ns
    }

    /// Returns nanoseconds from event receipt to the drawable presented-handler callback.
    #[must_use]
    pub const fn event_to_presented_handler_ns(self) -> Option<u64> {
        self.event_to_presented_handler_ns
    }

    /// Returns nanoseconds from event receipt to Alpine's correlated terminal record.
    #[must_use]
    pub const fn event_to_terminal_record_ns(self) -> u64 {
        self.event_to_terminal_record_ns
    }
}

/// Handle-free terminal evidence correlated across one native frame attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameTerminalEvidence {
    attempt: AttemptEvidence,
    target_timestamp_bits: u64,
    target_presentation_timestamp_bits: u64,
    observed_presentation_time_bits: u64,
    latency: Option<FrameLatencyEvidence>,
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
        latency: Option<FrameLatencyEvidence>,
        retained_bytes: usize,
        recovery: Option<RecoveryClassification>,
    ) -> Self {
        Self {
            attempt,
            target_timestamp_bits,
            target_presentation_timestamp_bits,
            observed_presentation_time_bits,
            latency,
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

    /// Returns process-monotonic stage evidence when an event produced this frame.
    #[must_use]
    pub const fn latency(self) -> Option<FrameLatencyEvidence> {
        self.latency
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
    /// A synchronized native owner was already borrowed or poisoned.
    OwnerConflict {
        /// Operation whose owner could not be acquired.
        operation: SurfaceOperation,
    },
    /// An operation reached a state forbidden by its checked contract.
    InvariantViolation {
        /// Operation whose invariant failed.
        operation: SurfaceOperation,
    },
    /// `AppKit` rejected Alpine's editor view as the active input responder.
    InputResponderRejected,
    /// Native-only qualification instrumentation rejected an observation.
    ValidationFailure {
        /// Operation under validation when evidence became invalid.
        operation: SurfaceOperation,
    },
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

impl SurfaceError {
    /// Constructs a handle-free synchronized-owner conflict.
    #[must_use]
    pub const fn owner_conflict(operation: SurfaceOperation) -> Self {
        Self::OwnerConflict { operation }
    }

    /// Constructs a handle-free invariant failure.
    #[must_use]
    pub const fn invariant(operation: SurfaceOperation) -> Self {
        Self::InvariantViolation { operation }
    }

    /// Constructs a native-validation-only failure.
    #[must_use]
    pub const fn validation(operation: SurfaceOperation) -> Self {
        Self::ValidationFailure { operation }
    }
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
            Self::NativeUnavailable {
                stage: SurfaceStage::Device,
            } => formatter.write_str("Metal device unavailable at Device stage"),
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
            Self::OwnerConflict { operation } => {
                write!(formatter, "synchronized {operation:?} owner is unavailable")
            }
            Self::InvariantViolation { operation } => {
                write!(formatter, "{operation:?} invariant failed")
            }
            Self::InputResponderRejected => {
                formatter.write_str("AppKit rejected Alpine input responder activation")
            }
            Self::ValidationFailure { operation } => {
                write!(formatter, "native validation failed during {operation:?}")
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
            | Self::OwnerConflict { .. }
            | Self::InvariantViolation { .. }
            | Self::InputResponderRejected
            | Self::ValidationFailure { .. }
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
    #[cfg(alpine_native_validation)]
    pause_confirmation_count: u64,
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
    peak_retained_bytes: usize,
    current_retained_bytes: usize,
    current_upload_bytes: usize,
    peak_upload_bytes: usize,
    frame_slot_capacity: u8,
    occupied_frame_slots: u8,
    submitted_frame_slots: u8,
    peak_occupied_frame_slots: u8,
    frame_slot_saturation_count: u64,
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

    /// Returns the largest renderer-owned retention observed by this surface.
    #[must_use]
    pub const fn peak_retained_bytes(self) -> usize {
        self.peak_retained_bytes
    }

    /// Returns reusable upload-buffer bytes currently retained by frame slots.
    #[must_use]
    pub const fn current_upload_bytes(self) -> usize {
        self.current_upload_bytes
    }

    /// Returns the largest simultaneous reusable upload retention observed.
    #[must_use]
    pub const fn peak_upload_bytes(self) -> usize {
        self.peak_upload_bytes
    }

    /// Returns the fixed number of reusable native presentation slots.
    #[must_use]
    pub const fn frame_slot_capacity(self) -> u8 {
        self.frame_slot_capacity
    }

    /// Returns frame slots currently owning encoding or submitted work.
    #[must_use]
    pub const fn occupied_frame_slots(self) -> u8 {
        self.occupied_frame_slots
    }

    /// Returns frame slots currently retaining committed GPU work.
    #[must_use]
    pub const fn submitted_frame_slots(self) -> u8 {
        self.submitted_frame_slots
    }

    /// Returns the largest observed frame-slot occupancy.
    #[must_use]
    pub const fn peak_occupied_frame_slots(self) -> u8 {
        self.peak_occupied_frame_slots
    }

    /// Returns valid frame attempts omitted because all slots were occupied.
    #[must_use]
    pub const fn frame_slot_saturation_count(self) -> u64 {
        self.frame_slot_saturation_count
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

/// Outcome of one nonblocking main-loop wake request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SurfaceWakeAdmission {
    /// A main-thread callback was scheduled.
    Scheduled,
    /// A callback was already pending, so this request was coalesced.
    Coalesced,
    /// The corresponding surface no longer admits callbacks.
    Closed,
}

/// Handle-free current evidence for one coalesced surface waker.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SurfaceWakeSnapshot {
    requests: u64,
    scheduled: u64,
    coalesced: u64,
    dispatched: u64,
    rejected: u64,
}

impl SurfaceWakeSnapshot {
    /// Returns all attempted wake requests.
    #[must_use]
    pub const fn requests(self) -> u64 {
        self.requests
    }

    /// Returns requests that scheduled a main-thread callback.
    #[must_use]
    pub const fn scheduled(self) -> u64 {
        self.scheduled
    }

    /// Returns requests merged into an already-pending callback.
    #[must_use]
    pub const fn coalesced(self) -> u64 {
        self.coalesced
    }

    /// Returns callbacks dispatched while the surface was live.
    #[must_use]
    pub const fn dispatched(self) -> u64 {
        self.dispatched
    }

    /// Returns requests or callbacks rejected after revocation.
    #[must_use]
    pub const fn rejected(self) -> u64 {
        self.rejected
    }
}

pub(crate) struct SurfaceWakeCounters {
    requests: AtomicU64,
    scheduled: AtomicU64,
    coalesced: AtomicU64,
    dispatched: AtomicU64,
    rejected: AtomicU64,
}

impl SurfaceWakeCounters {
    pub(crate) const fn new() -> Self {
        Self {
            requests: AtomicU64::new(0),
            scheduled: AtomicU64::new(0),
            coalesced: AtomicU64::new(0),
            dispatched: AtomicU64::new(0),
            rejected: AtomicU64::new(0),
        }
    }

    pub(crate) fn snapshot(&self) -> SurfaceWakeSnapshot {
        SurfaceWakeSnapshot {
            requests: self.requests.load(Ordering::Acquire),
            scheduled: self.scheduled.load(Ordering::Acquire),
            coalesced: self.coalesced.load(Ordering::Acquire),
            dispatched: self.dispatched.load(Ordering::Acquire),
            rejected: self.rejected.load(Ordering::Acquire),
        }
    }

    pub(crate) fn request(&self) {
        self.requests.fetch_add(1, Ordering::Relaxed);
    }

    #[cfg(any(test, all(target_os = "macos", target_arch = "aarch64")))]
    pub(crate) fn scheduled(&self) {
        self.scheduled.fetch_add(1, Ordering::Relaxed);
    }

    #[cfg(any(test, all(target_os = "macos", target_arch = "aarch64")))]
    pub(crate) fn coalesced(&self) {
        self.coalesced.fetch_add(1, Ordering::Relaxed);
    }

    #[cfg(any(test, all(target_os = "macos", target_arch = "aarch64")))]
    pub(crate) fn dispatched(&self) {
        self.dispatched.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn rejected(&self) {
        self.rejected.fetch_add(1, Ordering::Relaxed);
    }
}

/// Cloneable, handle-free admission token for waking the owned main loop.
#[derive(Clone)]
pub struct SurfaceWaker {
    request: Arc<dyn Fn() -> SurfaceWakeAdmission + Send + Sync + 'static>,
    counters: Arc<SurfaceWakeCounters>,
}

impl SurfaceWaker {
    pub(crate) fn new(
        request: impl Fn() -> SurfaceWakeAdmission + Send + Sync + 'static,
        counters: Arc<SurfaceWakeCounters>,
    ) -> Self {
        Self {
            request: Arc::new(request),
            counters,
        }
    }

    #[cfg(any(test, not(all(target_os = "macos", target_arch = "aarch64"))))]
    pub(crate) fn closed() -> Self {
        let counters = Arc::new(SurfaceWakeCounters::new());
        Self::new(
            {
                let counters = Arc::clone(&counters);
                move || {
                    counters.request();
                    counters.rejected();
                    SurfaceWakeAdmission::Closed
                }
            },
            counters,
        )
    }

    /// Requests one coalesced main-loop callback without requesting a frame.
    #[must_use]
    pub fn wake(&self) -> SurfaceWakeAdmission {
        (self.request)()
    }

    /// Returns handle-free wake admission evidence.
    #[must_use]
    pub fn snapshot(&self) -> SurfaceWakeSnapshot {
        self.counters.snapshot()
    }
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

    /// Runs `AppKit` while dispatching handle-free events on the main thread.
    ///
    /// A handler returns one bounded response. Repeated frame requests retain
    /// the surface's existing latest-wins coalescing and bounded presentation
    /// path. Clipboard writes complete through a later [`ClipboardEvent`].
    ///
    /// # Errors
    ///
    /// Returns the same structured lifecycle and native errors as [`Self::run`].
    pub fn run_with_event_handler<F>(&self, handler: F) -> Result<(), SurfaceError>
    where
        F: FnMut(SurfaceEvent) -> SurfaceResponse + 'static,
    {
        self.implementation.run_with_event_handler(handler)
    }

    /// Returns a thread-safe token that wakes this surface's main loop.
    #[must_use]
    pub fn waker(&self) -> SurfaceWaker {
        self.implementation.waker()
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
    /// Returns [`SurfaceError::OwnerConflict`] if synchronized callback state
    /// cannot be inspected.
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

#[cfg(test)]
mod surface_waker_tests {
    use super::*;

    #[test]
    fn handle_free_waker_reports_exact_admission_evidence() {
        let counters = Arc::new(SurfaceWakeCounters::new());
        let callback_counters = Arc::clone(&counters);
        let waker = SurfaceWaker::new(
            move || {
                callback_counters.request();
                callback_counters.scheduled();
                SurfaceWakeAdmission::Scheduled
            },
            Arc::clone(&counters),
        );

        assert_eq!(waker.wake(), SurfaceWakeAdmission::Scheduled);
        assert_eq!(waker.clone().wake(), SurfaceWakeAdmission::Scheduled);
        for _ in 0..3 {
            counters.coalesced();
        }
        for _ in 0..4 {
            counters.dispatched();
        }
        for _ in 0..5 {
            counters.rejected();
        }
        let evidence = waker.snapshot();
        assert_eq!(evidence.requests(), 2);
        assert_eq!(evidence.scheduled(), 2);
        assert_eq!(evidence.coalesced(), 3);
        assert_eq!(evidence.dispatched(), 4);
        assert_eq!(evidence.rejected(), 5);
    }

    #[test]
    fn closed_waker_rejects_without_native_handles() {
        let waker = SurfaceWaker::closed();
        assert_eq!(waker.wake(), SurfaceWakeAdmission::Closed);
        assert_eq!(waker.snapshot().requests(), 1);
        assert_eq!(waker.snapshot().rejected(), 1);
    }
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
    fn event_and_frame_values_preserve_public_identity() -> Result<(), SurfaceError> {
        assert_eq!(Modifiers::SHIFT, 1);
        assert_eq!(Modifiers::CONTROL, 2);
        assert_eq!(Modifiers::OPTION, 4);
        assert_eq!(Modifiers::COMMAND, 8);
        assert_eq!(Modifiers::CAPS_LOCK, 16);
        let modifiers = Modifiers::from_bits(u8::MAX);
        assert_eq!(modifiers.bits(), 0x1f);
        assert!(modifiers.contains(Modifiers::SHIFT | Modifiers::COMMAND));
        assert!(!modifiers.contains(1 << 5));

        let timestamp = EventTimestamp::new(23);
        let extent = SurfaceExtent::new(40.0, 20.0, 2.0)?;
        let position =
            Point::new(3.0, 4.0).ok_or(SurfaceError::invariant(SurfaceOperation::Application))?;
        let events = [
            SurfaceEvent::Keyboard {
                timestamp,
                state: KeyState::Down,
                physical_key: 4,
                logical_key: "a".into(),
                modifiers,
                repeat: false,
            },
            SurfaceEvent::Pointer {
                timestamp,
                action: PointerAction::Moved,
                position,
                button: PointerButton::None,
                modifiers,
            },
            SurfaceEvent::Scroll {
                timestamp,
                delta_x: 1.0,
                delta_y: -2.0,
                phase: ScrollPhase::Changed,
                precise: true,
                modifiers,
            },
            SurfaceEvent::Focus {
                timestamp,
                input_epoch: InputEpoch::INITIAL,
                focused: true,
            },
            SurfaceEvent::Resize { timestamp, extent },
            SurfaceEvent::Clipboard {
                timestamp,
                event: ClipboardEvent::CopyCompleted(Ok(())),
            },
            SurfaceEvent::Ime {
                timestamp,
                input_epoch: InputEpoch::INITIAL,
                event: ImeEvent::Started,
            },
            SurfaceEvent::Wake { timestamp },
            SurfaceEvent::CloseRequested { timestamp },
        ];
        for event in events {
            assert_eq!(event.timestamp().get(), 23);
        }
        assert_eq!(extent.logical_size(), Size::new(40.0, 20.0));

        let scene = alpine_scene::SceneBuilder::new(
            alpine_scene::SceneRevision::new(5),
            Size::new(40.0, 20.0).ok_or(SurfaceError::invariant(SurfaceOperation::Application))?,
        )
        .finish();
        let clear = LinearRgba::new(0.1, 0.2, 0.3, 1.0)
            .ok_or(SurfaceError::invariant(SurfaceOperation::Application))?;
        let frame = SurfaceFrame::new(scene.clone(), clear);
        assert_eq!(frame.scene(), &scene);
        assert_eq!(frame.clear(), clear);
        let response = SurfaceResponse::from(Some(frame.clone()));
        assert_eq!(response.frame(), Some(&frame));
        assert_eq!(response.into_frame(), Some(frame.clone()));
        assert_eq!(frame.into_parts(), (scene, clear));
        Ok(())
    }

    #[test]
    fn clipboard_and_response_values_preserve_public_identity() -> Result<(), SurfaceError> {
        assert_eq!(MAX_CLIPBOARD_TEXT_BYTES, 67_108_864);
        let text = ClipboardText::new("bounded")
            .map_err(|_| SurfaceError::invariant(SurfaceOperation::Application))?;
        assert_eq!(text.as_str(), "bounded");
        assert_eq!(text.clone().into_inner().as_ref(), "bounded");
        let write = ClipboardWrite::new(ClipboardOperation::Copy, text.clone())
            .map_err(|_| SurfaceError::invariant(SurfaceOperation::Application))?;
        assert_eq!(write.operation(), ClipboardOperation::Copy);
        assert_eq!(write.text(), &text);
        assert_eq!(
            write.clone().into_parts(),
            (ClipboardOperation::Copy, text.clone())
        );
        assert_eq!(
            ClipboardWrite::new(ClipboardOperation::Paste, text.clone()),
            Err(ClipboardError::InvalidWriteOperation)
        );
        let oversized: Box<str> = "x".repeat(MAX_CLIPBOARD_TEXT_BYTES + 1).into_boxed_str();
        assert_eq!(
            ClipboardText::new(oversized),
            Err(ClipboardError::TooLarge {
                bytes: MAX_CLIPBOARD_TEXT_BYTES + 1,
                limit: MAX_CLIPBOARD_TEXT_BYTES,
            })
        );
        assert_eq!(
            ClipboardError::Unavailable.to_string(),
            "plain UTF-8 clipboard text is unavailable"
        );
        assert_eq!(
            ClipboardError::WriteRejected.to_string(),
            "the platform rejected the clipboard write"
        );
        assert_eq!(
            ClipboardError::InvalidWriteOperation.to_string(),
            "paste cannot be returned as a clipboard write"
        );
        assert_eq!(
            ClipboardError::TooLarge { bytes: 9, limit: 8 }.to_string(),
            "clipboard text has 9 bytes; limit is 8"
        );
        assert_eq!(
            ClipboardEvent::CopyCompleted(Ok(())).operation(),
            ClipboardOperation::Copy
        );
        assert_eq!(
            ClipboardEvent::CutCompleted(Err(ClipboardError::WriteRejected)).operation(),
            ClipboardOperation::Cut
        );
        assert_eq!(
            ClipboardEvent::PasteCompleted(Ok(text.clone())).operation(),
            ClipboardOperation::Paste
        );

        let response = SurfaceResponse::new(None, Some(write.clone()), CloseDisposition::Cancel);
        assert!(response.frame().is_none());
        assert_eq!(response.clipboard_write(), Some(&write));
        assert_eq!(response.close_disposition(), CloseDisposition::Cancel);
        assert_eq!(
            response.into_parts(),
            (None, Some(write), CloseDisposition::Cancel)
        );
        assert!(SurfaceResponse::from(None).into_frame().is_none());
        Ok(())
    }

    #[test]
    fn accessibility_event_and_response_channel_preserve_identity() -> Result<(), Box<dyn Error>> {
        let timestamp = EventTimestamp::new(29);
        let request = AccessibilityRequest::snapshot(AccessibilityRequestId::new(7))?;
        let event = SurfaceEvent::Accessibility {
            timestamp,
            request: request.clone(),
        };
        assert_eq!(event.timestamp(), timestamp);

        let response = AccessibilityResponse::failure(
            &request,
            AccessibilityRevision::new(3, 5),
            AccessibilityError::InvalidTree,
        );
        let surface = SurfaceResponse::from_channels(
            None,
            None,
            CloseDisposition::NotRequested,
            Some(response.clone()),
        );
        assert_eq!(surface.accessibility_response(), Some(&response));
        assert_eq!(
            surface.into_channels(),
            (None, None, CloseDisposition::NotRequested, Some(response))
        );
        Ok(())
    }

    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    #[test]
    fn public_event_loop_wrapper_preserves_unsupported_error() {
        let surface = NativeSurface::from_implementation(implementation::NativeSurface);
        let waker = surface.waker();
        assert_eq!(waker.wake(), SurfaceWakeAdmission::Closed);
        assert_eq!(
            surface.run_with_event_handler(|_| SurfaceResponse::default()),
            Err(SurfaceError::UnsupportedPlatform)
        );
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
    #[allow(clippy::too_many_lines)]
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
            SurfaceError::NativeUnavailable {
                stage: SurfaceStage::Device,
            }
            .to_string(),
            "Metal device unavailable at Device stage"
        );
        assert_eq!(
            SurfaceError::owner_conflict(SurfaceOperation::Presentation).to_string(),
            "synchronized Presentation owner is unavailable"
        );
        assert_eq!(
            SurfaceError::invariant(SurfaceOperation::Input).to_string(),
            "Input invariant failed"
        );
        assert_eq!(
            SurfaceError::InputResponderRejected.to_string(),
            "AppKit rejected Alpine input responder activation"
        );
        assert_eq!(
            SurfaceError::validation(SurfaceOperation::Validation).to_string(),
            "native validation failed during Validation"
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
                SurfaceError::invariant(SurfaceOperation::Application),
                "Application invariant failed",
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
        let latency = FrameLatencyEvidence::new(
            EventTimestamp::new(67),
            71,
            Some(73),
            Some(79),
            Some(83),
            Some(89),
            97,
        );
        let terminal = FrameTerminalEvidence::new(
            attempt,
            73,
            79,
            83,
            Some(latency),
            89,
            Some(RecoveryClassification::RetryFrame),
        );
        assert_eq!(latency.event_timestamp(), EventTimestamp::new(67));
        assert_eq!(latency.event_handler_ns(), 71);
        assert_eq!(latency.frame_queue_ns(), Some(73));
        assert_eq!(latency.submission_ns(), Some(79));
        assert_eq!(latency.event_to_gpu_terminal_observed_ns(), Some(83));
        assert_eq!(latency.event_to_presented_handler_ns(), Some(89));
        assert_eq!(latency.event_to_terminal_record_ns(), 97);
        assert_eq!(terminal.latency(), Some(latency));

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
        let failed_terminal =
            FrameTerminalEvidence::new(failed_attempt, 97, 101, 0, None, 103, None);
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
            #[cfg(alpine_native_validation)]
            pause_confirmation_count: 25,
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
            peak_retained_bytes: 59,
            current_retained_bytes: 53,
            current_upload_bytes: 31,
            peak_upload_bytes: 37,
            frame_slot_capacity: 3,
            occupied_frame_slots: 2,
            submitted_frame_slots: 1,
            peak_occupied_frame_slots: 3,
            frame_slot_saturation_count: 59,
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
            #[cfg(alpine_native_validation)]
            pause_confirmation_count: 41,
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
            peak_retained_bytes: 73,
            current_retained_bytes: 71,
            current_upload_bytes: 41,
            peak_upload_bytes: 43,
            frame_slot_capacity: 3,
            occupied_frame_slots: 0,
            submitted_frame_slots: 0,
            peak_occupied_frame_slots: 2,
            frame_slot_saturation_count: 73,
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
        assert_eq!(snapshot.peak_retained_bytes(), 59);
        assert_eq!(snapshot.current_upload_bytes(), 31);
        assert_eq!(snapshot.peak_upload_bytes(), 37);
        assert_eq!(snapshot.frame_slot_capacity(), 3);
        assert_eq!(snapshot.occupied_frame_slots(), 2);
        assert_eq!(snapshot.submitted_frame_slots(), 1);
        assert_eq!(snapshot.peak_occupied_frame_slots(), 3);
        assert_eq!(snapshot.frame_slot_saturation_count(), 59);
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
        assert_eq!(inverse.peak_retained_bytes(), 73);
        assert_eq!(inverse.current_upload_bytes(), 41);
        assert_eq!(inverse.peak_upload_bytes(), 43);
        assert_eq!(inverse.frame_slot_capacity(), 3);
        assert_eq!(inverse.occupied_frame_slots(), 0);
        assert_eq!(inverse.submitted_frame_slots(), 0);
        assert_eq!(inverse.peak_occupied_frame_slots(), 2);
        assert_eq!(inverse.frame_slot_saturation_count(), 73);
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
        let viewport = alpine_core::Size::new(1.0, 1.0)
            .ok_or(SurfaceError::invariant(SurfaceOperation::Application))?;
        let scene = SceneBuilder::new(SceneRevision::new(1), viewport).finish();
        let clear = LinearRgba::new(0.0, 0.0, 0.0, 1.0)
            .ok_or(SurfaceError::invariant(SurfaceOperation::Application))?;

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
