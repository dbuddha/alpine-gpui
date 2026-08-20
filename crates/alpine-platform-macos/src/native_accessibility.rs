//! Private `AppKit` accessibility adapter over Alpine's bounded semantic transport.

#[cfg(alpine_native_validation)]
use core::mem;
use std::cell::Cell;

use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send,
    rc::{Retained, Weak},
    runtime::{NSObjectProtocol, Sel},
    sel,
};
use objc2_app_kit::{
    NSAccessibilityAnnouncementRequestedNotification, NSAccessibilityElement,
    NSAccessibilityFocusedUIElementChangedNotification, NSAccessibilityLayoutChangedNotification,
    NSAccessibilityPostNotification, NSAccessibilitySelectedTextChangedNotification,
    NSAccessibilityValueChangedNotification,
};
use objc2_foundation::{NSArray, NSRange, NSString};

use crate::{
    AccessibilityAction, AccessibilityActionResult, AccessibilityError, AccessibilityNode,
    AccessibilityNodeId, AccessibilityPayload, AccessibilityRequest, AccessibilityRequestId,
    AccessibilityResponse, AccessibilityRevision, AccessibilityRole, AccessibilitySnapshot,
    AccessibilityTextRange, SurfaceError, native::SurfaceView,
};

type RequestHandler =
    Box<dyn FnMut(&AccessibilityRequest) -> Result<AccessibilityResponse, SurfaceError> + 'static>;

#[derive(Clone, Copy)]
enum NotificationKind {
    Layout = 0,
    Focus = 1,
    Selection = 2,
    Value = 3,
    Announcement = 4,
}

struct NotificationIntent {
    element: Retained<NativeAccessibilityElement>,
    kind: NotificationKind,
}

struct RefreshOutcome {
    notifications: Vec<NotificationIntent>,
}

impl RefreshOutcome {
    fn post(self) {
        for intent in self.notifications {
            // SAFETY: the element remains retained for this synchronous AppKit call,
            // and each notification constant is process-lifetime immutable.
            unsafe {
                match intent.kind {
                    NotificationKind::Layout => NSAccessibilityPostNotification(
                        &intent.element,
                        NSAccessibilityLayoutChangedNotification,
                    ),
                    NotificationKind::Focus => NSAccessibilityPostNotification(
                        &intent.element,
                        NSAccessibilityFocusedUIElementChangedNotification,
                    ),
                    NotificationKind::Selection => NSAccessibilityPostNotification(
                        &intent.element,
                        NSAccessibilitySelectedTextChangedNotification,
                    ),
                    NotificationKind::Value => NSAccessibilityPostNotification(
                        &intent.element,
                        NSAccessibilityValueChangedNotification,
                    ),
                    NotificationKind::Announcement => NSAccessibilityPostNotification(
                        &intent.element,
                        NSAccessibilityAnnouncementRequestedNotification,
                    ),
                }
            }
        }
    }
}

struct CachedElement {
    id: AccessibilityNodeId,
    element: Retained<NativeAccessibilityElement>,
}

#[derive(Clone, Copy, Default)]
struct AdapterCounters {
    peak_elements: usize,
    created_elements: u64,
    released_elements: u64,
    requests: u64,
    failed_requests: u64,
    notifications: [u64; 5],
}

pub(crate) struct NativeAccessibilityAdapter {
    handler: Option<RequestHandler>,
    snapshot: Option<AccessibilitySnapshot>,
    elements: Vec<CachedElement>,
    next_request_id: u64,
    generation: u64,
    active: bool,
    counters: AdapterCounters,
}

impl NativeAccessibilityAdapter {
    pub(crate) const fn new() -> Self {
        Self {
            handler: None,
            snapshot: None,
            elements: Vec::new(),
            next_request_id: 1,
            generation: 1,
            active: false,
            counters: AdapterCounters {
                peak_elements: 0,
                created_elements: 0,
                released_elements: 0,
                requests: 0,
                failed_requests: 0,
                notifications: [0; 5],
            },
        }
    }

    pub(crate) fn install(&mut self, handler: RequestHandler) {
        self.handler = Some(handler);
    }

    pub(crate) fn refresh_view_if_active(view: &SurfaceView) -> Result<(), SurfaceError> {
        if !view.ivars().accessibility.borrow().active {
            return Ok(());
        }
        Self::refresh_view(view)
    }

