//! Bounded, non-shipping accessibility client contracts for physical assurance.

use std::{error::Error, fmt, time::Duration};

#[cfg(target_os = "macos")]
mod native;
#[cfg(target_os = "macos")]
pub use native::NativeAxClient;
#[cfg(target_os = "macos")]
mod native_factory;
#[cfg(target_os = "macos")]
pub use native_factory::NativeAxClientFactory;

/// Maximum admitted accessibility nodes in one snapshot.
pub const MAX_NODE_LIMIT: usize = 16_384;
/// Maximum admitted observer notifications retained between drains.
pub const MAX_EVENT_LIMIT: usize = 65_536;
/// Maximum admitted observer registrations for one process generation.
pub const MAX_REGISTRATION_LIMIT: usize = 65_536;
/// Maximum admitted accessibility hierarchy depth.
pub const MAX_DEPTH_LIMIT: u16 = 128;
/// Maximum admitted UTF-8 bytes retained for one queried text value.
pub const MAX_VALUE_BYTE_LIMIT: usize = 1_048_576;
/// Maximum admitted AX messaging timeout.
pub const MAX_MESSAGING_TIMEOUT: Duration = Duration::from_secs(5);

/// One nonzero Studio lifecycle generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AxGeneration(u64);

impl AxGeneration {
    /// Constructs a nonzero lifecycle generation.
    ///
    /// # Errors
    ///
    /// Returns [`AxClientError::InvalidGeneration`] when `value` is zero.
    pub fn new(value: u64) -> Result<Self, AxClientError> {
        if value == 0 {
            return Err(AxClientError::InvalidGeneration);
        }
        Ok(Self(value))
    }

    /// Returns the underlying monotonic generation value.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Explicit bounds for one attached accessibility client.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AxLimits {
    node_limit: usize,
    event_limit: usize,
    registration_limit: usize,
    depth_limit: u16,
    value_byte_limit: usize,
    messaging_timeout: Duration,
}

impl AxLimits {
    /// Validates and constructs accessibility capture bounds.
    ///
    /// # Errors
    ///
    /// Returns a structured limit or timeout error when any value is zero or
    /// exceeds its protocol ceiling.
    pub fn new(
        node_limit: usize,
        event_limit: usize,
        registration_limit: usize,
        depth_limit: u16,
        value_byte_limit: usize,
        messaging_timeout: Duration,
    ) -> Result<Self, AxClientError> {
        validate_limit("node", node_limit, MAX_NODE_LIMIT)?;
        validate_limit("event", event_limit, MAX_EVENT_LIMIT)?;
        validate_limit("registration", registration_limit, MAX_REGISTRATION_LIMIT)?;
        if depth_limit == 0 || depth_limit > MAX_DEPTH_LIMIT {
            return Err(AxClientError::InvalidLimit {
                name: "depth",
                value: usize::from(depth_limit),
                maximum: usize::from(MAX_DEPTH_LIMIT),
            });
        }
        validate_limit("value-byte", value_byte_limit, MAX_VALUE_BYTE_LIMIT)?;
        if messaging_timeout.is_zero() || messaging_timeout > MAX_MESSAGING_TIMEOUT {
            return Err(AxClientError::InvalidTimeout);
        }
        Ok(Self {
            node_limit,
            event_limit,
            registration_limit,
            depth_limit,
            value_byte_limit,
            messaging_timeout,
        })
    }

    /// Returns the admitted node count.
    #[must_use]
    pub const fn node_limit(self) -> usize {
        self.node_limit
    }

    /// Returns the admitted event count.
    #[must_use]
    pub const fn event_limit(self) -> usize {
        self.event_limit
    }

    /// Returns the admitted registration count.
    #[must_use]
    pub const fn registration_limit(self) -> usize {
        self.registration_limit
    }

    /// Returns the admitted hierarchy depth.
    #[must_use]
    pub const fn depth_limit(self) -> u16 {
        self.depth_limit
    }

    /// Returns the admitted bytes for one text value.
    #[must_use]
    pub const fn value_byte_limit(self) -> usize {
        self.value_byte_limit
    }

    /// Returns the AX messaging timeout.
    #[must_use]
    pub const fn messaging_timeout(self) -> Duration {
        self.messaging_timeout
    }
}

/// One CoreFoundation text range returned by the target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AxTextRange {
    /// UTF-16 location reported by `AppKit`.
    pub location: u64,
    /// UTF-16 length reported by `AppKit`.
    pub length: u64,
}

/// One top-left accessibility rectangle in screen points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AxRect {
    /// Horizontal origin.
    pub x: f64,
    /// Vertical origin.
    pub y: f64,
    /// Width in points.
    pub width: f64,
    /// Height in points.
    pub height: f64,
}

