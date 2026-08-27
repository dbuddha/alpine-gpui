use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicPtr, AtomicU8, AtomicU64, Ordering},
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Instant,
};
use std::{ffi::c_void, ptr::NonNull};

use alpine_core::Point;

#[cfg(alpine_native_validation)]
use std::time::Duration;

#[cfg(alpine_native_validation)]
use objc2::rc::autoreleasepool;
#[cfg(alpine_native_validation)]
use objc2::runtime::Bool;
use objc2::{
    AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send,
    rc::{Retained, Weak},
    runtime::{AnyObject, ProtocolObject, Sel},
};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSEvent,
    NSEventModifierFlags, NSEventPhase, NSPasteboard, NSPasteboardType, NSPasteboardTypeString,
    NSTextInputClient, NSView, NSWindow, NSWindowDelegate, NSWindowOcclusionState,
    NSWindowStyleMask,
};
#[cfg(alpine_native_validation)]
use objc2_app_kit::{NSEventType, NSScreen, NSWindowButton};
use objc2_core_foundation::{CFRunLoop, kCFRunLoopCommonModes};
use objc2_core_graphics::{CGColorSpace, kCGColorSpaceSRGB};
#[cfg(alpine_native_validation)]
use objc2_core_graphics::{CGEvent, CGEventFlags, CGScrollEventUnit};
use objc2_foundation::{
    NSArray, NSAttributedString, NSAttributedStringKey, NSNotification, NSObject, NSObjectProtocol,
    NSPoint, NSRange, NSRect, NSRunLoop, NSRunLoopCommonModes, NSSize, NSString,
    NSUTF8StringEncoding,
};
#[cfg(alpine_native_validation)]
use objc2_foundation::{NSDate, NSTimer};
use objc2_metal::{MTLCreateSystemDefaultDevice, MTLDevice, MTLDrawable, MTLPixelFormat};
use objc2_quartz_core::{
    CAMetalDisplayLink, CAMetalDisplayLinkDelegate, CAMetalDisplayLinkUpdate, CAMetalDrawable,
    CAMetalLayer,
};

use alpine_core::LinearRgba;
use alpine_metal::{MetalBackend, OffscreenDescriptor, RecoveryClassification, platform_spi};
use alpine_platform::{
    ApplicationState, DisplayLinkDirective, DisplayLinkState, FrameCompletionStatus,
    FrameOwnerGeneration, FrameSlotAdmission, FrameSlotLease, FrameSlotRing, FrameToken,
    PendingCancellationEvidence, PresentationAction, PresentationEvent, PresentationOutcome,
    PresentationState, PresentationTransition,
};
use alpine_scene::Scene;
use block2::RcBlock;
use dispatch2::DispatchQueue;

use crate::native_accessibility::{NativeAccessibilityAdapter, NativeAccessibilityElement};
use crate::{
    AccessibilityRequest, AccessibilityResponse, ClipboardError, ClipboardEvent,
    ClipboardOperation, ClipboardText, ClipboardWrite, CloseDisposition, EventTimestamp,
    FrameLatencyEvidence, FrameTerminalEvidence, ImeEvent, InputEpoch, InputEpochAdmission,
    KeyState, Modifiers, PointerAction, PointerButton, SURFACE_CLOSING, SURFACE_LIVE, ScrollPhase,
    SdrColorContract, StudioSignposts, SurfaceConfiguration, SurfaceDescriptor, SurfaceError,
    SurfaceEvent, SurfaceLifecycle, SurfaceObserver, SurfaceOperation, SurfaceResponse,
    SurfaceSnapshot, SurfaceStage, SurfaceWakeAdmission, SurfaceWakeCounters, SurfaceWaker,
    begin_close_observer_state, finish_close_observer_state, new_observer_state,
    presentation_visible,
};

type Device = Retained<ProtocolObject<dyn MTLDevice>>;
type SurfaceEventHandler = Box<dyn FnMut(SurfaceEvent) -> SurfaceResponse + 'static>;

struct NativeWakeBridge {
    delegate: AtomicPtr<DisplayLinkDelegate>,
    lifecycle: Arc<AtomicU8>,
    pending: AtomicBool,
    counters: Arc<SurfaceWakeCounters>,
}

impl NativeWakeBridge {
    fn new(delegate: &DisplayLinkDelegate, lifecycle: Arc<AtomicU8>) -> Arc<Self> {
        Arc::new(Self {
            delegate: AtomicPtr::new(core::ptr::from_ref(delegate).cast_mut()),
            lifecycle,
            pending: AtomicBool::new(false),
            counters: Arc::new(SurfaceWakeCounters::new()),
        })
    }

    fn request(self: &Arc<Self>) -> SurfaceWakeAdmission {
        self.counters.request();
        if self.lifecycle.load(Ordering::Acquire) != SURFACE_LIVE
            || self.delegate.load(Ordering::Acquire).is_null()
        {
            self.counters.rejected();
            return SurfaceWakeAdmission::Closed;
        }
        if self.pending.swap(true, Ordering::AcqRel) {
            self.counters.coalesced();
            return SurfaceWakeAdmission::Coalesced;
        }
        let bridge = Arc::clone(self);
        DispatchQueue::main().exec_async(move || bridge.dispatch());
        self.counters.scheduled();
        SurfaceWakeAdmission::Scheduled
    }

    fn dispatch(&self) {
        self.pending.store(false, Ordering::Release);
        if self.lifecycle.load(Ordering::Acquire) != SURFACE_LIVE {
            self.counters.rejected();
            return;
        }
        let delegate = self.delegate.load(Ordering::Acquire);
        let Some(delegate) = NonNull::new(delegate) else {
            self.counters.rejected();
            return;
        };
        // SAFETY: dispatch2 executes this closure on the process main queue.
        // The surface revokes the pointer on that same thread before releasing
        // its retained delegate.
        let delegate = unsafe { delegate.as_ref() };
        self.counters.dispatched();
        let _ = delegate.dispatch_surface_event(SurfaceEvent::Wake {
            timestamp: delegate.next_event_timestamp(),
        });
    }

    fn revoke(&self) {
        self.delegate
            .store(core::ptr::null_mut(), Ordering::Release);
    }

    fn waker(self: &Arc<Self>) -> SurfaceWaker {
        let bridge = Arc::clone(self);
        SurfaceWaker::new(move || bridge.request(), Arc::clone(&self.counters))
    }
}

#[cfg(alpine_native_validation)]
const NATIVE_OWNER_KINDS: usize = 10;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NativeOwnerKind {
    Application,
    Device,
    Renderer,
    Window,
    View,
    ColorSpace,
    Layer,
    Delegate,
    DisplayLink,
    #[allow(
        dead_code,
        reason = "the validation clipboard integration acquires this owner lazily"
    )]
    Pasteboard,
}

impl NativeOwnerKind {
    #[cfg(alpine_native_validation)]
    const fn index(self) -> usize {
        match self {
            Self::Application => 0,
            Self::Device => 1,
            Self::Renderer => 2,
            Self::Window => 3,
            Self::View => 4,
            Self::ColorSpace => 5,
            Self::Layer => 6,
            Self::Delegate => 7,
            Self::DisplayLink => 8,
            Self::Pasteboard => 9,
        }
    }
}

struct InitializationControl {
    #[cfg(alpine_native_validation)]
    fault_after: Option<SurfaceStage>,
    #[cfg(alpine_native_validation)]
    probe: Option<InitializationProbe>,
    #[cfg(alpine_native_validation)]
    lifecycle: Option<Arc<AtomicU8>>,
    #[cfg(alpine_native_validation)]
    callback_count: Option<Arc<AtomicU64>>,
    #[cfg(alpine_native_validation)]
    rejected_callback_count: Option<Arc<AtomicU64>>,
    #[cfg(alpine_native_validation)]
    device_loss: bool,
}

impl InitializationControl {
    const fn production() -> Self {
        Self {
            #[cfg(alpine_native_validation)]
            fault_after: None,
            #[cfg(alpine_native_validation)]
            probe: None,
            #[cfg(alpine_native_validation)]
            lifecycle: None,
            #[cfg(alpine_native_validation)]
            callback_count: None,
            #[cfg(alpine_native_validation)]
            rejected_callback_count: None,
            #[cfg(alpine_native_validation)]
            device_loss: false,
        }
    }

    #[allow(
        clippy::unused_self,
        reason = "shipping builds erase validation state while preserving one constructor path"
    )]
    fn observer_state(&self) -> (Arc<AtomicU8>, Arc<AtomicU64>, Arc<AtomicU64>) {
        #[cfg(alpine_native_validation)]
        if let (Some(lifecycle), Some(callback_count), Some(rejected_callback_count)) = (
            &self.lifecycle,
            &self.callback_count,
            &self.rejected_callback_count,
        ) {
            return (
                Arc::clone(lifecycle),
                Arc::clone(callback_count),
                Arc::clone(rejected_callback_count),
            );
        }
        new_observer_state()
    }

    #[allow(
        clippy::unnecessary_wraps,
        clippy::unused_self,
        reason = "shipping builds erase injected failures while preserving the audited sequence"
    )]
    fn checkpoint(&self, stage: SurfaceStage) -> Result<(), SurfaceError> {
        #[cfg(alpine_native_validation)]
        if self.fault_after == Some(stage) {
            return Err(native_unavailable(stage));
        }
        #[cfg(not(alpine_native_validation))]
        let _ = stage;
        Ok(())
    }

    #[allow(
        clippy::unused_self,
        reason = "shipping builds erase the validation-only capability decision"
    )]
    fn backend(&self, device: Device) -> Result<MetalBackend, SurfaceError> {
        #[cfg(alpine_native_validation)]
        if self.probe.is_some() {
            if self.device_loss {
                return platform_spi::new_validation_backend_with_device_loss(device)
                    .map_err(SurfaceError::from);
            }
            return platform_spi::new_validation_backend_with_device(device)
                .map_err(SurfaceError::from);
        }
        platform_spi::new_backend_with_device(device).map_err(SurfaceError::from)
    }

    #[cfg(alpine_native_validation)]
    fn validation(fault_after: Option<SurfaceStage>) -> Self {
        let (lifecycle, callback_count, rejected_callback_count) = new_observer_state();
        Self {
            fault_after,
            probe: Some(InitializationProbe::default()),
            lifecycle: Some(lifecycle),
            callback_count: Some(callback_count),
            rejected_callback_count: Some(rejected_callback_count),
            device_loss: false,
        }
    }

    #[cfg(alpine_native_validation)]
    fn validation_device_loss() -> Self {
        Self {
            device_loss: true,
            ..Self::validation(None)
        }
    }

    #[cfg(alpine_native_validation)]
    fn observer(&self) -> Option<SurfaceObserver> {
        Some(SurfaceObserver::new(
            Arc::clone(self.lifecycle.as_ref()?),
            Arc::clone(self.callback_count.as_ref()?),
            Arc::clone(self.rejected_callback_count.as_ref()?),
        ))
    }

    #[cfg(alpine_native_validation)]
    fn probe(&self) -> Option<InitializationProbe> {
        self.probe.clone()
    }
}

#[cfg(alpine_native_validation)]
#[derive(Clone, Default)]
struct InitializationProbe(Rc<InitializationProbeState>);

#[cfg(alpine_native_validation)]
#[derive(Default)]
struct InitializationProbeState {
    acquired: Cell<[u64; NATIVE_OWNER_KINDS]>,
    released: Cell<[u64; NATIVE_OWNER_KINDS]>,
    active: Cell<[u64; NATIVE_OWNER_KINDS]>,
    run_loop_registrations: Cell<u64>,
    link_invalidations: Cell<u64>,
    delegate_revocations: Cell<u64>,
    window_closes: Cell<u64>,
    pasteboard_releases: Cell<u64>,
    release_order_violations: Cell<u64>,
    window: RefCell<Option<Weak<NSWindow>>>,
}

#[cfg(alpine_native_validation)]
impl InitializationProbe {
    fn acquire(&self, kind: NativeOwnerKind) -> InitializationLease {
        increment(&self.0.acquired, kind);
        increment(&self.0.active, kind);
        InitializationLease {
            probe: self.clone(),
            kind,
        }
    }

    fn counts(
        &self,
    ) -> (
        [u64; NATIVE_OWNER_KINDS],
        [u64; NATIVE_OWNER_KINDS],
        [u64; NATIVE_OWNER_KINDS],
    ) {
        (
            self.0.acquired.get(),
            self.0.released.get(),
            self.0.active.get(),
        )
    }

    fn record_run_loop_registration(&self) {
        self.0
            .run_loop_registrations
            .set(self.0.run_loop_registrations.get() + 1);
    }

    fn record_link_invalidation(&self) {
        self.0
            .link_invalidations
            .set(self.0.link_invalidations.get() + 1);
    }

    fn record_delegate_revocation(&self) {
        self.0
            .delegate_revocations
            .set(self.0.delegate_revocations.get() + 1);
    }

    fn record_window_close(&self) {
        self.0.window_closes.set(self.0.window_closes.get() + 1);
    }

    fn record_window(&self, window: &Retained<NSWindow>) {
        self.0.window.replace(Some(Weak::from_retained(window)));
    }

    fn record_pasteboard_release(&self) {
        self.0
            .pasteboard_releases
            .set(self.0.pasteboard_releases.get() + 1);
    }

    fn evidence(&self) -> crate::native_validation::NativeOwnerEvidence {
        let (acquired, released, active) = self.counts();
        crate::native_validation::NativeOwnerEvidence::new(
            acquired,
            released,
            active,
            self.0.run_loop_registrations.get(),
            self.0.link_invalidations.get(),
            self.0.delegate_revocations.get(),
            self.0.window_closes.get(),
            self.0.pasteboard_releases.get(),
            self.0.release_order_violations.get(),
        )
    }
}

#[cfg(alpine_native_validation)]
fn increment(counts: &Cell<[u64; NATIVE_OWNER_KINDS]>, kind: NativeOwnerKind) {
    let mut values = counts.get();
    values[kind.index()] += 1;
    counts.set(values);
}

#[cfg(alpine_native_validation)]
struct InitializationLease {
    probe: InitializationProbe,
    kind: NativeOwnerKind,
}

#[cfg(alpine_native_validation)]
impl Drop for InitializationLease {
    fn drop(&mut self) {
        let released_after_cleanup = match self.kind {
            NativeOwnerKind::DisplayLink => self.probe.0.link_invalidations.get() > 0,
            NativeOwnerKind::Delegate => self.probe.0.delegate_revocations.get() > 0,
            NativeOwnerKind::Window => self.probe.0.window_closes.get() > 0,
            NativeOwnerKind::Pasteboard => self.probe.0.pasteboard_releases.get() > 0,
            NativeOwnerKind::Application
            | NativeOwnerKind::Device
            | NativeOwnerKind::Renderer
            | NativeOwnerKind::View
            | NativeOwnerKind::ColorSpace
            | NativeOwnerKind::Layer => true,
        };
        if !released_after_cleanup {
            self.probe
                .0
                .release_order_violations
                .set(self.probe.0.release_order_violations.get() + 1);
        }
        increment(&self.probe.0.released, self.kind);
        let mut active = self.probe.0.active.get();
        active[self.kind.index()] -= 1;
        self.probe.0.active.set(active);
    }
}

// A 120 Hz display can produce roughly 600 callbacks during the five-second
// terminal-observation budget used by native qualification.
const MAX_PRESENTATION_POLLS: u16 = 600;

#[derive(Default)]
struct FrameCounters {
    submissions: AtomicU64,
    direct_presents: AtomicU64,
    installed_presented_handlers: AtomicU64,
    presented: AtomicU64,
    qualified_presented: AtomicU64,
    superseded: AtomicU64,
    cancelled: AtomicU64,
    pending_cancellations: AtomicU64,
    last_presented_time_bits: AtomicU64,
    skipped: AtomicU64,
    failed: AtomicU64,
}

struct PendingFrame {
    scene: Scene,
    clear: LinearRgba,
    event_timing: Option<EventFrameTiming>,
}

#[derive(Clone, Copy)]
struct EventFrameTiming {
    timestamp: EventTimestamp,
    received_at: Instant,
    handler_finished_at: Instant,
    admitted_at: Instant,
}

#[derive(Clone, Copy, Default)]
struct AttemptTiming {
    target_timestamp_bits: u64,
    target_presentation_timestamp_bits: u64,
    event: Option<EventFrameTiming>,
    submission_started_at: Option<Instant>,
    submission_finished_at: Option<Instant>,
    gpu_terminal_observed_at: Option<Instant>,
    event_to_presented_handler_ns: Option<u64>,
}

impl AttemptTiming {
    fn from_update(update: &CAMetalDisplayLinkUpdate, event: Option<EventFrameTiming>) -> Self {
        Self {
            target_timestamp_bits: update.targetTimestamp().to_bits(),
            target_presentation_timestamp_bits: update.targetPresentationTimestamp().to_bits(),
            event,
            submission_started_at: None,
            submission_finished_at: None,
            gpu_terminal_observed_at: None,
            event_to_presented_handler_ns: None,
        }
    }

    fn latency_evidence(self, terminal_at: Instant) -> Option<FrameLatencyEvidence> {
        let event = self.event?;
        Some(FrameLatencyEvidence::new(
            event.timestamp,
            elapsed_ns(event.received_at, event.handler_finished_at),
            self.submission_started_at
                .map(|started| elapsed_ns(event.admitted_at, started)),
            self.submission_started_at
                .zip(self.submission_finished_at)
                .map(|(started, finished)| elapsed_ns(started, finished)),
            self.gpu_terminal_observed_at
                .map(|observed| elapsed_ns(event.received_at, observed)),
            self.event_to_presented_handler_ns,
            elapsed_ns(event.received_at, terminal_at),
        ))
    }
}

fn elapsed_ns(start: Instant, end: Instant) -> u64 {
    u64::try_from(end.saturating_duration_since(start).as_nanos()).unwrap_or(u64::MAX)
}

fn profile_latency_for_terminal(
    latency: Option<FrameLatencyEvidence>,
    recovery: Option<RecoveryClassification>,
) -> Option<FrameLatencyEvidence> {
    if matches!(recovery, Some(RecoveryClassification::RetryFrame)) {
        None
    } else {
        latency
    }
}

#[cfg(test)]
mod frame_latency_timing_tests {
    use std::time::{Duration, Instant};

    use super::{AttemptTiming, EventFrameTiming, profile_latency_for_terminal};
    use crate::{EventTimestamp, RecoveryClassification};

    #[test]
    fn timeline_preserves_exact_stages_and_absent_endpoints() -> Result<(), &'static str> {
        let origin = Instant::now();
        let event = EventFrameTiming {
            timestamp: EventTimestamp::new(11),
            received_at: origin,
            handler_finished_at: origin + Duration::from_nanos(13),
            admitted_at: origin + Duration::from_nanos(17),
        };
        let complete = AttemptTiming {
            event: Some(event),
            submission_started_at: Some(origin + Duration::from_nanos(23)),
            submission_finished_at: Some(origin + Duration::from_nanos(31)),
            gpu_terminal_observed_at: Some(origin + Duration::from_nanos(37)),
            event_to_presented_handler_ns: Some(41),
            ..AttemptTiming::default()
        }
        .latency_evidence(origin + Duration::from_nanos(43))
        .ok_or("event-driven frame evidence")?;
        assert_eq!(complete.event_timestamp(), EventTimestamp::new(11));
        assert_eq!(complete.event_handler_ns(), 13);
        assert_eq!(complete.frame_queue_ns(), Some(6));
        assert_eq!(complete.submission_ns(), Some(8));
        assert_eq!(complete.event_to_gpu_terminal_observed_ns(), Some(37));
        assert_eq!(complete.event_to_presented_handler_ns(), Some(41));
        assert_eq!(complete.event_to_terminal_record_ns(), 43);

        let absent = AttemptTiming {
            event: Some(event),
            ..AttemptTiming::default()
        }
        .latency_evidence(origin + Duration::from_nanos(19))
        .ok_or("cancelled event-driven frame evidence")?;
        assert_eq!(absent.frame_queue_ns(), None);
        assert_eq!(absent.submission_ns(), None);
        assert_eq!(absent.event_to_gpu_terminal_observed_ns(), None);
        assert_eq!(absent.event_to_presented_handler_ns(), None);
        assert_eq!(absent.event_to_terminal_record_ns(), 19);
        assert_eq!(AttemptTiming::default().latency_evidence(origin), None);
        assert_eq!(
            profile_latency_for_terminal(Some(complete), Some(RecoveryClassification::RetryFrame)),
            None
        );
        assert_eq!(
            profile_latency_for_terminal(Some(complete), None),
            Some(complete)
        );
        assert_eq!(profile_latency_for_terminal(None, None), None);
        Ok(())
    }
}

#[cfg(alpine_native_validation)]
struct PostCommitControl {
    configuration: Option<SurfaceConfiguration>,
    presented_time_bits: u64,
    close_generation: bool,
}

struct ActiveFrame {
    token: FrameToken,
    lease: FrameSlotLease,
    submission: platform_spi::DrawableSubmission,
    #[allow(
        dead_code,
        reason = "the callback drawable must remain retained until terminal presentation evidence"
    )]
    drawable: Retained<ProtocolObject<dyn CAMetalDrawable>>,
    frame: Option<PendingFrame>,
    observation: PresentationObservation,
    command_terminal: bool,
    presentation_polls: u16,
    timing: AttemptTiming,
}

struct PresentationObservation {
    signal: Arc<PresentationSignal>,
    #[cfg(alpine_native_validation)]
    injected: Option<InjectedPresentationObservation>,
}

#[cfg(alpine_native_validation)]
#[derive(Clone, Copy)]
struct InjectedPresentationObservation {
    presented_time_bits: u64,
    event_to_presented_handler_ns: Option<u64>,
}

impl PresentationObservation {
    fn new(signal: Arc<PresentationSignal>) -> Self {
        Self {
            signal,
            #[cfg(alpine_native_validation)]
            injected: None,
        }
    }

    fn observed(&self) -> bool {
        #[cfg(alpine_native_validation)]
        if self.injected.is_some() {
            return true;
        }
        self.signal.observed.load(Ordering::Acquire)
    }

    fn presented_time_bits(&self) -> u64 {
        #[cfg(alpine_native_validation)]
        if let Some(injected) = self.injected {
            return injected.presented_time_bits;
        }
        self.signal.time_bits.load(Ordering::Relaxed)
    }

    fn event_to_presented_handler_ns(&self) -> Option<u64> {
        #[cfg(alpine_native_validation)]
        if let Some(injected) = self.injected {
            return injected.event_to_presented_handler_ns;
        }
        self.signal.event_to_presented_handler_ns()
    }

    #[cfg(alpine_native_validation)]
    fn inject(&mut self, presented_time_bits: u64) {
        self.injected = Some(InjectedPresentationObservation {
            presented_time_bits,
            event_to_presented_handler_ns: self.signal.elapsed_from_event_now(),
        });
    }
}

struct PresentationSignal {
    observed: AtomicBool,
    time_bits: AtomicU64,
    event_to_presented_handler_ns: AtomicU64,
    event_received_at: Option<Instant>,
}

impl PresentationSignal {
    const MISSING_LATENCY_NS: u64 = u64::MAX;

    fn new(event_received_at: Option<Instant>) -> Self {
        Self {
            observed: AtomicBool::new(false),
            time_bits: AtomicU64::new(0),
            event_to_presented_handler_ns: AtomicU64::new(Self::MISSING_LATENCY_NS),
            event_received_at,
        }
    }

    fn publish(&self, presented_time_bits: u64) {
        if let Some(received_at) = self.event_received_at {
            let elapsed = elapsed_ns(received_at, Instant::now())
                .min(Self::MISSING_LATENCY_NS.saturating_sub(1));
            self.event_to_presented_handler_ns
                .store(elapsed, Ordering::Relaxed);
        }
        self.time_bits.store(presented_time_bits, Ordering::Relaxed);
        self.observed.store(true, Ordering::Release);
    }

    fn event_to_presented_handler_ns(&self) -> Option<u64> {
        let value = self.event_to_presented_handler_ns.load(Ordering::Relaxed);
        (value != Self::MISSING_LATENCY_NS).then_some(value)
    }

    #[cfg(alpine_native_validation)]
    fn elapsed_from_event_now(&self) -> Option<u64> {
        self.event_received_at.map(|received_at| {
            elapsed_ns(received_at, Instant::now()).min(Self::MISSING_LATENCY_NS.saturating_sub(1))
        })
    }
}