    pub(crate) fn refresh_view(view: &SurfaceView) -> Result<(), SurfaceError> {
        let outcome = view.ivars().accessibility.borrow_mut().refresh(view)?;
        outcome.post();
        Ok(())
    }

    pub(crate) fn surface_children(
        view: &SurfaceView,
    ) -> Retained<NSArray<NativeAccessibilityElement>> {
        if Self::refresh_view(view).is_err() {
            return NSArray::new();
        }
        let roots = {
            let adapter = view.ivars().accessibility.borrow();
            adapter
                .snapshot
                .as_ref()
                .and_then(|snapshot| adapter.element(snapshot.root()))
                .into_iter()
                .collect::<Vec<_>>()
        };
        NSArray::from_retained_slice(&roots)
    }

    pub(crate) fn revoke_view(view: &SurfaceView) {
        view.ivars().accessibility.borrow_mut().revoke();
    }

    fn next_id(&mut self) -> Result<AccessibilityRequestId, SurfaceError> {
        let id = self.next_request_id;
        self.next_request_id = self
            .next_request_id
            .checked_add(1)
            .ok_or(SurfaceError::DriverUnavailable)?;
        Ok(AccessibilityRequestId::new(id))
    }

    fn dispatch(
        &mut self,
        request: &AccessibilityRequest,
    ) -> Result<AccessibilityResponse, SurfaceError> {
        self.counters.requests = self.counters.requests.saturating_add(1);
        let Some(handler) = self.handler.as_mut() else {
            self.counters.failed_requests = self.counters.failed_requests.saturating_add(1);
            return Err(SurfaceError::DriverUnavailable);
        };
        let result = handler(request);
        match result {
            Ok(response) => {
                if response.validate_for(request).is_err() {
                    self.counters.failed_requests = self.counters.failed_requests.saturating_add(1);
                    return Err(SurfaceError::DriverUnavailable);
                }
                if response.result().is_err() {
                    self.counters.failed_requests = self.counters.failed_requests.saturating_add(1);
                }
                Ok(response)
            }
            Err(error) => {
                self.counters.failed_requests = self.counters.failed_requests.saturating_add(1);
                Err(error)
            }
        }
    }

    fn refresh(&mut self, view: &SurfaceView) -> Result<RefreshOutcome, SurfaceError> {
        let request = AccessibilityRequest::snapshot(self.next_id()?)
            .map_err(|_| SurfaceError::DriverUnavailable)?;
        let response = self.dispatch(&request)?;
        let snapshot = match response.result() {
            Ok(AccessibilityPayload::Snapshot(snapshot)) => snapshot.clone(),
            _ => return Err(SurfaceError::DriverUnavailable),
        };
        let previous = self.snapshot.clone();
        self.reconcile_elements(view, &snapshot)?;
        let notifications = self.notification_intents(previous.as_ref(), &snapshot)?;
        self.snapshot = Some(snapshot);
        self.active = true;
        Ok(RefreshOutcome { notifications })
    }

    fn reconcile_elements(
        &mut self,
        view: &SurfaceView,
        snapshot: &AccessibilitySnapshot,
    ) -> Result<(), SurfaceError> {
        let mut next = Vec::new();
        next.try_reserve_exact(snapshot.nodes().len())
            .map_err(|_| SurfaceError::DriverUnavailable)?;
        let mut created = 0_u64;
        let main_thread = MainThreadMarker::new().ok_or(SurfaceError::DriverUnavailable)?;
        for node in snapshot.nodes() {
            let element = self.element(node.id()).unwrap_or_else(|| {
                created = created.saturating_add(1);
                NativeAccessibilityElement::new(main_thread, view, node.id(), self.generation)
            });
            next.push(CachedElement {
                id: node.id(),
                element,
            });
        }
        let released = self
            .elements
            .iter()
            .filter(|entry| !snapshot.nodes().iter().any(|node| node.id() == entry.id))
            .count();
        self.elements = next;
        self.counters.created_elements = self.counters.created_elements.saturating_add(created);
        self.counters.released_elements = self
            .counters
            .released_elements
            .saturating_add(u64::try_from(released).unwrap_or(u64::MAX));
        self.counters.peak_elements = self.counters.peak_elements.max(self.elements.len());
        Ok(())
    }

