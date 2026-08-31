//! Audited generated-binding boundary for the non-shipping AX client.

use crate::{
    AxAction, AxClient, AxClientError, AxEventBatch, AxGeneration, AxLimits, AxNode,
    AxNotificationKind, AxObservedEvent, AxQueryResult, AxRect, AxTextRange,
};
use objc2_application_services::{AXError, AXObserver, AXUIElement, AXValue, AXValueType};
use objc2_core_foundation::{
    CFArray, CFBoolean, CFRange, CFRetained, CFRunLoop, CFRunLoopSource, CFString, CFType, CGPoint,
    CGSize, kCFRunLoopDefaultMode,
};
use std::{
    collections::BTreeMap,
    ffi::c_void,
    mem::ManuallyDrop,
    panic::{AssertUnwindSafe, catch_unwind},
    ptr::{NonNull, null},
    time::{Duration, Instant},
};

const ATTRIBUTE_IDENTIFIER: &str = "AXIdentifier";
const ATTRIBUTE_ROLE: &str = "AXRole";
const ATTRIBUTE_TITLE: &str = "AXTitle";
const ATTRIBUTE_DESCRIPTION: &str = "AXDescription";
const ATTRIBUTE_VALUE: &str = "AXValue";
const ATTRIBUTE_FOCUSED: &str = "AXFocused";
const ATTRIBUTE_SELECTED_TEXT: &str = "AXSelectedText";
const ATTRIBUTE_SELECTED_TEXT_RANGE: &str = "AXSelectedTextRange";
const ATTRIBUTE_POSITION: &str = "AXPosition";
const ATTRIBUTE_SIZE: &str = "AXSize";
const ATTRIBUTE_CHILDREN: &str = "AXChildren";

struct NotificationBinding {
    name: CFRetained<CFString>,
    kind: AxNotificationKind,
}

struct PendingEvent {
    generation: AxGeneration,
    kind: AxNotificationKind,
    element: CFRetained<AXUIElement>,
    monotonic_ns: u64,
}

struct CallbackState {
    generation: AxGeneration,
    started: Instant,
    active: bool,
    bindings: Vec<NotificationBinding>,
    pending: Vec<PendingEvent>,
    omitted: usize,
    stale: usize,
}

impl CallbackState {
    fn new(generation: AxGeneration, event_limit: usize) -> Self {
        let bindings = AxNotificationKind::ALL
            .into_iter()
            .map(|kind| NotificationBinding {
                name: CFString::from_static_str(kind.native_name()),
                kind,
            })
            .collect();
        Self {
            generation,
            started: Instant::now(),
            active: true,
            bindings,
            pending: Vec::with_capacity(event_limit),
            omitted: 0,
            stale: 0,
        }
    }

    fn push(&mut self, element: NonNull<AXUIElement>, notification: NonNull<CFString>) {
        if !self.active {
            self.stale = self.stale.saturating_add(1);
            return;
        }
        let Some(kind) = self.bindings.iter().find_map(|binding| {
            // SAFETY: CoreFoundation supplied a non-null notification for the
            // duration of this callback. No reference escapes the callback.
            let notification = unsafe { notification.as_ref() };
            (&*binding.name == notification).then_some(binding.kind)
        }) else {
            self.omitted = self.omitted.saturating_add(1);
            return;
        };
        if self.pending.len() == self.pending.capacity() {
            self.omitted = self.omitted.saturating_add(1);
            return;
        }
        let nanos = u64::try_from(self.started.elapsed().as_nanos()).unwrap_or(u64::MAX);
        // SAFETY: AXObserver guarantees the element is valid for the callback.
        // Retaining it gives PendingEvent independent +1 ownership.
        let element = unsafe { CFRetained::retain(element) };
        self.pending.push(PendingEvent {
            generation: self.generation,
            kind,
            element,
            monotonic_ns: nanos,
        });
    }
}

unsafe extern "C-unwind" fn observer_callback(
    _observer: NonNull<AXObserver>,
    element: NonNull<AXUIElement>,
    notification: NonNull<CFString>,
    refcon: *mut c_void,
) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let Some(state) = NonNull::new(refcon.cast::<CallbackState>()) else {
            return;
        };
        // SAFETY: NativeAxClient owns the boxed CallbackState until after it
        // removes the run-loop source and drops the observer.
        if let Some(state) = unsafe { state.as_ptr().as_mut() } {
            state.push(element, notification);
        }
    }));
}

struct Registration {
    element: CFRetained<AXUIElement>,
    kind: AxNotificationKind,
}