/// One bounded, handle-free accessibility tree node.
#[derive(Clone, Debug, PartialEq)]
pub struct AxNode {
    /// Stable Alpine accessibility identifier.
    pub identifier: String,
    /// Stable parent identifier, absent only for the application root.
    pub parent_identifier: Option<String>,
    /// Preorder hierarchy depth.
    pub depth: u16,
    /// Native AX role.
    pub role: String,
    /// Bounded title or description.
    pub label: String,
    /// Bounded textual value when represented by CoreFoundation text.
    pub value: Option<String>,
    /// Whether `AppKit` reports native focus on this node.
    pub focused: bool,
    /// Bounded selected text when available.
    pub selected_text: Option<String>,
    /// Selected UTF-16 range when available.
    pub selected_range: Option<AxTextRange>,
    /// Native position and size when both are available.
    pub frame: Option<AxRect>,
    /// Bounded action names reported by `AppKit`.
    pub enabled_actions: Vec<String>,
}

/// The only actions admitted by the physical qualification protocol.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AxAction {
    /// Activate a pressable control.
    Press,
    /// Confirm the current control.
    Confirm,
    /// Show the current control's menu.
    ShowMenu,
}

impl AxAction {
    /// Returns the native AX action string.
    #[must_use]
    pub const fn native_name(self) -> &'static str {
        match self {
            Self::Press => "AXPress",
            Self::Confirm => "AXConfirm",
            Self::ShowMenu => "AXShowMenu",
        }
    }
}

/// Approved AX observer event classes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AxNotificationKind {
    /// Native focus changed.
    Focus,
    /// Native value changed.
    Value,
    /// Native text selection changed.
    Selection,
    /// Native accessibility layout changed.
    Layout,
    /// Native announcement was requested.
    Announcement,
    /// A native window was minimized.
    Minimized,
    /// A native window was restored.
    Restored,
    /// A native accessibility element was destroyed.
    Destroyed,
}

impl AxNotificationKind {
    /// Returns the native AX notification string.
    #[must_use]
    pub const fn native_name(self) -> &'static str {
        match self {
            Self::Focus => "AXFocusedUIElementChanged",
            Self::Value => "AXValueChanged",
            Self::Selection => "AXSelectedTextChanged",
            Self::Layout => "AXLayoutChanged",
            Self::Announcement => "AXAnnouncementRequested",
            Self::Minimized => "AXWindowMiniaturized",
            Self::Restored => "AXWindowDeminiaturized",
            Self::Destroyed => "AXUIElementDestroyed",
        }
    }

    #[cfg(any(target_os = "macos", test))]
    const ALL: [Self; 8] = [
        Self::Focus,
        Self::Value,
        Self::Selection,
        Self::Layout,
        Self::Announcement,
        Self::Minimized,
        Self::Restored,
        Self::Destroyed,
    ];
}

/// One observer notification normalized outside the native callback.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AxObservedEvent {
    /// Studio lifecycle generation that admitted the callback.
    pub generation: AxGeneration,
    /// Approved notification class.
    pub kind: AxNotificationKind,
    /// Stable identifier captured before native destruction.
    pub identifier: String,
    /// Monotonic nanoseconds since this client attached.
    pub monotonic_ns: u64,
}

/// One bounded observer drain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AxEventBatch {
    /// Accepted events in callback order.
    pub events: Vec<AxObservedEvent>,
    /// Events omitted because the admitted queue was full or unrecognized.
    pub omitted_events: usize,
    /// Events rejected because they belonged to another generation.
    pub stale_events: usize,
}

/// Structured result of querying one retained stale element.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AxQueryResult {
    /// Native AX error code, where zero is success.
    pub ax_error: i32,
}