struct PresentationDriver {
    state: PresentationState,
    lifecycle: Arc<AtomicU8>,
    configuration: SurfaceConfiguration,
    pending: Option<PendingFrame>,
    active: Option<ActiveFrame>,
    frame_slots: FrameSlotRing,
    owner_generation: FrameOwnerGeneration,
    backend: MetalBackend,
    latency_signposts: StudioSignposts,
    last_error: Option<SurfaceError>,
    last_terminal: Option<FrameTerminalEvidence>,
    last_superseded: Option<FrameTerminalEvidence>,
    last_cancelled: Option<FrameTerminalEvidence>,
    last_pending_cancellation: Option<PendingCancellationEvidence>,
    #[cfg(alpine_native_validation)]
    post_commit_control: Option<PostCommitControl>,
}

impl PresentationDriver {
    fn new(
        backend: MetalBackend,
        configuration: SurfaceConfiguration,
        lifecycle: Arc<AtomicU8>,
    ) -> Result<Self, SurfaceError> {
        let mut state = PresentationState::new();
        state.apply(PresentationAction::SetSized(configuration.is_sized()))?;
        let owner_generation = FrameOwnerGeneration::new(1)
            .ok_or(SurfaceError::invariant(SurfaceOperation::Presentation))?;
        Ok(Self {
            state,
            lifecycle,
            configuration,
            pending: None,
            active: None,
            frame_slots: FrameSlotRing::new(),
            owner_generation,
            backend,
            latency_signposts: StudioSignposts::new(),
            last_error: None,
            last_terminal: None,
            last_superseded: None,
            last_cancelled: None,
            last_pending_cancellation: None,
            #[cfg(alpine_native_validation)]
            post_commit_control: None,
        })
    }

    fn apply_configuration(
        &mut self,
        configuration: SurfaceConfiguration,
    ) -> Result<DisplayLinkDirective, SurfaceError> {
        let prior_link = self.state.display_link();
        if self
            .configuration
            .geometry_or_display_differs(configuration)
        {
            if self.pending.is_none()
                && let Some(frame) = self.active.as_mut().and_then(|active| active.frame.take())
            {
                self.pending = Some(frame);
            }
            self.state.apply(PresentationAction::AdvanceSurfaceEpoch)?;
        }
        self.state
            .apply(PresentationAction::SetSized(configuration.is_sized()))?;
        self.state
            .apply(PresentationAction::SetVisible(configuration.visible))?;
        self.configuration = configuration;
        self.reconcile_link(prior_link)
    }

    fn reject_configuration(
        &mut self,
        error: SurfaceError,
    ) -> Result<DisplayLinkDirective, SurfaceError> {
        let prior_link = self.state.display_link();
        self.last_error = Some(error);
        self.state.apply(PresentationAction::SetSized(false))?;
        self.state.apply(PresentationAction::SetVisible(false))?;
        self.reconcile_link(prior_link)
    }

    fn request_frame(
        &mut self,
        scene: Scene,
        clear: LinearRgba,
    ) -> Result<(alpine_platform::PresentationRevision, DisplayLinkDirective), SurfaceError> {
        self.request_frame_with_event(scene, clear, None)
    }

    fn request_frame_with_event(
        &mut self,
        scene: Scene,
        clear: LinearRgba,
        event_timing: Option<EventFrameTiming>,
    ) -> Result<(alpine_platform::PresentationRevision, DisplayLinkDirective), SurfaceError> {
        let prior_link = self.state.display_link();
        let transition = self.state.apply(PresentationAction::Invalidate)?;
        let PresentationEvent::Invalidated(revision) = transition.event() else {
            return Err(SurfaceError::invariant(SurfaceOperation::Presentation));
        };
        self.pending = Some(PendingFrame {
            scene,
            clear,
            event_timing,
        });
        let directive = self.reconcile_link(prior_link)?;
        Ok((revision, directive))
    }

    fn reconcile_link(
        &mut self,
        prior_link: DisplayLinkState,
    ) -> Result<DisplayLinkDirective, SurfaceError> {
        let owns_callback_work = self.pending.is_some() || self.active.is_some();
        if self.state.needs_resume() && owns_callback_work {
            self.state.apply(PresentationAction::Resume)?;
        }
        match (prior_link, self.state.display_link()) {
            (DisplayLinkState::Paused, DisplayLinkState::Running) => {
                Ok(DisplayLinkDirective::Resume)
            }
            (DisplayLinkState::Running, DisplayLinkState::Paused) => {
                Ok(DisplayLinkDirective::Pause)
            }
            _ => Ok(DisplayLinkDirective::None),
        }
    }

    const fn display_link_state(&self) -> DisplayLinkState {
        self.state.display_link()
    }

    fn update(
        &mut self,
        update: &CAMetalDisplayLinkUpdate,
        counters: &FrameCounters,
    ) -> DisplayLinkDirective {
        match self.try_update(update, counters) {
            Ok(directive) => directive,
            Err(error) => {
                let recovery = render_recovery(&error);
                if discards_pending_work(recovery) {
                    self.pending = None;
                }
                self.last_error = Some(error);
                counters.failed.fetch_add(1, Ordering::Relaxed);
                let active = self.active.take();
                let timing = active.as_ref().map_or_else(
                    || AttemptTiming::from_update(update, None),
                    |active| active.timing,
                );
                let token = active
                    .as_ref()
                    .map(|active| active.token)
                    .or_else(|| self.state.active_token());
                if let Some(token) = token {
                    self.state
                        .apply(PresentationAction::FailActive(token))
                        .ok()
                        .and_then(|transition| {
                            self.record_terminal(transition, timing, 0, recovery, counters)
                                .ok()
                        })
                        .unwrap_or(DisplayLinkDirective::Pause)
                } else {
                    DisplayLinkDirective::Pause
                }
            }
        }
    }

    fn try_update(
        &mut self,
        update: &CAMetalDisplayLinkUpdate,
        counters: &FrameCounters,
    ) -> Result<DisplayLinkDirective, SurfaceError> {
        if self.lifecycle.load(Ordering::Acquire) != SURFACE_LIVE {
            return Ok(DisplayLinkDirective::Invalidate);
        }
        if let Some(directive) = self.reconcile_active_presentation(counters)? {
            return Ok(directive);
        }
        self.submit_pending(update, counters)
    }

    fn reconcile_active_presentation(
        &mut self,
        counters: &FrameCounters,
    ) -> Result<Option<DisplayLinkDirective>, SurfaceError> {
        if let Some(directive) = self.poll_active_command(counters)? {
            return Ok(Some(directive));
        }
        if self
            .active
            .as_ref()
            .is_some_and(|active| active.observation.observed())
        {
            let Some(active) = self.active.take() else {
                return Err(SurfaceError::invariant(SurfaceOperation::Presentation));
            };
            let token = active.token;
            let presented_time_bits = active.observation.presented_time_bits();
            if presented_time_bits != 0 {
                counters
                    .last_presented_time_bits
                    .store(presented_time_bits, Ordering::Relaxed);
                counters.presented.fetch_add(1, Ordering::Relaxed);
                let transition = self
                    .state
                    .apply(PresentationAction::CompletePresentation(token))?;
                let mut timing = active.timing;
                timing.event_to_presented_handler_ns =
                    active.observation.event_to_presented_handler_ns();
                return self
                    .record_terminal(transition, timing, presented_time_bits, None, counters)
                    .map(Some);
            }

            counters.skipped.fetch_add(1, Ordering::Relaxed);
            counters.failed.fetch_add(1, Ordering::Relaxed);
            let transition = self.state.apply(PresentationAction::FailActive(token))?;
            let directive = self.record_terminal(transition, active.timing, 0, None, counters)?;
            if self.pending.is_some() {
                // A newer immutable frame is genuine work, not a replay of the
                // dropped attempt. The physical link is still running in this
                // callback, so reconcile the portable pause before allowing
                // that newer frame to enter on the next callback.
                self.state.apply(PresentationAction::Resume)?;
                return Ok(Some(DisplayLinkDirective::None));
            }
            // Apple defines a zero presentedTime as a terminal drawable that
            // was not presented. Release it and pause rather than replaying the
            // same frame and event correlation indefinitely. A later genuine
            // invalidation starts a new revision and resumes demand normally.
            return Ok(Some(directive));
        }
        if let Some(active) = &mut self.active {
            active.presentation_polls = active.presentation_polls.saturating_add(1);
            if active.presentation_polls >= MAX_PRESENTATION_POLLS {
                return Err(SurfaceError::PresentationNotObserved {
                    callbacks: active.presentation_polls,
                });
            }
            return Ok(Some(DisplayLinkDirective::None));
        }
        Ok(None)
    }

    fn poll_active_command(
        &mut self,
        counters: &FrameCounters,
    ) -> Result<Option<DisplayLinkDirective>, SurfaceError> {
        let Some(active) = self.active.as_ref() else {
            return Ok(None);
        };
        if active.command_terminal {
            return Ok(None);
        }
        let submission = active.submission;
        let lease = active.lease;
        let platform_spi::DrawableCompletionPoll::Complete(attempt) =
            platform_spi::poll_callback_drawable(&mut self.backend, submission)
        else {
            return Ok(Some(DisplayLinkDirective::None));
        };
        let terminal_observed_at = Instant::now();
        let result = attempt.into_result();
        self.active
            .as_mut()
            .ok_or(SurfaceError::invariant(SurfaceOperation::Presentation))?
            .timing
            .gpu_terminal_observed_at = Some(terminal_observed_at);
        let status = if result.is_ok() {
            FrameCompletionStatus::Completed
        } else {
            FrameCompletionStatus::Failed
        };
        self.frame_slots
            .complete(
                lease,
                status,
                self.owner_generation,
                self.state.requested_revision(),
                self.state.surface_epoch(),
            )
            .map_err(|_| SurfaceError::invariant(SurfaceOperation::Presentation))?;
        if let Err(error) = result {
            let active = self
                .active
                .take()
                .ok_or(SurfaceError::invariant(SurfaceOperation::Presentation))?;
            let error = SurfaceError::from(error);
            let recovery = render_recovery(&error);
            if discards_pending_work(recovery) {
                self.pending = None;
            }
            self.last_error = Some(error);
            counters.failed.fetch_add(1, Ordering::Relaxed);
            let transition = self
                .state
                .apply(PresentationAction::FailActive(active.token))?;
            return self
                .record_terminal(transition, active.timing, 0, recovery, counters)
                .map(Some);
        }
        self.active
            .as_mut()
            .ok_or(SurfaceError::invariant(SurfaceOperation::Presentation))?
            .command_terminal = true;
        Ok(None)
    }

    fn submit_pending(
        &mut self,
        update: &CAMetalDisplayLinkUpdate,
        counters: &FrameCounters,
    ) -> Result<DisplayLinkDirective, SurfaceError> {
        let prepared = self.state.apply(PresentationAction::Prepare)?;
        let PresentationEvent::Prepared(token) = prepared.event() else {
            return Err(SurfaceError::invariant(SurfaceOperation::Presentation));
        };
        let Some(frame) = self.pending.take() else {
            return Err(SurfaceError::invariant(SurfaceOperation::Presentation));
        };
        self.state.apply(PresentationAction::BeginUpdate(token))?;
        let mut timing = AttemptTiming::from_update(update, frame.event_timing);
        if self.lifecycle.load(Ordering::Acquire) != SURFACE_LIVE {
            return self.cancel_attempt(token, timing, counters);
        }

        #[allow(
            clippy::cast_possible_truncation,
            reason = "validated finite scale is narrowed to the renderer's f32 coordinate contract"
        )]
        let descriptor = OffscreenDescriptor::new(
            self.configuration.physical_width,
            self.configuration.physical_height,
            self.configuration.scale as f32,
            frame.clear,
        )
        .map_err(alpine_metal::RenderError::from)?;

        let drawable = update.drawable();
        let texture = drawable.texture();
        let drawable_protocol = ProtocolObject::from_ref(&*drawable);
        let presentation = install_observation(drawable_protocol, frame.event_timing, counters);
        let admission = self
            .frame_slots
            .acquire(token, self.owner_generation)
            .map_err(|_| SurfaceError::invariant(SurfaceOperation::Presentation))?;
        let FrameSlotAdmission::Acquired(lease) = admission else {
            self.pending = Some(frame);
            return Err(SurfaceError::invariant(SurfaceOperation::Presentation));
        };
        let slot = platform_spi::DrawableSlot::new(lease.slot().get())
            .ok_or(SurfaceError::invariant(SurfaceOperation::Presentation))?;
        timing.submission_started_at = Some(Instant::now());
        let attempt = platform_spi::submit_callback_drawable(
            &mut self.backend,
            slot,
            &frame.scene,
            descriptor,
            &texture,
            drawable_protocol,
        );
        timing.submission_finished_at = Some(Instant::now());
        match attempt {
            platform_spi::DrawableSubmitAttempt::Submitted(submission) => {
                self.frame_slots
                    .mark_submitted(lease)
                    .map_err(|_| SurfaceError::invariant(SurfaceOperation::Presentation))?;
                self.state.apply(PresentationAction::Submit(token))?;
                counters.submissions.fetch_add(1, Ordering::Relaxed);
                self.state.apply(PresentationAction::CallPresent(token))?;
                counters.direct_presents.fetch_add(1, Ordering::Relaxed);
                self.active = Some(ActiveFrame {
                    token,
                    lease,
                    submission,
                    drawable,
                    frame: Some(frame),
                    observation: PresentationObservation::new(presentation),
                    command_terminal: false,
                    presentation_polls: 0,
                    timing,
                });
                #[cfg(alpine_native_validation)]
                if self.post_commit_control.is_some() {
                    let directive = self.apply_post_commit_control()?;
                    if self.lifecycle.load(Ordering::Acquire) != SURFACE_LIVE {
                        let _ = self.begin_shutdown(counters);
                        return Ok(DisplayLinkDirective::None);
                    }
                    return Ok(directive);
                }
                if self.lifecycle.load(Ordering::Acquire) != SURFACE_LIVE {
                    let _ = self.begin_shutdown(counters);
                    return Ok(DisplayLinkDirective::None);
                }
                Ok(DisplayLinkDirective::None)
            }
            platform_spi::DrawableSubmitAttempt::Rejected(attempt) => {
                self.frame_slots
                    .cancel_encoding(lease)
                    .map_err(|_| SurfaceError::invariant(SurfaceOperation::Presentation))?;
                if attempt.committed() {
                    self.state.apply(PresentationAction::Submit(token))?;
                    counters.submissions.fetch_add(1, Ordering::Relaxed);
                }
                if attempt.present_called() {
                    self.state.apply(PresentationAction::CallPresent(token))?;
                    counters.direct_presents.fetch_add(1, Ordering::Relaxed);
                }
                attempt
                    .into_result()
                    .map(|_| DisplayLinkDirective::None)
                    .map_err(SurfaceError::from)
            }
        }
    }

    fn record_terminal(
        &mut self,
        transition: PresentationTransition,
        timing: AttemptTiming,
        observed_presentation_time_bits: u64,
        recovery: Option<RecoveryClassification>,
        counters: &FrameCounters,
    ) -> Result<DisplayLinkDirective, SurfaceError> {
        let PresentationEvent::Terminal(attempt) = transition.event() else {
            return Err(SurfaceError::invariant(SurfaceOperation::Presentation));
        };
        match attempt.outcome() {
            PresentationOutcome::Presented => {
                counters.qualified_presented.fetch_add(1, Ordering::Relaxed);
            }
            PresentationOutcome::Superseded => {
                counters.superseded.fetch_add(1, Ordering::Relaxed);
            }
            PresentationOutcome::Cancelled => {
                counters.cancelled.fetch_add(1, Ordering::Relaxed);
            }
            PresentationOutcome::None | PresentationOutcome::Failed => {}
        }
        let evidence = FrameTerminalEvidence::new(
            attempt,
            timing.target_timestamp_bits,
            timing.target_presentation_timestamp_bits,
            observed_presentation_time_bits,
            timing.latency_evidence(Instant::now()),
            self.backend.accounting().current_retained_bytes(),
            recovery,
        );
        if let Some(latency) = profile_latency_for_terminal(evidence.latency(), recovery) {
            let _emitted = self.latency_signposts.emit_frame_latency(latency);
        }
        if matches!(attempt.outcome(), PresentationOutcome::Superseded) {
            self.last_superseded = Some(evidence);
        }
        if matches!(attempt.outcome(), PresentationOutcome::Cancelled) {
            self.last_cancelled = Some(evidence);
        }
        self.last_terminal = Some(evidence);
        Ok(transition.display_link())
    }

    fn cancel_attempt(
        &mut self,
        token: FrameToken,
        timing: AttemptTiming,
        counters: &FrameCounters,
    ) -> Result<DisplayLinkDirective, SurfaceError> {
        if self.lifecycle.load(Ordering::Acquire) != SURFACE_LIVE
            && matches!(self.state.application(), ApplicationState::Running)
        {
            let shutdown = self.state.apply(PresentationAction::BeginShutdown)?;
            match shutdown.event() {
                PresentationEvent::Terminal(_) => {
                    return self.record_terminal(shutdown, timing, 0, None, counters);
                }
                PresentationEvent::ShutdownDraining => {}
                PresentationEvent::Unchanged
                | PresentationEvent::Invalidated(_)
                | PresentationEvent::SurfaceAdvanced(_)
                | PresentationEvent::VisibilityChanged(_)
                | PresentationEvent::SizeEligibilityChanged(_)
                | PresentationEvent::PacingResumed
                | PresentationEvent::Prepared(_)
                | PresentationEvent::UpdateBegan(_)
                | PresentationEvent::StaleDiscarded(_)
                | PresentationEvent::Submitted(_)
                | PresentationEvent::PresentCalled(_)
                | PresentationEvent::PendingCancelled(_)
                | PresentationEvent::Stopped => {
                    return Err(SurfaceError::invariant(SurfaceOperation::Presentation));
                }
            }
        }
        let transition = self.state.apply(PresentationAction::CancelActive(token))?;
        let directive = self.record_terminal(transition, timing, 0, None, counters)?;
        if matches!(self.state.application(), ApplicationState::Stopping) {
            return self
                .state
                .apply(PresentationAction::StopAfterDrain)
                .map(PresentationTransition::display_link)
                .map_err(SurfaceError::from);
        }
        Ok(directive)
    }

    #[cfg(alpine_native_validation)]
    fn inject_post_commit_observation(
        &mut self,
        display_identity: Option<usize>,
        presented_time: f64,
    ) {
        let configuration = display_identity
            .map(|identity| configuration_with_display_identity(self.configuration, identity));
        self.post_commit_control = Some(PostCommitControl {
            configuration,
            presented_time_bits: presented_time.to_bits(),
            close_generation: false,
        });
    }

    #[cfg(alpine_native_validation)]
    fn inject_post_commit_close(&mut self) {
        self.post_commit_control = Some(PostCommitControl {
            configuration: None,
            presented_time_bits: 0,
            close_generation: true,
        });
    }

    #[cfg(alpine_native_validation)]
    fn apply_post_commit_control(&mut self) -> Result<DisplayLinkDirective, SurfaceError> {
        let control = self
            .post_commit_control
            .take()
            .ok_or(SurfaceError::invariant(SurfaceOperation::Presentation))?;
        let directive = control
            .configuration
            .map_or(Ok(DisplayLinkDirective::None), |configuration| {
                self.apply_configuration(configuration)
            })?;
        if control.close_generation {
            begin_close_observer_state(&self.lifecycle);
        }
        let active = self
            .active
            .as_mut()
            .ok_or(SurfaceError::invariant(SurfaceOperation::Presentation))?;
        if !control.close_generation {
            active.observation.inject(control.presented_time_bits);
        }
        Ok(if control.close_generation {
            DisplayLinkDirective::None
        } else {
            directive
        })
    }

    fn take_error(&mut self) -> Option<SurfaceError> {
        self.last_error.take()
    }

    fn record_error(&mut self, error: SurfaceError) {
        self.last_error = Some(error);
    }

    fn begin_shutdown(&mut self, counters: &FrameCounters) -> bool {
        if matches!(self.state.application(), ApplicationState::Running)
            && let Some(generation) = self
                .owner_generation
                .get()
                .checked_add(1)
                .and_then(FrameOwnerGeneration::new)
        {
            self.owner_generation = generation;
        }
        let shutdown = self.state.apply(PresentationAction::BeginShutdown);
        if let Ok(transition) = shutdown {
            match transition.event() {
                PresentationEvent::Terminal(_) => {
                    let timing = self
                        .active
                        .as_ref()
                        .map_or_else(AttemptTiming::default, |active| active.timing);
                    let _ = self.record_terminal(transition, timing, 0, None, counters);
                }
                PresentationEvent::PendingCancelled(evidence) => {
                    counters
                        .pending_cancellations
                        .fetch_add(1, Ordering::Relaxed);
                    self.last_pending_cancellation = Some(evidence);
                }
                PresentationEvent::Unchanged
                | PresentationEvent::Invalidated(_)
                | PresentationEvent::SurfaceAdvanced(_)
                | PresentationEvent::VisibilityChanged(_)
                | PresentationEvent::SizeEligibilityChanged(_)
                | PresentationEvent::PacingResumed
                | PresentationEvent::Prepared(_)
                | PresentationEvent::UpdateBegan(_)
                | PresentationEvent::StaleDiscarded(_)
                | PresentationEvent::Submitted(_)
                | PresentationEvent::PresentCalled(_)
                | PresentationEvent::ShutdownDraining
                | PresentationEvent::Stopped => {}
            }
        }
        self.pending = None;
        if self
            .active
            .as_ref()
            .is_some_and(|active| active.command_terminal)
            && let Some(active) = self.active.take()
        {
            let _ = self.cancel_attempt(active.token, active.timing, counters);
        }
        if matches!(self.state.application(), ApplicationState::Stopping) {
            return false;
        }
        self.backend.shutdown();
        true
    }

    fn drain_shutdown(&mut self, counters: &FrameCounters) -> DisplayLinkDirective {
        let Some(submission) = self.active.as_ref().map(|active| active.submission) else {
            if matches!(self.state.application(), ApplicationState::Stopping) {
                let _ = self.state.apply(PresentationAction::StopAfterDrain);
            }
            self.backend.shutdown();
            return DisplayLinkDirective::Invalidate;
        };
        match platform_spi::poll_callback_drawable(&mut self.backend, submission) {
            platform_spi::DrawableCompletionPoll::Pending => DisplayLinkDirective::None,
            platform_spi::DrawableCompletionPoll::Complete(attempt) => {
                let Some(active) = self.active.take() else {
                    return DisplayLinkDirective::Invalidate;
                };
                let native_result = attempt.into_result();
                let _ = self.frame_slots.complete(
                    active.lease,
                    FrameCompletionStatus::Cancelled,
                    self.owner_generation,
                    self.state.requested_revision(),
                    self.state.surface_epoch(),
                );
                if let Err(error) = native_result {
                    self.last_error = Some(error.into());
                    counters.failed.fetch_add(1, Ordering::Relaxed);
                }
                let _ = self.cancel_attempt(active.token, active.timing, counters);
                self.backend.shutdown();
                DisplayLinkDirective::Invalidate
            }
        }
    }

    fn shutdown(&mut self, counters: &FrameCounters) {
        if self.begin_shutdown(counters) {
            return;
        }
        self.backend.shutdown();
    }
}

#[cfg(alpine_native_validation)]
fn configuration_with_display_identity(
    configuration: SurfaceConfiguration,
    display_identity: usize,
) -> SurfaceConfiguration {
    SurfaceConfiguration {
        display_identity,
        ..configuration
    }
}

fn render_recovery(error: &SurfaceError) -> Option<RecoveryClassification> {
    match error {
        SurfaceError::Render(error) => Some(error.recovery()),
        SurfaceError::InvalidDimension { .. }
        | SurfaceError::PhysicalDimensionOutOfRange { .. }
        | SurfaceError::UnsupportedPlatform
        | SurfaceError::NativeUnavailable { .. }
        | SurfaceError::RendererInitialization(_)
        | SurfaceError::Presentation(_)
        | SurfaceError::InvariantViolation { .. }
        | SurfaceError::OwnerConflict { .. }
        | SurfaceError::InputResponderRejected
        | SurfaceError::ValidationFailure { .. }
        | SurfaceError::PresentationNotObserved { .. }
        | SurfaceError::PresentationsSkipped { .. }
        | SurfaceError::RunLoopNotRunnable { .. }
        | SurfaceError::UnexpectedRunLoopExit { .. } => None,
    }
}

const fn discards_pending_work(recovery: Option<RecoveryClassification>) -> bool {
    matches!(recovery, Some(RecoveryClassification::RecreateBackend))
}