/// Native implementation hidden behind the handle-free [`AxClient`] trait.
pub struct NativeAxClient {
    pid: i32,
    generation: AxGeneration,
    limits: AxLimits,
    application: CFRetained<AXUIElement>,
    observer: CFRetained<AXObserver>,
    run_loop: CFRetained<CFRunLoop>,
    run_loop_source: CFRetained<CFRunLoopSource>,
    callback: Box<CallbackState>,
    elements: BTreeMap<String, CFRetained<AXUIElement>>,
    element_identifiers: BTreeMap<usize, String>,
    registrations: Vec<Registration>,
    stale_element: Option<CFRetained<AXUIElement>>,
    closed: bool,
}

pub(crate) fn is_trusted() -> bool {
    // SAFETY: Passing no options performs a read-only trust query and cannot
    // prompt or mutate the macOS privacy database.
    unsafe { objc2_application_services::AXIsProcessTrusted() }
}

impl NativeAxClient {
    pub(crate) fn attach(
        pid: i32,
        generation: AxGeneration,
        limits: AxLimits,
    ) -> Result<Self, AxClientError> {
        if pid <= 0 {
            return Err(AxClientError::InvalidPid);
        }
        if !is_trusted() {
            return Err(AxClientError::AccessibilityUntrusted);
        }

        // SAFETY: PID is positive and the returned CoreFoundation object is
        // owned by the generated Create-rule wrapper.
        let application = unsafe { AXUIElement::new_application(pid) };
        verify_pid(&application, pid)?;
        let timeout = limits.messaging_timeout().as_secs_f32();
        // SAFETY: The generated binding borrows a valid AXUIElement and the
        // validated timeout is finite, positive, and at most five seconds.
        ax_result("set-messaging-timeout", unsafe {
            application.set_messaging_timeout(timeout)
        })?;

        let mut observer_pointer = std::ptr::null_mut();
        // SAFETY: observer_pointer is a valid out parameter and the callback
        // obeys AXObserver's ABI. Its refcon is installed only after creation.
        ax_result("create-observer", unsafe {
            AXObserver::create(
                pid,
                Some(observer_callback),
                NonNull::from(&mut observer_pointer),
            )
        })?;
        let observer_pointer = NonNull::new(observer_pointer).ok_or(AxClientError::Native {
            operation: "create-observer-null",
            code: AXError::Failure.0,
        })?;
        // SAFETY: AXObserverCreate returned success and one Create-rule retain.
        let observer = unsafe { CFRetained::from_raw(observer_pointer) };

        // objc2 0.3.2 models AXObserverGetRunLoopSource as Create-rule owned,
        // but Apple's Get rule returns a borrowed source. Prevent the generated
        // temporary from releasing borrowed ownership, then establish exactly
        // one Alpine retain before exposing the source to safe fields.
        // SAFETY: observer is live and owns the borrowed source.
        let borrowed_source = ManuallyDrop::new(unsafe { observer.run_loop_source() });
        // SAFETY: The borrowed source remains valid while observer is live;
        // CFRetain creates the one +1 ownership held by NativeAxClient.
        let run_loop_source = unsafe { CFRetained::retain(CFRetained::as_ptr(&borrowed_source)) };
        let run_loop = CFRunLoop::current().ok_or(AxClientError::Native {
            operation: "current-run-loop",
            code: AXError::Failure.0,
        })?;
        // SAFETY: CoreFoundation publishes a non-null default mode on macOS.
        let mode = unsafe { kCFRunLoopDefaultMode }.ok_or(AxClientError::Native {
            operation: "default-run-loop-mode",
            code: AXError::Failure.0,
        })?;
        run_loop.add_source(Some(&run_loop_source), Some(mode));

        Ok(Self {
            pid,
            generation,
            limits,
            application,
            observer,
            run_loop,
            run_loop_source,
            callback: Box::new(CallbackState::new(generation, limits.event_limit())),
            elements: BTreeMap::new(),
            element_identifiers: BTreeMap::new(),
            registrations: Vec::new(),
            stale_element: None,
            closed: false,
        })
    }