/// Structured accessibility client failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AxClientError {
    /// The current target does not support the native client.
    UnsupportedPlatform,
    /// Accessibility permission is absent and was not prompted for.
    AccessibilityUntrusted,
    /// The requested PID is not positive.
    InvalidPid,
    /// A returned element belongs to another process.
    PidMismatch {
        /// PID requested by the capture orchestrator.
        expected: i32,
        /// PID returned by the native accessibility element.
        actual: i32,
    },
    /// A lifecycle generation was zero.
    InvalidGeneration,
    /// A numeric bound was zero or exceeded the protocol ceiling.
    InvalidLimit {
        /// Protocol-bound quantity.
        name: &'static str,
        /// Rejected value.
        value: usize,
        /// Inclusive protocol ceiling.
        maximum: usize,
    },
    /// The AX messaging timeout was zero or exceeded the protocol ceiling.
    InvalidTimeout,
    /// A native operation returned an AX error.
    Native {
        /// Stable Alpine operation name.
        operation: &'static str,
        /// Native `AXError` value.
        code: i32,
    },
    /// A required native attribute was absent.
    MissingAttribute {
        /// Required native attribute.
        attribute: &'static str,
        /// Stable identifier when already available.
        identifier: Option<String>,
    },
    /// A native value had an unexpected CoreFoundation type.
    InvalidAttributeType {
        /// Native attribute with an unexpected CoreFoundation type.
        attribute: &'static str,
    },
    /// A stable identifier appeared more than once.
    DuplicateIdentifier(String),
    /// The native tree exceeded an admitted bound.
    TreeBoundExceeded {
        /// Exceeded tree quantity.
        name: &'static str,
        /// Admitted inclusive limit.
        limit: usize,
    },
    /// One queried text value exceeded its admitted byte ceiling.
    ValueBoundExceeded {
        /// Native text attribute.
        attribute: &'static str,
        /// Admitted UTF-8 byte ceiling.
        limit: usize,
    },
    /// The requested stable identifier is not in the attached snapshot.
    UnknownIdentifier(String),
    /// No element has been retained for the stale-query control.
    MissingStaleElement,
    /// An operation used a generation other than the attached generation.
    StaleGeneration {
        /// Attached lifecycle generation.
        expected: u64,
        /// Rejected lifecycle generation.
        actual: u64,
    },
    /// The client has already been closed.
    Closed,
}

impl fmt::Display for AxClientError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedPlatform => formatter.write_str("AX client requires macOS"),
            Self::AccessibilityUntrusted => {
                formatter.write_str("Accessibility permission is absent; no prompt was issued")
            }
            Self::InvalidPid => formatter.write_str("AX target PID must be positive"),
            Self::PidMismatch { expected, actual } => {
                write!(
                    formatter,
                    "AX element PID {actual} does not match target {expected}"
                )
            }
            Self::InvalidGeneration => formatter.write_str("AX generation must be nonzero"),
            Self::InvalidLimit {
                name,
                value,
                maximum,
            } => write!(
                formatter,
                "AX {name} limit {value} must be within 1..={maximum}"
            ),
            Self::InvalidTimeout => {
                let maximum = MAX_MESSAGING_TIMEOUT;
                write!(
                    formatter,
                    "AX messaging timeout must be within 1 ns..={maximum:?}"
                )
            }
            Self::Native { operation, code } => {
                write!(formatter, "AX operation {operation} failed with {code}")
            }
            Self::MissingAttribute {
                attribute,
                identifier,
            } => write!(
                formatter,
                "AX attribute {attribute} is absent on {}",
                identifier.as_deref().unwrap_or("unidentified element")
            ),
            Self::InvalidAttributeType { attribute } => {
                write!(formatter, "AX attribute {attribute} has an unexpected type")
            }
            Self::DuplicateIdentifier(identifier) => {
                write!(formatter, "AX identifier {identifier:?} is duplicated")
            }
            Self::TreeBoundExceeded { name, limit } => {
                write!(formatter, "AX tree exceeded {name} limit {limit}")
            }
            Self::ValueBoundExceeded { attribute, limit } => write!(
                formatter,
                "AX attribute {attribute} exceeded value-byte limit {limit}"
            ),
            Self::UnknownIdentifier(identifier) => {
                write!(
                    formatter,
                    "AX identifier {identifier:?} is not in the snapshot"
                )
            }
            Self::MissingStaleElement => {
                formatter.write_str("no AX element is retained for stale-query control")
            }
            Self::StaleGeneration { expected, actual } => write!(
                formatter,
                "AX generation {actual} does not match attached generation {expected}"
            ),
            Self::Closed => formatter.write_str("AX client is closed"),
        }
    }
}

impl Error for AxClientError {}

/// Handle-free client behavior consumed by the physical capture orchestrator.
pub trait AxClient {
    /// Returns the attached lifecycle generation.
    fn generation(&self) -> AxGeneration;
    /// Captures one bounded preorder tree and installs approved observers.
    ///
    /// # Errors
    ///
    /// Returns a structured native, identity, type, or bound error.
    fn snapshot_tree(&mut self) -> Result<Vec<AxNode>, AxClientError>;
    /// Runs the observer source for at most `timeout` and drains accepted events.
    ///
    /// # Errors
    ///
    /// Returns an error for a closed client, stale generation, invalid timeout,
    /// PID mismatch, or unknown callback identity.
    fn drain_events(
        &mut self,
        generation: AxGeneration,
        timeout: Duration,
    ) -> Result<AxEventBatch, AxClientError>;
    /// Performs one allowlisted action on a stable identifier.
    ///
    /// # Errors
    ///
    /// Returns an error for a stale generation, unknown identity, closed
    /// client, or native PID query failure.
    fn perform_action(
        &mut self,
        generation: AxGeneration,
        identifier: &str,
        action: AxAction,
    ) -> Result<i32, AxClientError>;
    /// Retains one current-generation element for the destruction control.
    ///
    /// # Errors
    ///
    /// Returns an error for a stale generation, unknown identity, or closed
    /// client.
    fn retain_for_stale_query(
        &mut self,
        generation: AxGeneration,
        identifier: &str,
    ) -> Result<(), AxClientError>;
    /// Queries the retained element without mutating target state.
    ///
    /// # Errors
    ///
    /// Returns an error for a stale generation, missing retained element, or
    /// closed client.
    fn query_retained_stale(
        &mut self,
        generation: AxGeneration,
    ) -> Result<AxQueryResult, AxClientError>;
    /// Removes observers and releases native ownership deterministically.
    ///
    /// # Errors
    ///
    /// Returns an error for a stale generation or already closed client.
    fn close(&mut self, generation: AxGeneration) -> Result<(), AxClientError>;
}