fn install_presented_handler(
    drawable: &ProtocolObject<dyn MTLDrawable>,
    presentation: &Arc<PresentationSignal>,
    counters: &FrameCounters,
) {
    type PresentedHandler = dyn Fn(NonNull<ProtocolObject<dyn MTLDrawable>>);

    let signal = Arc::clone(presentation);
    let handler: RcBlock<PresentedHandler> =
        RcBlock::new(move |drawable: NonNull<ProtocolObject<dyn MTLDrawable>>| {
            // SAFETY: Metal invokes the registered handler with a valid borrowed
            // drawable for the complete block call. The reference does not escape.
            let drawable = unsafe { drawable.as_ref() };
            signal.publish(drawable.presentedTime().to_bits());
        });
    // SAFETY: The generated selector signature matches the retained block.
    // Metal copies the escaping block and keeps its captured Arc alive until
    // it invokes or releases the handler.
    unsafe {
        drawable.addPresentedHandler(RcBlock::as_ptr(&handler).cast::<c_void>().cast());
    }
    counters
        .installed_presented_handlers
        .fetch_add(1, Ordering::Relaxed);
}

fn install_observation(
    drawable: &ProtocolObject<dyn MTLDrawable>,
    event_timing: Option<EventFrameTiming>,
    counters: &FrameCounters,
) -> Arc<PresentationSignal> {
    let presentation = Arc::new(PresentationSignal::new(
        event_timing.map(|event| event.received_at),
    ));
    install_presented_handler(drawable, &presentation, counters);
    presentation
}

pub(crate) type NativeInputHandler = Box<dyn FnMut(NativeInputEvent) + 'static>;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum NativeInputEvent {
    Keyboard {
        state: KeyState,
        physical_key: u16,
        logical_key: Box<str>,
        modifiers: Modifiers,
        repeat: bool,
    },
    Pointer {
        action: PointerAction,
        position: Point,
        button: PointerButton,
        modifiers: Modifiers,
    },
    Scroll {
        delta_x: f32,
        delta_y: f32,
        phase: ScrollPhase,
        precise: bool,
        modifiers: Modifiers,
    },
    Ime {
        input_epoch: InputEpoch,
        event: ImeEvent,
    },
}

pub(crate) struct SurfaceViewIvars {
    input_handler: RefCell<Option<NativeInputHandler>>,
    input_dispatch_failed: Cell<bool>,
    marked_text: RefCell<Box<str>>,
    marked_selection: Cell<NSRange>,
    input_epoch: Cell<InputEpoch>,
    input_active: Cell<bool>,
    discarding_marked_text: Cell<bool>,
    rejected_ime_callbacks: Cell<u64>,
    pub(crate) accessibility: RefCell<NativeAccessibilityAdapter>,
}

define_class!(
    // SAFETY:
    // - NSView supports subclassing and SurfaceView calls its designated frame initializer.
    // - SurfaceView is main-thread-only, matching AppKit responder dispatch.
    // - No AppKit object escapes a callback; emitted values own their text.
    #[unsafe(super = NSView)]
    #[thread_kind = MainThreadOnly]
    #[ivars = SurfaceViewIvars]
    pub(crate) struct SurfaceView;

    // SAFETY: NSObjectProtocol adds no methods with unfulfilled invariants.
    unsafe impl NSObjectProtocol for SurfaceView {}

    unsafe impl NSTextInputClient for SurfaceView {
        #[allow(
            non_snake_case,
            reason = "the generated protocol requires this Rust method name"
        )]
        #[unsafe(method(insertText:replacementRange:))]
        unsafe fn insertText_replacementRange(
            &self,
            string: &AnyObject,
            _replacement_range: NSRange,
        ) {
            if !self.accepts_ime_callback() {
                return;
            }
            let text = input_text(string);
            self.clear_marked_text();
            if !text.is_empty() {
                self.emit_ime(ImeEvent::Committed(text));
            }
        }

        #[allow(
            non_snake_case,
            reason = "the generated protocol requires this Rust method name"
        )]
        #[unsafe(method(doCommandBySelector:))]
        unsafe fn doCommandBySelector(&self, _selector: Sel) {}

        #[allow(
            non_snake_case,
            reason = "the generated protocol requires this Rust method name"
        )]
        #[unsafe(method(setMarkedText:selectedRange:replacementRange:))]
        unsafe fn setMarkedText_selectedRange_replacementRange(
            &self,
            string: &AnyObject,
            selected_range: NSRange,
            _replacement_range: NSRange,
        ) {
            if !self.accepts_ime_callback() {
                return;
            }
            let text = input_text(string);
            if text.is_empty() {
                if self.has_marked_text_value() {
                    self.clear_marked_text();
                    self.emit_ime(ImeEvent::Cancelled);
                }
                return;
            }

            if !self.has_marked_text_value() {
                self.emit_ime(ImeEvent::Started);
            }
            self.ivars().marked_text.replace(text.clone());
            self.ivars().marked_selection.set(selected_range);
            self.emit_ime(ImeEvent::Updated {
                text,
                selected_start_utf16: saturating_u32(selected_range.location),
                selected_length_utf16: saturating_u32(selected_range.length),
            });
        }

        #[unsafe(method(unmarkText))]
        #[allow(
            non_snake_case,
            reason = "the generated protocol requires this Rust method name"
        )]
        fn unmarkText(&self) {
            if self.ivars().discarding_marked_text.get() {
                self.clear_marked_text();
                return;
            }
            if !self.accepts_ime_callback() {
                return;
            }
            let text = self.ivars().marked_text.borrow().clone();
            self.clear_marked_text();
            if !text.is_empty() {
                self.emit_ime(ImeEvent::Committed(text));
            }
        }

        #[unsafe(method(selectedRange))]
        #[allow(
            non_snake_case,
            reason = "the generated protocol requires this Rust method name"
        )]
        fn selectedRange(&self) -> NSRange {
            NSRange::new(0, 0)
        }

        #[unsafe(method(markedRange))]
        #[allow(
            non_snake_case,
            reason = "the generated protocol requires this Rust method name"
        )]
        fn markedRange(&self) -> NSRange {
            if self.has_marked_text_value() {
                NSRange::new(0, self.ivars().marked_text.borrow().encode_utf16().count())
            } else {
                NSRange::new(NSUInteger::MAX, 0)
            }
        }

        #[unsafe(method(hasMarkedText))]
        #[allow(
            non_snake_case,
            reason = "the generated protocol requires this Rust method name"
        )]
        fn hasMarkedText(&self) -> bool {
            self.has_marked_text_value()
        }

        #[allow(
            non_snake_case,
            reason = "the generated protocol requires this Rust method name"
        )]
        #[unsafe(method_id(attributedSubstringForProposedRange:actualRange:))]
        unsafe fn attributedSubstringForProposedRange_actualRange(
            &self,
            _range: NSRange,
            _actual_range: *mut NSRange,
        ) -> Option<objc2::rc::Retained<NSAttributedString>> {
            None
        }

        #[unsafe(method_id(validAttributesForMarkedText))]
        #[allow(
            non_snake_case,
            reason = "the generated protocol requires this Rust method name"
        )]
        fn validAttributesForMarkedText(
            &self,
        ) -> objc2::rc::Retained<NSArray<NSAttributedStringKey>> {
            NSArray::new()
        }

        #[allow(
            non_snake_case,
            reason = "the generated protocol requires this Rust method name"
        )]
        #[unsafe(method(firstRectForCharacterRange:actualRange:))]
        unsafe fn firstRectForCharacterRange_actualRange(
            &self,
            range: NSRange,
            actual_range: *mut NSRange,
        ) -> NSRect {
            // SAFETY: AppKit supplies either null or a writable output pointer
            // for the duration of this synchronous callback.
            if let Some(actual_range) = unsafe { actual_range.as_mut() } {
                *actual_range = range;
            }
            let local = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1.0, 1.0));
            self.window()
                .map_or(local, |window| window.convertRectToScreen(local))
        }

        #[allow(
            non_snake_case,
            reason = "the generated protocol requires this Rust method name"
        )]
        #[unsafe(method(characterIndexForPoint:))]
        fn characterIndexForPoint(&self, _point: NSPoint) -> usize {
            0
        }
    }

    impl SurfaceView {
        #[unsafe(method(isAccessibilityElement))]
        fn is_accessibility_element(&self) -> bool {
            false
        }

        #[unsafe(method_id(accessibilityChildren))]
        fn accessibility_children(&self) -> Retained<NSArray<NativeAccessibilityElement>> {
            NativeAccessibilityAdapter::surface_children(self)
        }

        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            true
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            self.emit(keyboard_event(event, KeyState::Down));
            self.interpretKeyEvents(&NSArray::from_retained_slice(&[event.retain()]));
        }

        #[unsafe(method(keyUp:))]
        fn key_up(&self, event: &NSEvent) {
            self.emit(keyboard_event(event, KeyState::Up));
        }

        #[unsafe(method(flagsChanged:))]
        fn flags_changed(&self, event: &NSEvent) {
            self.emit(keyboard_event(event, KeyState::ModifiersChanged));
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            self.emit_pointer(event, PointerAction::Down, PointerButton::Primary);
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, event: &NSEvent) {
            self.emit_pointer(event, PointerAction::Up, PointerButton::Primary);
        }

        #[unsafe(method(rightMouseDown:))]
        fn right_mouse_down(&self, event: &NSEvent) {
            self.emit_pointer(event, PointerAction::Down, PointerButton::Secondary);
        }

        #[unsafe(method(rightMouseUp:))]
        fn right_mouse_up(&self, event: &NSEvent) {
            self.emit_pointer(event, PointerAction::Up, PointerButton::Secondary);
        }

        #[unsafe(method(otherMouseDown:))]
        fn other_mouse_down(&self, event: &NSEvent) {
            self.emit_pointer(event, PointerAction::Down, pointer_button(event));
        }

        #[unsafe(method(otherMouseUp:))]
        fn other_mouse_up(&self, event: &NSEvent) {
            self.emit_pointer(event, PointerAction::Up, pointer_button(event));
        }

        #[unsafe(method(mouseMoved:))]
        fn mouse_moved(&self, event: &NSEvent) {
            self.emit_pointer(event, PointerAction::Moved, PointerButton::None);
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            self.emit_pointer(event, PointerAction::Moved, PointerButton::Primary);
        }

        #[unsafe(method(rightMouseDragged:))]
        fn right_mouse_dragged(&self, event: &NSEvent) {
            self.emit_pointer(event, PointerAction::Moved, PointerButton::Secondary);
        }

        #[unsafe(method(otherMouseDragged:))]
        fn other_mouse_dragged(&self, event: &NSEvent) {
            self.emit_pointer(event, PointerAction::Moved, pointer_button(event));
        }

        #[unsafe(method(scrollWheel:))]
        fn scroll_wheel(&self, event: &NSEvent) {
            let Some(delta_x) = finite_f32(event.scrollingDeltaX()) else {
                self.ivars().input_dispatch_failed.set(true);
                return;
            };
            let Some(delta_y) = finite_f32(event.scrollingDeltaY()) else {
                self.ivars().input_dispatch_failed.set(true);
                return;
            };
            self.emit(NativeInputEvent::Scroll {
                delta_x,
                delta_y,
                phase: scroll_phase(event.phase(), event.momentumPhase()),
                precise: event.hasPreciseScrollingDeltas(),
                modifiers: modifiers(event.modifierFlags()),
            });
        }
    }
);

impl SurfaceView {
    pub(crate) fn new(main_thread: MainThreadMarker, frame: NSRect) -> objc2::rc::Retained<Self> {
        let allocated = Self::alloc(main_thread).set_ivars(SurfaceViewIvars {
            input_handler: RefCell::new(None),
            input_dispatch_failed: Cell::new(false),
            marked_text: RefCell::new(Box::default()),
            marked_selection: Cell::new(NSRange::new(0, 0)),
            input_epoch: Cell::new(InputEpoch::INITIAL),
            input_active: Cell::new(true),
            discarding_marked_text: Cell::new(false),
            rejected_ime_callbacks: Cell::new(0),
            accessibility: RefCell::new(NativeAccessibilityAdapter::new()),
        });
        // SAFETY: `frame` is finite and positive because the surface descriptor
        // validated it before allocating this view.
        unsafe { msg_send![super(allocated), initWithFrame: frame] }
    }

    pub(crate) fn install_input_handler(&self, handler: NativeInputHandler) -> bool {
        let Ok(mut installed) = self.ivars().input_handler.try_borrow_mut() else {
            return false;
        };
        if installed.is_some() {
            return false;
        }
        self.ivars().input_dispatch_failed.set(false);
        *installed = Some(handler);
        true
    }

    pub(crate) fn clear_input_handler(&self) {
        let _ = self.suspend_input_epoch();
        if let Ok(mut installed) = self.ivars().input_handler.try_borrow_mut() {
            installed.take();
        }
        self.clear_marked_text();
    }

    #[cfg(alpine_native_validation)]
    fn detach_input_handler_for_validation(&self) {
        if let Ok(mut installed) = self.ivars().input_handler.try_borrow_mut() {
            installed.take();
        }
    }

    pub(crate) fn take_input_dispatch_failure(&self) -> bool {
        self.ivars().input_dispatch_failed.replace(false)
    }

    fn install_accessibility_delegate(&self, delegate: &Retained<DisplayLinkDelegate>) {
        let weak = Weak::from_retained(delegate);
        self.ivars()
            .accessibility
            .borrow_mut()
            .install(Box::new(move |request| {
                weak.load()
                    .ok_or(SurfaceError::invariant(SurfaceOperation::Input))?
                    .dispatch_accessibility_request(request)
            }));
    }

    pub(crate) fn refresh_accessibility_if_active(&self) -> Result<(), SurfaceError> {
        NativeAccessibilityAdapter::refresh_view_if_active(self)
    }

    pub(crate) fn revoke_accessibility(&self) {
        NativeAccessibilityAdapter::revoke_view(self);
    }

    fn emit(&self, event: NativeInputEvent) {
        let Ok(mut installed) = self.ivars().input_handler.try_borrow_mut() else {
            self.ivars().input_dispatch_failed.set(true);
            return;
        };
        let Some(handler) = installed.as_mut() else {
            self.ivars().input_dispatch_failed.set(true);
            return;
        };
        handler(event);
    }

    fn emit_ime(&self, event: ImeEvent) {
        self.emit_ime_at_epoch(self.ivars().input_epoch.get(), event);
    }

    fn emit_ime_at_epoch(&self, input_epoch: InputEpoch, event: ImeEvent) {
        if !self.ivars().input_active.get()
            || self.ivars().input_epoch.get().classify(input_epoch) != InputEpochAdmission::Current
        {
            self.reject_ime_callback();
            return;
        }
        self.emit(NativeInputEvent::Ime { input_epoch, event });
    }

    fn accepts_ime_callback(&self) -> bool {
        if self.ivars().input_active.get() {
            true
        } else {
            self.reject_ime_callback();
            self.clear_marked_text();
            false
        }
    }

    fn reject_ime_callback(&self) {
        self.ivars()
            .rejected_ime_callbacks
            .set(self.ivars().rejected_ime_callbacks.get().saturating_add(1));
    }

    fn resume_input_epoch(&self) -> Option<InputEpoch> {
        if self.ivars().input_active.replace(true) {
            None
        } else {
            Some(self.ivars().input_epoch.get())
        }
    }

    fn input_focus_state(&self) -> (InputEpoch, bool) {
        (
            self.ivars().input_epoch.get(),
            self.ivars().input_active.get(),
        )
    }

    #[cfg(alpine_native_validation)]
    fn set_input_focus_state_for_validation(&self, input_epoch: InputEpoch, focused: bool) {
        self.ivars().input_epoch.set(input_epoch);
        self.ivars().input_active.set(focused);
    }

    fn suspend_input_epoch(&self) -> Option<InputEpoch> {
        if !self.ivars().input_active.get() {
            self.clear_marked_text();
            return None;
        }
        let input_epoch = self.ivars().input_epoch.get();
        let had_marked_text = self.has_marked_text_value();
        self.ivars().discarding_marked_text.set(true);
        // SAFETY: `inputContext` is inherited from NSResponder and returns a
        // nullable Objective-C object valid for this synchronous main-thread
        // callback. The generated AppKit feature set does not expose the
        // concrete NSTextInputContext type, so the two selectors remain local
        // to this reviewed unsafe boundary.
        let input_context: Option<Retained<AnyObject>> = unsafe { msg_send![self, inputContext] };
        if let Some(input_context) = input_context {
            // SAFETY: AppKit's input context implements discardMarkedText and
            // the retained receiver remains live for the synchronous message.
            unsafe {
                let _: () = msg_send![&*input_context, discardMarkedText];
            }
        }
        self.ivars().discarding_marked_text.set(false);
        self.clear_marked_text();
        if had_marked_text {
            self.emit_ime_at_epoch(input_epoch, ImeEvent::Cancelled);
        }
        let Some(next) = input_epoch.checked_next() else {
            self.ivars().input_active.set(false);
            self.ivars().input_dispatch_failed.set(true);
            return None;
        };
        self.ivars().input_epoch.set(next);
        self.ivars().input_active.set(false);
        Some(next)
    }

    #[cfg(alpine_native_validation)]
    fn rejected_ime_callbacks(&self) -> u64 {
        self.ivars().rejected_ime_callbacks.get()
    }

    fn emit_pointer(&self, event: &NSEvent, action: PointerAction, button: PointerButton) {
        let local = self.convertPoint_fromView(event.locationInWindow(), None);
        let bounds = self.bounds();
        let Some(x) = finite_f32(local.x) else {
            self.ivars().input_dispatch_failed.set(true);
            return;
        };
        let Some(y) = finite_f32(bounds.size.height - local.y) else {
            self.ivars().input_dispatch_failed.set(true);
            return;
        };
        let Some(position) = Point::new(x, y) else {
            self.ivars().input_dispatch_failed.set(true);
            return;
        };
        self.emit(NativeInputEvent::Pointer {
            action,
            position,
            button,
            modifiers: modifiers(event.modifierFlags()),
        });
    }

    fn clear_marked_text(&self) {
        self.ivars().marked_text.replace(Box::default());
        self.ivars().marked_selection.set(NSRange::new(0, 0));
    }

    fn has_marked_text_value(&self) -> bool {
        !self.ivars().marked_text.borrow().is_empty()
    }
}

fn keyboard_event(event: &NSEvent, state: KeyState) -> NativeInputEvent {
    let (logical_key, repeat) = keyboard_text_metadata(
        state,
        || {
            event
                .charactersIgnoringModifiers()
                .map(|characters| characters.to_string().into_boxed_str())
        },
        || event.isARepeat(),
    );
    NativeInputEvent::Keyboard {
        state,
        physical_key: event.keyCode(),
        logical_key,
        modifiers: modifiers(event.modifierFlags()),
        repeat,
    }
}

fn keyboard_text_metadata(
    state: KeyState,
    characters: impl FnOnce() -> Option<Box<str>>,
    repeat: impl FnOnce() -> bool,
) -> (Box<str>, bool) {
    if matches!(state, KeyState::ModifiersChanged) {
        return (Box::default(), false);
    }
    (characters().unwrap_or_default(), repeat())
}

fn clipboard_shortcut(event: &NativeInputEvent) -> Option<ClipboardOperation> {
    let NativeInputEvent::Keyboard {
        state: KeyState::Down,
        logical_key,
        modifiers,
        repeat: false,
        ..
    } = event
    else {
        return None;
    };
    if !modifiers.contains(Modifiers::COMMAND)
        || modifiers.contains(Modifiers::CONTROL)
        || modifiers.contains(Modifiers::OPTION)
        || modifiers.contains(Modifiers::SHIFT)
    {
        return None;
    }
    if logical_key.eq_ignore_ascii_case("c") {
        Some(ClipboardOperation::Copy)
    } else if logical_key.eq_ignore_ascii_case("x") {
        Some(ClipboardOperation::Cut)
    } else if logical_key.eq_ignore_ascii_case("v") {
        Some(ClipboardOperation::Paste)
    } else {
        None
    }
}

fn plain_text_pasteboard_type() -> &'static NSPasteboardType {
    // SAFETY: AppKit exports NSPasteboardTypeString as a non-null,
    // process-lifetime NSString constant on every supported macOS version.
    unsafe { NSPasteboardTypeString }
}

fn modifiers(flags: NSEventModifierFlags) -> Modifiers {
    let mut bits = 0;
    if flags.contains(NSEventModifierFlags::Shift) {
        bits |= Modifiers::SHIFT;
    }
    if flags.contains(NSEventModifierFlags::Control) {
        bits |= Modifiers::CONTROL;
    }
    if flags.contains(NSEventModifierFlags::Option) {
        bits |= Modifiers::OPTION;
    }
    if flags.contains(NSEventModifierFlags::Command) {
        bits |= Modifiers::COMMAND;
    }
    if flags.contains(NSEventModifierFlags::CapsLock) {
        bits |= Modifiers::CAPS_LOCK;
    }
    Modifiers::from_bits(bits)
}

fn pointer_button(event: &NSEvent) -> PointerButton {
    match event.buttonNumber() {
        0 => PointerButton::Primary,
        1 => PointerButton::Secondary,
        2 => PointerButton::Middle,
        number => PointerButton::Other(u8::try_from(number).unwrap_or(u8::MAX)),
    }
}

fn scroll_phase(phase: NSEventPhase, momentum_phase: NSEventPhase) -> ScrollPhase {
    let phase = if phase == NSEventPhase::None {
        momentum_phase
    } else {
        phase
    };
    if phase.contains(NSEventPhase::Cancelled) {
        ScrollPhase::Cancelled
    } else if phase.intersects(NSEventPhase::Began) || phase.intersects(NSEventPhase::MayBegin) {
        ScrollPhase::Began
    } else if phase.intersects(NSEventPhase::Changed) || phase.intersects(NSEventPhase::Stationary)
    {
        ScrollPhase::Changed
    } else if phase.contains(NSEventPhase::Ended) {
        ScrollPhase::Ended
    } else {
        ScrollPhase::None
    }
}

fn finite_f32(value: f64) -> Option<f32> {
    if !value.is_finite() || value < f64::from(f32::MIN) || value > f64::from(f32::MAX) {
        None
    } else {
        #[allow(
            clippy::cast_possible_truncation,
            reason = "the finite f32 range was checked immediately above"
        )]
        Some(value as f32)
    }
}

fn input_text(value: &AnyObject) -> Box<str> {
    if let Some(string) = value.downcast_ref::<NSString>() {
        return string.to_string().into_boxed_str();
    }
    value
        .downcast_ref::<NSAttributedString>()
        .map_or_else(Box::default, |string| {
            string.string().to_string().into_boxed_str()
        })
}

fn saturating_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

fn resolve_input_dispatch(
    result: Result<(), SurfaceError>,
    input_dispatch_failed: bool,
) -> Result<(), SurfaceError> {
    match result {
        Err(error) => Err(error),
        Ok(()) if input_dispatch_failed => Err(SurfaceError::invariant(SurfaceOperation::Input)),
        Ok(()) => Ok(()),
    }
}

type NSUInteger = usize;

#[cfg(test)]
mod native_input_tests {
    use super::*;

    #[test]
    fn modifier_keyboard_metadata_never_queries_character_fields() {
        let characters_queried = Cell::new(false);
        let repeat_queried = Cell::new(false);
        let (logical_key, repeat) = keyboard_text_metadata(
            KeyState::ModifiersChanged,
            || {
                characters_queried.set(true);
                Some(Box::from("unsafe"))
            },
            || {
                repeat_queried.set(true);
                true
            },
        );
        assert!(logical_key.is_empty());
        assert!(!repeat);
        assert!(!characters_queried.get());
        assert!(!repeat_queried.get());

        let (logical_key, repeat) =
            keyboard_text_metadata(KeyState::Down, || Some(Box::from("s")), || true);
        assert_eq!(&*logical_key, "s");
        assert!(repeat);
    }

    #[test]
    fn modifiers_preserve_only_alpine_supported_bits() {
        let native = NSEventModifierFlags::Shift
            | NSEventModifierFlags::Control
            | NSEventModifierFlags::Option
            | NSEventModifierFlags::Command
            | NSEventModifierFlags::CapsLock
            | NSEventModifierFlags::NumericPad;
        let translated = modifiers(native);
        assert_eq!(
            translated.bits(),
            Modifiers::SHIFT
                | Modifiers::CONTROL
                | Modifiers::OPTION
                | Modifiers::COMMAND
                | Modifiers::CAPS_LOCK
        );
    }

    #[test]
    fn utf16_ranges_saturate_at_public_width() {
        assert_eq!(saturating_u32(17), 17);
        if usize::BITS > u32::BITS {
            assert_eq!(saturating_u32(usize::MAX), u32::MAX);
        }
    }