    fn require_generation(&self, generation: AxGeneration) -> Result<(), AxClientError> {
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

    fn install_observers(&mut self) -> Result<(), AxClientError> {
        self.remove_observers();
        let required = self
            .elements
            .len()
            .checked_mul(AxNotificationKind::ALL.len())
            .ok_or(AxClientError::TreeBoundExceeded {
                name: "registration",
                limit: self.limits.registration_limit(),
            })?;
        if required > self.limits.registration_limit() {
            return Err(AxClientError::TreeBoundExceeded {
                name: "registration",
                limit: self.limits.registration_limit(),
            });
        }
        let elements = self
            .elements
            .values()
            .map(|element| {
                // SAFETY: Each map value is a live retained AXUIElement.
                unsafe { CFRetained::retain(CFRetained::as_ptr(element)) }
            })
            .collect::<Vec<_>>();
        let refcon = NonNull::from(self.callback.as_mut())
            .as_ptr()
            .cast::<c_void>();
        for element in elements {
            for kind in AxNotificationKind::ALL {
                let name = self.notification_name(kind);
                // SAFETY: observer, element, and name are live. refcon points
                // to a boxed state that outlives every registration.
                let result = unsafe { self.observer.add_notification(&element, name, refcon) };
                if result == AXError::Success || result == AXError::NotificationAlreadyRegistered {
                    // SAFETY: element is live and this registration owns a
                    // distinct retain until removal.
                    let retained = unsafe { CFRetained::retain(CFRetained::as_ptr(&element)) };
                    self.registrations.push(Registration {
                        element: retained,
                        kind,
                    });
                } else if result != AXError::NotificationUnsupported
                    && result != AXError::AttributeUnsupported
                {
                    return Err(AxClientError::Native {
                        operation: "add-notification",
                        code: result.0,
                    });
                }
            }
        }
        Ok(())
    }

    fn notification_name(&self, kind: AxNotificationKind) -> &CFString {
        self.callback
            .bindings
            .iter()
            .find(|binding| binding.kind == kind)
            .map_or_else(
                || unreachable!("all approved notification bindings are installed"),
                |binding| &*binding.name,
            )
    }

    fn remove_observers(&mut self) {
        let registrations = std::mem::take(&mut self.registrations);
        for registration in registrations {
            let name = self.notification_name(registration.kind);
            // SAFETY: Every entry records one successful registration on this
            // observer. Teardown ignores process-exit and already-removed codes.
            let _ = unsafe {
                self.observer
                    .remove_notification(&registration.element, name)
            };
        }
    }

    fn close_inner(&mut self) {
        if self.closed {
            return;
        }
        self.callback.active = false;
        self.remove_observers();
        // SAFETY: CoreFoundation publishes a non-null default mode on macOS.
        if let Some(mode) = unsafe { kCFRunLoopDefaultMode } {
            self.run_loop
                .remove_source(Some(&self.run_loop_source), Some(mode));
        }
        self.callback.pending.clear();
        self.elements.clear();
        self.element_identifiers.clear();
        self.stale_element = None;
        self.closed = true;
    }
}

impl AxClient for NativeAxClient {
    fn generation(&self) -> AxGeneration {
        self.generation
    }

    fn snapshot_tree(&mut self) -> Result<Vec<AxNode>, AxClientError> {
        if self.closed {
            return Err(AxClientError::Closed);
        }
        self.remove_observers();
        self.elements.clear();
        self.element_identifiers.clear();

        // SAFETY: application is a live retained element.
        let root = unsafe { CFRetained::retain(CFRetained::as_ptr(&self.application)) };
        let mut stack = vec![(root, None, 0_u16)];
        let mut nodes = Vec::new();
        while let Some((element, parent_identifier, depth)) = stack.pop() {
            if nodes.len() == self.limits.node_limit() {
                return Err(AxClientError::TreeBoundExceeded {
                    name: "node",
                    limit: self.limits.node_limit(),
                });
            }
            if depth > self.limits.depth_limit() {
                return Err(AxClientError::TreeBoundExceeded {
                    name: "depth",
                    limit: usize::from(self.limits.depth_limit()),
                });
            }
            verify_pid(&element, self.pid)?;
            let node = query_node(
                &element,
                parent_identifier,
                depth,
                self.limits.value_byte_limit(),
            )?;
            let identifier = node.identifier.clone();
            if self.elements.contains_key(&identifier) {
                return Err(AxClientError::DuplicateIdentifier(identifier));
            }
            let pointer = CFRetained::as_ptr(&element).as_ptr().addr();
            self.element_identifiers.insert(pointer, identifier.clone());
            // SAFETY: element is live and the map owns a distinct retain.
            self.elements.insert(identifier.clone(), unsafe {
                CFRetained::retain(CFRetained::as_ptr(&element))
            });
            let children = copy_children(&element)?;
            for child in children.into_iter().rev() {
                stack.push((child, Some(identifier.clone()), depth.saturating_add(1)));
            }
            nodes.push(node);
        }
        self.install_observers()?;
        Ok(nodes)
    }