/// Factory behavior that separates fake orchestration from native ownership.
pub trait AxClientFactory {
    /// Concrete client returned after successful attachment.
    type Client: AxClient;
    /// Reports trust without prompting or changing system privacy state.
    ///
    /// # Errors
    ///
    /// Returns [`AxClientError::UnsupportedPlatform`] outside macOS.
    fn is_trusted(&self) -> Result<bool, AxClientError>;
    /// Attaches one bounded client to an exact positive PID.
    ///
    /// # Errors
    ///
    /// Returns a structured trust, PID, platform, or native ownership error.
    fn attach(
        &self,
        pid: i32,
        generation: AxGeneration,
        limits: AxLimits,
    ) -> Result<Self::Client, AxClientError>;
}

/// Native generated-binding factory on macOS and an unsupported stub elsewhere.
#[cfg(not(target_os = "macos"))]
#[derive(Clone, Copy, Debug, Default)]
pub struct NativeAxClientFactory;

#[cfg(not(target_os = "macos"))]
impl AxClientFactory for NativeAxClientFactory {
    type Client = UnsupportedAxClient;

    fn is_trusted(&self) -> Result<bool, AxClientError> {
        Err(AxClientError::UnsupportedPlatform)
    }

    fn attach(
        &self,
        _pid: i32,
        _generation: AxGeneration,
        _limits: AxLimits,
    ) -> Result<Self::Client, AxClientError> {
        Err(AxClientError::UnsupportedPlatform)
    }
}

/// Non-macOS placeholder that cannot perform native operations.
#[cfg(not(target_os = "macos"))]
#[derive(Debug)]
pub struct UnsupportedAxClient {
    generation: AxGeneration,
}

#[cfg(not(target_os = "macos"))]
impl AxClient for UnsupportedAxClient {
    fn generation(&self) -> AxGeneration {
        self.generation
    }

    fn snapshot_tree(&mut self) -> Result<Vec<AxNode>, AxClientError> {
        Err(AxClientError::UnsupportedPlatform)
    }

    fn drain_events(
        &mut self,
        _generation: AxGeneration,
        _timeout: Duration,
    ) -> Result<AxEventBatch, AxClientError> {
        Err(AxClientError::UnsupportedPlatform)
    }

    fn perform_action(
        &mut self,
        _generation: AxGeneration,
        _identifier: &str,
        _action: AxAction,
    ) -> Result<i32, AxClientError> {
        Err(AxClientError::UnsupportedPlatform)
    }

    fn retain_for_stale_query(
        &mut self,
        _generation: AxGeneration,
        _identifier: &str,
    ) -> Result<(), AxClientError> {
        Err(AxClientError::UnsupportedPlatform)
    }

    fn query_retained_stale(
        &mut self,
        _generation: AxGeneration,
    ) -> Result<AxQueryResult, AxClientError> {
        Err(AxClientError::UnsupportedPlatform)
    }

    fn close(&mut self, _generation: AxGeneration) -> Result<(), AxClientError> {
        Err(AxClientError::UnsupportedPlatform)
    }
}

fn validate_limit(name: &'static str, value: usize, maximum: usize) -> Result<(), AxClientError> {
    if value == 0 || value > maximum {
        return Err(AxClientError::InvalidLimit {
            name,
            value,
            maximum,
        });
    }
    Ok(())
}

#[cfg(test)]
#[allow(
    clippy::panic,
    reason = "fixture setup failures are unrecoverable test harness defects"
)]
mod tests {
    use super::{
        AxAction, AxClient, AxClientError, AxClientFactory, AxEventBatch, AxGeneration, AxLimits,
        AxNode, AxNotificationKind, AxObservedEvent, AxQueryResult, MAX_DEPTH_LIMIT,
        MAX_EVENT_LIMIT, MAX_MESSAGING_TIMEOUT, MAX_NODE_LIMIT, MAX_REGISTRATION_LIMIT,
        MAX_VALUE_BYTE_LIMIT,
    };
    use std::{collections::BTreeSet, time::Duration};