    #[test]
    fn scroll_phase_prefers_direct_then_momentum_identity() {
        assert_eq!(
            scroll_phase(NSEventPhase::Began, NSEventPhase::None),
            ScrollPhase::Began
        );
        assert_eq!(
            scroll_phase(NSEventPhase::MayBegin, NSEventPhase::None),
            ScrollPhase::Began
        );
        assert_eq!(
            scroll_phase(NSEventPhase::Changed, NSEventPhase::Ended),
            ScrollPhase::Changed
        );
        assert_eq!(
            scroll_phase(NSEventPhase::Stationary, NSEventPhase::None),
            ScrollPhase::Changed
        );
        assert_eq!(
            scroll_phase(NSEventPhase::None, NSEventPhase::Ended),
            ScrollPhase::Ended
        );
        assert_eq!(
            scroll_phase(NSEventPhase::Cancelled, NSEventPhase::None),
            ScrollPhase::Cancelled
        );
    }

    #[test]
    fn deferred_pause_confirmation_only_accepts_portable_paused_state() {
        assert!(should_confirm_display_link_pause(DisplayLinkState::Paused));
        assert!(!should_confirm_display_link_pause(
            DisplayLinkState::Running
        ));
        assert!(!should_confirm_display_link_pause(
            DisplayLinkState::Invalid
        ));
    }

    #[test]
    fn every_pause_directive_schedules_post_callback_confirmation() {
        assert!(should_schedule_display_link_pause_confirmation(
            DisplayLinkDirective::Pause,
            DisplayLinkState::Paused,
            true,
        ));
        assert!(should_schedule_display_link_pause_confirmation(
            DisplayLinkDirective::None,
            DisplayLinkState::Paused,
            false,
        ));
        assert!(!should_schedule_display_link_pause_confirmation(
            DisplayLinkDirective::None,
            DisplayLinkState::Paused,
            true,
        ));
        assert!(!should_schedule_display_link_pause_confirmation(
            DisplayLinkDirective::None,
            DisplayLinkState::Running,
            false,
        ));
        assert!(!should_schedule_display_link_pause_confirmation(
            DisplayLinkDirective::Resume,
            DisplayLinkState::Paused,
            false,
        ));
        assert!(!should_schedule_display_link_pause_confirmation(
            DisplayLinkDirective::Invalidate,
            DisplayLinkState::Paused,
            false,
        ));
    }

    #[cfg(alpine_native_validation)]
    #[test]
    fn pause_diagnostics_record_and_decode_every_state() {
        let counters = PauseConfirmationCounters::default();
        counters.record_callback(
            DisplayLinkDirective::None,
            DisplayLinkState::Paused,
            false,
            Some((true, false)),
        );
        assert_eq!(counters.callback_observations.load(Ordering::Acquire), 1);
        assert_eq!(counters.last_directive.load(Ordering::Acquire), 1);
        assert_eq!(counters.last_portable_state.load(Ordering::Acquire), 1);
        assert!(!counters.last_native_paused_before.load(Ordering::Acquire));
        assert!(counters.last_pending.load(Ordering::Acquire));
        assert!(!counters.last_active.load(Ordering::Acquire));

        counters.record_callback(
            DisplayLinkDirective::Resume,
            DisplayLinkState::Running,
            true,
            Some((false, true)),
        );
        assert_eq!(counters.callback_observations.load(Ordering::Acquire), 2);
        assert_eq!(counters.last_directive.load(Ordering::Acquire), 2);
        assert_eq!(counters.last_portable_state.load(Ordering::Acquire), 2);
        assert!(counters.last_native_paused_before.load(Ordering::Acquire));
        assert!(!counters.last_pending.load(Ordering::Acquire));
        assert!(counters.last_active.load(Ordering::Acquire));

        counters.record_callback(
            DisplayLinkDirective::Pause,
            DisplayLinkState::Invalid,
            false,
            None,
        );
        assert_eq!(counters.callback_observations.load(Ordering::Acquire), 3);
        assert_eq!(counters.last_directive.load(Ordering::Acquire), 3);
        assert_eq!(counters.last_portable_state.load(Ordering::Acquire), 3);
        assert!(!counters.last_pending.load(Ordering::Acquire));
        assert!(counters.last_active.load(Ordering::Acquire));

        counters.record_callback(
            DisplayLinkDirective::Invalidate,
            DisplayLinkState::Paused,
            true,
            Some((false, false)),
        );
        assert_eq!(counters.callback_observations.load(Ordering::Acquire), 4);
        assert_eq!(counters.last_directive.load(Ordering::Acquire), 4);
        assert_eq!(counters.last_portable_state.load(Ordering::Acquire), 1);

        use crate::native_validation::{PauseDirectiveEvidence, PausePortableStateEvidence};
        assert_eq!(pause_directive_evidence(0), PauseDirectiveEvidence::Unknown);
        assert_eq!(pause_directive_evidence(1), PauseDirectiveEvidence::None);
        assert_eq!(pause_directive_evidence(2), PauseDirectiveEvidence::Resume);
        assert_eq!(pause_directive_evidence(3), PauseDirectiveEvidence::Pause);
        assert_eq!(
            pause_directive_evidence(4),
            PauseDirectiveEvidence::Invalidate
        );
        assert_eq!(pause_directive_evidence(5), PauseDirectiveEvidence::Unknown);
        assert_eq!(
            pause_portable_state_evidence(0),
            PausePortableStateEvidence::Unknown
        );
        assert_eq!(
            pause_portable_state_evidence(1),
            PausePortableStateEvidence::Paused
        );
        assert_eq!(
            pause_portable_state_evidence(2),
            PausePortableStateEvidence::Running
        );
        assert_eq!(
            pause_portable_state_evidence(3),
            PausePortableStateEvidence::Invalid
        );
        assert_eq!(
            pause_portable_state_evidence(4),
            PausePortableStateEvidence::Unknown
        );
    }

    #[test]
    fn finite_f32_rejects_invalid_or_unrepresentable_values() {
        assert_eq!(finite_f32(1.25), Some(1.25));
        assert_eq!(finite_f32(f64::from(f32::MIN)), Some(f32::MIN));
        assert_eq!(finite_f32(f64::from(f32::MAX)), Some(f32::MAX));
        assert_eq!(finite_f32(f64::from(f32::MIN) * 2.0), None);
        assert_eq!(finite_f32(f64::from(f32::MAX) * 2.0), None);
        assert_eq!(finite_f32(f64::NAN), None);
        assert_eq!(finite_f32(f64::INFINITY), None);
        assert_eq!(finite_f32(f64::MAX), None);
    }

    #[test]
    fn clipboard_shortcuts_require_exact_nonrepeating_command_identity() {
        let event = |logical_key: &str, modifiers: u8, repeat| NativeInputEvent::Keyboard {
            state: KeyState::Down,
            physical_key: 0,
            logical_key: logical_key.into(),
            modifiers: Modifiers::from_bits(modifiers),
            repeat,
        };
        assert_eq!(
            clipboard_shortcut(&event("c", Modifiers::COMMAND, false)),
            Some(ClipboardOperation::Copy)
        );
        assert_eq!(
            clipboard_shortcut(&event(
                "X",
                Modifiers::COMMAND | Modifiers::CAPS_LOCK,
                false
            )),
            Some(ClipboardOperation::Cut)
        );
        assert_eq!(
            clipboard_shortcut(&event("v", Modifiers::COMMAND, false)),
            Some(ClipboardOperation::Paste)
        );
        assert_eq!(
            clipboard_shortcut(&event("v", Modifiers::COMMAND | Modifiers::SHIFT, false)),
            None
        );
        assert_eq!(
            clipboard_shortcut(&event("v", Modifiers::COMMAND | Modifiers::CONTROL, false)),
            None
        );
        assert_eq!(
            clipboard_shortcut(&event("v", Modifiers::COMMAND | Modifiers::OPTION, false)),
            None
        );
        assert_eq!(
            clipboard_shortcut(&event("v", Modifiers::COMMAND, true)),
            None
        );
        assert_eq!(clipboard_shortcut(&event("v", 0, false)), None);
    }

    #[test]
    fn input_dispatch_preserves_root_error_and_reports_independent_failure() {
        assert_eq!(resolve_input_dispatch(Ok(()), false), Ok(()));
        assert_eq!(
            resolve_input_dispatch(Ok(()), true),
            Err(SurfaceError::invariant(SurfaceOperation::Input))
        );
        assert_eq!(
            resolve_input_dispatch(
                Err(SurfaceError::RunLoopNotRunnable {
                    lifecycle: SurfaceLifecycle::Closed,
                }),
                true
            ),
            Err(SurfaceError::RunLoopNotRunnable {
                lifecycle: SurfaceLifecycle::Closed,
            })
        );
    }
}

struct DisplayLinkDelegateIvars {
    lifecycle: Arc<AtomicU8>,
    window_close_started: Arc<AtomicBool>,
    callback_count: Arc<AtomicU64>,
    rejected_callback_count: Arc<AtomicU64>,
    #[cfg(alpine_native_validation)]
    pause_confirmation: Arc<PauseConfirmationCounters>,
    counters: Arc<FrameCounters>,
    driver: Option<Rc<RefCell<PresentationDriver>>>,
    application: Retained<NSApplication>,
    window: Option<Retained<NSWindow>>,
    view: Option<Retained<SurfaceView>>,
    layer: Option<Retained<CAMetalLayer>>,
    display_link: Option<Retained<CAMetalDisplayLink>>,
    event_handler: RefCell<Option<SurfaceEventHandler>>,
    event_sequence: Cell<u64>,
    #[cfg(alpine_native_validation)]
    validation_pasteboard: RefCell<Option<ValidationPasteboard>>,
    #[cfg(alpine_native_validation)]
    clipboard_fault: Cell<Option<ClipboardError>>,
    #[cfg(alpine_native_validation)]
    validation_probe: Option<InitializationProbe>,
}

#[cfg(alpine_native_validation)]
#[derive(Default)]
struct PauseConfirmationCounters {
    requested: AtomicU64,
    enqueued: AtomicU64,
    executed: AtomicU64,
    eligible: AtomicU64,
    observed: AtomicU64,
    callback_observations: AtomicU64,
    last_directive: AtomicU8,
    last_portable_state: AtomicU8,
    last_native_paused_before: AtomicBool,
    last_native_paused_after: AtomicBool,
    last_pending: AtomicBool,
    last_active: AtomicBool,
}

#[cfg(alpine_native_validation)]
impl PauseConfirmationCounters {
    fn record_callback(
        &self,
        directive: DisplayLinkDirective,
        state: DisplayLinkState,
        native_paused: bool,
        ownership: Option<(bool, bool)>,
    ) {
        self.callback_observations.fetch_add(1, Ordering::Relaxed);
        self.last_directive.store(
            match directive {
                DisplayLinkDirective::None => 1,
                DisplayLinkDirective::Resume => 2,
                DisplayLinkDirective::Pause => 3,
                DisplayLinkDirective::Invalidate => 4,
            },
            Ordering::Release,
        );
        self.last_portable_state.store(
            match state {
                DisplayLinkState::Paused => 1,
                DisplayLinkState::Running => 2,
                DisplayLinkState::Invalid => 3,
            },
            Ordering::Release,
        );
        self.last_native_paused_before
            .store(native_paused, Ordering::Release);
        if let Some((pending, active)) = ownership {
            self.last_pending.store(pending, Ordering::Release);
            self.last_active.store(active, Ordering::Release);
        }
    }
}

#[cfg(alpine_native_validation)]
const fn pause_directive_evidence(value: u8) -> crate::native_validation::PauseDirectiveEvidence {
    match value {
        1 => crate::native_validation::PauseDirectiveEvidence::None,
        2 => crate::native_validation::PauseDirectiveEvidence::Resume,
        3 => crate::native_validation::PauseDirectiveEvidence::Pause,
        4 => crate::native_validation::PauseDirectiveEvidence::Invalidate,
        _ => crate::native_validation::PauseDirectiveEvidence::Unknown,
    }
}

#[cfg(alpine_native_validation)]
const fn pause_portable_state_evidence(
    value: u8,
) -> crate::native_validation::PausePortableStateEvidence {
    match value {
        1 => crate::native_validation::PausePortableStateEvidence::Paused,
        2 => crate::native_validation::PausePortableStateEvidence::Running,
        3 => crate::native_validation::PausePortableStateEvidence::Invalid,
        _ => crate::native_validation::PausePortableStateEvidence::Unknown,
    }
}

#[cfg(alpine_native_validation)]
struct ValidationPasteboard {
    pasteboard: Retained<NSPasteboard>,
    probe: Option<InitializationProbe>,
    lease: RefCell<Option<InitializationLease>>,
    released: Cell<bool>,
}

#[cfg(alpine_native_validation)]
impl ValidationPasteboard {
    fn new(probe: Option<InitializationProbe>) -> Self {
        let pasteboard = NSPasteboard::pasteboardWithUniqueName();
        let lease = probe
            .as_ref()
            .map(|probe| probe.acquire(NativeOwnerKind::Pasteboard));
        Self {
            pasteboard,
            probe,
            lease: RefCell::new(lease),
            released: Cell::new(false),
        }
    }

    fn retained(&self) -> Retained<NSPasteboard> {
        self.pasteboard.clone()
    }

    fn release(&self) {
        if self.released.replace(true) {
            return;
        }
        // SAFETY: `pasteboardWithUniqueName` returns a registered server-side
        // pasteboard, and AppKit documents `releaseGlobally` as its matching
        // resource-release operation. The retained receiver remains alive for
        // the duration of this message.
        let _: () = unsafe { msg_send![&*self.pasteboard, releaseGlobally] };
        if let Some(probe) = &self.probe {
            probe.record_pasteboard_release();
        }
        drop(self.lease.borrow_mut().take());
    }
}

#[cfg(alpine_native_validation)]
impl Drop for ValidationPasteboard {
    fn drop(&mut self) {
        self.release();
    }
}

define_class!(
    // SAFETY:
    // - NSObject has no subclassing requirements.
    // - DisplayLinkDelegate has no custom Drop implementation.
    // - The object is main-thread-only, matching both the AppKit delegate and
    //   the main-run-loop registration of its display-link callback.
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = DisplayLinkDelegateIvars]
    struct DisplayLinkDelegate;

    // SAFETY: NSObjectProtocol adds no methods with unfulfilled invariants.
    unsafe impl NSObjectProtocol for DisplayLinkDelegate {}

    // SAFETY: The selector and Rust signature exactly match the generated
    // CAMetalDisplayLinkDelegate protocol. Both arguments are borrowed only
    // for this callback and neither escapes.
    unsafe impl CAMetalDisplayLinkDelegate for DisplayLinkDelegate {
        #[allow(
            non_snake_case,
            reason = "the generated protocol requires this Rust method name"
        )]
        #[unsafe(method(metalDisplayLink:needsUpdate:))]
        fn metalDisplayLink_needsUpdate(
            &self,
            link: &CAMetalDisplayLink,
            update: &CAMetalDisplayLinkUpdate,
        ) {
            let lifecycle = self.ivars().lifecycle.load(Ordering::Acquire);
            if lifecycle == SURFACE_LIVE {
                if !admit_callback(
                    &self.ivars().lifecycle,
                    &self.ivars().callback_count,
                    &self.ivars().rejected_callback_count,
                ) {
                    return;
                }
            } else if lifecycle != SURFACE_CLOSING {
                self.ivars()
                    .rejected_callback_count
                    .fetch_add(1, Ordering::Relaxed);
                return;
            }
            if MainThreadMarker::new().is_none() {
                self.ivars().counters.failed.fetch_add(1, Ordering::Relaxed);
                return;
            }
            if let Some(driver) = &self.ivars().driver {
                let (directive, display_link_state) = driver.try_borrow_mut().map_or_else(
                    |_| {
                        self.ivars().counters.failed.fetch_add(1, Ordering::Relaxed);
                        (DisplayLinkDirective::Pause, DisplayLinkState::Paused)
                    },
                    |mut driver| {
                        let directive = if lifecycle == SURFACE_CLOSING {
                            driver.drain_shutdown(&self.ivars().counters)
                        } else {
                            driver.update(update, &self.ivars().counters)
                        };
                        (directive, driver.display_link_state())
                    },
                );
                #[cfg(alpine_native_validation)]
                self.ivars().pause_confirmation.record_callback(
                    directive,
                    display_link_state,
                    link.isPaused(),
                    driver
                        .try_borrow()
                        .ok()
                        .map(|driver| (driver.pending.is_some(), driver.active.is_some())),
                );
                if lifecycle == SURFACE_CLOSING
                    && matches!(directive, DisplayLinkDirective::Invalidate)
                {
                    link.setPaused(true);
                    stop_event_loop(&self.ivars().application);
                } else {
                    apply_display_link_directive(link, directive);
                    #[cfg(alpine_native_validation)]
                    self.ivars()
                        .pause_confirmation
                        .last_native_paused_after
                        .store(link.isPaused(), Ordering::Release);
                    if should_schedule_display_link_pause_confirmation(
                        directive,
                        display_link_state,
                        link.isPaused(),
                    ) && let Some(display_link) = &self.ivars().display_link
                    {
                        #[cfg(alpine_native_validation)]
                        schedule_display_link_pause_confirmation(
                            display_link,
                            driver,
                            Arc::clone(&self.ivars().pause_confirmation),
                        );
                        #[cfg(not(alpine_native_validation))]
                        schedule_display_link_pause_confirmation(display_link, driver);
                    }
                }
                #[cfg(alpine_native_validation)]
                if self.ivars().lifecycle.load(Ordering::Acquire) != SURFACE_LIVE
                    && !self.ivars().window_close_started.load(Ordering::Acquire)
                    && let Some(window) = &self.ivars().window
                {
                    schedule_validation_window_close(window, Duration::ZERO);
                }
            }
        }
    }

    // SAFETY: Each implemented selector exactly matches NSWindowDelegate.
    // AppKit invokes these callbacks on the main thread and no notification
    // object or native reference escapes the callback.
    unsafe impl NSWindowDelegate for DisplayLinkDelegate {
        #[allow(
            non_snake_case,
            reason = "the generated protocol requires this Rust method name"
        )]
        #[unsafe(method(windowShouldClose:))]
        fn windowShouldClose(&self, _sender: &NSWindow) -> bool {
            match self.dispatch_surface_event(SurfaceEvent::CloseRequested {
                timestamp: self.next_event_timestamp(),
            }) {
                Ok(CloseDisposition::Allow) => true,
                Ok(CloseDisposition::Cancel | CloseDisposition::NotRequested) => false,
                Err(error) => {
                    self.record_dispatch_error(error);
                    false
                }
            }
        }

        #[unsafe(method(windowDidResize:))]
        fn window_did_resize(&self, _notification: &NSNotification) {
            let _ = self.synchronize_native_configuration_from_callback();
        }

        #[unsafe(method(windowDidBecomeKey:))]
        fn window_did_become_key(&self, _notification: &NSNotification) {
            self.publish_input_focus(true);
        }

        #[unsafe(method(windowDidResignKey:))]
        fn window_did_resign_key(&self, _notification: &NSNotification) {
            self.publish_input_focus(false);
        }

        #[unsafe(method(windowDidChangeScreen:))]
        fn window_did_change_screen(&self, _notification: &NSNotification) {
            let _ = self.synchronize_native_configuration_from_callback();
        }

        #[unsafe(method(windowDidChangeBackingProperties:))]
        fn window_did_change_backing_properties(&self, _notification: &NSNotification) {
            let _ = self.synchronize_native_configuration_from_callback();
        }

        #[unsafe(method(windowDidChangeOcclusionState:))]
        fn window_did_change_occlusion_state(&self, _notification: &NSNotification) {
            let _ = self.synchronize_native_configuration_from_callback();
            if self.ivars().window.as_ref().is_some_and(|window| {
                !presentation_visible(
                    window.isVisible(),
                    window.isMiniaturized(),
                    window
                        .occlusionState()
                        .contains(NSWindowOcclusionState::Visible),
                )
            }) {
                self.publish_input_focus(false);
            }
        }

        #[unsafe(method(windowDidMiniaturize:))]
        fn window_did_miniaturize(&self, _notification: &NSNotification) {
            self.publish_input_focus(false);
            let _ = self.synchronize_native_configuration_from_callback();
        }

        #[unsafe(method(windowDidDeminiaturize:))]
        fn window_did_deminiaturize(&self, _notification: &NSNotification) {
            let _ = self.synchronize_native_configuration_from_callback();
        }

        #[unsafe(method(windowWillClose:))]
        fn window_will_close(&self, _notification: &NSNotification) {
            self.begin_native_close();
        }
    }
);

fn admit_callback(
    lifecycle: &AtomicU8,
    callback_count: &AtomicU64,
    rejected_callback_count: &AtomicU64,
) -> bool {
    if lifecycle.load(Ordering::Acquire) == SURFACE_LIVE {
        callback_count.fetch_add(1, Ordering::Relaxed);
        true
    } else {
        rejected_callback_count.fetch_add(1, Ordering::Relaxed);
        false
    }
}

impl DisplayLinkDelegate {
    fn new(main_thread: MainThreadMarker, ivars: DisplayLinkDelegateIvars) -> Retained<Self> {
        let allocated = Self::alloc(main_thread).set_ivars(ivars);
        // SAFETY: The message is NSObject's parameterless init initializer and
        // the allocated object already contains fully initialized Rust ivars.
        unsafe { msg_send![super(allocated), init] }
    }

    fn synchronize_native_configuration(&self) -> Result<(), SurfaceError> {
        let result = self.try_synchronize_native_configuration();
        if let Err(error) = &result {
            let directive = self
                .ivars()
                .driver
                .as_ref()
                .and_then(|driver| driver.try_borrow_mut().ok())
                .map_or(DisplayLinkDirective::Pause, |mut driver| {
                    driver
                        .reject_configuration(error.clone())
                        .unwrap_or(DisplayLinkDirective::Pause)
                });
            if let Some(display_link) = &self.ivars().display_link {
                apply_display_link_directive(display_link, directive);
            }
        }
        result
    }

    fn synchronize_native_configuration_from_callback(&self) -> bool {
        if self.ivars().lifecycle.load(Ordering::Acquire) != SURFACE_LIVE {
            return false;
        }
        let _ = self.synchronize_native_configuration();
        true
    }

    fn try_synchronize_native_configuration(&self) -> Result<(), SurfaceError> {
        if self.ivars().lifecycle.load(Ordering::Acquire) != SURFACE_LIVE {
            return Err(SurfaceError::invariant(
                SurfaceOperation::NativeConfiguration,
            ));
        }
        let (Some(window), Some(view), Some(layer), Some(display_link), Some(driver)) = (
            &self.ivars().window,
            &self.ivars().view,
            &self.ivars().layer,
            &self.ivars().display_link,
            &self.ivars().driver,
        ) else {
            return Err(SurfaceError::invariant(
                SurfaceOperation::NativeConfiguration,
            ));
        };
        let configuration = native_configuration(window, view)?;
        apply_layer_configuration(layer, view, configuration);
        let directive = driver
            .try_borrow_mut()
            .map_err(|_| SurfaceError::owner_conflict(SurfaceOperation::NativeConfiguration))?
            .apply_configuration(configuration)?;
        apply_display_link_directive(display_link, directive);
        let extent = crate::SurfaceExtent::new(
            configuration.logical_width,
            configuration.logical_height,
            configuration.scale,
        )?;
        self.dispatch_surface_event(SurfaceEvent::Resize {
            timestamp: self.next_event_timestamp(),
            extent,
        })?;
        Ok(())
    }

    fn next_event_timestamp(&self) -> EventTimestamp {
        let next = self.ivars().event_sequence.get().saturating_add(1);
        self.ivars().event_sequence.set(next);
        EventTimestamp::new(next)
    }

    fn publish_input_focus(&self, focused: bool) {
        let Some(view) = &self.ivars().view else {
            return;
        };
        let input_epoch = if focused {
            view.resume_input_epoch()
        } else {
            view.suspend_input_epoch()
        };
        if let Some(input_epoch) = input_epoch {
            self.dispatch_callback_event(SurfaceEvent::Focus {
                timestamp: self.next_event_timestamp(),
                input_epoch,
                focused,
            });
        }
    }

    fn dispatch_native_input_event(&self, event: NativeInputEvent) {
        let received_at = Instant::now();
        if let Err(error) = self.try_dispatch_native_input_event(event, received_at) {
            self.record_dispatch_error(error);
        }
    }

