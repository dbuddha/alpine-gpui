//! Private `AppKit` accessibility adapter over Alpine's bounded semantic transport.

use core::mem;
use std::{cell::Cell, sync::Arc};

use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send,
    rc::{Retained, Weak},
    runtime::{AnyObject, Bool, NSObjectProtocol, ProtocolObject, Sel},
    sel,
};
use objc2_app_kit::{
    NSAccessibilityAnnouncementKey, NSAccessibilityAnnouncementRequestedNotification,
    NSAccessibilityElement, NSAccessibilityFocusedUIElementChangedNotification,
    NSAccessibilityLayoutChangedNotification, NSAccessibilityPostNotification,
    NSAccessibilityPostNotificationWithUserInfo, NSAccessibilityPriorityKey,
    NSAccessibilityPriorityLevel, NSAccessibilitySelectedTextChangedNotification,
    NSAccessibilityUIElementDestroyedNotification, NSAccessibilityUIElementsKey,
    NSAccessibilityValueChangedNotification,
};
use objc2_foundation::{
    NSArray, NSMutableDictionary, NSNumber, NSPoint, NSRange, NSRect, NSSize, NSString,
};

use crate::{
    AccessibilityAction, AccessibilityActionResult, AccessibilityError, AccessibilityNode,
    AccessibilityNodeId, AccessibilityPayload, AccessibilityRequest, AccessibilityRequestId,
    AccessibilityResponse, AccessibilityRevision, AccessibilityRole, AccessibilitySnapshot,
    AccessibilityTextRange, SurfaceError, native::SurfaceView,
};

#[cfg(alpine_native_validation)]
use crate::AccessibilityBounds;

type RequestHandler =
    Box<dyn FnMut(&AccessibilityRequest) -> Result<AccessibilityResponse, SurfaceError> + 'static>;

#[derive(Clone, Copy)]
enum NotificationKind {
    Layout = 0,
    Focus = 1,
    Selection = 2,
    Value = 3,
    Announcement = 4,
    Destroyed = 5,
}

#[derive(Clone, Copy, Default)]
struct PostedNotificationRecord {
    #[cfg(alpine_native_validation)]
    kind_index: u8,
    #[cfg(alpine_native_validation)]
    target: u64,
    #[cfg(alpine_native_validation)]
    payload_elements: usize,
    payload_bytes: usize,
    #[cfg(alpine_native_validation)]
    priority: isize,
}

impl PostedNotificationRecord {
    #[cfg(alpine_native_validation)]
    const EMPTY: Self = Self {
        kind_index: 0,
        target: 0,
        payload_elements: 0,
        payload_bytes: 0,
        priority: 0,
    };

    const fn payload_bytes(self) -> usize {
        self.payload_bytes
    }
}

enum NotificationIntent {
    Plain {
        element: Retained<NativeAccessibilityElement>,
        id: AccessibilityNodeId,
        kind: NotificationKind,
    },
    Layout {
        element: Retained<NativeAccessibilityElement>,
        id: AccessibilityNodeId,
        affected: Vec<Retained<NativeAccessibilityElement>>,
    },
    Announcement {
        element: Retained<NativeAccessibilityElement>,
        id: AccessibilityNodeId,
        text: Arc<str>,
    },
    Destroyed {
        element: Retained<NativeAccessibilityElement>,
        id: AccessibilityNodeId,
    },
}

struct RefreshOutcome {
    notifications: Vec<NotificationIntent>,
}

struct NotificationPostReceipt {
    invoked: bool,
    user_info_valid: bool,
}

struct PostedNotifications {
    counts: [u64; 6],
    payload_bytes: usize,
    peak_retained_bytes: usize,
    #[cfg(alpine_native_validation)]
    records: [PostedNotificationRecord; MAX_VALIDATION_NOTIFICATION_RECORDS],
    #[cfg(alpine_native_validation)]
    record_count: usize,
    #[cfg(alpine_native_validation)]
    omitted_records: u64,
    #[cfg(alpine_native_validation)]
    invalid_user_info: u64,
}

impl RefreshOutcome {
    fn post(self) -> PostedNotifications {
        let peak_retained_bytes = self.retained_bytes();
        let mut posted = PostedNotifications {
            counts: [0; 6],
            payload_bytes: 0,
            peak_retained_bytes,
            #[cfg(alpine_native_validation)]
            records: [PostedNotificationRecord::EMPTY; MAX_VALIDATION_NOTIFICATION_RECORDS],
            #[cfg(alpine_native_validation)]
            record_count: 0,
            #[cfg(alpine_native_validation)]
            omitted_records: 0,
            #[cfg(alpine_native_validation)]
            invalid_user_info: 0,
        };
        for intent in self.notifications {
            let kind = intent.kind();
            let record = intent.record();
            let receipt = intent.post();
            if !receipt.invoked {
                continue;
            }
            posted.payload_bytes = posted.payload_bytes.saturating_add(record.payload_bytes());
            #[cfg(alpine_native_validation)]
            posted.push_record(record);
            #[cfg(alpine_native_validation)]
            if !receipt.user_info_valid {
                posted.invalid_user_info = posted.invalid_user_info.saturating_add(1);
            }
            #[cfg(not(alpine_native_validation))]
            let _ = receipt.user_info_valid;
            posted.counts[kind as usize] = posted.counts[kind as usize].saturating_add(1);
        }
        posted
    }