    fn notification_intents(
        &mut self,
        previous: Option<&AccessibilitySnapshot>,
        current: &AccessibilitySnapshot,
    ) -> Result<Vec<NotificationIntent>, SurfaceError> {
        let Some(previous) = previous else {
            return Ok(Vec::new());
        };
        let mut intents = Vec::new();
        intents
            .try_reserve(current.nodes().len().saturating_add(4))
            .map_err(|_| SurfaceError::DriverUnavailable)?;
        let tree_changed = structural_tree_changed(previous, current);
        if tree_changed {
            self.push_notification(&mut intents, current.root(), NotificationKind::Layout);
        }
        let previous_focus = previous
            .nodes()
            .iter()
            .find(|node| node.is_focused())
            .map(AccessibilityNode::id);
        let current_focus = current
            .nodes()
            .iter()
            .find(|node| node.is_focused())
            .map(AccessibilityNode::id);
        if previous_focus != current_focus
            && let Some(id) = current_focus
        {
            self.push_notification(&mut intents, id, NotificationKind::Focus);
        }
        if previous.selection() != current.selection()
            && let Some(id) = role_id(current, AccessibilityRole::CodeEditor)
        {
            self.push_notification(&mut intents, id, NotificationKind::Selection);
        }
        if (previous.revision() != current.revision()
            || previous.text_len_utf16() != current.text_len_utf16()
            || previous.is_dirty() != current.is_dirty())
            && let Some(id) = role_id(current, AccessibilityRole::CodeEditor)
        {
            self.push_notification(&mut intents, id, NotificationKind::Value);
        }
        for node in current.nodes().iter().filter(|node| node.announces()) {
            let changed = previous
                .nodes()
                .iter()
                .find(|prior| prior.id() == node.id())
                != Some(node);
            if changed {
                self.push_notification(&mut intents, node.id(), NotificationKind::Announcement);
            }
        }
        Ok(intents)
    }

    fn push_notification(
        &mut self,
        intents: &mut Vec<NotificationIntent>,
        id: AccessibilityNodeId,
        kind: NotificationKind,
    ) {
        if let Some(element) = self.element(id) {
            self.counters.notifications[kind as usize] =
                self.counters.notifications[kind as usize].saturating_add(1);
            intents.push(NotificationIntent { element, kind });
        }
    }

    fn revoke(&mut self) {
        self.handler.take();
        self.snapshot.take();
        self.counters.released_elements = self
            .counters
            .released_elements
            .saturating_add(u64::try_from(self.elements.len()).unwrap_or(u64::MAX));
        self.elements.clear();
        self.active = false;
        self.generation = self.generation.saturating_add(1);
    }

    fn valid(&self, generation: u64, id: AccessibilityNodeId) -> bool {
        self.active
            && self.generation == generation
            && self
                .snapshot
                .as_ref()
                .is_some_and(|snapshot| snapshot.nodes().iter().any(|node| node.id() == id))
    }

    fn node(&self, generation: u64, id: AccessibilityNodeId) -> Option<&AccessibilityNode> {
        if !self.valid(generation, id) {
            return None;
        }
        self.snapshot
            .as_ref()?
            .nodes()
            .iter()
            .find(|node| node.id() == id)
    }

    fn element(&self, id: AccessibilityNodeId) -> Option<Retained<NativeAccessibilityElement>> {
        self.elements
            .iter()
            .find(|entry| entry.id == id)
            .map(|entry| entry.element.clone())
    }

    fn children(
        &self,
        generation: u64,
        id: AccessibilityNodeId,
    ) -> Vec<Retained<NativeAccessibilityElement>> {
        let Some(snapshot) = self
            .snapshot
            .as_ref()
            .filter(|_| self.valid(generation, id))
        else {
            return Vec::new();
        };
        snapshot
            .nodes()
            .iter()
            .filter(|node| node.parent() == Some(id))
            .filter_map(|node| self.element(node.id()))
            .collect()
    }

    fn text(
        &mut self,
        revision: AccessibilityRevision,
        range: AccessibilityTextRange,
    ) -> Option<Box<str>> {
        let request = AccessibilityRequest::text(self.next_id().ok()?, revision, range).ok()?;
        let response = self.dispatch(&request).ok()?;
        match response.result() {
            Ok(AccessibilityPayload::Text(text)) => Some(text.as_str().into()),
            _ => None,
        }
    }