    fn try_dispatch_native_input_event(
        &self,
        event: NativeInputEvent,
        received_at: Instant,
    ) -> Result<(), SurfaceError> {
        let clipboard_operation = clipboard_shortcut(&event);
        let timestamp = self.next_event_timestamp();
        let event = match event {
            NativeInputEvent::Keyboard {
                state,
                physical_key,
                logical_key,
                modifiers,
                repeat,
            } => SurfaceEvent::Keyboard {
                timestamp,
                state,
                physical_key,
                logical_key,
                modifiers,
                repeat,
            },
            NativeInputEvent::Pointer {
                action,
                position,
                button,
                modifiers,
            } => SurfaceEvent::Pointer {
                timestamp,
                action,
                position,
                button,
                modifiers,
            },
            NativeInputEvent::Scroll {
                delta_x,
                delta_y,
                phase,
                precise,
                modifiers,
            } => SurfaceEvent::Scroll {
                timestamp,
                delta_x,
                delta_y,
                phase,
                precise,
                modifiers,
            },
            NativeInputEvent::Ime { input_epoch, event } => SurfaceEvent::Ime {
                timestamp,
                input_epoch,
                event,
            },
        };
        let _close = self.dispatch_surface_event_at(event, received_at)?;
        if clipboard_operation == Some(ClipboardOperation::Paste) {
            let event = ClipboardEvent::PasteCompleted(self.read_clipboard());
            let _close = self.dispatch_surface_event_inner(
                SurfaceEvent::Clipboard {
                    timestamp: self.next_event_timestamp(),
                    event,
                },
                false,
                Instant::now(),
            )?;
        }
        Ok(())
    }

    fn install_event_handler<F>(&self, handler: F) -> Result<(), SurfaceError>
    where
        F: FnMut(SurfaceEvent) -> SurfaceResponse + 'static,
    {
        let mut installed = self
            .ivars()
            .event_handler
            .try_borrow_mut()
            .map_err(|_| SurfaceError::owner_conflict(SurfaceOperation::EventHandler))?;
        if installed.is_some() {
            return Err(SurfaceError::invariant(SurfaceOperation::EventHandler));
        }
        *installed = Some(Box::new(handler));
        Ok(())
    }

    fn clear_event_handler(&self) {
        if let Ok(mut installed) = self.ivars().event_handler.try_borrow_mut() {
            installed.take();
        }
    }

    fn dispatch_surface_event(
        &self,
        event: SurfaceEvent,
    ) -> Result<CloseDisposition, SurfaceError> {
        self.dispatch_surface_event_at(event, Instant::now())
    }

    fn dispatch_surface_event_at(
        &self,
        event: SurfaceEvent,
        received_at: Instant,
    ) -> Result<CloseDisposition, SurfaceError> {
        self.dispatch_surface_event_inner(event, true, received_at)
    }

    fn submit_surface_frame(
        &self,
        frame: crate::SurfaceFrame,
        operation: SurfaceOperation,
    ) -> Result<(), SurfaceError> {
        let (scene, clear) = frame.into_parts();
        let (_, directive) = self
            .ivars()
            .driver
            .as_ref()
            .ok_or(SurfaceError::invariant(operation))?
            .try_borrow_mut()
            .map_err(|_| SurfaceError::owner_conflict(operation))?
            .request_frame(scene, clear)?;
        let display_link = self
            .ivars()
            .display_link
            .as_ref()
            .ok_or(SurfaceError::invariant(operation))?;
        apply_display_link_directive(display_link, directive);
        Ok(())
    }

    fn dispatch_surface_event_inner(
        &self,
        event: SurfaceEvent,
        clipboard_write_allowed: bool,
        received_at: Instant,
    ) -> Result<CloseDisposition, SurfaceError> {
        let event_timestamp = event.timestamp();
        let close_requested = matches!(event, SurfaceEvent::CloseRequested { .. });
        let response = {
            let mut installed = self
                .ivars()
                .event_handler
                .try_borrow_mut()
                .map_err(|_| SurfaceError::owner_conflict(SurfaceOperation::Input))?;
            installed
                .as_mut()
                .map_or_else(SurfaceResponse::default, |handler| handler(event))
        };
        let handler_finished_at = Instant::now();
        let (frame, clipboard_write, close, accessibility) = response.into_channels();
        if accessibility.is_some() {
            return Err(SurfaceError::invariant(SurfaceOperation::Input));
        }
        if close_requested {
            if close == CloseDisposition::NotRequested {
                return Ok(close);
            }
        } else if close != CloseDisposition::NotRequested {
            return Err(SurfaceError::invariant(SurfaceOperation::Input));
        }

        if let Some(frame) = frame {
            let (scene, clear) = frame.into_parts();
            let event_timing = EventFrameTiming {
                timestamp: event_timestamp,
                received_at,
                handler_finished_at,
                admitted_at: Instant::now(),
            };
            let (_, directive) = self
                .ivars()
                .driver
                .as_ref()
                .ok_or(SurfaceError::invariant(SurfaceOperation::Input))?
                .try_borrow_mut()
                .map_err(|_| SurfaceError::owner_conflict(SurfaceOperation::Input))?
                .request_frame_with_event(scene, clear, Some(event_timing))?;
            let display_link = self
                .ivars()
                .display_link
                .as_ref()
                .ok_or(SurfaceError::invariant(SurfaceOperation::Input))?;
            apply_display_link_directive(display_link, directive);
        }

        if let Some(write) = clipboard_write {
            if !clipboard_write_allowed {
                return Err(SurfaceError::invariant(SurfaceOperation::Input));
            }
            let operation = write.operation();
            let result = self.write_clipboard(write);
            let event = match operation {
                ClipboardOperation::Copy => ClipboardEvent::CopyCompleted(result),
                ClipboardOperation::Cut => ClipboardEvent::CutCompleted(result),
                ClipboardOperation::Paste => {
                    return Err(SurfaceError::invariant(SurfaceOperation::Input));
                }
            };
            let _close = self.dispatch_surface_event_inner(
                SurfaceEvent::Clipboard {
                    timestamp: self.next_event_timestamp(),
                    event,
                },
                false,
                Instant::now(),
            )?;
        }
        if close != CloseDisposition::Allow {
            self.ivars()
                .view
                .as_ref()
                .ok_or(SurfaceError::invariant(SurfaceOperation::Input))?
                .refresh_accessibility_if_active()?;
        }
        Ok(close)
    }

    fn dispatch_accessibility_request(
        &self,
        request: &AccessibilityRequest,
    ) -> Result<AccessibilityResponse, SurfaceError> {
        let event = SurfaceEvent::Accessibility {
            timestamp: self.next_event_timestamp(),
            request: request.clone(),
        };
        let response = {
            let mut installed = self
                .ivars()
                .event_handler
                .try_borrow_mut()
                .map_err(|_| SurfaceError::owner_conflict(SurfaceOperation::Accessibility))?;
            installed
                .as_mut()
                .map_or_else(SurfaceResponse::default, |handler| handler(event))
        };
        let (frame, clipboard, close, accessibility) = response.into_channels();
        if clipboard.is_some() || close != CloseDisposition::NotRequested {
            return Err(SurfaceError::invariant(SurfaceOperation::Accessibility));
        }
        let response =
            accessibility.ok_or(SurfaceError::invariant(SurfaceOperation::Accessibility))?;
        response
            .validate_for(request)
            .map_err(|_| SurfaceError::invariant(SurfaceOperation::Accessibility))?;
        if let Some(frame) = frame {
            let successful_action = Self::accessibility_frame_admitted(
                request.kind() == crate::AccessibilityRequestKind::Action,
                matches!(
                    response.result(),
                    Ok(crate::AccessibilityPayload::Action(
                        crate::AccessibilityActionResult::Applied
                            | crate::AccessibilityActionResult::Unchanged
                    ))
                ),
            );
            if !successful_action {
                return Err(SurfaceError::invariant(SurfaceOperation::Accessibility));
            }
            self.submit_surface_frame(frame, SurfaceOperation::Accessibility)?;
        }
        Ok(response)
    }

    const fn accessibility_frame_admitted(
        action_request: bool,
        applied_or_unchanged: bool,
    ) -> bool {
        action_request && applied_or_unchanged
    }

    fn dispatch_callback_event(&self, event: SurfaceEvent) {
        if let Err(error) = self.dispatch_surface_event(event) {
            self.record_dispatch_error(error);
        }
    }

    fn record_dispatch_error(&self, error: SurfaceError) {
        if let Some(driver) = &self.ivars().driver
            && let Ok(mut driver) = driver.try_borrow_mut()
        {
            driver.record_error(error);
        }
    }

    #[cfg_attr(
        not(alpine_native_validation),
        allow(
            clippy::unused_self,
            reason = "validation selects the delegate-owned isolated pasteboard"
        )
    )]
    fn pasteboard(&self) -> Retained<NSPasteboard> {
        #[cfg(alpine_native_validation)]
        {
            let mut pasteboard = self.ivars().validation_pasteboard.borrow_mut();
            return pasteboard
                .get_or_insert_with(|| {
                    ValidationPasteboard::new(self.ivars().validation_probe.clone())
                })
                .retained();
        }
        #[cfg(not(alpine_native_validation))]
        {
            NSPasteboard::generalPasteboard()
        }
    }

    #[cfg_attr(
        not(alpine_native_validation),
        allow(
            clippy::unused_self,
            reason = "validation consumes the delegate-owned injected fault"
        )
    )]
    fn take_clipboard_fault(&self) -> Option<ClipboardError> {
        #[cfg(alpine_native_validation)]
        {
            return self.ivars().clipboard_fault.take();
        }
        #[cfg(not(alpine_native_validation))]
        {
            None
        }
    }

    fn write_clipboard(&self, write: ClipboardWrite) -> Result<(), ClipboardError> {
        if let Some(error) = self.take_clipboard_fault() {
            return Err(error);
        }
        let pasteboard = self.pasteboard();
        let (_, text) = write.into_parts();
        let text = NSString::from_str(text.as_str());
        let _change_count = pasteboard.clearContents();
        if pasteboard.setString_forType(&text, plain_text_pasteboard_type()) {
            Ok(())
        } else {
            Err(ClipboardError::WriteRejected)
        }
    }

    fn read_clipboard(&self) -> Result<ClipboardText, ClipboardError> {
        if let Some(error) = self.take_clipboard_fault() {
            return Err(error);
        }
        let text = self
            .pasteboard()
            .stringForType(plain_text_pasteboard_type())
            .ok_or(ClipboardError::Unavailable)?;
        let bytes = text.lengthOfBytesUsingEncoding(NSUTF8StringEncoding);
        validate_clipboard_text_bytes(bytes)?;
        ClipboardText::new(text.to_string().into_boxed_str())
    }

    fn begin_native_close(&self) {
        if self
            .ivars()
            .window_close_started
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        if let Some(view) = &self.ivars().view {
            view.revoke_accessibility();
        }
        self.publish_input_focus(false);
        begin_close_observer_state(&self.ivars().lifecycle);
        #[cfg(alpine_native_validation)]
        if let Some(probe) = &self.ivars().validation_probe {
            probe.record_window_close();
        }
        let mut drained = true;
        if let Some(driver) = &self.ivars().driver
            && let Ok(mut driver) = driver.try_borrow_mut()
        {
            drained = driver.begin_shutdown(&self.ivars().counters);
        }
        if drained {
            self.finish_native_close();
        }
    }

    fn finish_native_close(&self) {
        if let Some(display_link) = &self.ivars().display_link {
            display_link.setPaused(true);
            display_link.invalidate();
            display_link.setDelegate(None);
        }
        stop_event_loop(&self.ivars().application);
    }
}

fn validate_clipboard_text_bytes(bytes: usize) -> Result<(), ClipboardError> {
    if bytes > crate::MAX_CLIPBOARD_TEXT_BYTES {
        Err(ClipboardError::TooLarge {
            bytes,
            limit: crate::MAX_CLIPBOARD_TEXT_BYTES,
        })
    } else {
        Ok(())
    }
}

fn native_configuration(
    window: &NSWindow,
    view: &NSView,
) -> Result<SurfaceConfiguration, SurfaceError> {
    let bounds = view.bounds();
    let display_identity = window
        .screen()
        .map_or(0, |screen| Retained::as_ptr(&screen) as usize);
    let visible = presentation_visible(
        window.isVisible(),
        window.isMiniaturized(),
        window
            .occlusionState()
            .contains(NSWindowOcclusionState::Visible),
    );
    SurfaceConfiguration::from_native(
        bounds.size.width,
        bounds.size.height,
        window.backingScaleFactor(),
        display_identity,
        visible,
    )
}

fn apply_layer_configuration(
    layer: &CAMetalLayer,
    view: &NSView,
    configuration: SurfaceConfiguration,
) {
    layer.setFrame(view.bounds());
    layer.setContentsScale(configuration.scale);
    layer.setDrawableSize(NSSize::new(
        f64::from(configuration.physical_width),
        f64::from(configuration.physical_height),
    ));
}

fn layer_sdr_color_contract(layer: &CAMetalLayer) -> Option<SdrColorContract> {
    let color_space = layer.colorspace()?;
    let name = CGColorSpace::name(Some(&color_space))?;
    // SAFETY: CoreGraphics exports this process-lifetime immutable CFString
    // constant on every supported macOS version.
    let standard_srgb_name = unsafe { kCGColorSpaceSRGB };
    recognizes_sdr_color_contract(
        layer.pixelFormat(),
        layer.wantsExtendedDynamicRangeContent(),
        &*name == standard_srgb_name,
    )
    .then_some(SdrColorContract::LinearSrgbToBgra8UnormSrgb)
}

fn recognizes_sdr_color_contract(
    pixel_format: MTLPixelFormat,
    extended_dynamic_range: bool,
    standard_srgb_color_space: bool,
) -> bool {
    pixel_format == MTLPixelFormat::BGRA8Unorm_sRGB
        && !extended_dynamic_range
        && standard_srgb_color_space
}

#[cfg(any(test, alpine_native_validation))]
fn centered_window_origin(window: NSRect, visible: NSRect) -> NSPoint {
    let available_x = (visible.size.width - window.size.width).max(0.0);
    let available_y = (visible.size.height - window.size.height).max(0.0);
    NSPoint::new(
        visible.origin.x + available_x / 2.0,
        visible.origin.y + available_y / 2.0,
    )
}

#[cfg(alpine_native_validation)]
fn validation_screen_configuration(
    index: usize,
    screen: &NSScreen,
) -> crate::native_validation::ValidationScreenConfiguration {
    let visible = screen.visibleFrame();
    crate::native_validation::ValidationScreenConfiguration::new(
        index,
        core::ptr::from_ref(screen) as usize,
        screen.backingScaleFactor(),
        visible.origin.x,
        visible.origin.y,
        visible.size.width,
        visible.size.height,
    )
}

fn apply_display_link_directive(link: &CAMetalDisplayLink, directive: DisplayLinkDirective) {
    match directive {
        DisplayLinkDirective::None => {}
        DisplayLinkDirective::Resume => link.setPaused(false),
        DisplayLinkDirective::Pause => link.setPaused(true),
        DisplayLinkDirective::Invalidate => {
            link.setPaused(true);
            link.invalidate();
        }
    }
}

const fn should_confirm_display_link_pause(state: DisplayLinkState) -> bool {
    matches!(state, DisplayLinkState::Paused)
}

const fn should_schedule_display_link_pause_confirmation(
    directive: DisplayLinkDirective,
    state: DisplayLinkState,
    native_paused: bool,
) -> bool {
    matches!(directive, DisplayLinkDirective::Pause)
        || (matches!(directive, DisplayLinkDirective::None)
            && matches!(state, DisplayLinkState::Paused)
            && !native_paused)
}

fn schedule_display_link_pause_confirmation(
    display_link: &Retained<CAMetalDisplayLink>,
    driver: &Rc<RefCell<PresentationDriver>>,
    #[cfg(alpine_native_validation)] pause_confirmation: Arc<PauseConfirmationCounters>,
) {
    #[cfg(alpine_native_validation)]
    pause_confirmation.requested.fetch_add(1, Ordering::Relaxed);
    if MainThreadMarker::new().is_none() {
        display_link.setPaused(true);
        return;
    }
    let Some(run_loop) = CFRunLoop::main() else {
        return;
    };
    // SAFETY: Core Foundation publishes this process-lifetime constant as a
    // CFRunLoopMode, which is a CFString and therefore a valid CFType mode.
    let Some(common_modes) = (unsafe { kCFRunLoopCommonModes }) else {
        return;
    };
    let display_link = display_link.clone();
    let driver = Rc::downgrade(driver);
    #[cfg(alpine_native_validation)]
    let pause_confirmation_for_block = Arc::clone(&pause_confirmation);
    let confirmation: RcBlock<dyn Fn()> = RcBlock::new(move || {
        #[cfg(alpine_native_validation)]
        pause_confirmation_for_block
            .executed
            .fetch_add(1, Ordering::Relaxed);
        if MainThreadMarker::new().is_none() {
            return;
        }
        let should_pause = driver.upgrade().is_some_and(|driver| {
            driver
                .try_borrow()
                .is_ok_and(|driver| should_confirm_display_link_pause(driver.display_link_state()))
        });
        if should_pause {
            #[cfg(alpine_native_validation)]
            pause_confirmation_for_block
                .eligible
                .fetch_add(1, Ordering::Relaxed);
            display_link.setPaused(true);
            #[cfg(alpine_native_validation)]
            if display_link.isPaused() {
                pause_confirmation_for_block
                    .observed
                    .fetch_add(1, Ordering::Relaxed);
            }
        }
    });
    // SAFETY: this call copies the block before returning. The block captures
    // main-thread-only owners, is enqueued on the main run loop, and the common
    // mode constant above has the required CFType identity.
    unsafe {
        run_loop.perform_block(Some(common_modes), Some(&confirmation));
    }
    #[cfg(alpine_native_validation)]
    pause_confirmation.enqueued.fetch_add(1, Ordering::Relaxed);
    run_loop.wake_up();
}

#[cfg(alpine_native_validation)]
fn schedule_validation_window_close(window: &Retained<NSWindow>, delay: Duration) {
    let window = window.clone();
    let close_block: RcBlock<dyn Fn(NonNull<NSTimer>)> =
        RcBlock::new(move |timer: NonNull<NSTimer>| {
            // SAFETY: Foundation supplies a valid borrowed timer for the
            // complete callback, and the reference does not escape.
            unsafe { timer.as_ref() }.invalidate();
            window.close();
        });
    // SAFETY: The block and retained window remain main-thread-only,
    // Foundation copies the block for the scheduled timer lifetime, and the
    // callback receives a valid NSTimer after the display-link callback exits.
    let _timer = unsafe {
        NSTimer::scheduledTimerWithTimeInterval_repeats_block(
            delay.as_secs_f64(),
            false,
            &close_block,
        )
    };
}

#[cfg(alpine_native_validation)]
fn schedule_validation_user_window_close(
    window: &Retained<NSWindow>,
    lifecycle: &Arc<AtomicU8>,
    counters: &Arc<FrameCounters>,
    delay: Duration,
) {
    schedule_validation_qualified_window_close(
        window,
        lifecycle,
        counters,
        delay,
        ValidationCloseAction::UserButton,
    );
}

#[cfg(alpine_native_validation)]
fn schedule_validation_programmatic_window_close(
    window: &Retained<NSWindow>,
    delegate: &Retained<DisplayLinkDelegate>,
    driver: &Rc<RefCell<PresentationDriver>>,
    lifecycle: &Arc<AtomicU8>,
    counters: &Arc<FrameCounters>,
    delay: Duration,
) {
    schedule_validation_qualified_window_close(
        window,
        lifecycle,
        counters,
        delay,
        ValidationCloseAction::Programmatic {
            delegate: delegate.clone(),
            driver: Rc::clone(driver),
            observed_frames: Cell::new(0),
        },
    );
}

#[cfg(alpine_native_validation)]
const VALIDATION_CLOSE_OBSERVATION_LIMIT: u8 = 8;

#[cfg(alpine_native_validation)]
const VALIDATION_CLOSE_PRESENTED_TIME: f64 = 2.0;

#[cfg(alpine_native_validation)]
const VALIDATION_CLOSE_RETRY_DELAY: Duration = Duration::from_millis(37);

#[cfg(alpine_native_validation)]
fn next_validation_close_observation(
    observed_count: u8,
    active_observed: Option<bool>,
) -> Option<u8> {
    match active_observed {
        Some(false) => observed_count
            .checked_add(1)
            .filter(|next_count| *next_count <= VALIDATION_CLOSE_OBSERVATION_LIMIT),
        None | Some(true) => None,
    }
}

#[cfg(alpine_native_validation)]
const fn validation_close_resources_drained(
    has_pending: bool,
    has_active: bool,
    occupied_slots: u8,
) -> bool {
    matches!((has_pending, has_active, occupied_slots), (false, false, 0))
}

#[cfg(alpine_native_validation)]
const fn validation_close_should_retry(qualified_presented: u64, resources_drained: bool) -> bool {
    qualified_presented == 0 || !resources_drained
}

#[cfg(alpine_native_validation)]
enum ValidationCloseAction {
    UserButton,
    Programmatic {
        delegate: Retained<DisplayLinkDelegate>,
        driver: Rc<RefCell<PresentationDriver>>,
        observed_frames: Cell<u8>,
    },
}

#[cfg(alpine_native_validation)]
fn schedule_validation_qualified_window_close(
    window: &Retained<NSWindow>,
    lifecycle: &Arc<AtomicU8>,
    counters: &Arc<FrameCounters>,
    delay: Duration,
    action: ValidationCloseAction,
) {
    let window = window.clone();
    let lifecycle = Arc::clone(lifecycle);
    let counters = Arc::clone(counters);
    let close_block: RcBlock<dyn Fn(NonNull<NSTimer>)> =
        RcBlock::new(move |timer: NonNull<NSTimer>| {
            // SAFETY: Foundation supplies a valid borrowed timer for the
            // complete callback, and the reference does not escape.
            let timer = unsafe { timer.as_ref() };
            if lifecycle.load(Ordering::Acquire) != SURFACE_LIVE {
                timer.invalidate();
                return;
            }
            let ready_to_close = if let ValidationCloseAction::Programmatic {
                driver,
                observed_frames,
                ..
            } = &action
            {
                driver.try_borrow_mut().is_ok_and(|mut driver| {
                    let active_observed = driver
                        .active
                        .as_ref()
                        .map(|active| active.observation.observed());
                    if let Some(next_count) =
                        next_validation_close_observation(observed_frames.get(), active_observed)
                        && let Some(active) = driver.active.as_mut()
                    {
                        active
                            .observation
                            .inject(VALIDATION_CLOSE_PRESENTED_TIME.to_bits());
                        observed_frames.set(next_count);
                    }
                    validation_close_resources_drained(
                        driver.pending.is_some(),
                        driver.active.is_some(),
                        driver.frame_slots.snapshot().occupied_slots(),
                    )
                })
            } else {
                true
            };
            // Hosted runners do not provide qualifying physical presentation
            // callbacks. Bootstrap the observation only after a real frame is
            // active, then wait for its terminal accounting and full slot drain
            // before exercising the production close delegates. This keeps the
            // complete journey inside one NSApplication run-loop invocation.
            if validation_close_should_retry(
                counters.qualified_presented.load(Ordering::Acquire),
                ready_to_close,
            ) {
                timer.setFireDate(&NSDate::dateWithTimeIntervalSinceNow(
                    VALIDATION_CLOSE_RETRY_DELAY.as_secs_f64(),
                ));
                return;
            }
            match &action {
                ValidationCloseAction::UserButton => {
                    // SAFETY: `standardWindowButton:` is an AppKit selector on
                    // NSWindow. The returned button remains owned by the retained
                    // window for this immediate main-thread activation.
                    let close_button: *mut AnyObject = unsafe {
                        msg_send![&**window, standardWindowButton: NSWindowButton::CloseButton]
                    };
                    let Some(close_button) = NonNull::new(close_button) else {
                        timer.invalidate();
                        return;
                    };
                    // SAFETY: The standard close button and captured window are
                    // main-thread-only. AppKit routes this user-equivalent activation
                    // through the installed production NSWindowDelegate.
                    let _: () = unsafe {
                        msg_send![close_button.as_ref(), performClick: None::<&AnyObject>]
                    };
                }
                ValidationCloseAction::Programmatic { delegate, .. } => {
                    // SAFETY: The selector and return type exactly match the
                    // production NSWindowDelegate method implemented above.
                    // Both retained objects remain main-thread-only and are
                    // borrowed only for this synchronous validation dispatch.
                    let should_close: Bool =
                        unsafe { msg_send![&**delegate, windowShouldClose: &**window] };
                    if bool::from(should_close) {
                        // Headless AppKit may terminate its run loop without
                        // delivering windowWillClose. Invoke the same
                        // idempotent production teardown authority directly;
                        // a later native callback is safely coalesced.
                        delegate.begin_native_close();
                        window.close();
                    }
                }
            }
            timer.invalidate();
        });
    // SAFETY: The block and retained window remain main-thread-only,
    // Foundation copies it for the scheduled timer lifetime, and the
    // callback receives a valid NSTimer after the run loop starts.
    let _timer = unsafe {
        NSTimer::scheduledTimerWithTimeInterval_repeats_block(
            delay.as_secs_f64(),
            true,
            &close_block,
        )
    };
}