    struct FakeFactory {
        trusted: bool,
        pid: i32,
        nodes: Vec<AxNode>,
        events: Vec<AxObservedEvent>,
    }

    struct FakeClient {
        generation: AxGeneration,
        limits: AxLimits,
        nodes: Vec<AxNode>,
        events: Vec<AxObservedEvent>,
        identifiers: BTreeSet<String>,
        retained: Option<String>,
        closed: bool,
    }

    impl AxClientFactory for FakeFactory {
        type Client = FakeClient;

        fn is_trusted(&self) -> Result<bool, AxClientError> {
            Ok(self.trusted)
        }

        fn attach(
            &self,
            pid: i32,
            generation: AxGeneration,
            limits: AxLimits,
        ) -> Result<Self::Client, AxClientError> {
            if !self.trusted {
                return Err(AxClientError::AccessibilityUntrusted);
            }
            if pid <= 0 {
                return Err(AxClientError::InvalidPid);
            }
            if pid != self.pid {
                return Err(AxClientError::PidMismatch {
                    expected: pid,
                    actual: self.pid,
                });
            }
            Ok(FakeClient {
                generation,
                limits,
                nodes: self.nodes.clone(),
                events: self.events.clone(),
                identifiers: BTreeSet::new(),
                retained: None,
                closed: false,
            })
        }
    }

    impl FakeClient {
        fn require_open_generation(&self, generation: AxGeneration) -> Result<(), AxClientError> {
            if self.closed {
                return Err(AxClientError::Closed);
            }
            if generation != self.generation {
                return Err(AxClientError::StaleGeneration {
                    expected: self.generation.get(),
                    actual: generation.get(),
                });
            }
            Ok(())
        }
    }

    impl AxClient for FakeClient {
        fn generation(&self) -> AxGeneration {
            self.generation
        }

        fn snapshot_tree(&mut self) -> Result<Vec<AxNode>, AxClientError> {
            if self.closed {
                return Err(AxClientError::Closed);
            }
            if self.nodes.len() > self.limits.node_limit() {
                return Err(AxClientError::TreeBoundExceeded {
                    name: "node",
                    limit: self.limits.node_limit(),
                });
            }
            self.identifiers.clear();
            for node in &self.nodes {
                if node.depth > self.limits.depth_limit() {
                    return Err(AxClientError::TreeBoundExceeded {
                        name: "depth",
                        limit: usize::from(self.limits.depth_limit()),
                    });
                }
                if !self.identifiers.insert(node.identifier.clone()) {
                    return Err(AxClientError::DuplicateIdentifier(node.identifier.clone()));
                }
            }
            Ok(self.nodes.clone())
        }

        fn drain_events(
            &mut self,
            generation: AxGeneration,
            _timeout: Duration,
        ) -> Result<AxEventBatch, AxClientError> {
            self.require_open_generation(generation)?;
            let mut accepted = Vec::new();
            let mut stale_events = 0;
            let mut omitted_events = 0;
            for event in self.events.drain(..) {
                if event.generation != self.generation {
                    stale_events += 1;
                } else if accepted.len() == self.limits.event_limit() {
                    omitted_events += 1;
                } else {
                    accepted.push(event);
                }
            }
            Ok(AxEventBatch {
                events: accepted,
                omitted_events,
                stale_events,
            })
        }

        fn perform_action(
            &mut self,
            generation: AxGeneration,
            identifier: &str,
            _action: AxAction,
        ) -> Result<i32, AxClientError> {
            self.require_open_generation(generation)?;
            if !self.identifiers.contains(identifier) {
                return Err(AxClientError::UnknownIdentifier(identifier.to_owned()));
            }
            Ok(0)
        }

        fn retain_for_stale_query(
            &mut self,
            generation: AxGeneration,
            identifier: &str,
        ) -> Result<(), AxClientError> {
            self.require_open_generation(generation)?;
            if !self.identifiers.contains(identifier) {
                return Err(AxClientError::UnknownIdentifier(identifier.to_owned()));
            }
            self.retained = Some(identifier.to_owned());
            Ok(())
        }

        fn query_retained_stale(
            &mut self,
            generation: AxGeneration,
        ) -> Result<AxQueryResult, AxClientError> {
            self.require_open_generation(generation)?;
            self.retained
                .as_ref()
                .ok_or(AxClientError::MissingStaleElement)?;
            Ok(AxQueryResult { ax_error: -25_202 })
        }