    fn range_request(
        &mut self,
        revision: AccessibilityRevision,
        build: impl FnOnce(AccessibilityRequestId) -> Result<AccessibilityRequest, AccessibilityError>,
    ) -> Option<AccessibilityTextRange> {
        let request = build(self.next_id().ok()?).ok()?;
        let response = self.dispatch(&request).ok()?;
        if response.observed_revision() != revision {
            return None;
        }
        match response.result() {
            Ok(AccessibilityPayload::Range(range)) => Some(*range),
            _ => None,
        }
    }

    fn line_for_index(&mut self, revision: AccessibilityRevision, index: usize) -> Option<usize> {
        let request =
            AccessibilityRequest::line_for_index(self.next_id().ok()?, revision, index).ok()?;
        let response = self.dispatch(&request).ok()?;
        match response.result() {
            Ok(AccessibilityPayload::Line(line)) => Some(*line),
            _ => None,
        }
    }

    fn set_selection(
        &mut self,
        revision: AccessibilityRevision,
        range: AccessibilityTextRange,
    ) -> bool {
        let request = AccessibilityRequest::action(
            match self.next_id() {
                Ok(id) => id,
                Err(_) => return false,
            },
            AccessibilityAction::set_selection(
                revision,
                range.start_utf16(),
                range.end_utf16().unwrap_or(range.start_utf16()),
            ),
        );
        let Ok(request) = request else {
            return false;
        };
        let Ok(response) = self.dispatch(&request) else {
            return false;
        };
        matches!(
            response.result(),
            Ok(AccessibilityPayload::Action(
                AccessibilityActionResult::Applied | AccessibilityActionResult::Unchanged
            ))
        )
    }

    fn snapshot_metadata(
        &self,
        generation: u64,
        id: AccessibilityNodeId,
    ) -> Option<(AccessibilityRevision, usize, AccessibilityTextRange)> {
        if self.node(generation, id)?.role() != AccessibilityRole::CodeEditor {
            return None;
        }
        let snapshot = self.snapshot.as_ref()?;
        Some((
            snapshot.revision(),
            snapshot.text_len_utf16(),
            snapshot.selection().range(),
        ))
    }

    #[cfg(alpine_native_validation)]
    fn retained_slot_bytes(&self) -> usize {
        self.elements
            .len()
            .saturating_mul(mem::size_of::<Retained<NativeAccessibilityElement>>())
    }