pub(crate) struct NativeSurface {
    callback_count: Arc<AtomicU64>,
    rejected_callback_count: Arc<AtomicU64>,
    counters: Arc<FrameCounters>,
    driver: Rc<RefCell<PresentationDriver>>,
    lifecycle: Arc<AtomicU8>,
    window_close_started: Arc<AtomicBool>,
    display_link: Retained<CAMetalDisplayLink>,
    #[allow(
        dead_code,
        reason = "AppKit and CAMetalDisplayLink retain delegates weakly, so Alpine must retain it"
    )]
    delegate: Retained<DisplayLinkDelegate>,
    wake_bridge: Arc<NativeWakeBridge>,
    layer: Retained<CAMetalLayer>,
    #[allow(
        dead_code,
        reason = "the surface explicitly retains the color space installed on its layer"
    )]
    color_space: Retained<CGColorSpace>,
    #[allow(
        dead_code,
        reason = "the custom content view owns the layer attachment for the surface lifetime"
    )]
    view: Retained<SurfaceView>,
    window: Retained<NSWindow>,
    #[allow(
        dead_code,
        reason = "the layer's device remains owned explicitly for the surface lifetime"
    )]
    device: Device,
    #[allow(
        dead_code,
        reason = "the shared application remains part of the explicit native owner graph"
    )]
    application: Retained<NSApplication>,
    #[cfg(alpine_native_validation)]
    validation_probe: Option<InitializationProbe>,
    #[cfg(alpine_native_validation)]
    validation_event_loop_started: Cell<bool>,
    #[cfg(alpine_native_validation)]
    #[allow(
        dead_code,
        reason = "validation leases record release only when native owner fields drop"
    )]
    validation_leases: Vec<InitializationLease>,
}

fn standard_window_style_mask() -> NSWindowStyleMask {
    NSWindowStyleMask::Titled
        .union(NSWindowStyleMask::Closable)
        .union(NSWindowStyleMask::Miniaturizable)
        .union(NSWindowStyleMask::Resizable)
}

impl NativeSurface {
    #[allow(
        clippy::too_many_lines,
        reason = "the creation sequence keeps partial native ownership and rollback auditable"
    )]
    pub(crate) fn new(descriptor: &SurfaceDescriptor) -> Result<Self, SurfaceError> {
        Self::new_with_control(descriptor, &InitializationControl::production())
    }

    #[cfg(alpine_native_validation)]
    pub(crate) fn new_for_validation(descriptor: &SurfaceDescriptor) -> Result<Self, SurfaceError> {
        Self::new_with_control(descriptor, &InitializationControl::validation(None))
    }

    #[cfg(alpine_native_validation)]
    pub(crate) fn new_for_validation_device_loss(
        descriptor: &SurfaceDescriptor,
    ) -> Result<Self, SurfaceError> {
        Self::new_with_control(descriptor, &InitializationControl::validation_device_loss())
    }

    #[allow(
        clippy::too_many_lines,
        reason = "the creation sequence keeps partial native ownership and rollback auditable"
    )]
    fn new_with_control(
        descriptor: &SurfaceDescriptor,
        control: &InitializationControl,
    ) -> Result<Self, SurfaceError> {
        let Some(main_thread) = MainThreadMarker::new() else {
            return Err(native_unavailable(SurfaceStage::MainThread));
        };

        let extent = descriptor.extent();
        let (lifecycle, callback_count, rejected_callback_count) = control.observer_state();
        let mut builder = NativeSurfaceBuilder::new(
            lifecycle,
            callback_count,
            rejected_callback_count,
            #[cfg(alpine_native_validation)]
            control.probe.clone(),
        );
        control.checkpoint(SurfaceStage::MainThread)?;

        builder.application = Some(NSApplication::sharedApplication(main_thread));
        builder.track(NativeOwnerKind::Application);
        builder.device = Some(require_device(MTLCreateSystemDefaultDevice())?);
        builder.track(NativeOwnerKind::Device);
        control.checkpoint(SurfaceStage::Device)?;
        let device = builder
            .device
            .as_ref()
            .ok_or_else(|| native_unavailable(SurfaceStage::Device))?;
        builder.backend = Some(control.backend(device.clone())?);
        builder.track(NativeOwnerKind::Renderer);
        control.checkpoint(SurfaceStage::Renderer)?;

        let frame = NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(extent.logical_width(), extent.logical_height()),
        );
        // SAFETY: The validated finite positive content rectangle satisfies
        // NSWindow's initializer contract. Defer the window-server device until
        // first show so failed construction and never-shown surfaces do not
        // allocate backing resources. Alpine disables release-on-close
        // immediately below and retains the returned window for its lifetime.
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(main_thread),
                frame,
                standard_window_style_mask(),
                NSBackingStoreType::Buffered,
                true,
            )
        };
        #[cfg(alpine_native_validation)]
        if let Some(probe) = control.probe() {
            probe.record_window(&window);
        }
        // SAFETY: This window is created without a window controller and is
        // retained by NativeSurface until close, so AppKit must not release it.
        unsafe { window.setReleasedWhenClosed(false) };
        window.setTitle(&NSString::from_str(descriptor.title()));
        builder.window = Some(window);
        builder.track(NativeOwnerKind::Window);
        control.checkpoint(SurfaceStage::Window)?;

        let view = SurfaceView::new(main_thread, frame);
        builder.view = Some(view);
        builder.track(NativeOwnerKind::View);
        control.checkpoint(SurfaceStage::View)?;

        // SAFETY: CoreGraphics exports this process-lifetime immutable CFString
        // constant on every supported macOS version.
        let standard_srgb_name = unsafe { kCGColorSpaceSRGB };
        let color_space = CGColorSpace::with_name(Some(standard_srgb_name))
            .ok_or_else(|| native_unavailable(SurfaceStage::ColorSpace))?;
        builder.color_space = Some(color_space.into());
        builder.track(NativeOwnerKind::ColorSpace);
        control.checkpoint(SurfaceStage::ColorSpace)?;

        let layer = CAMetalLayer::layer();
        let device = builder
            .device
            .as_ref()
            .ok_or_else(|| native_unavailable(SurfaceStage::Device))?;
        layer.setDevice(Some(device));
        layer.setPixelFormat(MTLPixelFormat::BGRA8Unorm_sRGB);
        let color_space = builder
            .color_space
            .as_ref()
            .ok_or_else(|| native_unavailable(SurfaceStage::ColorSpace))?;
        layer.setColorspace(Some(color_space));
        layer.setWantsExtendedDynamicRangeContent(false);
        layer.setFramebufferOnly(true);
        layer.setMaximumDrawableCount(3);
        layer.setDisplaySyncEnabled(true);
        layer.setAllowsNextDrawableTimeout(true);
        layer.setOpaque(true);
        layer.setContentsScale(extent.scale());
        layer.setDrawableSize(NSSize::new(
            f64::from(extent.physical_width()),
            f64::from(extent.physical_height()),
        ));
        let view = builder
            .view
            .as_ref()
            .ok_or_else(|| native_unavailable(SurfaceStage::View))?;
        view.setWantsLayer(true);
        view.setLayer(Some(&layer));
        let window = builder
            .window
            .clone()
            .ok_or_else(|| native_unavailable(SurfaceStage::Window))?;
        window.setContentView(Some(view));
        builder.layer = Some(layer);
        builder.track(NativeOwnerKind::Layer);
        control.checkpoint(SurfaceStage::Layer)?;

        let layer = builder
            .layer
            .as_ref()
            .ok_or_else(|| native_unavailable(SurfaceStage::Layer))?;
        let display_link =
            CAMetalDisplayLink::initWithMetalLayer(CAMetalDisplayLink::alloc(), layer);
        display_link.setPreferredFrameLatency(2.0);
        display_link.setPaused(true);
        builder.display_link = Some(display_link);
        builder.track(NativeOwnerKind::DisplayLink);

        let window = builder
            .window
            .clone()
            .ok_or_else(|| native_unavailable(SurfaceStage::Window))?;
        let initial_configuration = SurfaceConfiguration::from_extent(
            extent,
            window
                .screen()
                .map_or(0, |screen| Retained::as_ptr(&screen) as usize),
            false,
        );
        let backend = take_owner(&mut builder.backend, SurfaceStage::Renderer)?;
        let driver = Rc::new(RefCell::new(PresentationDriver::new(
            backend,
            initial_configuration,
            Arc::clone(&builder.lifecycle),
        )?));
        builder.driver = Some(Rc::clone(&driver));
        let delegate = DisplayLinkDelegate::new(
            main_thread,
            DisplayLinkDelegateIvars {
                lifecycle: Arc::clone(&builder.lifecycle),
                window_close_started: Arc::clone(&builder.window_close_started),
                callback_count: Arc::clone(&builder.callback_count),
                rejected_callback_count: Arc::clone(&builder.rejected_callback_count),
                #[cfg(alpine_native_validation)]
                pause_confirmation: Arc::new(PauseConfirmationCounters::default()),
                counters: Arc::clone(&builder.counters),
                driver: Some(driver),
                application: builder
                    .application
                    .as_ref()
                    .ok_or_else(|| native_unavailable(SurfaceStage::MainThread))?
                    .clone(),
                window: builder.window.clone(),
                view: builder.view.clone(),
                layer: builder.layer.clone(),
                display_link: builder.display_link.clone(),
                event_handler: RefCell::new(None),
                event_sequence: Cell::new(0),
                #[cfg(alpine_native_validation)]
                validation_pasteboard: RefCell::new(None),
                #[cfg(alpine_native_validation)]
                clipboard_fault: Cell::new(None),
                #[cfg(alpine_native_validation)]
                validation_probe: builder.validation_probe.clone(),
            },
        );
        builder.delegate = Some(delegate);
        builder.track(NativeOwnerKind::Delegate);
        let delegate = builder
            .delegate
            .as_ref()
            .ok_or_else(|| native_unavailable(SurfaceStage::DisplayLink))?;
        let view = builder
            .view
            .as_ref()
            .ok_or_else(|| native_unavailable(SurfaceStage::View))?;
        view.install_accessibility_delegate(delegate);
        let display_link = builder
            .display_link
            .as_ref()
            .ok_or_else(|| native_unavailable(SurfaceStage::DisplayLink))?;
        display_link.setDelegate(Some(ProtocolObject::from_ref(&**delegate)));
        window.setDelegate(Some(ProtocolObject::from_ref(&**delegate)));
        control.checkpoint(SurfaceStage::DisplayLink)?;

        let run_loop = NSRunLoop::mainRunLoop();
        let display_link = builder
            .display_link
            .as_ref()
            .ok_or_else(|| native_unavailable(SurfaceStage::DisplayLink))?;
        // SAFETY: Construction is admitted only with MainThreadMarker, this is
        // the process main run loop, and the common mode object is static for
        // the process lifetime. Drop invalidates the link before owners release.
        unsafe { display_link.addToRunLoop_forMode(&run_loop, NSRunLoopCommonModes) };
        builder.record_run_loop_registration();
        control.checkpoint(SurfaceStage::RunLoop)?;

        builder.finish()
    }

    pub(crate) fn show(&self) -> Result<(), SurfaceError> {
        if !self.application.isRunning() {
            let _ = self
                .application
                .setActivationPolicy(NSApplicationActivationPolicy::Regular);
            self.application.finishLaunching();
        }
        self.window.makeKeyAndOrderFront(None);
        #[allow(
            deprecated,
            reason = "the initial standalone surface must activate an unbundled Rust executable"
        )]
        self.application.activateIgnoringOtherApps(true);
        self.delegate.synchronize_native_configuration()
    }

    fn activate_input_responder(&self) -> Result<(), SurfaceError> {
        self.window.setAcceptsMouseMovedEvents(true);
        if self.window.makeFirstResponder(Some(&self.view)) {
            Ok(())
        } else {
            Err(SurfaceError::InputResponderRejected)
        }
    }

    pub(crate) fn run(&self) -> Result<(), SurfaceError> {
        let _main_thread = MainThreadMarker::new().ok_or(SurfaceError::NativeUnavailable {
            stage: SurfaceStage::MainThread,
        })?;

        if !matches!(
            surface_lifecycle(self.lifecycle.load(Ordering::Acquire)),
            SurfaceLifecycle::Live,
        ) {
            return Err(SurfaceError::RunLoopNotRunnable {
                lifecycle: surface_lifecycle(self.lifecycle.load(Ordering::Acquire)),
            });
        }

        self.application.run();

        if matches!(
            surface_lifecycle(self.lifecycle.load(Ordering::Acquire)),
            SurfaceLifecycle::Live,
        ) {
            return Err(SurfaceError::UnexpectedRunLoopExit {
                lifecycle: SurfaceLifecycle::Live,
            });
        }

        match self.take_error()? {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    pub(crate) fn run_with_event_handler<F>(&self, handler: F) -> Result<(), SurfaceError>
    where
        F: FnMut(SurfaceEvent) -> SurfaceResponse + 'static,
    {
        self.activate_input_responder()?;
        self.delegate.install_event_handler(handler)?;
        let delegate = self.delegate.clone();
        if !self.view.install_input_handler(Box::new(move |event| {
            delegate.dispatch_native_input_event(event);
        })) {
            self.delegate.clear_event_handler();
            return Err(SurfaceError::invariant(SurfaceOperation::RunLoop));
        }
        let (input_epoch, focused) = self.view.input_focus_state();
        if input_epoch != InputEpoch::INITIAL || !focused {
            let _close = self.delegate.dispatch_surface_event(SurfaceEvent::Focus {
                timestamp: self.delegate.next_event_timestamp(),
                input_epoch,
                focused,
            })?;
        }
        let wake_result = self.delegate.dispatch_surface_event(SurfaceEvent::Wake {
            timestamp: self.delegate.next_event_timestamp(),
        });
        let run_result = wake_result.and_then(|_| self.run());
        self.wake_bridge.revoke();
        self.view.revoke_accessibility();
        self.view.clear_input_handler();
        self.delegate.clear_event_handler();
        resolve_input_dispatch(run_result, self.view.take_input_dispatch_failure())
    }

    pub(crate) fn waker(&self) -> SurfaceWaker {
        self.wake_bridge.waker()
    }

    #[cfg(alpine_native_validation)]
    pub(crate) fn replay_surface_events<F>(
        &self,
        events: &[SurfaceEvent],
        handler: F,
    ) -> Result<(), SurfaceError>
    where
        F: FnMut(SurfaceEvent) -> SurfaceResponse + 'static,
    {
        self.delegate.install_event_handler(handler)?;
        let result = events
            .iter()
            .cloned()
            .try_for_each(|event| self.delegate.dispatch_surface_event(event).map(|_| ()));
        self.delegate.clear_event_handler();
        result
    }

    #[cfg(alpine_native_validation)]
    pub(crate) fn replay_native_accessibility_path<F>(
        &self,
        handler: F,
    ) -> Result<crate::native_validation::NativeAccessibilityEvidence, SurfaceError>
    where
        F: FnMut(SurfaceEvent) -> SurfaceResponse + 'static,
    {
        self.delegate.install_event_handler(handler)?;
        let roots: Retained<NSArray<NativeAccessibilityElement>> =
            unsafe { msg_send![&*self.view, accessibilityChildren] };
        let result = if roots.firstObject().is_none() {
            Err(SurfaceError::validation(SurfaceOperation::Validation))
        } else {
            self.delegate
                .dispatch_surface_event(SurfaceEvent::Wake {
                    timestamp: self.delegate.next_event_timestamp(),
                })
                .and_then(|_| NativeAccessibilityAdapter::validate_view(&self.view))
        };
        self.delegate.clear_event_handler();
        result
    }

    #[cfg(alpine_native_validation)]
    pub(crate) fn inspect_native_accessibility_tree<F>(
        &self,
        handler: F,
    ) -> Result<crate::native_validation::NativeAccessibilityTreeEvidence, SurfaceError>
    where
        F: FnMut(SurfaceEvent) -> SurfaceResponse + 'static,
    {
        self.delegate.install_event_handler(handler)?;
        let result = NativeAccessibilityAdapter::inspect_view(&self.view);
        self.delegate.clear_event_handler();
        result
    }

    #[cfg(alpine_native_validation)]
    pub(crate) fn activate_named_native_accessibility_node<F>(
        &self,
        role: crate::AccessibilityRole,
        label: &str,
        handler: F,
    ) -> Result<crate::native_validation::NativeAccessibilityActivationEvidence, SurfaceError>
    where
        F: FnMut(SurfaceEvent) -> SurfaceResponse + 'static,
    {
        self.delegate.install_event_handler(handler)?;
        let result = NativeAccessibilityAdapter::activate_named_view(&self.view, role, label);
        self.delegate.clear_event_handler();
        result
    }

    #[cfg(alpine_native_validation)]
    pub(crate) fn replay_native_input_path<F>(&self, handler: F) -> Result<(), SurfaceError>
    where
        F: FnMut(SurfaceEvent) -> SurfaceResponse + 'static,
    {
        self.window.setAcceptsMouseMovedEvents(false);
        self.activate_input_responder()?;
        if !self.window.acceptsMouseMovedEvents() {
            return Err(SurfaceError::validation(SurfaceOperation::Validation));
        }
        self.delegate.install_event_handler(handler)?;
        let delegate = self.delegate.clone();
        if !self.view.install_input_handler(Box::new(move |event| {
            delegate.dispatch_native_input_event(event);
        })) {
            self.delegate.clear_event_handler();
            return Err(SurfaceError::validation(SurfaceOperation::Validation));
        }

        let characters = NSString::from_str("A");
        let ignoring_modifiers = NSString::from_str("a");
        let event = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
            NSEventType::KeyDown,
            NSPoint::new(4.0, 4.0),
            NSEventModifierFlags::Shift,
            0.0,
            self.window.windowNumber(),
            None,
            &characters,
            &ignoring_modifiers,
            false,
            0,
        )
        .ok_or_else(|| native_unavailable(SurfaceStage::View));
        let replay_result = event.and_then(|event| {
            self.view.keyDown(&event);
            let marked = NSString::from_str("漢字");
            // SAFETY: These messages target selectors implemented by
            // SurfaceView's NSTextInputClient conformance. Every object and
            // range remains valid for each synchronous call.
            unsafe {
                let _: () = msg_send![
                    &*self.view,
                    setMarkedText: &*marked,
                    selectedRange: NSRange::new(1, 1),
                    replacementRange: NSRange::new(usize::MAX, 0)
                ];
            }
            let has_marked: bool = unsafe { msg_send![&*self.view, hasMarkedText] };
            let marked_range: NSRange = unsafe { msg_send![&*self.view, markedRange] };
            if !has_marked {
                return Err(SurfaceError::validation(SurfaceOperation::Validation));
            }
            if marked_range != NSRange::new(0, 2) {
                return Err(SurfaceError::validation(SurfaceOperation::Validation));
            }
            unsafe {
                let _: () = msg_send![&*self.view, unmarkText];
            }
            let has_marked: bool = unsafe { msg_send![&*self.view, hasMarkedText] };
            let marked_range: NSRange = unsafe { msg_send![&*self.view, markedRange] };
            if has_marked {
                return Err(SurfaceError::validation(SurfaceOperation::Validation));
            }
            if marked_range != NSRange::new(NSUInteger::MAX, 0) {
                return Err(SurfaceError::validation(SurfaceOperation::Validation));
            }

            let stale_epoch = self.view.ivars().input_epoch.get();
            let marked = NSString::from_str("かな");
            unsafe {
                let _: () = msg_send![
                    &*self.view,
                    setMarkedText: &*marked,
                    selectedRange: NSRange::new(1, 0),
                    replacementRange: NSRange::new(usize::MAX, 0)
                ];
            }
            self.delegate.publish_input_focus(false);
            let rejected_before = self.view.rejected_ime_callbacks();
            self.view
                .emit_ime_at_epoch(stale_epoch, ImeEvent::Committed("stale".into()));
            if self.view.rejected_ime_callbacks() != rejected_before.saturating_add(1) {
                return Err(SurfaceError::validation(SurfaceOperation::Validation));
            }
            self.delegate.publish_input_focus(true);

            let current_epoch = self.view.ivars().input_epoch.get();
            let rejected_before = self.view.rejected_ime_callbacks();
            self.view
                .emit_ime_at_epoch(stale_epoch, ImeEvent::Committed("active-stale".into()));
            if self.view.rejected_ime_callbacks() != rejected_before.saturating_add(1) {
                return Err(SurfaceError::validation(SurfaceOperation::Validation));
            }

            self.view
                .set_input_focus_state_for_validation(current_epoch, false);
            let rejected_before = self.view.rejected_ime_callbacks();
            self.view
                .emit_ime_at_epoch(current_epoch, ImeEvent::Committed("inactive-current".into()));
            if self.view.rejected_ime_callbacks() != rejected_before.saturating_add(1) {
                return Err(SurfaceError::validation(SurfaceOperation::Validation));
            }

            let rejected_before = self.view.rejected_ime_callbacks();
            let blocked = NSString::from_str("blocked");
            unsafe {
                let _: () = msg_send![
                    &*self.view,
                    setMarkedText: &*blocked,
                    selectedRange: NSRange::new(0, 0),
                    replacementRange: NSRange::new(usize::MAX, 0)
                ];
            }
            if self.view.rejected_ime_callbacks() != rejected_before.saturating_add(1) {
                return Err(SurfaceError::validation(SurfaceOperation::Validation));
            }
            if self.view.has_marked_text_value() {
                return Err(SurfaceError::validation(SurfaceOperation::Validation));
            }
            self.view
                .set_input_focus_state_for_validation(current_epoch, true);

            let pointer = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
                NSEventType::LeftMouseDown,
                NSPoint::new(12.0, 18.0),
                NSEventModifierFlags::Command,
                0.0,
                self.window.windowNumber(),
                None,
                1,
                1,
                1.0,
            )
            .ok_or_else(|| native_unavailable(SurfaceStage::View))?;
            self.view.mouseDown(&pointer);

            let cg_scroll = CGEvent::new_scroll_wheel_event2(
                None,
                CGScrollEventUnit::Line,
                2,
                -3,
                4,
                0,
            )
            .ok_or_else(|| native_unavailable(SurfaceStage::View))?;
            CGEvent::set_flags(Some(&cg_scroll), CGEventFlags::empty());
            let scroll = NSEvent::eventWithCGEvent(&cg_scroll)
                .ok_or_else(|| native_unavailable(SurfaceStage::View))?;
            self.view.scrollWheel(&scroll);
            Ok(())
        });

        self.view.clear_input_handler();
        self.delegate.clear_event_handler();
        resolve_input_dispatch(replay_result, self.view.take_input_dispatch_failure())?;
        self.view.emit(NativeInputEvent::Ime {
            input_epoch: self.view.ivars().input_epoch.get(),
            event: ImeEvent::Cancelled,
        });
        if !self.view.take_input_dispatch_failure() {
            return Err(SurfaceError::validation(SurfaceOperation::Validation));
        }
        if self.view.take_input_dispatch_failure() {
            return Err(SurfaceError::validation(SurfaceOperation::Validation));
        }
        Ok(())
    }

    #[cfg(alpine_native_validation)]
    pub(crate) fn commit_native_text<F>(&self, text: &str, handler: F) -> Result<(), SurfaceError>
    where
        F: FnMut(SurfaceEvent) -> SurfaceResponse + 'static,
    {
        self.delegate.install_event_handler(handler)?;
        let delegate = self.delegate.clone();
        if !self.view.install_input_handler(Box::new(move |event| {
            delegate.dispatch_native_input_event(event);
        })) {
            self.delegate.clear_event_handler();
            return Err(SurfaceError::validation(SurfaceOperation::Input));
        }
        if let Err(error) = self.activate_input_responder() {
            self.view.detach_input_handler_for_validation();
            self.delegate.clear_event_handler();
            return Err(error);
        }
        let (input_epoch, focused) = self.view.input_focus_state();
        if let Err(error) = self.delegate.dispatch_surface_event(SurfaceEvent::Focus {
            timestamp: self.delegate.next_event_timestamp(),
            input_epoch,
            focused,
        }) {
            self.view.detach_input_handler_for_validation();
            self.delegate.clear_event_handler();
            return Err(error);
        }

        let rejected_before = self.view.rejected_ime_callbacks();
        let committed = NSString::from_str(text);
        // SAFETY: SurfaceView implements NSTextInputClient, the retained
        // NSString remains live for the synchronous selector, and no native
        // object crosses the validation boundary.
        unsafe {
            let _: () = msg_send![
                &*self.view,
                insertText: &*committed,
                replacementRange: NSRange::new(NSUInteger::MAX, 0)
            ];
        }
        let replay_result = if self.view.rejected_ime_callbacks() == rejected_before {
            Ok(())
        } else {
            Err(SurfaceError::validation(SurfaceOperation::Input))
        };

        self.view.detach_input_handler_for_validation();
        self.delegate.clear_event_handler();
        resolve_input_dispatch(replay_result, self.view.take_input_dispatch_failure())
    }

    #[cfg(alpine_native_validation)]
    pub(crate) fn set_input_focus_state_for_validation(
        &self,
        input_epoch: InputEpoch,
        focused: bool,
    ) {
        self.view
            .set_input_focus_state_for_validation(input_epoch, focused);
    }

    #[cfg(alpine_native_validation)]
    pub(crate) fn replay_callback_surface_events<F>(
        &self,
        events: &[SurfaceEvent],
        handler: F,
    ) -> Result<(), SurfaceError>
    where
        F: FnMut(SurfaceEvent) -> SurfaceResponse + 'static,
    {
        self.delegate.install_event_handler(handler)?;
        events
            .iter()
            .cloned()
            .for_each(|event| self.delegate.dispatch_callback_event(event));
        self.delegate.clear_event_handler();
        Ok(())
    }

    #[cfg(alpine_native_validation)]
    pub(crate) fn replay_native_clipboard_operation<F>(
        &self,
        operation: ClipboardOperation,
        handler: F,
    ) -> Result<(), SurfaceError>
    where
        F: FnMut(SurfaceEvent) -> SurfaceResponse + 'static,
    {
        self.delegate.install_event_handler(handler)?;
        let logical_key: Box<str> = match operation {
            ClipboardOperation::Copy => "c".into(),
            ClipboardOperation::Cut => "x".into(),
            ClipboardOperation::Paste => "v".into(),
        };
        let result = self.delegate.try_dispatch_native_input_event(
            NativeInputEvent::Keyboard {
                state: KeyState::Down,
                physical_key: 0,
                logical_key,
                modifiers: Modifiers::from_bits(Modifiers::COMMAND),
                repeat: false,
            },
            Instant::now(),
        );
        self.delegate.clear_event_handler();
        result
    }

    #[cfg(alpine_native_validation)]
    pub(crate) fn inject_clipboard_error(&self, error: ClipboardError) {
        self.delegate.ivars().clipboard_fault.set(Some(error));
    }

    #[cfg(alpine_native_validation)]
    pub(crate) fn replay_close(
        &self,
        scenario: crate::native_validation::CloseReplayScenario,
    ) -> Result<bool, SurfaceError> {
        use crate::native_validation::CloseReplayScenario;

        match scenario {
            CloseReplayScenario::MissingHandler => self.window.performClose(None),
            CloseReplayScenario::ReentrantHandler => {
                let _borrow = self
                    .delegate
                    .ivars()
                    .event_handler
                    .try_borrow_mut()
                    .map_err(|_| SurfaceError::validation(SurfaceOperation::Validation))?;
                self.window.performClose(None);
            }
            CloseReplayScenario::Cancel | CloseReplayScenario::Allow => {
                let close = if scenario == CloseReplayScenario::Cancel {
                    CloseDisposition::Cancel
                } else {
                    CloseDisposition::Allow
                };
                self.delegate
                    .install_event_handler(move |_| SurfaceResponse::new(None, None, close))?;
                self.window.performClose(None);
                self.delegate.clear_event_handler();
            }
        }
        Ok(self.window_close_started.load(Ordering::Acquire))
    }

    #[cfg(alpine_native_validation)]
    pub(crate) fn replay_close_with_handler<F>(&self, handler: F) -> Result<bool, SurfaceError>
    where
        F: FnMut(SurfaceEvent) -> SurfaceResponse + 'static,
    {
        self.delegate.install_event_handler(handler)?;
        self.window.performClose(None);
        self.delegate.clear_event_handler();
        Ok(self.window_close_started.load(Ordering::Acquire))
    }

    pub(crate) fn request_frame(
        &self,
        scene: Scene,
        clear: LinearRgba,
    ) -> Result<alpine_platform::PresentationRevision, SurfaceError> {
        let (revision, directive) = self
            .driver
            .try_borrow_mut()
            .map_err(|_| SurfaceError::owner_conflict(SurfaceOperation::Presentation))?
            .request_frame(scene, clear)?;
        apply_display_link_directive(&self.display_link, directive);
        Ok(revision)
    }

    #[cfg(alpine_native_validation)]
    pub(crate) fn run_until_frame_terminal(&self, timeout: Duration) {
        let counters = Arc::clone(&self.counters);
        let initial_presented = counters.presented.load(Ordering::Acquire);
        let initial_failed = counters.failed.load(Ordering::Acquire);
        let initial_cancelled = counters.cancelled.load(Ordering::Acquire);
        let deadline = Instant::now() + timeout;
        if self.validation_event_loop_started.replace(true) {
            while {
                let terminal_observed = counters.presented.load(Ordering::Acquire)
                    > initial_presented
                    || counters.failed.load(Ordering::Acquire) > initial_failed
                    || counters.cancelled.load(Ordering::Acquire) > initial_cancelled;
                let frame_slots_drained = self
                    .driver
                    .try_borrow()
                    .is_ok_and(|driver| driver.frame_slots.snapshot().occupied_slots() == 0);
                (!terminal_observed || !frame_slots_drained) && Instant::now() < deadline
            } {
                NSRunLoop::mainRunLoop().runUntilDate(&NSDate::dateWithTimeIntervalSinceNow(0.005));
            }
            return;
        }
        let driver = Rc::downgrade(&self.driver);
        let timer_block: RcBlock<dyn Fn(NonNull<NSTimer>)> =
            RcBlock::new(move |timer: NonNull<NSTimer>| {
                let terminal_observed = counters.presented.load(Ordering::Acquire)
                    > initial_presented
                    || counters.failed.load(Ordering::Acquire) > initial_failed
                    || counters.cancelled.load(Ordering::Acquire) > initial_cancelled;
                let frame_slots_drained = driver.upgrade().is_some_and(|driver| {
                    driver
                        .try_borrow()
                        .is_ok_and(|driver| driver.frame_slots.snapshot().occupied_slots() == 0)
                });
                let terminal =
                    (terminal_observed && frame_slots_drained) || Instant::now() >= deadline;
                if terminal {
                    // SAFETY: Foundation supplies a valid borrowed timer for
                    // the complete callback, and the reference does not escape.
                    unsafe { timer.as_ref() }.invalidate();
                    if let Some(main_thread) = MainThreadMarker::new() {
                        stop_validation_event_loop(&NSApplication::sharedApplication(main_thread));
                    }
                }
            });
        // SAFETY: The captured values are thread-safe and the timer block is
        // scheduled on the process main run loop. Foundation copies it for the
        // timer lifetime and supplies a valid NSTimer argument on each call.
        let timer = unsafe {
            NSTimer::scheduledTimerWithTimeInterval_repeats_block(0.005, true, &timer_block)
        };
        self.application.run();
        timer.invalidate();
    }

    #[cfg(alpine_native_validation)]
    pub(crate) fn run_until_frame_terminal_with_handler<F>(
        &self,
        timeout: Duration,
        handler: F,
    ) -> Result<(), SurfaceError>
    where
        F: FnMut(SurfaceEvent) -> SurfaceResponse + 'static,
    {
        self.delegate.install_event_handler(handler)?;
        self.run_until_frame_terminal(timeout);
        self.delegate.clear_event_handler();
        Ok(())
    }

    #[cfg(alpine_native_validation)]
    pub(crate) fn arm_run_timeout(
        &self,
        timeout: Duration,
        expired: Arc<std::sync::atomic::AtomicBool>,
        cancelled: Arc<std::sync::atomic::AtomicBool>,
    ) {
        let timer_block: RcBlock<dyn Fn(NonNull<NSTimer>)> =
            RcBlock::new(move |timer: NonNull<NSTimer>| {
                // SAFETY: Foundation supplies a valid borrowed timer for the
                // complete callback, and the reference does not escape.
                unsafe { timer.as_ref() }.invalidate();
                if !cancelled.swap(true, Ordering::AcqRel) {
                    expired.store(true, Ordering::Release);
                    if let Some(main_thread) = MainThreadMarker::new() {
                        stop_validation_event_loop(&NSApplication::sharedApplication(main_thread));
                    }
                }
            });
        // SAFETY: The block is scheduled on the process main run loop,
        // Foundation copies it for the timer lifetime, and the callback
        // receives a valid NSTimer. The scheduled timer retains itself.
        let _timer = unsafe {
            NSTimer::scheduledTimerWithTimeInterval_repeats_block(
                timeout.as_secs_f64(),
                false,
                &timer_block,
            )
        };
    }

    #[cfg(alpine_native_validation)]
    pub(crate) fn arm_window_close(&self, delay: Duration) {
        schedule_validation_window_close(&self.window, delay);
    }

    #[cfg(alpine_native_validation)]
    pub(crate) fn arm_user_window_close(&self, delay: Duration) {
        schedule_validation_user_window_close(&self.window, &self.lifecycle, &self.counters, delay);
    }

    #[cfg(alpine_native_validation)]
    pub(crate) fn arm_programmatic_window_close(&self, delay: Duration) {
        schedule_validation_programmatic_window_close(
            &self.window,
            &self.delegate,
            &self.driver,
            &self.lifecycle,
            &self.counters,
            delay,
        );
    }

    #[cfg(alpine_native_validation)]
    pub(crate) fn revoke_waker_for_validation(&self) {
        self.wake_bridge.revoke();
    }

    #[allow(
        clippy::too_many_lines,
        reason = "the handle-free snapshot copies independent native evidence without abstraction"
    )]
    pub(crate) fn snapshot(&self) -> SurfaceSnapshot {
        let driver = self.driver.try_borrow();
        let (surface_epoch, sized, presentation_visible) =
            driver.as_ref().map_or((0, false, false), |driver| {
                (
                    driver.state.surface_epoch().get(),
                    driver.state.is_sized(),
                    driver.state.is_visible(),
                )
            });
        let drawable_size = self.layer.drawableSize();
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "Alpine installs validated integral u32 drawable dimensions on its private layer"
        )]
        let (physical_width, physical_height) =
            (drawable_size.width as u32, drawable_size.height as u32);
        let (
            allocated_bytes,
            peak_retained_bytes,
            current_retained_bytes,
            current_upload_bytes,
            peak_upload_bytes,
            occupied_frame_slots,
            submitted_frame_slots,
            peak_occupied_frame_slots,
            frame_slot_saturation_count,
            last_terminal,
            last_superseded,
            last_cancelled,
            last_pending_cancellation,
        ) = driver.as_ref().map_or(
            (0, 0, 0, 0, 0, 0, 0, 0, 0, None, None, None, None),
            |driver| {
                let accounting = driver.backend.accounting();
                let presentation =
                    alpine_metal::platform_spi::presentation_snapshot(&driver.backend);
                let slots = driver.frame_slots.snapshot();
                (
                    accounting.allocated_bytes(),
                    accounting.peak_retained_bytes(),
                    accounting.current_retained_bytes(),
                    presentation.current_upload_bytes(),
                    presentation.peak_upload_bytes(),
                    slots.occupied_slots(),
                    slots.submitted_slots(),
                    slots.peak_occupied_slots(),
                    slots.saturation_count(),
                    driver.last_terminal,
                    driver.last_superseded,
                    driver.last_cancelled,
                    driver.last_pending_cancellation,
                )
            },
        );
        SurfaceSnapshot {
            physical_width,
            physical_height,
            surface_epoch,
            sized,
            presentation_visible,
            sdr_color_contract: layer_sdr_color_contract(&self.layer),
            extended_dynamic_range: self.layer.wantsExtendedDynamicRangeContent(),
            framebuffer_only: self.layer.framebufferOnly(),
            display_sync_enabled: self.layer.displaySyncEnabled(),
            allows_next_drawable_timeout: self.layer.allowsNextDrawableTimeout(),
            maximum_drawable_count: u8::try_from(self.layer.maximumDrawableCount()).unwrap_or(0),
            regular_activation_policy: self.application.activationPolicy()
                == NSApplicationActivationPolicy::Regular,
            display_link_paused: self.display_link.isPaused(),
            #[cfg(alpine_native_validation)]
            pause_confirmation_count: self
                .delegate
                .ivars()
                .pause_confirmation
                .observed
                .load(Ordering::Acquire),
            visible: self.window.isVisible(),
            callback_count: self.callback_count.load(Ordering::Acquire),
            rejected_callback_count: self.rejected_callback_count.load(Ordering::Acquire),
            submission_count: self.counters.submissions.load(Ordering::Acquire),
            direct_present_count: self.counters.direct_presents.load(Ordering::Acquire),
            installed_presented_handler_count: self
                .counters
                .installed_presented_handlers
                .load(Ordering::Acquire),
            presented_count: self.counters.presented.load(Ordering::Acquire),
            qualified_presented_count: self.counters.qualified_presented.load(Ordering::Acquire),
            superseded_count: self.counters.superseded.load(Ordering::Acquire),
            cancelled_count: self.counters.cancelled.load(Ordering::Acquire),
            pending_cancellation_count: self.counters.pending_cancellations.load(Ordering::Acquire),
            last_presented_time_bits: self
                .counters
                .last_presented_time_bits
                .load(Ordering::Acquire),
            skipped_count: self.counters.skipped.load(Ordering::Acquire),
            failed_count: self.counters.failed.load(Ordering::Acquire),
            allocated_bytes,
            peak_retained_bytes,
            current_retained_bytes,
            current_upload_bytes,
            peak_upload_bytes,
            frame_slot_capacity: 3,
            occupied_frame_slots,
            submitted_frame_slots,
            peak_occupied_frame_slots,
            frame_slot_saturation_count,
            last_terminal,
            last_superseded,
            last_cancelled,
            last_pending_cancellation,
        }
    }

    #[cfg(alpine_native_validation)]
    pub(crate) fn pause_confirmation_evidence(
        &self,
    ) -> crate::native_validation::PauseConfirmationEvidence {
        let counters = &self.delegate.ivars().pause_confirmation;
        crate::native_validation::PauseConfirmationEvidence::new(
            counters.requested.load(Ordering::Acquire),
            counters.enqueued.load(Ordering::Acquire),
            counters.executed.load(Ordering::Acquire),
            counters.eligible.load(Ordering::Acquire),
            counters.observed.load(Ordering::Acquire),
            counters.callback_observations.load(Ordering::Acquire),
            pause_directive_evidence(counters.last_directive.load(Ordering::Acquire)),
            pause_portable_state_evidence(counters.last_portable_state.load(Ordering::Acquire)),
            counters.last_native_paused_before.load(Ordering::Acquire),
            counters.last_native_paused_after.load(Ordering::Acquire),
            counters.last_pending.load(Ordering::Acquire),
            counters.last_active.load(Ordering::Acquire),
        )
    }

    pub(crate) fn take_error(&self) -> Result<Option<SurfaceError>, SurfaceError> {
        Ok(self
            .driver
            .try_borrow_mut()
            .map_err(|_| SurfaceError::owner_conflict(SurfaceOperation::Presentation))?
            .take_error())
    }

    #[cfg(alpine_native_validation)]
    pub(crate) fn inject_driver_error(&self, error: SurfaceError) {
        if let Ok(mut driver) = self.driver.try_borrow_mut() {
            driver.record_error(error);
        }
    }

    #[cfg(alpine_native_validation)]
    pub(crate) fn inject_post_commit_observation(
        &self,
        display_identity: Option<usize>,
        presented_time: f64,
    ) -> Result<(), SurfaceError> {
        if !presented_time.is_finite() || presented_time < 0.0 {
            return Err(SurfaceError::validation(SurfaceOperation::Validation));
        }
        self.driver
            .try_borrow_mut()
            .map_err(|_| SurfaceError::validation(SurfaceOperation::Validation))?
            .inject_post_commit_observation(display_identity, presented_time);
        Ok(())
    }

    #[cfg(alpine_native_validation)]
    pub(crate) fn inject_post_commit_close(&self) {
        if let Ok(mut driver) = self.driver.try_borrow_mut() {
            driver.inject_post_commit_close();
        }
    }

    #[cfg(alpine_native_validation)]
    pub(crate) fn inject_late_callback(&self) {
        let _ = admit_callback(
            &self.lifecycle,
            &self.callback_count,
            &self.rejected_callback_count,
        );
    }

    #[cfg(alpine_native_validation)]
    pub(crate) fn inject_configuration_callback(&self) -> bool {
        self.delegate
            .synchronize_native_configuration_from_callback()
    }

    #[cfg(alpine_native_validation)]
    pub(crate) fn inject_surface_configuration(
        &self,
        logical_width: f64,
        logical_height: f64,
        scale: f64,
        display_identity: usize,
        visible: bool,
    ) -> Result<(), SurfaceError> {
        let configuration = match SurfaceConfiguration::from_native(
            logical_width,
            logical_height,
            scale,
            display_identity,
            visible,
        ) {
            Ok(configuration) => configuration,
            Err(error) => {
                let directive = self
                    .driver
                    .try_borrow_mut()
                    .map_err(|_| SurfaceError::validation(SurfaceOperation::Validation))?
                    .reject_configuration(error.clone())?;
                apply_display_link_directive(&self.display_link, directive);
                return Err(error);
            }
        };
        apply_layer_configuration(&self.layer, &self.view, configuration);
        let directive = self
            .driver
            .try_borrow_mut()
            .map_err(|_| SurfaceError::validation(SurfaceOperation::Validation))?
            .apply_configuration(configuration)?;
        apply_display_link_directive(&self.display_link, directive);
        Ok(())
    }

    #[cfg(alpine_native_validation)]
    pub(crate) fn resize_content(&self, logical_width: f64, logical_height: f64) {
        self.window
            .setContentSize(NSSize::new(logical_width, logical_height));
    }

    #[cfg(alpine_native_validation)]
    pub(crate) fn validation_screen_configurations(
        &self,
    ) -> Vec<crate::native_validation::ValidationScreenConfiguration> {
        let Some(mtm) = MainThreadMarker::new() else {
            return Vec::new();
        };
        NSScreen::screens(mtm)
            .iter()
            .enumerate()
            .map(|(index, screen)| validation_screen_configuration(index, &screen))
            .collect()
    }

    #[cfg(alpine_native_validation)]
    pub(crate) fn move_window_to_screen(
        &self,
        index: usize,
    ) -> Result<crate::native_validation::ValidationScreenConfiguration, SurfaceError> {
        let mtm = MainThreadMarker::new()
            .ok_or(SurfaceError::validation(SurfaceOperation::Validation))?;
        let screens = NSScreen::screens(mtm);
        let screen = screens
            .iter()
            .nth(index)
            .ok_or(SurfaceError::validation(SurfaceOperation::Validation))?;
        self.window.setFrameOrigin(centered_window_origin(
            self.window.frame(),
            screen.visibleFrame(),
        ));
        if !self
            .delegate
            .synchronize_native_configuration_from_callback()
        {
            return Err(SurfaceError::validation(SurfaceOperation::Validation));
        }
        let current = self
            .window
            .screen()
            .ok_or(SurfaceError::validation(SurfaceOperation::Validation))?;
        let current_identity = Retained::as_ptr(&current) as usize;
        let current_index = screens
            .iter()
            .position(|candidate| Retained::as_ptr(&candidate) as usize == current_identity)
            .ok_or(SurfaceError::validation(SurfaceOperation::Validation))?;
        if current_index != index {
            return Err(SurfaceError::validation(SurfaceOperation::Validation));
        }
        Ok(validation_screen_configuration(current_index, &current))
    }

    #[cfg(alpine_native_validation)]
    pub(crate) fn close_window(&self) {
        self.window.close();
    }

    pub(crate) fn observer(&self) -> SurfaceObserver {
        SurfaceObserver::new(
            Arc::clone(&self.lifecycle),
            Arc::clone(&self.callback_count),
            Arc::clone(&self.rejected_callback_count),
        )
    }

    #[cfg(alpine_native_validation)]
    pub(crate) fn close_with_owner_evidence(
        self,
    ) -> Result<crate::native_validation::NativeOwnerEvidence, SurfaceError> {
        let probe = self
            .validation_probe
            .clone()
            .ok_or(SurfaceError::validation(SurfaceOperation::Validation))?;
        drop(self);
        Ok(probe.evidence())
    }
}