    fn retained_bytes(&self) -> usize {
        self.notifications.iter().fold(0_usize, |bytes, intent| {
            bytes.saturating_add(intent.retained_bytes())
        })
    }
}

impl PostedNotifications {
    #[cfg(alpine_native_validation)]
    fn push_record(&mut self, record: PostedNotificationRecord) {
        if let Some(slot) = self.records.get_mut(self.record_count) {
            *slot = record;
            self.record_count = self.record_count.saturating_add(1);
        } else {
            self.omitted_records = self.omitted_records.saturating_add(1);
        }
    }
}

impl NotificationIntent {
    fn kind(&self) -> NotificationKind {
        match self {
            Self::Plain { kind, .. } => *kind,
            Self::Layout { .. } => NotificationKind::Layout,
            Self::Announcement { .. } => NotificationKind::Announcement,
            Self::Destroyed { .. } => NotificationKind::Destroyed,
        }
    }

    fn record(&self) -> PostedNotificationRecord {
        let (_id, _payload_elements, payload_bytes, _priority) = match self {
            Self::Plain { id, .. } | Self::Destroyed { id, .. } => (*id, 0, 0, 0),
            Self::Layout { id, affected, .. } => (
                *id,
                affected.len(),
                affected
                    .len()
                    .saturating_mul(mem::size_of::<Retained<NativeAccessibilityElement>>()),
                0,
            ),
            Self::Announcement { id, text, .. } => {
                (*id, 0, text.len(), NSAccessibilityPriorityLevel::Medium.0)
            }
        };
        PostedNotificationRecord {
            #[cfg(alpine_native_validation)]
            kind_index: self.kind() as u8,
            #[cfg(alpine_native_validation)]
            target: _id.get(),
            #[cfg(alpine_native_validation)]
            payload_elements: _payload_elements,
            payload_bytes,
            #[cfg(alpine_native_validation)]
            priority: _priority,
        }
    }

    fn retained_bytes(&self) -> usize {
        let element_slot = mem::size_of::<Retained<NativeAccessibilityElement>>();
        match self {
            Self::Plain { .. } | Self::Destroyed { .. } => element_slot,
            Self::Layout { affected, .. } => {
                element_slot.saturating_add(affected.len().saturating_mul(element_slot))
            }
            Self::Announcement { text, .. } => element_slot.saturating_add(text.len()),
        }
    }

    fn post(self) -> NotificationPostReceipt {
        // SAFETY: every target and payload element remains retained for this
        // synchronous AppKit call, and notification constants are immutable.
        unsafe {
            match self {
                Self::Plain { element, kind, .. } => {
                    let notification = match kind {
                        NotificationKind::Focus => {
                            NSAccessibilityFocusedUIElementChangedNotification
                        }
                        NotificationKind::Selection => {
                            NSAccessibilitySelectedTextChangedNotification
                        }
                        NotificationKind::Value => NSAccessibilityValueChangedNotification,
                        NotificationKind::Layout
                        | NotificationKind::Announcement
                        | NotificationKind::Destroyed => {
                            return NotificationPostReceipt {
                                invoked: false,
                                user_info_valid: false,
                            };
                        }
                    };
                    NSAccessibilityPostNotification(&element, notification);
                    NotificationPostReceipt {
                        invoked: true,
                        user_info_valid: true,
                    }
                }
                Self::Layout {
                    element, affected, ..
                } => {
                    let elements = NSArray::from_retained_slice(&affected);
                    let user_info = NSMutableDictionary::<_, AnyObject>::new();
                    user_info.setObject_forKey(
                        &*elements,
                        ProtocolObject::from_ref(NSAccessibilityUIElementsKey),
                    );
                    let valid = layout_user_info_valid(
                        user_info.count(),
                        user_info
                            .objectForKey(NSAccessibilityUIElementsKey)
                            .is_some(),
                    );
                    NSAccessibilityPostNotificationWithUserInfo(
                        &element,
                        NSAccessibilityLayoutChangedNotification,
                        Some(&user_info),
                    );
                    NotificationPostReceipt {
                        invoked: true,
                        user_info_valid: valid,
                    }
                }
                Self::Announcement { element, text, .. } => {
                    let text = NSString::from_str(&text);
                    let priority = NSNumber::new_isize(NSAccessibilityPriorityLevel::Medium.0);
                    let user_info = NSMutableDictionary::<_, AnyObject>::new();
                    user_info.setObject_forKey(
                        &*text,
                        ProtocolObject::from_ref(NSAccessibilityAnnouncementKey),
                    );
                    user_info.setObject_forKey(
                        &*priority,
                        ProtocolObject::from_ref(NSAccessibilityPriorityKey),
                    );
                    let valid = announcement_user_info_valid(
                        user_info.count(),
                        user_info
                            .objectForKey(NSAccessibilityAnnouncementKey)
                            .is_some(),
                        user_info.objectForKey(NSAccessibilityPriorityKey).is_some(),
                    );
                    NSAccessibilityPostNotificationWithUserInfo(
                        &element,
                        NSAccessibilityAnnouncementRequestedNotification,
                        Some(&user_info),
                    );
                    NotificationPostReceipt {
                        invoked: true,
                        user_info_valid: valid,
                    }
                }
                Self::Destroyed { element, .. } => {
                    NSAccessibilityPostNotification(
                        &element,
                        NSAccessibilityUIElementDestroyedNotification,
                    );
                    NotificationPostReceipt {
                        invoked: true,
                        user_info_valid: true,
                    }
                }
            }
        }
    }
}