    #[cfg(alpine_native_validation)]
    pub(crate) fn validate_view(
        view: &Retained<SurfaceView>,
    ) -> Result<crate::native_validation::NativeAccessibilityEvidence, SurfaceError> {
        let roots: Retained<NSArray<NativeAccessibilityElement>> =
            unsafe { msg_send![&**view, accessibilityChildren] };
        let repeated: Retained<NSArray<NativeAccessibilityElement>> =
            unsafe { msg_send![&**view, accessibilityChildren] };
        let root = roots.firstObject().ok_or(SurfaceError::DriverUnavailable)?;
        let stable_root_identity = repeated
            .firstObject()
            .is_some_and(|candidate| core::ptr::eq(&*root, &*candidate));
        let editor = view
            .ivars()
            .accessibility
            .borrow()
            .snapshot
            .as_ref()
            .and_then(|snapshot| role_id(snapshot, AccessibilityRole::CodeEditor))
            .and_then(|id| view.ivars().accessibility.borrow().element(id))
            .ok_or(SurfaceError::DriverUnavailable)?;
        let role: Retained<NSString> = unsafe { msg_send![&*editor, accessibilityRole] };
        let label: Retained<NSString> = unsafe { msg_send![&*editor, accessibilityLabel] };
        let text_length_utf16: usize =
            unsafe { msg_send![&*editor, accessibilityNumberOfCharacters] };
        let selected_text: Retained<NSString> =
            unsafe { msg_send![&*editor, accessibilitySelectedText] };
        let bounded_text: Option<Retained<NSString>> =
            unsafe { msg_send![&*editor, accessibilityStringForRange: NSRange::new(5, 3)] };
        let bounded_text = bounded_text.ok_or(SurfaceError::DriverUnavailable)?;
        let selected_range: NSRange =
            unsafe { msg_send![&*editor, accessibilitySelectedTextRange] };
        let line_for_index: usize =
            unsafe { msg_send![&*editor, accessibilityLineForIndex: 6usize] };
        let range_for_line: NSRange =
            unsafe { msg_send![&*editor, accessibilityRangeForLine: 1usize] };
        let range_for_index: NSRange =
            unsafe { msg_send![&*editor, accessibilityRangeForIndex: 3usize] };
        let bounded_text_selector_allowed: bool = unsafe {
            msg_send![&*editor, isAccessibilitySelectorAllowed: sel!(accessibilityStringForRange:)]
        };
        let geometry_selector_allowed: bool = unsafe {
            msg_send![&*editor, isAccessibilitySelectorAllowed: sel!(accessibilityFrameForRange:)]
        };
        let tab = view
            .ivars()
            .accessibility
            .borrow()
            .snapshot
            .as_ref()
            .and_then(|snapshot| role_id(snapshot, AccessibilityRole::Tab))
            .and_then(|id| view.ivars().accessibility.borrow().element(id))
            .ok_or(SurfaceError::DriverUnavailable)?;
        let status = view
            .ivars()
            .accessibility
            .borrow()
            .snapshot
            .as_ref()
            .and_then(|snapshot| role_id(snapshot, AccessibilityRole::Status))
            .and_then(|id| view.ivars().accessibility.borrow().element(id))
            .ok_or(SurfaceError::DriverUnavailable)?;
        let editor_parent: Option<Retained<NativeAccessibilityElement>> =
            unsafe { msg_send![&*editor, accessibilityParent] };
        let editor_focused: bool = unsafe { msg_send![&*editor, isAccessibilityFocused] };
        let tab_selected: bool = unsafe { msg_send![&*tab, isAccessibilitySelected] };
        let status_value: Option<Retained<NSString>> =
            unsafe { msg_send![&*status, accessibilityValue] };
        let role_mapping_valid = view
            .ivars()
            .accessibility
            .borrow()
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| {
                snapshot.nodes().iter().all(|node| {
                    view.ivars()
                        .accessibility
                        .borrow()
                        .element(node.id())
                        .is_some_and(|element| {
                            let actual: Retained<NSString> =
                                unsafe { msg_send![&*element, accessibilityRole] };
                            actual.to_string() == role_name(node.role())
                        })
                })
            });
        let semantic_tree_valid = role_mapping_valid
            && editor_focused
            && tab_selected
            && status_value.is_some_and(|value| value.to_string() == "Ready")
            && editor_parent.is_some_and(|parent| core::ptr::eq(&*parent, &*root));
        let status_text_selector_allowed: bool = unsafe {
            msg_send![&*status, isAccessibilitySelectorAllowed: sel!(accessibilityStringForRange:)]
        };
        let status_character_count: usize =
            unsafe { msg_send![&*status, accessibilityNumberOfCharacters] };
        let text_selector_scope_valid =
            !status_text_selector_allowed && status_character_count == 0;
        let accepted_native = NSRange::new(2, 2);
        let _: () =
            unsafe { msg_send![&*editor, setAccessibilitySelectedTextRange: accepted_native] };
        let accepted_selection: NSRange =
            unsafe { msg_send![&*editor, accessibilitySelectedTextRange] };
        let stale_revision = view
            .ivars()
            .accessibility
            .borrow()
            .snapshot
            .as_ref()
            .map(AccessibilitySnapshot::revision)
            .ok_or(SurfaceError::DriverUnavailable)?;
        let stale_action_rejected = !view.ivars().accessibility.borrow_mut().set_selection(
            AccessibilityRevision::new(
                stale_revision.document(),
                stale_revision.buffer().saturating_sub(1),
            ),
            AccessibilityTextRange::new(0, 0),
        );
        let counters = view.ivars().accessibility.borrow().counters;
        let peak_elements = counters.peak_elements;
        let created_elements = counters.created_elements;
        let notification_counts = counters.notifications;
        let retained_slot_bytes_before_revoke =
            view.ivars().accessibility.borrow().retained_slot_bytes();
        Self::revoke_view(view);
        let late_length: usize = unsafe { msg_send![&*editor, accessibilityNumberOfCharacters] };
        let final_counters = view.ivars().accessibility.borrow().counters;
        let retained_slot_bytes_after_revoke =
            view.ivars().accessibility.borrow().retained_slot_bytes();
        Ok(crate::native_validation::NativeAccessibilityEvidence {
            root_children: roots.len(),
            stable_root_identity,
            role: role.to_string().into_boxed_str(),
            label: label.to_string().into_boxed_str(),
            text_length_utf16,
            selected_text: selected_text.to_string().into_boxed_str(),
            bounded_text: bounded_text.to_string().into_boxed_str(),
            selected_range: from_ns_range(selected_range).ok_or(SurfaceError::DriverUnavailable)?,
            line_for_index,
            range_for_line: from_ns_range(range_for_line).ok_or(SurfaceError::DriverUnavailable)?,
            range_for_index: from_ns_range(range_for_index)
                .ok_or(SurfaceError::DriverUnavailable)?,
            bounded_text_selector_allowed,
            geometry_selector_allowed,
            semantic_tree_valid,
            text_selector_scope_valid,
            accepted_selection: from_ns_range(accepted_selection)
                .ok_or(SurfaceError::DriverUnavailable)?,
            stale_action_rejected,
            peak_elements,
            created_elements,
            released_elements: final_counters.released_elements,
            notification_counts,
            current_elements_after_revoke: view.ivars().accessibility.borrow().elements.len(),
            retained_slot_bytes_before_revoke,
            retained_slot_bytes_after_revoke,
            late_selector_rejected: late_length == 0,
        })
    }
}