#[cfg(alpine_native_validation)]
fn stop_validation_event_loop(application: &NSApplication) {
    stop_event_loop(application);
}

fn stop_event_loop(application: &NSApplication) {
    application.stop(None);
    #[cfg(alpine_native_validation)]
    if let Some(event) = NSEvent::otherEventWithType_location_modifierFlags_timestamp_windowNumber_context_subtype_data1_data2(
        NSEventType::ApplicationDefined,
        NSPoint::new(0.0, 0.0),
        NSEventModifierFlags::empty(),
        0.0,
        0,
        None,
        0,
        0,
        0,
    ) {
        application.postEvent_atStart(&event, true);
    }
}

#[allow(clippy::match_like_matches_macro)]
fn surface_lifecycle(state: u8) -> SurfaceLifecycle {
    match state {
        SURFACE_LIVE => SurfaceLifecycle::Live,
        SURFACE_CLOSING => SurfaceLifecycle::Closing,
        _ => SurfaceLifecycle::Closed,
    }
}

fn require_device(device: Option<Device>) -> Result<Device, SurfaceError> {
    device.ok_or(SurfaceError::NativeUnavailable {
        stage: SurfaceStage::Device,
    })
}

impl Drop for NativeSurface {
    fn drop(&mut self) {
        self.wake_bridge.revoke();
        self.view.revoke_accessibility();
        let native_close_started = self.lifecycle.load(Ordering::Acquire) != SURFACE_LIVE;
        let must_close_window = !self.window_close_started.load(Ordering::Acquire);
        if !native_close_started {
            begin_close_observer_state(&self.lifecycle);
            self.display_link.setPaused(true);
            self.display_link.invalidate();
        }
        // A reentrant windowWillClose callback may be unable to borrow the
        // driver while it still revokes callback admission. Owner teardown is
        // therefore the final idempotent shutdown boundary in both paths.
        if let Ok(mut driver) = self.driver.try_borrow_mut() {
            driver.shutdown(&self.counters);
        }
        #[cfg(alpine_native_validation)]
        if let Some(probe) = &self.validation_probe {
            probe.record_link_invalidation();
        }
        self.display_link.setDelegate(None);
        #[cfg(alpine_native_validation)]
        if let Some(probe) = &self.validation_probe {
            probe.record_delegate_revocation();
        }
        self.window.setDelegate(None);
        #[cfg(alpine_native_validation)]
        if let Some(pasteboard) = self
            .delegate
            .ivars()
            .validation_pasteboard
            .borrow()
            .as_ref()
        {
            pasteboard.release();
        }
        if must_close_window {
            self.window.orderOut(None);
            self.window.close();
            #[cfg(alpine_native_validation)]
            if let Some(probe) = &self.validation_probe {
                probe.record_window_close();
            }
        }
        finish_close_observer_state(&self.lifecycle);
    }
}

fn native_unavailable(stage: SurfaceStage) -> SurfaceError {
    SurfaceError::NativeUnavailable { stage }
}

struct NativeSurfaceBuilder {
    callback_count: Arc<AtomicU64>,
    rejected_callback_count: Arc<AtomicU64>,
    counters: Arc<FrameCounters>,
    backend: Option<MetalBackend>,
    driver: Option<Rc<RefCell<PresentationDriver>>>,
    lifecycle: Arc<AtomicU8>,
    window_close_started: Arc<AtomicBool>,
    display_link: Option<Retained<CAMetalDisplayLink>>,
    delegate: Option<Retained<DisplayLinkDelegate>>,
    layer: Option<Retained<CAMetalLayer>>,
    color_space: Option<Retained<CGColorSpace>>,
    view: Option<Retained<SurfaceView>>,
    window: Option<Retained<NSWindow>>,
    device: Option<Device>,
    application: Option<Retained<NSApplication>>,
    completed: bool,
    #[cfg(alpine_native_validation)]
    validation_probe: Option<InitializationProbe>,
    #[cfg(alpine_native_validation)]
    validation_leases: Vec<InitializationLease>,
}