        fn close(&mut self, generation: AxGeneration) -> Result<(), AxClientError> {
            self.require_open_generation(generation)?;
            self.closed = true;
            self.identifiers.clear();
            self.retained = None;
            Ok(())
        }
    }

    fn limits() -> AxLimits {
        AxLimits::new(3, 2, 24, 3, 128, Duration::from_millis(100))
            .unwrap_or_else(|error| panic!("valid limits: {error}"))
    }

    fn generation(value: u64) -> AxGeneration {
        AxGeneration::new(value).unwrap_or_else(|error| panic!("valid generation: {error}"))
    }

    fn node(identifier: &str, depth: u16) -> AxNode {
        AxNode {
            identifier: identifier.to_owned(),
            parent_identifier: (depth > 0).then(|| "root".to_owned()),
            depth,
            role: if depth == 0 {
                "AXApplication".to_owned()
            } else {
                "AXTextArea".to_owned()
            },
            label: String::new(),
            value: None,
            focused: depth > 0,
            selected_text: None,
            selected_range: None,
            frame: None,
            enabled_actions: vec!["AXConfirm".to_owned()],
        }
    }

    #[test]
    fn limits_and_generations_fail_closed() {
        assert_eq!(AxGeneration::new(0), Err(AxClientError::InvalidGeneration));
        let maximum = AxLimits::new(
            MAX_NODE_LIMIT,
            MAX_EVENT_LIMIT,
            MAX_REGISTRATION_LIMIT,
            MAX_DEPTH_LIMIT,
            MAX_VALUE_BYTE_LIMIT,
            MAX_MESSAGING_TIMEOUT,
        )
        .unwrap_or_else(|error| panic!("maximum limits: {error}"));
        assert_eq!(maximum.node_limit(), MAX_NODE_LIMIT);
        assert_eq!(maximum.event_limit(), MAX_EVENT_LIMIT);
        assert_eq!(maximum.registration_limit(), MAX_REGISTRATION_LIMIT);
        assert_eq!(maximum.depth_limit(), MAX_DEPTH_LIMIT);
        assert_eq!(maximum.value_byte_limit(), MAX_VALUE_BYTE_LIMIT);
        assert_eq!(maximum.messaging_timeout(), MAX_MESSAGING_TIMEOUT);

        for result in [
            AxLimits::new(0, 1, 1, 1, 1, Duration::from_nanos(1)),
            AxLimits::new(1, 0, 1, 1, 1, Duration::from_nanos(1)),
            AxLimits::new(1, 1, 0, 1, 1, Duration::from_nanos(1)),
            AxLimits::new(1, 1, 1, 0, 1, Duration::from_nanos(1)),
            AxLimits::new(1, 1, 1, 1, 0, Duration::from_nanos(1)),
            AxLimits::new(1, 1, 1, 1, 1, Duration::ZERO),
        ] {
            assert!(result.is_err());
        }
        assert!(AxLimits::new(MAX_NODE_LIMIT + 1, 1, 1, 1, 1, Duration::from_nanos(1),).is_err());
        assert!(AxLimits::new(1, MAX_EVENT_LIMIT + 1, 1, 1, 1, Duration::from_nanos(1),).is_err());
        assert!(
            AxLimits::new(
                1,
                1,
                MAX_REGISTRATION_LIMIT + 1,
                1,
                1,
                Duration::from_nanos(1),
            )
            .is_err()
        );
        assert!(AxLimits::new(1, 1, 1, MAX_DEPTH_LIMIT + 1, 1, Duration::from_nanos(1),).is_err());
        assert!(
            AxLimits::new(
                1,
                1,
                1,
                1,
                MAX_VALUE_BYTE_LIMIT + 1,
                Duration::from_nanos(1),
            )
            .is_err()
        );
        assert!(
            AxLimits::new(
                1,
                1,
                1,
                1,
                1,
                MAX_MESSAGING_TIMEOUT + Duration::from_nanos(1)
            )
            .is_err()
        );
    }

    #[test]
    fn fake_factory_rejects_absent_trust_and_pid_replacement() {
        let untrusted = FakeFactory {
            trusted: false,
            pid: 42,
            nodes: Vec::new(),
            events: Vec::new(),
        };
        assert_eq!(untrusted.is_trusted(), Ok(false));
        assert!(matches!(
            untrusted.attach(42, generation(1), limits()),
            Err(AxClientError::AccessibilityUntrusted)
        ));

        let invalid_pid = FakeFactory {
            trusted: true,
            pid: 42,
            nodes: Vec::new(),
            events: Vec::new(),
        };
        assert_eq!(
            invalid_pid.attach(0, generation(1), limits()).err(),
            Some(AxClientError::InvalidPid)
        );

        let replaced = FakeFactory {
            trusted: true,
            pid: 43,
            nodes: Vec::new(),
            events: Vec::new(),
        };
        assert!(matches!(
            replaced.attach(42, generation(1), limits()),
            Err(AxClientError::PidMismatch {
                expected: 42,
                actual: 43
            })
        ));
    }