pub(crate) struct NativeAccessibilityElementIvars {
    view: Weak<SurfaceView>,
    id: AccessibilityNodeId,
    generation: u64,
    dispatch_failed: Cell<bool>,
}

define_class!(
    // SAFETY: NSAccessibilityElement supports subclassing, the element is
    // main-thread-only, and it retains no Studio or native owner.
    #[unsafe(super = NSAccessibilityElement)]
    #[thread_kind = MainThreadOnly]
    #[ivars = NativeAccessibilityElementIvars]
    pub(crate) struct NativeAccessibilityElement;

    unsafe impl NSObjectProtocol for NativeAccessibilityElement {}

    impl NativeAccessibilityElement {
        #[unsafe(method(isAccessibilityElement))]
        fn is_accessibility_element(&self) -> bool { self.with_adapter(|adapter| adapter.valid(self.ivars().generation, self.ivars().id)).unwrap_or(false) }

        #[unsafe(method_id(accessibilityRole))]
        fn accessibility_role(&self) -> Retained<NSString> {
            NSString::from_str(self.node().map_or("AXGroup", |node| role_name(node.role())))
        }

        #[unsafe(method_id(accessibilityLabel))]
        fn accessibility_label(&self) -> Retained<NSString> {
            self.node()
                .map_or_else(NSString::new, |node| NSString::from_str(node.name()))
        }

        #[unsafe(method_id(accessibilityValue))]
        fn accessibility_value(&self) -> Option<Retained<NSString>> {
            self.accessibility_value_impl()
        }

        #[unsafe(method_id(accessibilityParent))]
        fn accessibility_parent(&self) -> Option<Retained<NativeAccessibilityElement>> {
            self.accessibility_parent_impl()
        }

        #[unsafe(method_id(accessibilityChildren))]
        fn accessibility_children(&self) -> Retained<NSArray<NativeAccessibilityElement>> {
            let children = self.with_adapter(|adapter| adapter.children(self.ivars().generation, self.ivars().id)).unwrap_or_default();
            NSArray::from_retained_slice(&children)
        }

        #[unsafe(method(isAccessibilityFocused))]
        fn is_accessibility_focused(&self) -> bool { self.node().is_some_and(|node| node.is_focused()) }

        #[unsafe(method(isAccessibilitySelected))]
        fn is_accessibility_selected(&self) -> bool { self.node().is_some_and(|node| node.is_selected()) }

        #[unsafe(method(accessibilityNumberOfCharacters))]
        fn accessibility_number_of_characters(&self) -> usize {
            self.metadata().map_or(0, |(_, length, _)| length)
        }

        #[unsafe(method_id(accessibilitySelectedText))]
        fn accessibility_selected_text(&self) -> Retained<NSString> {
            self.accessibility_selected_text_impl()
        }

        #[unsafe(method(accessibilitySelectedTextRange))]
        fn accessibility_selected_text_range(&self) -> NSRange {
            self.metadata().map_or(not_found_range(), |(_, _, range)| to_ns_range(range))
        }

        #[unsafe(method_id(accessibilityStringForRange:))]
        fn accessibility_string_for_range(&self, range: NSRange) -> Option<Retained<NSString>> {
            self.accessibility_string_for_range_impl(range)
        }

        #[unsafe(method(accessibilityLineForIndex:))]
        fn accessibility_line_for_index(&self, index: usize) -> usize {
            let Some((revision, _, _)) = self.metadata().filter(|(_, length, _)| index <= *length) else { return usize::MAX; };
            self.with_adapter_mut(|adapter| adapter.line_for_index(revision, index)).flatten().unwrap_or(usize::MAX)
        }

        #[unsafe(method(accessibilityRangeForLine:))]
        fn accessibility_range_for_line(&self, line: usize) -> NSRange {
            let Some((revision, _, _)) = self.metadata() else { return not_found_range(); };
            self.with_adapter_mut(|adapter| adapter.range_request(revision, |id| AccessibilityRequest::range_for_line(id, revision, line))).flatten().map_or_else(not_found_range, to_ns_range)
        }

        #[unsafe(method(accessibilityRangeForIndex:))]
        fn accessibility_range_for_index(&self, index: usize) -> NSRange {
            let Some((revision, _, _)) = self.metadata().filter(|(_, length, _)| index <= *length) else { return not_found_range(); };
            self.with_adapter_mut(|adapter| adapter.range_request(revision, |id| AccessibilityRequest::range_for_index(id, revision, index))).flatten().map_or_else(not_found_range, to_ns_range)
        }

        #[unsafe(method(setAccessibilitySelectedTextRange:))]
        fn set_accessibility_selected_text_range(&self, range: NSRange) {
            let Some((revision, length, _)) = self.metadata() else { return; };
            let Some(range) = checked_range(range, length) else { return; };
            let applied = self.with_adapter_mut(|adapter| adapter.set_selection(revision, range)).unwrap_or(false);
            if applied
                && let Some(view) = self.ivars().view.load()
                && NativeAccessibilityAdapter::refresh_view(&view).is_err()
            {
                self.ivars().dispatch_failed.set(true);
            }
        }

        #[unsafe(method(isAccessibilitySelectorAllowed:))]
        fn is_accessibility_selector_allowed(&self, selector: Sel) -> bool {
            let text_editor = self
                .node()
                .is_some_and(|node| node.role() == AccessibilityRole::CodeEditor);
            selector == sel!(accessibilityRole)
                || selector == sel!(accessibilityLabel)
                || selector == sel!(accessibilityValue)
                || selector == sel!(accessibilityParent)
                || selector == sel!(accessibilityChildren)
                || selector == sel!(isAccessibilityFocused)
                || selector == sel!(isAccessibilitySelected)
                || (text_editor
                    && (selector == sel!(accessibilityNumberOfCharacters)
                        || selector == sel!(accessibilitySelectedText)
                        || selector == sel!(accessibilitySelectedTextRange)
                        || selector == sel!(accessibilityStringForRange:)
                        || selector == sel!(accessibilityLineForIndex:)
                        || selector == sel!(accessibilityRangeForLine:)
                        || selector == sel!(accessibilityRangeForIndex:)
                        || selector == sel!(setAccessibilitySelectedTextRange:)))
        }
    }
);