    fn drain_events(
        &mut self,
        generation: AxGeneration,
        timeout: Duration,
    ) -> Result<AxEventBatch, AxClientError> {
        self.require_generation(generation)?;
        if timeout > self.limits.messaging_timeout() {
            return Err(AxClientError::InvalidTimeout);
        }
        // SAFETY: CoreFoundation publishes a non-null default mode on macOS.
        let mode = unsafe { kCFRunLoopDefaultMode }.ok_or(AxClientError::Native {
            operation: "default-run-loop-mode",
            code: AXError::Failure.0,
        })?;
        let _ = CFRunLoop::run_in_mode(Some(mode), timeout.as_secs_f64(), false);

        let pending = self.callback.pending.drain(..).collect::<Vec<_>>();
        let omitted_events = std::mem::take(&mut self.callback.omitted);
        let mut stale_events = std::mem::take(&mut self.callback.stale);
        let mut events = Vec::with_capacity(pending.len());
        for pending in pending {
            if pending.generation != self.generation {
                stale_events = stale_events.saturating_add(1);
                continue;
            }
            verify_pid(&pending.element, self.pid)?;
            let pointer = CFRetained::as_ptr(&pending.element).as_ptr().addr();
            let identifier = self
                .element_identifiers
                .get(&pointer)
                .cloned()
                .ok_or_else(|| AxClientError::UnknownIdentifier(format!("pointer-{pointer:x}")))?;
            events.push(AxObservedEvent {
                generation: pending.generation,
                kind: pending.kind,
                identifier,
                monotonic_ns: pending.monotonic_ns,
            });
        }
        Ok(AxEventBatch {
            events,
            omitted_events,
            stale_events,
        })
    }

    fn perform_action(
        &mut self,
        generation: AxGeneration,
        identifier: &str,
        action: AxAction,
    ) -> Result<i32, AxClientError> {
        self.require_generation(generation)?;
        let element = self
            .elements
            .get(identifier)
            .ok_or_else(|| AxClientError::UnknownIdentifier(identifier.to_owned()))?;
        verify_pid(element, self.pid)?;
        let action_name = CFString::from_static_str(action.native_name());
        // SAFETY: element and action string are live; AxAction is a closed
        // allowlist and does not admit arbitrary native action names.
        let result = unsafe { element.perform_action(&action_name) };
        Ok(result.0)
    }

    fn retain_for_stale_query(
        &mut self,
        generation: AxGeneration,
        identifier: &str,
    ) -> Result<(), AxClientError> {
        self.require_generation(generation)?;
        let element = self
            .elements
            .get(identifier)
            .ok_or_else(|| AxClientError::UnknownIdentifier(identifier.to_owned()))?;
        // SAFETY: map ownership keeps this element live while the independent
        // stale control retain is established.
        self.stale_element = Some(unsafe { CFRetained::retain(CFRetained::as_ptr(element)) });
        Ok(())
    }

    fn query_retained_stale(
        &mut self,
        generation: AxGeneration,
    ) -> Result<AxQueryResult, AxClientError> {
        self.require_generation(generation)?;
        let element = self
            .stale_element
            .as_ref()
            .ok_or(AxClientError::MissingStaleElement)?;
        let role = CFString::from_static_str(ATTRIBUTE_ROLE);
        let mut raw = null();
        // SAFETY: raw is a valid out pointer and role is a valid attribute.
        // A success result owns one copied value, released immediately below.
        let result = unsafe { element.copy_attribute_value(&role, NonNull::from(&mut raw)) };
        if result == AXError::Success
            && let Some(pointer) = NonNull::new(raw.cast_mut())
        {
            // SAFETY: CopyAttributeValue returned one Create-rule retain.
            drop(unsafe { CFRetained::<CFType>::from_raw(pointer) });
        }
        Ok(AxQueryResult { ax_error: result.0 })
    }