const fn layout_user_info_valid(entry_count: usize, has_elements: bool) -> bool {
    entry_count == 1 && has_elements
}

const fn announcement_user_info_valid(
    entry_count: usize,
    has_announcement: bool,
    has_priority: bool,
) -> bool {
    entry_count == 2 && has_announcement && has_priority
}

#[cfg(alpine_native_validation)]
const MAX_VALIDATION_NOTIFICATION_RECORDS: usize = 32;

struct CachedElement {
    id: AccessibilityNodeId,
    instance_generation: u64,
    element: Retained<NativeAccessibilityElement>,
}

#[derive(Clone, Copy, Default)]
struct AdapterCounters {
    peak_elements: usize,
    created_elements: u64,
    released_elements: u64,
    requests: u64,
    failed_requests: u64,
    notifications: [u64; 6],
    posted_payload_bytes: usize,
    peak_notification_retained_bytes: usize,
    posts_after_handler_revocation: u64,
    revoke_starts: u64,
    #[cfg(alpine_native_validation)]
    notification_records: [PostedNotificationRecord; MAX_VALIDATION_NOTIFICATION_RECORDS],
    #[cfg(alpine_native_validation)]
    notification_record_count: usize,
    #[cfg(alpine_native_validation)]
    omitted_notification_records: u64,
    #[cfg(alpine_native_validation)]
    invalid_notification_user_info: u64,
}