impl NativeAccessibilityElement {
    fn new(
        main_thread: MainThreadMarker,
        view: &SurfaceView,
        id: AccessibilityNodeId,
        generation: u64,
    ) -> Retained<Self> {
        let retained_view = view.retain();
        let allocated = Self::alloc(main_thread).set_ivars(NativeAccessibilityElementIvars {
            view: Weak::from_retained(&retained_view),
            id,
            generation,
            dispatch_failed: Cell::new(false),
        });
        // SAFETY: NSAccessibilityElement's parameterless initializer accepts a
        // fully initialized main-thread-only subclass instance.
        unsafe { msg_send![super(allocated), init] }
    }

    fn with_adapter<R>(
        &self,
        operation: impl FnOnce(&NativeAccessibilityAdapter) -> R,
    ) -> Option<R> {
        let view = self.ivars().view.load()?;
        let borrowed = view.ivars().accessibility.try_borrow().ok()?;
        Some(operation(&borrowed))
    }

    fn with_adapter_mut<R>(
        &self,
        operation: impl FnOnce(&mut NativeAccessibilityAdapter) -> R,
    ) -> Option<R> {
        let view = self.ivars().view.load()?;
        let mut borrowed = view.ivars().accessibility.try_borrow_mut().ok()?;
        Some(operation(&mut borrowed))
    }