    fn close(&mut self, generation: AxGeneration) -> Result<(), AxClientError> {
        self.require_generation(generation)?;
        self.close_inner();
        Ok(())
    }
}

impl Drop for NativeAxClient {
    fn drop(&mut self) {
        self.close_inner();
    }
}

fn query_node(
    element: &AXUIElement,
    parent_identifier: Option<String>,
    depth: u16,
    byte_limit: usize,
) -> Result<AxNode, AxClientError> {
    let identifier = required_string(element, ATTRIBUTE_IDENTIFIER, byte_limit, None)?;
    let role = required_string(
        element,
        ATTRIBUTE_ROLE,
        byte_limit,
        Some(identifier.clone()),
    )?;
    let label = optional_string(element, ATTRIBUTE_TITLE, byte_limit)?
        .or(optional_string(element, ATTRIBUTE_DESCRIPTION, byte_limit)?)
        .unwrap_or_default();
    let value = optional_string(element, ATTRIBUTE_VALUE, byte_limit)?;
    let focused = optional_bool(element, ATTRIBUTE_FOCUSED)?.unwrap_or(false);
    let selected_text = optional_string(element, ATTRIBUTE_SELECTED_TEXT, byte_limit)?;
    let selected_range = optional_range(element, ATTRIBUTE_SELECTED_TEXT_RANGE)?;
    let position = optional_point(element, ATTRIBUTE_POSITION)?;
    let size = optional_size(element, ATTRIBUTE_SIZE)?;
    let frame = position.zip(size).map(|(position, size)| AxRect {
        x: position.x,
        y: position.y,
        width: size.width,
        height: size.height,
    });
    let enabled_actions = copy_action_names(element, byte_limit)?;
    Ok(AxNode {
        identifier,
        parent_identifier,
        depth,
        role,
        label,
        value,
        focused,
        selected_text,
        selected_range,
        frame,
        enabled_actions,
    })
}

fn required_string(
    element: &AXUIElement,
    attribute: &'static str,
    byte_limit: usize,
    identifier: Option<String>,
) -> Result<String, AxClientError> {
    optional_string(element, attribute, byte_limit)?.ok_or(AxClientError::MissingAttribute {
        attribute,
        identifier,
    })
}

fn optional_string(
    element: &AXUIElement,
    attribute: &'static str,
    byte_limit: usize,
) -> Result<Option<String>, AxClientError> {
    let Some(value) = copy_attribute(element, attribute)? else {
        return Ok(None);
    };
    let string = value
        .downcast_ref::<CFString>()
        .ok_or(AxClientError::InvalidAttributeType { attribute })?
        .to_string();
    if string.len() > byte_limit {
        return Err(AxClientError::ValueBoundExceeded {
            attribute,
            limit: byte_limit,
        });
    }
    Ok(Some(string))
}

fn optional_bool(
    element: &AXUIElement,
    attribute: &'static str,
) -> Result<Option<bool>, AxClientError> {
    let Some(value) = copy_attribute(element, attribute)? else {
        return Ok(None);
    };
    Ok(Some(
        value
            .downcast_ref::<CFBoolean>()
            .ok_or(AxClientError::InvalidAttributeType { attribute })?
            .value(),
    ))
}

fn optional_range(
    element: &AXUIElement,
    attribute: &'static str,
) -> Result<Option<AxTextRange>, AxClientError> {
    let Some(value) = copy_attribute(element, attribute)? else {
        return Ok(None);
    };
    let value = value
        .downcast_ref::<AXValue>()
        .ok_or(AxClientError::InvalidAttributeType { attribute })?;
    let mut range = CFRange {
        location: 0,
        length: 0,
    };
    // SAFETY: range is correctly aligned writable CFRange storage and its
    // type tag is requested explicitly.
    let copied = unsafe {
        value.value(
            AXValueType::CFRange,
            NonNull::from(&mut range).cast::<c_void>(),
        )
    };
    if !copied || range.location < 0 || range.length < 0 {
        return Err(AxClientError::InvalidAttributeType { attribute });
    }
    let location = u64::try_from(range.location)
        .map_err(|_| AxClientError::InvalidAttributeType { attribute })?;
    let length = u64::try_from(range.length)
        .map_err(|_| AxClientError::InvalidAttributeType { attribute })?;
    Ok(Some(AxTextRange { location, length }))
}

fn optional_point(
    element: &AXUIElement,
    attribute: &'static str,
) -> Result<Option<CGPoint>, AxClientError> {
    let Some(value) = copy_attribute(element, attribute)? else {
        return Ok(None);
    };
    let value = value
        .downcast_ref::<AXValue>()
        .ok_or(AxClientError::InvalidAttributeType { attribute })?;
    let mut point = CGPoint::default();
    // SAFETY: point is correctly aligned writable CGPoint storage.
    if !unsafe {
        value.value(
            AXValueType::CGPoint,
            NonNull::from(&mut point).cast::<c_void>(),
        )
    } {
        return Err(AxClientError::InvalidAttributeType { attribute });
    }
    Ok(Some(point))
}

fn optional_size(
    element: &AXUIElement,
    attribute: &'static str,
) -> Result<Option<CGSize>, AxClientError> {
    let Some(value) = copy_attribute(element, attribute)? else {
        return Ok(None);
    };
    let value = value
        .downcast_ref::<AXValue>()
        .ok_or(AxClientError::InvalidAttributeType { attribute })?;
    let mut size = CGSize::default();
    // SAFETY: size is correctly aligned writable CGSize storage.
    if !unsafe {
        value.value(
            AXValueType::CGSize,
            NonNull::from(&mut size).cast::<c_void>(),
        )
    } {
        return Err(AxClientError::InvalidAttributeType { attribute });
    }
    Ok(Some(size))
}

fn copy_attribute(
    element: &AXUIElement,
    attribute: &'static str,
) -> Result<Option<CFRetained<CFType>>, AxClientError> {
    let attribute_name = CFString::from_static_str(attribute);
    let mut raw = null();
    // SAFETY: raw is a valid out pointer and attribute_name is live.
    let result = unsafe { element.copy_attribute_value(&attribute_name, NonNull::from(&mut raw)) };
    if result == AXError::AttributeUnsupported || result == AXError::NoValue {
        return Ok(None);
    }
    ax_result("copy-attribute", result)?;
    let pointer = NonNull::new(raw.cast_mut()).ok_or(AxClientError::Native {
        operation: "copy-attribute-null",
        code: AXError::Failure.0,
    })?;
    // SAFETY: CopyAttributeValue returned success and one Create-rule retain.
    Ok(Some(unsafe { CFRetained::from_raw(pointer) }))
}

fn copy_children(element: &AXUIElement) -> Result<Vec<CFRetained<AXUIElement>>, AxClientError> {
    let Some(value) = copy_attribute(element, ATTRIBUTE_CHILDREN)? else {
        return Ok(Vec::new());
    };
    let array = value
        .downcast::<CFArray>()
        .map_err(|_| AxClientError::InvalidAttributeType {
            attribute: ATTRIBUTE_CHILDREN,
        })?;
    // SAFETY: AXChildren is documented as a CFArray of AXUIElement values.
    let array = unsafe { CFRetained::cast_unchecked::<CFArray<AXUIElement>>(array) };
    Ok(array.to_vec())
}

fn copy_action_names(
    element: &AXUIElement,
    byte_limit: usize,
) -> Result<Vec<String>, AxClientError> {
    let mut raw = null();
    // SAFETY: raw is a valid out pointer.
    let result = unsafe { element.copy_action_names(NonNull::from(&mut raw)) };
    if result == AXError::ActionUnsupported || result == AXError::NoValue {
        return Ok(Vec::new());
    }
    ax_result("copy-action-names", result)?;
    let pointer = NonNull::new(raw.cast_mut()).ok_or(AxClientError::Native {
        operation: "copy-action-names-null",
        code: AXError::Failure.0,
    })?;
    // SAFETY: CopyActionNames returned a retained CFArray of CFString values.
    let array = unsafe { CFRetained::<CFArray>::from_raw(pointer) };
    // SAFETY: AXUIElementCopyActionNames documents CFString members.
    let array = unsafe { CFRetained::cast_unchecked::<CFArray<CFString>>(array) };
    let mut names = Vec::with_capacity(array.len());
    let mut bytes = 0_usize;
    for name in array.iter() {
        let name = name.to_string();
        bytes = bytes.saturating_add(name.len());
        if bytes > byte_limit {
            return Err(AxClientError::ValueBoundExceeded {
                attribute: "AXActionNames",
                limit: byte_limit,
            });
        }
        names.push(name);
    }
    Ok(names)
}

fn verify_pid(element: &AXUIElement, expected: i32) -> Result<(), AxClientError> {
    let mut actual = 0_i32;
    // SAFETY: actual is a valid writable pid_t-compatible i32 on macOS.
    ax_result("element-pid", unsafe {
        element.pid(NonNull::from(&mut actual))
    })?;
    if actual != expected {
        return Err(AxClientError::PidMismatch { expected, actual });
    }
    Ok(())
}

fn ax_result(operation: &'static str, result: AXError) -> Result<(), AxClientError> {
    if result == AXError::Success {
        Ok(())
    } else {
        Err(AxClientError::Native {
            operation,
            code: result.0,
        })
    }
}