    #[test]
    fn fake_tree_enforces_identity_depth_and_node_bounds() {
        let duplicate = FakeFactory {
            trusted: true,
            pid: 42,
            nodes: vec![node("root", 0), node("root", 1)],
            events: Vec::new(),
        };
        let mut client = duplicate
            .attach(42, generation(1), limits())
            .unwrap_or_else(|error| panic!("attach fake: {error}"));
        assert!(matches!(
            client.snapshot_tree(),
            Err(AxClientError::DuplicateIdentifier(_))
        ));

        let oversized = FakeFactory {
            trusted: true,
            pid: 42,
            nodes: vec![node("root", 0), node("a", 1), node("b", 2), node("c", 3)],
            events: Vec::new(),
        };
        let mut client = oversized
            .attach(42, generation(1), limits())
            .unwrap_or_else(|error| panic!("attach fake: {error}"));
        assert!(matches!(
            client.snapshot_tree(),
            Err(AxClientError::TreeBoundExceeded {
                name: "node",
                limit: 3
            })
        ));

        let too_deep = FakeFactory {
            trusted: true,
            pid: 42,
            nodes: vec![node("root", 4)],
            events: Vec::new(),
        };
        let mut client = too_deep
            .attach(42, generation(1), limits())
            .unwrap_or_else(|error| panic!("attach fake: {error}"));
        assert!(matches!(
            client.snapshot_tree(),
            Err(AxClientError::TreeBoundExceeded {
                name: "depth",
                limit: 3
            })
        ));
    }

    #[test]
    fn fake_events_reject_stale_generation_and_bound_overflow() {
        let current = generation(7);
        let event = |generation, monotonic_ns| AxObservedEvent {
            generation,
            kind: AxNotificationKind::Value,
            identifier: "editor".to_owned(),
            monotonic_ns,
        };
        let factory = FakeFactory {
            trusted: true,
            pid: 42,
            nodes: vec![node("root", 0), node("editor", 1)],
            events: vec![
                event(generation(6), 1),
                event(current, 2),
                event(current, 3),
                event(current, 4),
            ],
        };
        let mut client = factory
            .attach(42, current, limits())
            .unwrap_or_else(|error| panic!("attach fake: {error}"));
        let batch = client
            .drain_events(current, Duration::ZERO)
            .unwrap_or_else(|error| panic!("drain fake: {error}"));
        assert_eq!(batch.events.len(), 2);
        assert_eq!(batch.stale_events, 1);
        assert_eq!(batch.omitted_events, 1);
        assert!(matches!(
            client.drain_events(generation(8), Duration::ZERO),
            Err(AxClientError::StaleGeneration {
                expected: 7,
                actual: 8
            })
        ));
    }

    #[test]
    fn fake_action_stale_query_and_teardown_are_generation_bound() {
        let current = generation(2);
        let factory = FakeFactory {
            trusted: true,
            pid: 42,
            nodes: vec![node("root", 0), node("editor", 1)],
            events: Vec::new(),
        };
        let mut client = factory
            .attach(42, current, limits())
            .unwrap_or_else(|error| panic!("attach fake: {error}"));
        assert_eq!(client.generation(), current);
        client
            .snapshot_tree()
            .unwrap_or_else(|error| panic!("snapshot fake: {error}"));
        assert_eq!(
            client.perform_action(current, "editor", AxAction::Confirm),
            Ok(0)
        );
        assert_eq!(
            client.query_retained_stale(current),
            Err(AxClientError::MissingStaleElement)
        );
        assert!(matches!(
            client.perform_action(current, "missing", AxAction::Confirm),
            Err(AxClientError::UnknownIdentifier(identifier)) if identifier == "missing"
        ));
        assert!(matches!(
            client.retain_for_stale_query(current, "missing"),
            Err(AxClientError::UnknownIdentifier(identifier)) if identifier == "missing"
        ));
        client
            .retain_for_stale_query(current, "editor")
            .unwrap_or_else(|error| panic!("retain fake: {error}"));
        assert_eq!(
            client.query_retained_stale(current),
            Ok(AxQueryResult { ax_error: -25_202 })
        );
        client
            .close(current)
            .unwrap_or_else(|error| panic!("close fake: {error}"));
        assert_eq!(client.snapshot_tree(), Err(AxClientError::Closed));
        assert_eq!(
            client.perform_action(current, "editor", AxAction::Confirm),
            Err(AxClientError::Closed)
        );
        assert_eq!(client.close(current), Err(AxClientError::Closed));
    }