pub(crate) struct NativeAccessibilityAdapter {
    handler: Option<RequestHandler>,
    snapshot: Option<AccessibilitySnapshot>,
    elements: Vec<CachedElement>,
    next_request_id: u64,
    generation: u64,
    next_element_generation: u64,
    active: bool,
    revoking: bool,
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
            next_element_generation: 1,
            active: false,
            revoking: false,
            counters: AdapterCounters {
                peak_elements: 0,
                created_elements: 0,
                released_elements: 0,
                requests: 0,
                failed_requests: 0,
                notifications: [0; 6],
                posted_payload_bytes: 0,
                peak_notification_retained_bytes: 0,
                posts_after_handler_revocation: 0,
                revoke_starts: 0,
                #[cfg(alpine_native_validation)]
                notification_records: [PostedNotificationRecord::EMPTY;
                    MAX_VALIDATION_NOTIFICATION_RECORDS],
                #[cfg(alpine_native_validation)]
                notification_record_count: 0,
                #[cfg(alpine_native_validation)]
                omitted_notification_records: 0,
                #[cfg(alpine_native_validation)]
                invalid_notification_user_info: 0,
            },
        }
    }

    pub(crate) fn install(&mut self, handler: RequestHandler) {
        self.handler = Some(handler);
        self.revoking = false;
    }

    pub(crate) fn refresh_view_if_active(view: &SurfaceView) -> Result<(), SurfaceError> {
        if !view.ivars().accessibility.borrow().active {
            return Ok(());
        }
        Self::refresh_view(view)
    }

    pub(crate) fn refresh_view(view: &SurfaceView) -> Result<(), SurfaceError> {
        let outcome = view.ivars().accessibility.borrow_mut().refresh(view)?;
        let posted = outcome.post();
        view.ivars()
            .accessibility
            .borrow_mut()
            .record_posted(&posted);
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
        let outcome = view.ivars().accessibility.borrow_mut().begin_revoke();
        let Some(outcome) = outcome else {
            return;
        };
        let posted = outcome.post();
        let mut adapter = view.ivars().accessibility.borrow_mut();
        adapter.record_posted(&posted);
        adapter.finish_revoke();
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
        if self.revoking || self.handler.is_none() {
            return Err(SurfaceError::DriverUnavailable);
        }
        let request = AccessibilityRequest::snapshot(self.next_id()?)
            .map_err(|_| SurfaceError::DriverUnavailable)?;
        let response = self.dispatch(&request)?;
        let snapshot = match response.result() {
            Ok(AccessibilityPayload::Snapshot(snapshot)) => snapshot.clone(),
            _ => return Err(SurfaceError::DriverUnavailable),
        };
        let previous = self.snapshot.clone();
        let mut notifications = Vec::new();
        notifications
            .try_reserve(
                self.elements
                    .len()
                    .saturating_add(snapshot.nodes().len())
                    .saturating_add(5),
            )
            .map_err(|_| SurfaceError::DriverUnavailable)?;
        self.reconcile_elements(view, &snapshot, &mut notifications)?;
        self.append_notification_intents(previous.as_ref(), &snapshot, &mut notifications)?;
        self.snapshot = Some(snapshot);
        self.active = true;
        Ok(RefreshOutcome { notifications })
    }

    fn reconcile_elements(
        &mut self,
        view: &SurfaceView,
        snapshot: &AccessibilitySnapshot,
        notifications: &mut Vec<NotificationIntent>,
    ) -> Result<(), SurfaceError> {
        let mut next = Vec::new();
        next.try_reserve_exact(snapshot.nodes().len())
            .map_err(|_| SurfaceError::DriverUnavailable)?;
        let mut created = 0_u64;
        let main_thread = MainThreadMarker::new().ok_or(SurfaceError::DriverUnavailable)?;
        for node in snapshot.nodes() {
            let reusable = self
                .elements
                .iter()
                .find(|entry| entry.id == node.id())
                .and_then(|entry| {
                    self.snapshot
                        .as_ref()
                        .and_then(|prior| {
                            prior.nodes().iter().find(|prior| prior.id() == node.id())
                        })
                        .filter(|prior| reusable_semantics(prior, node))
                        .map(|_| (entry.element.clone(), entry.instance_generation))
                });
            let (element, instance_generation) = if let Some(reusable) = reusable {
                reusable
            } else {
                let instance_generation = self.next_element_generation;
                self.next_element_generation = self
                    .next_element_generation
                    .checked_add(1)
                    .ok_or(SurfaceError::DriverUnavailable)?;
                created = created.saturating_add(1);
                (
                    NativeAccessibilityElement::new(
                        main_thread,
                        view,
                        node.id(),
                        self.generation,
                        instance_generation,
                    ),
                    instance_generation,
                )
            };
            next.push(CachedElement {
                id: node.id(),
                instance_generation,
                element,
            });
        }
        let retired = self
            .elements
            .iter()
            .filter(|entry| {
                !next.iter().any(|candidate| {
                    candidate.id == entry.id
                        && candidate.instance_generation == entry.instance_generation
                })
            })
            .map(|entry| (entry.id, entry.element.clone()))
            .collect::<Vec<_>>();
        self.elements = next;
        notifications.extend(
            retired
                .iter()
                .map(|(id, element)| NotificationIntent::Destroyed {
                    element: element.clone(),
                    id: *id,
                }),
        );
        self.counters.created_elements = self.counters.created_elements.saturating_add(created);
        self.counters.released_elements = self
            .counters
            .released_elements
            .saturating_add(u64::try_from(retired.len()).unwrap_or(u64::MAX));
        self.counters.peak_elements = self.counters.peak_elements.max(self.elements.len());
        Ok(())
    }

    fn append_notification_intents(
        &self,
        previous: Option<&AccessibilitySnapshot>,
        current: &AccessibilitySnapshot,
        intents: &mut Vec<NotificationIntent>,
    ) -> Result<(), SurfaceError> {
        let Some(previous) = previous else {
            return Ok(());
        };
        let tree_changed = structural_tree_changed(previous, current);
        if tree_changed {
            self.push_layout_notification(intents, previous, current)?;
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
            self.push_notification(intents, id, NotificationKind::Focus);
        }
        if previous.selection() != current.selection()
            && let Some(id) = role_id(current, AccessibilityRole::CodeEditor)
        {
            self.push_notification(intents, id, NotificationKind::Selection);
        }
        if (previous.revision() != current.revision()
            || previous.text_len_utf16() != current.text_len_utf16()
            || previous.is_dirty() != current.is_dirty())
            && let Some(id) = role_id(current, AccessibilityRole::CodeEditor)
        {
            self.push_notification(intents, id, NotificationKind::Value);
        }
        for node in current.nodes().iter().filter(|node| node.announces()) {
            let changed = previous
                .nodes()
                .iter()
                .find(|prior| prior.id() == node.id())
                != Some(node);
            if changed {
                self.push_announcement(intents, current.root(), node);
            }
        }
        Ok(())
    }

    fn push_notification(
        &self,
        intents: &mut Vec<NotificationIntent>,
        id: AccessibilityNodeId,
        kind: NotificationKind,
    ) {
        if let Some(element) = self.element(id) {
            intents.push(NotificationIntent::Plain { element, id, kind });
        }
    }

    fn push_layout_notification(
        &self,
        intents: &mut Vec<NotificationIntent>,
        previous: &AccessibilitySnapshot,
        current: &AccessibilitySnapshot,
    ) -> Result<(), SurfaceError> {
        let Some(element) = self.element(current.root()) else {
            return Ok(());
        };
        let mut affected = Vec::new();
        affected
            .try_reserve_exact(current.nodes().len())
            .map_err(|_| SurfaceError::DriverUnavailable)?;
        for node in current.nodes() {
            let changed = node.id() == current.root()
                || previous
                    .nodes()
                    .iter()
                    .find(|prior| prior.id() == node.id())
                    .is_none_or(|prior| layout_semantics_changed(prior, node));
            if changed && let Some(affected_element) = self.element(node.id()) {
                affected.push(affected_element);
            }
        }
        intents.push(NotificationIntent::Layout {
            element,
            id: current.root(),
            affected,
        });
        Ok(())
    }

    fn push_announcement(
        &self,
        intents: &mut Vec<NotificationIntent>,
        root: AccessibilityNodeId,
        source: &AccessibilityNode,
    ) {
        if let Some(element) = self.element(root) {
            intents.push(NotificationIntent::Announcement {
                element,
                id: root,
                text: source.retained_name(),
            });
        }
    }

    fn record_posted(&mut self, posted: &PostedNotifications) {
        if self.handler.is_none() {
            self.counters.posts_after_handler_revocation = self
                .counters
                .posts_after_handler_revocation
                .saturating_add(posted.counts.into_iter().sum());
        }
        for (count, posted_count) in self.counters.notifications.iter_mut().zip(posted.counts) {
            *count = count.saturating_add(posted_count);
        }
        self.counters.posted_payload_bytes = self
            .counters
            .posted_payload_bytes
            .saturating_add(posted.payload_bytes);
        self.counters.peak_notification_retained_bytes = self
            .counters
            .peak_notification_retained_bytes
            .max(posted.peak_retained_bytes);
        #[cfg(alpine_native_validation)]
        {
            for record in posted.records.iter().copied().take(posted.record_count) {
                if let Some(slot) = self
                    .counters
                    .notification_records
                    .get_mut(self.counters.notification_record_count)
                {
                    *slot = record;
                    self.counters.notification_record_count =
                        self.counters.notification_record_count.saturating_add(1);
                } else {
                    self.counters.omitted_notification_records =
                        self.counters.omitted_notification_records.saturating_add(1);
                }
            }
            self.counters.omitted_notification_records = self
                .counters
                .omitted_notification_records
                .saturating_add(posted.omitted_records);
            self.counters.invalid_notification_user_info = self
                .counters
                .invalid_notification_user_info
                .saturating_add(posted.invalid_user_info);
        }
    }

    fn begin_revoke(&mut self) -> Option<RefreshOutcome> {
        if self.revoking || self.handler.is_none() {
            return None;
        }
        self.counters.revoke_starts = self.counters.revoke_starts.saturating_add(1);
        self.revoking = true;
        self.active = false;
        self.snapshot.take();
        let retired = mem::take(&mut self.elements);
        let released = u64::try_from(retired.len()).unwrap_or(u64::MAX);
        self.counters.released_elements = self.counters.released_elements.saturating_add(released);
        self.generation = self.generation.saturating_add(1);
        Some(RefreshOutcome {
            notifications: retired
                .into_iter()
                .map(|entry| NotificationIntent::Destroyed {
                    element: entry.element,
                    id: entry.id,
                })
                .collect(),
        })
    }

    fn finish_revoke(&mut self) {
        self.handler.take();
        self.revoking = false;
    }

    fn valid(&self, generation: u64, instance_generation: u64, id: AccessibilityNodeId) -> bool {
        self.active
            && self.generation == generation
            && self
                .elements
                .iter()
                .any(|entry| entry.id == id && entry.instance_generation == instance_generation)
    }

    fn node(
        &self,
        generation: u64,
        instance_generation: u64,
        id: AccessibilityNodeId,
    ) -> Option<&AccessibilityNode> {
        if !self.valid(generation, instance_generation, id) {
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
        instance_generation: u64,
        id: AccessibilityNodeId,
    ) -> Vec<Retained<NativeAccessibilityElement>> {
        let Some(snapshot) = self
            .snapshot
            .as_ref()
            .filter(|_| self.valid(generation, instance_generation, id))
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

    fn activate(
        &mut self,
        generation: u64,
        instance_generation: u64,
        id: AccessibilityNodeId,
    ) -> bool {
        let Some((revision, enabled)) =
            self.node(generation, instance_generation, id).map(|node| {
                (
                    self.snapshot.as_ref().map(AccessibilitySnapshot::revision),
                    node.supports_activate() && node.is_enabled(),
                )
            })
        else {
            return false;
        };
        let Some(revision) = revision.filter(|_| enabled) else {
            return false;
        };
        let Ok(request_id) = self.next_id() else {
            return false;
        };
        let Ok(request) =
            AccessibilityRequest::action(request_id, AccessibilityAction::activate(revision, id))
        else {
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
        instance_generation: u64,
        id: AccessibilityNodeId,
    ) -> Option<(AccessibilityRevision, usize, AccessibilityTextRange)> {
        if self.node(generation, instance_generation, id)?.role() != AccessibilityRole::CodeEditor {
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
        let identifier: Retained<NSString> = unsafe { msg_send![&*tab, accessibilityIdentifier] };
        let repeated_identifier: Retained<NSString> =
            unsafe { msg_send![&*tab, accessibilityIdentifier] };
        let stable_external_identifier = !identifier.to_string().is_empty()
            && identifier.to_string() == repeated_identifier.to_string();
        let frame: NSRect = unsafe { msg_send![&*tab, accessibilityFrame] };
        let bounded_screen_frame = frame.origin.x.is_finite()
            && frame.origin.y.is_finite()
            && frame.size.width.is_finite()
            && frame.size.height.is_finite()
            && frame.size.width > 0.0
            && frame.size.height > 0.0;
        let tab_activate_selector_allowed: bool = unsafe {
            msg_send![&*tab, isAccessibilitySelectorAllowed: sel!(accessibilityPerformPress)]
        };
        let accepted_activation: bool = unsafe { msg_send![&*tab, accessibilityPerformPress] };
        let status = view
            .ivars()
            .accessibility
            .borrow()
            .snapshot
            .as_ref()
            .and_then(|snapshot| role_id(snapshot, AccessibilityRole::Status))
            .and_then(|id| view.ivars().accessibility.borrow().element(id))
            .ok_or(SurfaceError::DriverUnavailable)?;
        let status_activate_selector_allowed: bool = unsafe {
            msg_send![&*status, isAccessibilitySelectorAllowed: sel!(accessibilityPerformPress)]
        };
        let status_activation: bool = unsafe { msg_send![&*status, accessibilityPerformPress] };
        let activate_selector_allowed = tab_activate_selector_allowed
            && !status_activate_selector_allowed
            && !status_activation;
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
        let reusable_semantics_valid = {
            let snapshot = view
                .ivars()
                .accessibility
                .borrow()
                .snapshot
                .clone()
                .ok_or(SurfaceError::DriverUnavailable)?;
            let current = snapshot
                .nodes()
                .iter()
                .find(|node| node.role() == AccessibilityRole::Tab)
                .ok_or(SurfaceError::DriverUnavailable)?;
            let changed_parent = AccessibilityNode::new(
                current.id(),
                None,
                current.role(),
                current.name().into(),
                current.is_focused(),
                current.is_selected(),
                current.announces(),
            )
            .map_err(|_| SurfaceError::DriverUnavailable)?
            .with_bounds(current.bounds())
            .with_activate(current.is_enabled());
            let changed_role = AccessibilityNode::new(
                current.id(),
                current.parent(),
                AccessibilityRole::Status,
                current.name().into(),
                current.is_focused(),
                current.is_selected(),
                current.announces(),
            )
            .map_err(|_| SurfaceError::DriverUnavailable)?
            .with_bounds(current.bounds())
            .with_activate(current.is_enabled());
            let changed_name = AccessibilityNode::new(
                current.id(),
                current.parent(),
                current.role(),
                "different semantic target".into(),
                current.is_focused(),
                current.is_selected(),
                current.announces(),
            )
            .map_err(|_| SurfaceError::DriverUnavailable)?
            .with_bounds(current.bounds())
            .with_activate(current.is_enabled());
            let changed_action = AccessibilityNode::new(
                current.id(),
                current.parent(),
                current.role(),
                current.name().into(),
                current.is_focused(),
                current.is_selected(),
                current.announces(),
            )
            .map_err(|_| SurfaceError::DriverUnavailable)?
            .with_bounds(current.bounds());
            reusable_semantics(current, current)
                && !reusable_semantics(current, &changed_parent)
                && !reusable_semantics(current, &changed_role)
                && !reusable_semantics(current, &changed_name)
                && !reusable_semantics(current, &changed_action)
        };
        let layout_semantics_changed_valid = {
            let snapshot = view
                .ivars()
                .accessibility
                .borrow()
                .snapshot
                .clone()
                .ok_or(SurfaceError::DriverUnavailable)?;
            let current = snapshot
                .nodes()
                .iter()
                .find(|node| node.role() == AccessibilityRole::Tab)
                .ok_or(SurfaceError::DriverUnavailable)?;
            let make_node = |parent, role, name: Arc<str>, bounds, activate| {
                AccessibilityNode::new(
                    current.id(),
                    parent,
                    role,
                    name,
                    current.is_focused(),
                    current.is_selected(),
                    current.announces(),
                )
                .map(|node| {
                    let node = node.with_bounds(bounds);
                    match activate {
                        Some(enabled) => node.with_activate(enabled),
                        None => node,
                    }
                })
            };
            let changed_parent = make_node(
                None,
                current.role(),
                current.name().into(),
                current.bounds(),
                Some(current.is_enabled()),
            )
            .map_err(|_| SurfaceError::DriverUnavailable)?;
            let changed_role = make_node(
                current.parent(),
                AccessibilityRole::Status,
                current.name().into(),
                current.bounds(),
                Some(current.is_enabled()),
            )
            .map_err(|_| SurfaceError::DriverUnavailable)?;
            let changed_name = make_node(
                current.parent(),
                current.role(),
                "different layout target".into(),
                current.bounds(),
                Some(current.is_enabled()),
            )
            .map_err(|_| SurfaceError::DriverUnavailable)?;
            let changed_bounds = make_node(
                current.parent(),
                current.role(),
                current.name().into(),
                AccessibilityBounds::new(1.0, 1.0, 1.0, 1.0)
                    .map_err(|_| SurfaceError::DriverUnavailable)?,
                Some(current.is_enabled()),
            )
            .map_err(|_| SurfaceError::DriverUnavailable)?;
            let changed_enabled = make_node(
                current.parent(),
                current.role(),
                current.name().into(),
                current.bounds(),
                Some(false),
            )
            .map_err(|_| SurfaceError::DriverUnavailable)?;
            let changed_action = make_node(
                current.parent(),
                current.role(),
                current.name().into(),
                current.bounds(),
                None,
            )
            .map_err(|_| SurfaceError::DriverUnavailable)?;
            !layout_semantics_changed(current, current)
                && layout_semantics_changed(current, &changed_parent)
                && layout_semantics_changed(current, &changed_role)
                && layout_semantics_changed(current, &changed_name)
                && layout_semantics_changed(current, &changed_bounds)
                && layout_semantics_changed(current, &changed_enabled)
                && layout_semantics_changed(current, &changed_action)
        };
        let notification_user_info_controls_valid = layout_user_info_valid(1, true)
            && !layout_user_info_valid(0, true)
            && !layout_user_info_valid(1, false)
            && announcement_user_info_valid(2, true, true)
            && !announcement_user_info_valid(1, true, true)
            && !announcement_user_info_valid(2, false, true)
            && !announcement_user_info_valid(2, true, false);
        let semantic_tree_valid = role_mapping_valid
            && editor_focused
            && tab_selected
            && reusable_semantics_valid
            && layout_semantics_changed_valid
            && status_value.is_some_and(|value| value.to_string() == "Opened main.rs")
            && editor_parent.is_some_and(|parent| core::ptr::eq(&*parent, &*root));
        let status_text_selector_allowed: bool = unsafe {
            msg_send![&*status, isAccessibilitySelectorAllowed: sel!(accessibilityStringForRange:)]
        };
        let status_character_count: usize =
            unsafe { msg_send![&*status, accessibilityNumberOfCharacters] };
        let checked_range_contract_valid = checked_range(NSRange::new(0, 2), 2)
            == Some(AccessibilityTextRange::new(0, 2))
            && checked_range(NSRange::new(usize::MAX, 0), usize::MAX).is_none()
            && checked_range(NSRange::new(usize::MAX - 1, 2), usize::MAX).is_none();
        let text_selector_scope_valid = !status_text_selector_allowed
            && status_character_count == 0
            && checked_range_contract_valid;
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
        let retained_slot_bytes_before_revoke =
            view.ivars().accessibility.borrow().retained_slot_bytes();
        view.revoke_accessibility();
        view.revoke_accessibility();
        let late_length: usize = unsafe { msg_send![&*editor, accessibilityNumberOfCharacters] };
        let revoked_activation_rejected: bool =
            unsafe { msg_send![&*tab, accessibilityPerformPress] };
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
            stable_external_identifier,
            bounded_screen_frame,
            activate_selector_allowed,
            accepted_activation,
            revoked_activation_rejected: !revoked_activation_rejected,
            peak_elements,
            created_elements,
            released_elements: final_counters.released_elements,
            notification_counts: final_counters.notifications,
            notification_records: final_counters.notification_records
                [..final_counters.notification_record_count]
                .iter()
                .map(|record| {
                    crate::native_validation::NativeAccessibilityNotificationRecord::new(
                        record.kind_index,
                        record.target,
                        record.payload_elements,
                        record.payload_bytes,
                        record.priority,
                    )
                })
                .collect(),
            omitted_notification_records: final_counters.omitted_notification_records,
            invalid_notification_user_info: final_counters.invalid_notification_user_info,
            notification_user_info_controls_valid,
            posts_after_handler_revocation: final_counters.posts_after_handler_revocation,
            revoke_starts: final_counters.revoke_starts,
            revoke_terminal: view.ivars().accessibility.borrow().handler.is_none()
                && !view.ivars().accessibility.borrow().revoking,
            posted_notification_payload_bytes: final_counters.posted_payload_bytes,
            peak_notification_retained_bytes: final_counters.peak_notification_retained_bytes,
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
    instance_generation: u64,
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
        fn is_accessibility_element(&self) -> bool { self.with_adapter(|adapter| adapter.valid(self.ivars().generation, self.ivars().instance_generation, self.ivars().id)).unwrap_or(false) }

        #[unsafe(method_id(accessibilityIdentifier))]
        fn accessibility_identifier(&self) -> Retained<NSString> {
            NSString::from_str(&format!("alpine.ax.{}.{}.{}", self.ivars().generation, self.ivars().instance_generation, self.ivars().id.get()))
        }

        #[unsafe(method(accessibilityFrame))]
        fn accessibility_frame(&self) -> NSRect { self.accessibility_frame_impl() }

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
            let children = self.with_adapter(|adapter| adapter.children(self.ivars().generation, self.ivars().instance_generation, self.ivars().id)).unwrap_or_default();
            NSArray::from_retained_slice(&children)
        }

        #[unsafe(method(isAccessibilityFocused))]
        fn is_accessibility_focused(&self) -> bool { self.node().is_some_and(|node| node.is_focused()) }

        #[unsafe(method(isAccessibilitySelected))]
        fn is_accessibility_selected(&self) -> bool { self.node().is_some_and(|node| node.is_selected()) }

        #[unsafe(method(accessibilityPerformPress))]
        fn accessibility_perform_press(&self) -> Bool {
            let applied = self.with_adapter_mut(|adapter| adapter.activate(self.ivars().generation, self.ivars().instance_generation, self.ivars().id)).unwrap_or(false);
            if applied
                && let Some(view) = self.ivars().view.load()
                && NativeAccessibilityAdapter::refresh_view(&view).is_err()
            {
                self.ivars().dispatch_failed.set(true);
                return false.into();
            }
            applied.into()
        }

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
                || selector == sel!(accessibilityIdentifier)
                || selector == sel!(accessibilityFrame)
                || selector == sel!(accessibilityLabel)
                || selector == sel!(accessibilityValue)
                || selector == sel!(accessibilityParent)
                || selector == sel!(accessibilityChildren)
                || selector == sel!(isAccessibilityFocused)
                || selector == sel!(isAccessibilitySelected)
                || (self.node().is_some_and(|node| node.supports_activate() && node.is_enabled())
                    && selector == sel!(accessibilityPerformPress))
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
        instance_generation: u64,
    ) -> Retained<Self> {
        let retained_view = view.retain();
        let allocated = Self::alloc(main_thread).set_ivars(NativeAccessibilityElementIvars {
            view: Weak::from_retained(&retained_view),
            id,
            generation,
            instance_generation,
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
                .node(
                    self.ivars().generation,
                    self.ivars().instance_generation,
                    self.ivars().id,
                )
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

    fn accessibility_frame_impl(&self) -> NSRect {
        let Some(node) = self.node() else {
            return NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(0.0, 0.0));
        };
        let bounds = node.bounds();
        let local = NSRect::new(
            NSPoint::new(f64::from(bounds.x()), f64::from(bounds.y())),
            NSSize::new(f64::from(bounds.width()), f64::from(bounds.height())),
        );
        self.ivars()
            .view
            .load()
            .and_then(|view| {
                view.window()
                    .map(|window| window.convertRectToScreen(local))
            })
            .unwrap_or(local)
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
            adapter.snapshot_metadata(
                self.ivars().generation,
                self.ivars().instance_generation,
                self.ivars().id,
            )
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
                    || left.bounds() != right.bounds()
                    || left.is_enabled() != right.is_enabled()
                    || left.supports_activate() != right.supports_activate()
            })
}

fn layout_semantics_changed(previous: &AccessibilityNode, current: &AccessibilityNode) -> bool {
    previous.parent() != current.parent()
        || previous.role() != current.role()
        || previous.name() != current.name()
        || previous.bounds() != current.bounds()
        || previous.is_enabled() != current.is_enabled()
        || previous.supports_activate() != current.supports_activate()
}

fn reusable_semantics(previous: &AccessibilityNode, current: &AccessibilityNode) -> bool {
    previous.parent() == current.parent()
        && previous.role() == current.role()
        && previous.name() == current.name()
        && previous.supports_activate() == current.supports_activate()
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
        AccessibilityRole::ListItem => "AXRow",
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