    fn node(&self) -> Option<AccessibilityNode> {
        self.with_adapter(|adapter| {
            adapter
                .node(self.ivars().generation, self.ivars().id)
                .cloned()
        })
        .flatten()
    }

    fn accessibility_value_impl(&self) -> Option<Retained<NSString>> {
        let node = self.node()?;
        (node.role() != AccessibilityRole::CodeEditor).then(|| NSString::from_str(node.name()))
    }

    fn accessibility_parent_impl(&self) -> Option<Retained<NativeAccessibilityElement>> {
        let parent = self.node()?.parent()?;
        self.with_adapter(|adapter| adapter.element(parent))
            .flatten()
    }

    fn accessibility_selected_text_impl(&self) -> Retained<NSString> {
        let Some((revision, _, range)) = self.metadata() else {
            return NSString::new();
        };
        let text = self
            .with_adapter_mut(|adapter| adapter.text(revision, range))
            .flatten()
            .unwrap_or_default();
        NSString::from_str(&text)
    }

    fn accessibility_string_for_range_impl(&self, range: NSRange) -> Option<Retained<NSString>> {
        let (revision, length, _) = self.metadata()?;
        let range = checked_range(range, length)?;
        self.with_adapter_mut(|adapter| adapter.text(revision, range))
            .flatten()
            .map(|text| NSString::from_str(&text))
    }

    fn metadata(&self) -> Option<(AccessibilityRevision, usize, AccessibilityTextRange)> {
        self.with_adapter(|adapter| {
            adapter.snapshot_metadata(self.ivars().generation, self.ivars().id)
        })
        .flatten()
    }
}

fn role_id(
    snapshot: &AccessibilitySnapshot,
    role: AccessibilityRole,
) -> Option<AccessibilityNodeId> {
    snapshot
        .nodes()
        .iter()
        .find(|node| node.role() == role)
        .map(AccessibilityNode::id)
}

fn structural_tree_changed(
    previous: &AccessibilitySnapshot,
    current: &AccessibilitySnapshot,
) -> bool {
    previous.root() != current.root()
        || previous.nodes().len() != current.nodes().len()
        || previous
            .nodes()
            .iter()
            .zip(current.nodes())
            .any(|(left, right)| {
                left.id() != right.id()
                    || left.parent() != right.parent()
                    || left.role() != right.role()
                    || left.name() != right.name()
            })
}

const fn role_name(role: AccessibilityRole) -> &'static str {
    match role {
        AccessibilityRole::Window | AccessibilityRole::Dialog => "AXGroup",
        AccessibilityRole::TabList => "AXTabGroup",
        AccessibilityRole::Tab => "AXRadioButton",
        AccessibilityRole::CodeEditor => "AXTextArea",
        AccessibilityRole::FileTree => "AXOutline",
        AccessibilityRole::SearchField => "AXTextField",
        AccessibilityRole::Status => "AXStaticText",
    }
}

const fn to_ns_range(range: AccessibilityTextRange) -> NSRange {
    NSRange::new(range.start_utf16(), range.length_utf16())
}

#[cfg(alpine_native_validation)]
const fn from_ns_range(range: NSRange) -> Option<AccessibilityTextRange> {
    if range.location == usize::MAX {
        None
    } else {
        Some(AccessibilityTextRange::new(range.location, range.length))
    }
}

fn checked_range(range: NSRange, text_length: usize) -> Option<AccessibilityTextRange> {
    if range.location == usize::MAX || range.location.checked_add(range.length)? > text_length {
        return None;
    }
    Some(AccessibilityTextRange::new(range.location, range.length))
}

const fn not_found_range() -> NSRange {
    NSRange::new(usize::MAX, 0)
}