    #[test]
    fn approved_native_names_are_exact_and_closed() {
        assert_eq!(AxAction::Press.native_name(), "AXPress");
        assert_eq!(AxAction::Confirm.native_name(), "AXConfirm");
        assert_eq!(AxAction::ShowMenu.native_name(), "AXShowMenu");
        assert_eq!(
            AxNotificationKind::ALL.map(AxNotificationKind::native_name),
            [
                "AXFocusedUIElementChanged",
                "AXValueChanged",
                "AXSelectedTextChanged",
                "AXLayoutChanged",
                "AXAnnouncementRequested",
                "AXWindowMiniaturized",
                "AXWindowDeminiaturized",
                "AXUIElementDestroyed",
            ]
        );
    }

    #[test]
    fn errors_have_stable_exhaustive_messages() {
        let cases = [
            (
                AxClientError::UnsupportedPlatform,
                "AX client requires macOS",
            ),
            (
                AxClientError::AccessibilityUntrusted,
                "Accessibility permission is absent; no prompt was issued",
            ),
            (AxClientError::InvalidPid, "AX target PID must be positive"),
            (
                AxClientError::PidMismatch {
                    expected: 7,
                    actual: 8,
                },
                "AX element PID 8 does not match target 7",
            ),
            (
                AxClientError::InvalidGeneration,
                "AX generation must be nonzero",
            ),
            (
                AxClientError::InvalidLimit {
                    name: "node",
                    value: 0,
                    maximum: 3,
                },
                "AX node limit 0 must be within 1..=3",
            ),
            (
                AxClientError::InvalidTimeout,
                "AX messaging timeout must be within 1 ns..=5s",
            ),
            (
                AxClientError::Native {
                    operation: "attach",
                    code: -25_205,
                },
                "AX operation attach failed with -25205",
            ),
            (
                AxClientError::MissingAttribute {
                    attribute: "AXRole",
                    identifier: Some("editor".to_owned()),
                },
                "AX attribute AXRole is absent on editor",
            ),
            (
                AxClientError::MissingAttribute {
                    attribute: "AXRole",
                    identifier: None,
                },
                "AX attribute AXRole is absent on unidentified element",
            ),
            (
                AxClientError::InvalidAttributeType {
                    attribute: "AXValue",
                },
                "AX attribute AXValue has an unexpected type",
            ),
            (
                AxClientError::DuplicateIdentifier("editor".to_owned()),
                "AX identifier \"editor\" is duplicated",
            ),
            (
                AxClientError::TreeBoundExceeded {
                    name: "depth",
                    limit: 3,
                },
                "AX tree exceeded depth limit 3",
            ),
            (
                AxClientError::ValueBoundExceeded {
                    attribute: "AXValue",
                    limit: 128,
                },
                "AX attribute AXValue exceeded value-byte limit 128",
            ),
            (
                AxClientError::UnknownIdentifier("missing".to_owned()),
                "AX identifier \"missing\" is not in the snapshot",
            ),
            (
                AxClientError::MissingStaleElement,
                "no AX element is retained for stale-query control",
            ),
            (
                AxClientError::StaleGeneration {
                    expected: 2,
                    actual: 3,
                },
                "AX generation 3 does not match attached generation 2",
            ),
            (AxClientError::Closed, "AX client is closed"),
        ];
        for (error, expected) in cases {
            assert_eq!(error.to_string(), expected);
        }
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn unsupported_factory_and_client_fail_structurally() {
        let current = generation(9);
        let factory = super::NativeAxClientFactory;
        assert_eq!(
            factory.is_trusted(),
            Err(AxClientError::UnsupportedPlatform)
        );
        assert!(matches!(
            factory.attach(42, current, limits()),
            Err(AxClientError::UnsupportedPlatform)
        ));

        let mut client = super::UnsupportedAxClient {
            generation: current,
        };
        assert_eq!(client.generation(), current);
        assert_eq!(
            client.snapshot_tree(),
            Err(AxClientError::UnsupportedPlatform)
        );
        assert_eq!(
            client.drain_events(current, Duration::ZERO),
            Err(AxClientError::UnsupportedPlatform)
        );
        assert_eq!(
            client.perform_action(current, "editor", AxAction::Press),
            Err(AxClientError::UnsupportedPlatform)
        );
        assert_eq!(
            client.retain_for_stale_query(current, "editor"),
            Err(AxClientError::UnsupportedPlatform)
        );
        assert_eq!(
            client.query_retained_stale(current),
            Err(AxClientError::UnsupportedPlatform)
        );
        assert_eq!(
            client.close(current),
            Err(AxClientError::UnsupportedPlatform)
        );
    }
}