impl NativeSurfaceBuilder {
    fn new(
        lifecycle: Arc<AtomicU8>,
        callback_count: Arc<AtomicU64>,
        rejected_callback_count: Arc<AtomicU64>,
        #[cfg(alpine_native_validation)] validation_probe: Option<InitializationProbe>,
    ) -> Self {
        Self {
            callback_count,
            rejected_callback_count,
            counters: Arc::new(FrameCounters::default()),
            backend: None,
            driver: None,
            lifecycle,
            window_close_started: Arc::new(AtomicBool::new(false)),
            display_link: None,
            delegate: None,
            layer: None,
            color_space: None,
            view: None,
            window: None,
            device: None,
            application: None,
            completed: false,
            #[cfg(alpine_native_validation)]
            validation_probe,
            #[cfg(alpine_native_validation)]
            validation_leases: Vec::with_capacity(NATIVE_OWNER_KINDS),
        }
    }

    #[allow(
        clippy::unused_self,
        reason = "shipping builds erase validation leases while retaining stage calls"
    )]
    fn track(&mut self, kind: NativeOwnerKind) {
        #[cfg(alpine_native_validation)]
        if let Some(probe) = &self.validation_probe {
            self.validation_leases.push(probe.acquire(kind));
        }
        #[cfg(not(alpine_native_validation))]
        let _ = kind;
    }

    #[allow(
        clippy::unused_self,
        reason = "shipping builds erase validation counters while retaining the lifecycle call"
    )]
    fn record_run_loop_registration(&self) {
        #[cfg(alpine_native_validation)]
        if let Some(probe) = &self.validation_probe {
            probe.record_run_loop_registration();
        }
    }

    fn finish(mut self) -> Result<NativeSurface, SurfaceError> {
        let delegate = take_owner(&mut self.delegate, SurfaceStage::DisplayLink)?;
        let wake_bridge = NativeWakeBridge::new(&delegate, Arc::clone(&self.lifecycle));
        let surface = NativeSurface {
            callback_count: Arc::clone(&self.callback_count),
            rejected_callback_count: Arc::clone(&self.rejected_callback_count),
            counters: Arc::clone(&self.counters),
            driver: take_owner(&mut self.driver, SurfaceStage::Renderer)?,
            lifecycle: Arc::clone(&self.lifecycle),
            window_close_started: Arc::clone(&self.window_close_started),
            display_link: take_owner(&mut self.display_link, SurfaceStage::DisplayLink)?,
            delegate,
            wake_bridge,
            layer: take_owner(&mut self.layer, SurfaceStage::Layer)?,
            color_space: take_owner(&mut self.color_space, SurfaceStage::ColorSpace)?,
            view: take_owner(&mut self.view, SurfaceStage::View)?,
            window: take_owner(&mut self.window, SurfaceStage::Window)?,
            device: take_owner(&mut self.device, SurfaceStage::Device)?,
            application: take_owner(&mut self.application, SurfaceStage::MainThread)?,
            #[cfg(alpine_native_validation)]
            validation_probe: self.validation_probe.take(),
            #[cfg(alpine_native_validation)]
            validation_event_loop_started: Cell::new(false),
            #[cfg(alpine_native_validation)]
            validation_leases: core::mem::take(&mut self.validation_leases),
        };
        self.completed = true;
        Ok(surface)
    }
}

impl Drop for NativeSurfaceBuilder {
    fn drop(&mut self) {
        if self.completed {
            return;
        }
        begin_close_observer_state(&self.lifecycle);
        if let Some(display_link) = &self.display_link {
            display_link.setPaused(true);
            display_link.invalidate();
            #[cfg(alpine_native_validation)]
            if let Some(probe) = &self.validation_probe {
                probe.record_link_invalidation();
            }
            display_link.setDelegate(None);
            #[cfg(alpine_native_validation)]
            if let Some(probe) = &self.validation_probe {
                probe.record_delegate_revocation();
            }
        }
        if let Some(window) = &self.window {
            window.setDelegate(None);
            window.orderOut(None);
            window.close();
            #[cfg(alpine_native_validation)]
            if let Some(probe) = &self.validation_probe {
                probe.record_window_close();
            }
        }
        #[cfg(alpine_native_validation)]
        if let Some(delegate) = &self.delegate {
            if let Some(pasteboard) = delegate.ivars().validation_pasteboard.borrow().as_ref() {
                pasteboard.release();
            }
        }
        finish_close_observer_state(&self.lifecycle);
    }
}

fn take_owner<T>(owner: &mut Option<T>, stage: SurfaceStage) -> Result<T, SurfaceError> {
    owner.take().ok_or_else(|| native_unavailable(stage))
}

#[cfg(alpine_native_validation)]
pub(crate) fn exercise_initialization_fault(
    stage: SurfaceStage,
) -> Result<crate::native_validation::NativeOwnerEvidence, SurfaceError> {
    let descriptor = SurfaceDescriptor::new("Alpine initialization stage", 32.0, 24.0, 1.0)?;
    let control = InitializationControl::validation(Some(stage));
    let probe = control
        .probe()
        .ok_or(SurfaceError::validation(SurfaceOperation::Validation))?;
    let failed_at_expected_stage = autoreleasepool(|_| {
        let result = NativeSurface::new_with_control(&descriptor, &control);
        let failed_at_expected_stage = matches!(
            &result,
            Err(SurfaceError::NativeUnavailable {
                stage: failed_stage,
            }) if *failed_stage == stage
        );
        drop(result);
        failed_at_expected_stage
    });
    if !failed_at_expected_stage {
        return Err(native_unavailable(stage));
    }
    let observed_window_deallocation = probe
        .0
        .window
        .borrow()
        .as_ref()
        .map(|window| window.load().is_none());
    let expected_window_deallocation = match stage {
        SurfaceStage::MainThread | SurfaceStage::Device | SurfaceStage::Renderer => None,
        SurfaceStage::Window
        | SurfaceStage::View
        | SurfaceStage::ColorSpace
        | SurfaceStage::Layer
        | SurfaceStage::DisplayLink
        | SurfaceStage::RunLoop => Some(true),
    };
    if observed_window_deallocation != expected_window_deallocation {
        return Err(native_unavailable(SurfaceStage::Window));
    }
    Ok(probe.evidence())
}

#[cfg(alpine_native_validation)]
pub(crate) fn validate_initialization_rollback() -> Result<(), SurfaceError> {
    use crate::SurfaceLifecycle;

    let descriptor = SurfaceDescriptor::new("Alpine initialization rollback", 96.0, 64.0, 2.0)?;
    let stages = [
        (SurfaceStage::MainThread, 0),
        (SurfaceStage::Device, 2),
        (SurfaceStage::Renderer, 3),
        (SurfaceStage::Window, 4),
        (SurfaceStage::View, 5),
        (SurfaceStage::ColorSpace, 6),
        (SurfaceStage::Layer, 7),
        (SurfaceStage::DisplayLink, 9),
        (SurfaceStage::RunLoop, 9),
    ];

    for (stage, owner_count) in stages {
        let expected = expected_owner_counts(owner_count);
        let fault_evidence = exercise_initialization_fault(stage)?;
        assert_eq!(
            fault_evidence.acquired(),
            expected,
            "fault acquisition after {stage:?}"
        );
        assert_eq!(
            fault_evidence.released(),
            expected,
            "fault release after {stage:?}"
        );
        assert_eq!(
            fault_evidence.active(),
            [0; NATIVE_OWNER_KINDS],
            "fault active after {stage:?}"
        );
        assert_eq!(fault_evidence.release_order_violations(), 0);

        let control = InitializationControl::validation(Some(stage));
        let Some(observer) = control.observer() else {
            return Err(native_unavailable(SurfaceStage::MainThread));
        };
        let Some(probe) = control.probe() else {
            return Err(native_unavailable(SurfaceStage::MainThread));
        };
        let result = NativeSurface::new_with_control(&descriptor, &control);
        let failed_at_expected_stage = matches!(
            &result,
            Err(SurfaceError::NativeUnavailable {
                stage: failed_stage,
            }) if *failed_stage == stage
        );
        drop(result);

        let (acquired, released, active) = probe.counts();
        assert!(failed_at_expected_stage, "fault after {stage:?}");
        assert_eq!(acquired, expected, "acquisition after {stage:?}");
        assert_eq!(released, expected, "release after {stage:?}");
        assert_eq!(active, [0; NATIVE_OWNER_KINDS], "active after {stage:?}");
        assert_eq!(observer.lifecycle(), SurfaceLifecycle::Closed);
        assert_eq!(observer.callback_count(), 0);
        assert_eq!(
            probe.0.run_loop_registrations.get(),
            u64::from(stage == SurfaceStage::RunLoop)
        );
        assert_eq!(
            probe.0.link_invalidations.get(),
            u64::from(owner_count > NativeOwnerKind::DisplayLink.index())
        );
        assert_eq!(
            probe.0.delegate_revocations.get(),
            u64::from(owner_count > NativeOwnerKind::Delegate.index())
        );
        assert_eq!(probe.0.window_closes.get(), u64::from(owner_count >= 4));
        assert_eq!(probe.0.release_order_violations.get(), 0);
    }

    let control = InitializationControl::validation(None);
    let Some(observer) = control.observer() else {
        return Err(native_unavailable(SurfaceStage::MainThread));
    };
    let Some(probe) = control.probe() else {
        return Err(native_unavailable(SurfaceStage::MainThread));
    };
    let surface = NativeSurface::new_with_control(&descriptor, &control)?;
    assert_eq!(observer.lifecycle(), SurfaceLifecycle::Live);
    drop(surface);

    let initialized_owners = expected_owner_counts(NativeOwnerKind::Pasteboard.index());
    let (acquired, released, active) = probe.counts();
    assert_eq!(acquired, initialized_owners);
    assert_eq!(released, initialized_owners);
    assert_eq!(active, [0; NATIVE_OWNER_KINDS]);
    assert_eq!(probe.0.run_loop_registrations.get(), 1);
    assert_eq!(probe.0.link_invalidations.get(), 1);
    assert_eq!(probe.0.delegate_revocations.get(), 1);
    assert_eq!(probe.0.window_closes.get(), 1);
    assert_eq!(probe.0.release_order_violations.get(), 0);
    assert_eq!(observer.lifecycle(), SurfaceLifecycle::Closed);
    assert_eq!(observer.callback_count(), 0);

    let faulty_cleanup = InitializationProbe::default();
    for kind in [
        NativeOwnerKind::Window,
        NativeOwnerKind::Delegate,
        NativeOwnerKind::DisplayLink,
        NativeOwnerKind::Pasteboard,
    ] {
        drop(faulty_cleanup.acquire(kind));
    }
    let (_, faulty_releases, faulty_active) = faulty_cleanup.counts();
    assert_eq!(faulty_releases, [0, 0, 0, 1, 0, 0, 0, 1, 1, 1]);
    assert_eq!(faulty_active, [0; NATIVE_OWNER_KINDS]);
    assert_eq!(faulty_cleanup.0.release_order_violations.get(), 4);

    let pasteboard_probe = InitializationProbe::default();
    drop(ValidationPasteboard::new(Some(pasteboard_probe.clone())));
    let (pasteboard_acquired, pasteboard_released, pasteboard_active) = pasteboard_probe.counts();
    let mut pasteboard_expected = [0; NATIVE_OWNER_KINDS];
    pasteboard_expected[NativeOwnerKind::Pasteboard.index()] = 1;
    assert_eq!(pasteboard_acquired, pasteboard_expected);
    assert_eq!(pasteboard_released, pasteboard_expected);
    assert_eq!(pasteboard_active, [0; NATIVE_OWNER_KINDS]);
    assert_eq!(pasteboard_probe.0.pasteboard_releases.get(), 1);
    assert_eq!(pasteboard_probe.0.release_order_violations.get(), 0);
    Ok(())
}

#[cfg(alpine_native_validation)]
fn expected_owner_counts(owner_count: usize) -> [u64; NATIVE_OWNER_KINDS] {
    let mut expected = [0; NATIVE_OWNER_KINDS];
    for count in expected.iter_mut().take(owner_count) {
        *count = 1;
    }
    expected
}

#[cfg(all(test, not(miri)))]
mod tests {
    use super::*;

    #[test]
    fn accessibility_frame_admission_requires_both_action_authorities() {
        assert!(DisplayLinkDelegate::accessibility_frame_admitted(
            true, true
        ));
        assert!(!DisplayLinkDelegate::accessibility_frame_admitted(
            false, true
        ));
        assert!(!DisplayLinkDelegate::accessibility_frame_admitted(
            true, false
        ));
        assert!(!DisplayLinkDelegate::accessibility_frame_admitted(
            false, false
        ));
    }

    #[test]
    fn clipboard_text_byte_limit_accepts_the_limit_and_rejects_overflow() {
        assert_eq!(
            validate_clipboard_text_bytes(crate::MAX_CLIPBOARD_TEXT_BYTES),
            Ok(())
        );
        assert_eq!(
            validate_clipboard_text_bytes(crate::MAX_CLIPBOARD_TEXT_BYTES + 1),
            Err(ClipboardError::TooLarge {
                bytes: crate::MAX_CLIPBOARD_TEXT_BYTES + 1,
                limit: crate::MAX_CLIPBOARD_TEXT_BYTES,
            })
        );
    }

    #[test]
    fn worker_thread_is_rejected_before_native_acquisition() -> Result<(), SurfaceError> {
        let descriptor = SurfaceDescriptor::new("Alpine", 64.0, 64.0, 1.0)?;

        assert!(matches!(
            NativeSurface::new(&descriptor),
            Err(SurfaceError::NativeUnavailable {
                stage: SurfaceStage::MainThread,
            })
        ));
        Ok(())
    }

    #[test]
    fn absent_device_is_classified_before_surface_creation() {
        assert!(matches!(
            require_device(None),
            Err(SurfaceError::NativeUnavailable {
                stage: SurfaceStage::Device,
            })
        ));
    }

    #[test]
    fn standard_window_style_has_exactly_the_supported_controls() {
        assert_eq!(standard_window_style_mask(), NSWindowStyleMask(0b1111));
    }

    #[test]
    fn only_lost_backend_recovery_discards_queued_work() {
        assert!(!discards_pending_work(None));
        assert!(!discards_pending_work(Some(
            RecoveryClassification::RetryFrame
        )));
        assert!(discards_pending_work(Some(
            RecoveryClassification::RecreateBackend
        )));
    }

    #[test]
    fn ownership_invariant_responder_and_validation_failures_never_recover() {
        let operations = [
            SurfaceOperation::Application,
            SurfaceOperation::Presentation,
            SurfaceOperation::NativeConfiguration,
            SurfaceOperation::EventHandler,
            SurfaceOperation::Input,
            SurfaceOperation::Clipboard,
            SurfaceOperation::Accessibility,
            SurfaceOperation::RunLoop,
            SurfaceOperation::Validation,
        ];
        for operation in operations {
            assert_eq!(
                render_recovery(&SurfaceError::owner_conflict(operation)),
                None
            );
            assert_eq!(render_recovery(&SurfaceError::invariant(operation)), None);
            assert_eq!(render_recovery(&SurfaceError::validation(operation)), None);
        }
        assert_eq!(render_recovery(&SurfaceError::InputResponderRejected), None);
    }

    #[test]
    fn sdr_color_contract_rejects_each_independent_policy_break() {
        assert!(recognizes_sdr_color_contract(
            MTLPixelFormat::BGRA8Unorm_sRGB,
            false,
            true
        ));
        for (pixel_format, extended_dynamic_range, standard_srgb_color_space) in [
            (MTLPixelFormat::BGRA8Unorm, false, true),
            (MTLPixelFormat::BGRA8Unorm_sRGB, true, true),
            (MTLPixelFormat::BGRA8Unorm_sRGB, false, false),
        ] {
            assert!(!recognizes_sdr_color_contract(
                pixel_format,
                extended_dynamic_range,
                standard_srgb_color_space,
            ));
        }
    }

    #[test]
    #[cfg(alpine_native_validation)]
    fn presentation_observation_requires_a_real_or_injected_signal() -> Result<(), &'static str> {
        let signal = Arc::new(PresentationSignal::new(None));
        let observation = PresentationObservation::new(Arc::clone(&signal));
        assert!(!observation.observed());
        assert_eq!(observation.event_to_presented_handler_ns(), None);

        signal.publish(17);
        assert!(observation.observed());
        assert_eq!(observation.presented_time_bits(), 17);
        assert_eq!(observation.event_to_presented_handler_ns(), None);

        let received_at = std::time::Instant::now()
            .checked_sub(std::time::Duration::from_secs(1))
            .ok_or("timed presentation origin")?;
        let timed_signal = Arc::new(PresentationSignal::new(Some(received_at)));
        let timed_observation = PresentationObservation::new(Arc::clone(&timed_signal));
        assert_eq!(timed_observation.event_to_presented_handler_ns(), None);
        timed_signal.publish(19);
        assert!(timed_observation.observed());
        assert_eq!(timed_observation.presented_time_bits(), 19);
        assert!(
            timed_observation
                .event_to_presented_handler_ns()
                .ok_or("timed presentation latency")?
                >= 1_000_000_000
        );

        let signal = Arc::new(PresentationSignal::new(Some(received_at)));
        let mut injected = PresentationObservation::new(Arc::clone(&signal));
        injected.inject(23);
        let injected_latency = injected
            .event_to_presented_handler_ns()
            .ok_or("injected presentation latency")?;
        assert!(injected_latency >= 1_000_000_000);
        signal.publish(29);
        assert!(injected.observed());
        assert_eq!(injected.presented_time_bits(), 23);
        assert_eq!(
            injected.event_to_presented_handler_ns(),
            Some(injected_latency)
        );
        assert!(
            signal
                .event_to_presented_handler_ns()
                .ok_or("late presentation latency")?
                >= injected_latency
        );
        Ok(())
    }

    #[test]
    #[cfg(alpine_native_validation)]
    fn validation_close_observation_is_active_only_and_bounded() {
        assert_eq!(next_validation_close_observation(0, None), None);
        assert_eq!(next_validation_close_observation(0, Some(true)), None);
        assert_eq!(next_validation_close_observation(0, Some(false)), Some(1));
        assert_eq!(
            next_validation_close_observation(VALIDATION_CLOSE_OBSERVATION_LIMIT - 1, Some(false),),
            Some(VALIDATION_CLOSE_OBSERVATION_LIMIT)
        );
        assert_eq!(
            next_validation_close_observation(VALIDATION_CLOSE_OBSERVATION_LIMIT, Some(false),),
            None
        );
        assert_eq!(
            next_validation_close_observation(u8::MAX, Some(false)),
            None
        );
    }

    #[test]
    #[cfg(alpine_native_validation)]
    fn validation_close_requires_qualified_presentation_and_complete_resource_drain() {
        assert!(validation_close_resources_drained(false, false, 0));
        assert!(!validation_close_resources_drained(true, false, 0));
        assert!(!validation_close_resources_drained(false, true, 0));
        assert!(!validation_close_resources_drained(true, true, 0));
        assert!(!validation_close_resources_drained(false, false, 1));
        assert!(!validation_close_resources_drained(false, false, 3));
        assert!(!validation_close_resources_drained(false, false, u8::MAX));

        assert!(validation_close_should_retry(0, false));
        assert!(validation_close_should_retry(0, true));
        assert!(validation_close_should_retry(1, false));
        assert!(!validation_close_should_retry(1, true));
        assert!(!validation_close_should_retry(u64::MAX, true));
    }

    #[test]
    fn centered_window_origin_never_moves_before_the_visible_origin() {
        let visible = NSRect::new(NSPoint::new(-100.0, 40.0), NSSize::new(800.0, 600.0));
        let centered = centered_window_origin(
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(400.0, 200.0)),
            visible,
        );
        assert_eq!(centered, NSPoint::new(100.0, 240.0));

        let oversized = centered_window_origin(
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(900.0, 700.0)),
            visible,
        );
        assert_eq!(oversized, visible.origin);
    }

    #[test]
    #[cfg(alpine_native_validation)]
    fn post_commit_display_replacement_changes_only_identity() -> Result<(), SurfaceError> {
        let base = SurfaceConfiguration::from_native(64.0, 48.0, 2.0, 7, true)?;
        let migrated = configuration_with_display_identity(base, 11);
        let expected = SurfaceConfiguration::from_native(64.0, 48.0, 2.0, 11, true)?;

        assert_eq!(migrated, expected);
        assert!(base.geometry_or_display_differs(migrated));
        Ok(())
    }

    #[test]
    #[cfg(alpine_native_validation)]
    fn callback_admission_classifies_live_and_revoked_generations() {
        let lifecycle = AtomicU8::new(SURFACE_LIVE);
        let admitted = AtomicU64::new(0);
        let rejected = AtomicU64::new(0);

        assert!(admit_callback(&lifecycle, &admitted, &rejected));
        assert_eq!(admitted.load(Ordering::Relaxed), 1);
        assert_eq!(rejected.load(Ordering::Relaxed), 0);
        lifecycle.store(SURFACE_CLOSING, Ordering::Release);
        assert!(!admit_callback(&lifecycle, &admitted, &rejected));
        assert_eq!(admitted.load(Ordering::Relaxed), 1);
        assert_eq!(rejected.load(Ordering::Relaxed), 1);
    }

    #[test]
    #[cfg(alpine_native_validation)]
    fn idle_driver_shutdown_stops_portable_and_backend_ownership() -> Result<(), SurfaceError> {
        let device = require_device(MTLCreateSystemDefaultDevice())?;
        let backend = platform_spi::new_validation_backend_with_device(device)?;
        let configuration = SurfaceConfiguration::from_native(64.0, 64.0, 1.0, 0, false)?;
        let mut driver = PresentationDriver::new(
            backend,
            configuration,
            Arc::new(AtomicU8::new(SURFACE_LIVE)),
        )?;

        driver.shutdown(&FrameCounters::default());

        assert_eq!(
            driver.state.application(),
            alpine_platform::ApplicationState::Stopped
        );
        assert_eq!(
            driver.backend.accounting().state(),
            alpine_metal::BackendState::Stopped
        );
        assert!(driver.pending.is_none());
        assert!(driver.active.is_none());
        Ok(())
    }

    #[test]
    #[cfg(alpine_native_validation)]
    fn committed_cancellation_continues_an_existing_drain() -> Result<(), SurfaceError> {
        let device = require_device(MTLCreateSystemDefaultDevice())?;
        let backend = platform_spi::new_validation_backend_with_device(device)?;
        let configuration = SurfaceConfiguration::from_native(64.0, 64.0, 1.0, 0, false)?;
        let lifecycle = Arc::new(AtomicU8::new(SURFACE_LIVE));
        let mut driver = PresentationDriver::new(backend, configuration, Arc::clone(&lifecycle))?;
        let counters = FrameCounters::default();

        driver.state.apply(PresentationAction::SetVisible(true))?;
        driver.state.apply(PresentationAction::Invalidate)?;
        driver.state.apply(PresentationAction::Resume)?;
        let prepared = driver.state.apply(PresentationAction::Prepare)?;
        let PresentationEvent::Prepared(token) = prepared.event() else {
            return Err(SurfaceError::invariant(SurfaceOperation::Presentation));
        };
        driver.state.apply(PresentationAction::BeginUpdate(token))?;
        driver.state.apply(PresentationAction::Submit(token))?;
        driver.state.apply(PresentationAction::CallPresent(token))?;
        driver.state.apply(PresentationAction::BeginShutdown)?;
        lifecycle.store(SURFACE_CLOSING, Ordering::Release);

        let directive = driver.cancel_attempt(
            token,
            AttemptTiming {
                target_timestamp_bits: 137,
                target_presentation_timestamp_bits: 139,
                ..AttemptTiming::default()
            },
            &counters,
        )?;

        assert_eq!(directive, DisplayLinkDirective::None);
        assert_eq!(driver.state.application(), ApplicationState::Stopped);
        assert_eq!(driver.state.outcome(), PresentationOutcome::Cancelled);
        assert_eq!(counters.cancelled.load(Ordering::Relaxed), 1);
        let evidence = driver
            .last_cancelled
            .ok_or(SurfaceError::invariant(SurfaceOperation::Presentation))?;
        assert_eq!(evidence.target_timestamp_bits(), 137);
        assert_eq!(evidence.target_presentation_timestamp_bits(), 139);
        assert_eq!(evidence.submission_count(), 1);
        assert_eq!(evidence.present_call_count(), 1);
        Ok(())
    }
}
