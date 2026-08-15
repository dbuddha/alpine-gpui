use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
};
use std::{cell::RefCell, rc::Rc};
use std::{ffi::c_void, ptr::NonNull};

#[cfg(alpine_native_validation)]
use std::{
    cell::Cell,
    time::{Duration, Instant},
};

use objc2::{AnyThread, DefinedClass, MainThreadOnly, define_class, msg_send, rc::Retained};
use objc2::{MainThreadMarker, runtime::ProtocolObject};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSView, NSWindow,
    NSWindowStyleMask,
};
#[cfg(alpine_native_validation)]
use objc2_app_kit::{NSEvent, NSEventModifierFlags, NSEventType};
#[cfg(alpine_native_validation)]
use objc2_foundation::{NSDate, NSTimer};
use objc2_foundation::{
    NSObject, NSObjectProtocol, NSPoint, NSRect, NSRunLoop, NSRunLoopCommonModes, NSSize, NSString,
};
use objc2_metal::{MTLCreateSystemDefaultDevice, MTLDevice, MTLDrawable, MTLPixelFormat};
use objc2_quartz_core::{
    CAMetalDisplayLink, CAMetalDisplayLinkDelegate, CAMetalDisplayLinkUpdate, CAMetalDrawable,
    CAMetalLayer,
};

use alpine_core::LinearRgba;
use alpine_metal::{MetalBackend, OffscreenDescriptor, platform_spi};
use alpine_platform::{
    DisplayLinkDirective, FrameToken, PresentationAction, PresentationEvent, PresentationState,
};
use alpine_scene::Scene;
use block2::RcBlock;

use crate::{
    SURFACE_LIVE, SurfaceDescriptor, SurfaceError, SurfaceObserver, SurfaceSnapshot, SurfaceStage,
    begin_close_observer_state, finish_close_observer_state, new_observer_state,
};

type Device = Retained<ProtocolObject<dyn MTLDevice>>;

#[cfg(alpine_native_validation)]
const NATIVE_OWNER_KINDS: usize = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NativeOwnerKind {
    Application,
    Device,
    Renderer,
    Window,
    View,
    Layer,
    Delegate,
    DisplayLink,
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
            Self::Layer => 5,
            Self::Delegate => 6,
            Self::DisplayLink => 7,
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
        }
    }

    #[allow(
        clippy::unused_self,
        reason = "shipping builds erase validation state while preserving one constructor path"
    )]
    fn observer_state(&self) -> (Arc<AtomicU8>, Arc<AtomicU64>) {
        #[cfg(alpine_native_validation)]
        if let (Some(lifecycle), Some(callback_count)) = (&self.lifecycle, &self.callback_count) {
            return (Arc::clone(lifecycle), Arc::clone(callback_count));
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
            return platform_spi::new_validation_backend_with_device(device)
                .map_err(SurfaceError::from);
        }
        platform_spi::new_backend_with_device(device).map_err(SurfaceError::from)
    }

    #[cfg(alpine_native_validation)]
    fn validation(fault_after: Option<SurfaceStage>) -> Self {
        let (lifecycle, callback_count) = new_observer_state();
        Self {
            fault_after,
            probe: Some(InitializationProbe::default()),
            lifecycle: Some(lifecycle),
            callback_count: Some(callback_count),
        }
    }

    #[cfg(alpine_native_validation)]
    fn observer(&self) -> Option<SurfaceObserver> {
        Some(SurfaceObserver::new(
            Arc::clone(self.lifecycle.as_ref()?),
            Arc::clone(self.callback_count.as_ref()?),
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
    release_order_violations: Cell<u64>,
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
            NativeOwnerKind::Application
            | NativeOwnerKind::Device
            | NativeOwnerKind::Renderer
            | NativeOwnerKind::View
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
    last_presented_time_bits: AtomicU64,
    skipped: AtomicU64,
    failed: AtomicU64,
}

struct PendingFrame {
    scene: Scene,
    descriptor: OffscreenDescriptor,
}

struct ActiveFrame {
    token: FrameToken,
    #[allow(
        dead_code,
        reason = "the callback drawable must remain retained until terminal presentation evidence"
    )]
    drawable: Retained<ProtocolObject<dyn CAMetalDrawable>>,
    frame: PendingFrame,
    presentation: Arc<PresentationSignal>,
    presentation_polls: u16,
}

#[derive(Default)]
struct PresentationSignal {
    observed: AtomicBool,
    time_bits: AtomicU64,
}

struct PresentationDriver {
    state: PresentationState,
    pending: Option<PendingFrame>,
    active: Option<ActiveFrame>,
    backend: MetalBackend,
    consecutive_skips: u16,
    last_error: Option<SurfaceError>,
}

impl PresentationDriver {
    fn new(backend: MetalBackend) -> Result<Self, SurfaceError> {
        let mut state = PresentationState::new();
        state.apply(PresentationAction::SetSized(true))?;
        Ok(Self {
            state,
            pending: None,
            active: None,
            backend,
            consecutive_skips: 0,
            last_error: None,
        })
    }

    fn set_visible(&mut self, visible: bool) -> Result<DisplayLinkDirective, SurfaceError> {
        let transition = self.state.apply(PresentationAction::SetVisible(visible))?;
        self.enact_resume(transition.display_link())
    }

    fn request_frame(
        &mut self,
        scene: Scene,
        descriptor: OffscreenDescriptor,
    ) -> Result<(alpine_platform::PresentationRevision, DisplayLinkDirective), SurfaceError> {
        let transition = self.state.apply(PresentationAction::Invalidate)?;
        let PresentationEvent::Invalidated(revision) = transition.event() else {
            return Err(SurfaceError::DriverUnavailable);
        };
        self.pending = Some(PendingFrame { scene, descriptor });
        let directive = self.enact_resume(transition.display_link())?;
        Ok((revision, directive))
    }

    fn enact_resume(
        &mut self,
        directive: DisplayLinkDirective,
    ) -> Result<DisplayLinkDirective, SurfaceError> {
        if directive == DisplayLinkDirective::Resume {
            Ok(self.state.apply(PresentationAction::Resume)?.display_link())
        } else {
            Ok(directive)
        }
    }

    fn update(
        &mut self,
        update: &CAMetalDisplayLinkUpdate,
        counters: &FrameCounters,
    ) -> DisplayLinkDirective {
        match self.try_update(update, counters) {
            Ok(directive) => directive,
            Err(error) => {
                self.last_error = Some(error);
                counters.failed.fetch_add(1, Ordering::Relaxed);
                let token = self
                    .active
                    .take()
                    .map(|active| active.token)
                    .or_else(|| self.state.active_token());
                if let Some(token) = token {
                    self.state
                        .apply(PresentationAction::FailActive(token))
                        .map_or(DisplayLinkDirective::Pause, |transition| {
                            transition.display_link()
                        })
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
        if self
            .active
            .as_ref()
            .is_some_and(|active| active.presentation.observed.load(Ordering::Acquire))
        {
            let Some(active) = self.active.take() else {
                return Err(SurfaceError::DriverUnavailable);
            };
            let token = active.token;
            let presented_time_bits = active.presentation.time_bits.load(Ordering::Relaxed);
            if presented_time_bits != 0 {
                self.consecutive_skips = 0;
                counters
                    .last_presented_time_bits
                    .store(presented_time_bits, Ordering::Relaxed);
                counters.presented.fetch_add(1, Ordering::Relaxed);
                return Ok(self
                    .state
                    .apply(PresentationAction::CompletePresentation(token))?
                    .display_link());
            }

            counters.skipped.fetch_add(1, Ordering::Relaxed);
            self.consecutive_skips = self.consecutive_skips.saturating_add(1);
            if self.pending.is_none() {
                self.pending = Some(active.frame);
            }
            self.state.apply(PresentationAction::FailActive(token))?;
            if self.consecutive_skips >= MAX_PRESENTATION_POLLS {
                return Err(SurfaceError::PresentationsSkipped {
                    attempts: self.consecutive_skips,
                });
            }
            // The physical link never paused during this callback. Reconcile
            // the portable fail-and-resume pair, then consume this update
            // rather than wasting one refresh before retrying.
            self.state.apply(PresentationAction::Resume)?;
        }
        if let Some(active) = &mut self.active {
            active.presentation_polls = active.presentation_polls.saturating_add(1);
            if active.presentation_polls >= MAX_PRESENTATION_POLLS {
                return Err(SurfaceError::PresentationNotObserved {
                    callbacks: active.presentation_polls,
                });
            }
            return Ok(DisplayLinkDirective::None);
        }

        let prepared = self.state.apply(PresentationAction::Prepare)?;
        let PresentationEvent::Prepared(token) = prepared.event() else {
            return Err(SurfaceError::DriverUnavailable);
        };
        let Some(frame) = self.pending.take() else {
            self.state.apply(PresentationAction::FailActive(token))?;
            return Err(SurfaceError::DriverUnavailable);
        };
        self.state.apply(PresentationAction::BeginUpdate(token))?;

        let drawable = update.drawable();
        let texture = drawable.texture();
        let drawable_protocol = ProtocolObject::from_ref(&*drawable);
        let presentation = Arc::new(PresentationSignal::default());
        install_presented_handler(drawable_protocol, &presentation, counters);
        let attempt = platform_spi::render_callback_drawable(
            &mut self.backend,
            &frame.scene,
            frame.descriptor,
            &texture,
            drawable_protocol,
        );
        if attempt.committed() {
            self.state.apply(PresentationAction::Submit(token))?;
            counters.submissions.fetch_add(1, Ordering::Relaxed);
        }
        if attempt.present_called() {
            self.state.apply(PresentationAction::CallPresent(token))?;
            counters.direct_presents.fetch_add(1, Ordering::Relaxed);
        }
        match attempt.into_result() {
            Ok(_) => {
                self.active = Some(ActiveFrame {
                    token,
                    drawable,
                    frame,
                    presentation,
                    presentation_polls: 0,
                });
                Ok(DisplayLinkDirective::None)
            }
            Err(error) => {
                self.state.apply(PresentationAction::FailActive(token))?;
                Err(error.into())
            }
        }
    }

    fn take_error(&mut self) -> Option<SurfaceError> {
        self.last_error.take()
    }

    #[cfg(alpine_native_validation)]
    fn inject_error(&mut self, error: SurfaceError) {
        self.last_error = Some(error);
    }

    fn shutdown(&mut self) {
        let _ = self.state.apply(PresentationAction::BeginShutdown);
        if let Some(active) = self.active.take() {
            let _ = self
                .state
                .apply(PresentationAction::FailActive(active.token));
        }
        let _ = self.state.apply(PresentationAction::StopAfterDrain);
        self.pending = None;
        self.backend.shutdown();
    }
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
            signal
                .time_bits
                .store(drawable.presentedTime().to_bits(), Ordering::Relaxed);
            signal.observed.store(true, Ordering::Release);
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

struct DisplayLinkDelegateIvars {
    lifecycle: Arc<AtomicU8>,
    callback_count: Arc<AtomicU64>,
    counters: Arc<FrameCounters>,
    driver: Option<Rc<RefCell<PresentationDriver>>>,
}

define_class!(
    // SAFETY:
    // - NSObject has no subclassing requirements.
    // - DisplayLinkDelegate has no custom Drop implementation.
    // - Its cross-thread signal ivars use atomics, mutable driver ownership is
    //   main-thread-only, and the callback refuses frame work
    //   unless it arrives on the main run loop where the link was registered.
    #[unsafe(super = NSObject)]
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
            if self.ivars().lifecycle.load(Ordering::Acquire) == SURFACE_LIVE {
                self.ivars().callback_count.fetch_add(1, Ordering::Relaxed);
                if MainThreadMarker::new().is_none() {
                    self.ivars().counters.failed.fetch_add(1, Ordering::Relaxed);
                    return;
                }
                if let Some(driver) = &self.ivars().driver {
                    let directive = driver.try_borrow_mut().map_or_else(
                        |_| {
                            self.ivars().counters.failed.fetch_add(1, Ordering::Relaxed);
                            DisplayLinkDirective::Pause
                        },
                        |mut driver| driver.update(update, &self.ivars().counters),
                    );
                    apply_display_link_directive(link, directive);
                }
            }
        }
    }
);

impl DisplayLinkDelegate {
    fn new(
        lifecycle: Arc<AtomicU8>,
        callback_count: Arc<AtomicU64>,
        counters: Arc<FrameCounters>,
        driver: Option<Rc<RefCell<PresentationDriver>>>,
    ) -> Retained<Self> {
        let allocated = Self::alloc().set_ivars(DisplayLinkDelegateIvars {
            lifecycle,
            callback_count,
            counters,
            driver,
        });
        // SAFETY: The message is NSObject's parameterless init initializer and
        // the allocated object already contains fully initialized Rust ivars.
        unsafe { msg_send![super(allocated), init] }
    }
}

fn apply_display_link_directive(link: &CAMetalDisplayLink, directive: DisplayLinkDirective) {
    match directive {
        DisplayLinkDirective::None => {}
        DisplayLinkDirective::Resume => link.setPaused(false),
        DisplayLinkDirective::Pause => link.setPaused(true),
        DisplayLinkDirective::Invalidate => link.invalidate(),
    }
}

pub(crate) struct NativeSurface {
    extent: crate::SurfaceExtent,
    callback_count: Arc<AtomicU64>,
    counters: Arc<FrameCounters>,
    driver: Rc<RefCell<PresentationDriver>>,
    lifecycle: Arc<AtomicU8>,
    display_link: Retained<CAMetalDisplayLink>,
    #[allow(
        dead_code,
        reason = "CAMetalDisplayLink retains its delegate weakly, so Alpine must retain it"
    )]
    delegate: Retained<DisplayLinkDelegate>,
    layer: Retained<CAMetalLayer>,
    #[allow(
        dead_code,
        reason = "the custom content view owns the layer attachment for the surface lifetime"
    )]
    view: Retained<NSView>,
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
        let (lifecycle, callback_count) = control.observer_state();
        let mut builder = NativeSurfaceBuilder::new(
            extent,
            lifecycle,
            callback_count,
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
        // NSWindow's initializer contract. Alpine disables release-on-close
        // immediately below and retains the returned window for its lifetime.
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(main_thread),
                frame,
                NSWindowStyleMask::Titled
                    | NSWindowStyleMask::Closable
                    | NSWindowStyleMask::Miniaturizable
                    | NSWindowStyleMask::Resizable,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        // SAFETY: This window is created without a window controller and is
        // retained by NativeSurface until close, so AppKit must not release it.
        unsafe { window.setReleasedWhenClosed(false) };
        window.setTitle(&NSString::from_str(descriptor.title()));
        builder.window = Some(window);
        builder.track(NativeOwnerKind::Window);
        control.checkpoint(SurfaceStage::Window)?;

        let view = NSView::initWithFrame(NSView::alloc(main_thread), frame);
        builder.view = Some(view);
        builder.track(NativeOwnerKind::View);
        control.checkpoint(SurfaceStage::View)?;

        let layer = CAMetalLayer::layer();
        let device = builder
            .device
            .as_ref()
            .ok_or_else(|| native_unavailable(SurfaceStage::Device))?;
        layer.setDevice(Some(device));
        layer.setPixelFormat(MTLPixelFormat::BGRA8Unorm);
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
            .as_ref()
            .ok_or_else(|| native_unavailable(SurfaceStage::Window))?;
        window.setContentView(Some(view));
        builder.layer = Some(layer);
        builder.track(NativeOwnerKind::Layer);
        control.checkpoint(SurfaceStage::Layer)?;

        let backend = take_owner(&mut builder.backend, SurfaceStage::Renderer)?;
        let driver = Rc::new(RefCell::new(PresentationDriver::new(backend)?));
        builder.driver = Some(Rc::clone(&driver));
        let delegate = DisplayLinkDelegate::new(
            Arc::clone(&builder.lifecycle),
            Arc::clone(&builder.callback_count),
            Arc::clone(&builder.counters),
            Some(driver),
        );
        builder.delegate = Some(delegate);
        builder.track(NativeOwnerKind::Delegate);
        let layer = builder
            .layer
            .as_ref()
            .ok_or_else(|| native_unavailable(SurfaceStage::Layer))?;
        let display_link =
            CAMetalDisplayLink::initWithMetalLayer(CAMetalDisplayLink::alloc(), layer);
        let delegate = builder
            .delegate
            .as_ref()
            .ok_or_else(|| native_unavailable(SurfaceStage::DisplayLink))?;
        display_link.setDelegate(Some(ProtocolObject::from_ref(&**delegate)));
        display_link.setPreferredFrameLatency(2.0);
        display_link.setPaused(true);
        builder.display_link = Some(display_link);
        builder.track(NativeOwnerKind::DisplayLink);
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
        let directive = self
            .driver
            .try_borrow_mut()
            .map_err(|_| SurfaceError::DriverUnavailable)?
            .set_visible(true)?;
        apply_display_link_directive(&self.display_link, directive);
        Ok(())
    }

    #[allow(
        clippy::cast_possible_truncation,
        reason = "validated finite scale is narrowed to the renderer's f32 coordinate contract"
    )]
    pub(crate) fn request_frame(
        &self,
        scene: Scene,
        clear: LinearRgba,
    ) -> Result<alpine_platform::PresentationRevision, SurfaceError> {
        let descriptor = OffscreenDescriptor::new(
            self.extent.physical_width(),
            self.extent.physical_height(),
            self.extent.scale() as f32,
            clear,
        )
        .map_err(alpine_metal::RenderError::from)?;
        let (revision, directive) = self
            .driver
            .try_borrow_mut()
            .map_err(|_| SurfaceError::DriverUnavailable)?
            .request_frame(scene, descriptor)?;
        apply_display_link_directive(&self.display_link, directive);
        Ok(revision)
    }

    #[cfg(alpine_native_validation)]
    pub(crate) fn run_until_frame_terminal(&self, timeout: Duration) {
        let counters = Arc::clone(&self.counters);
        let initial_presented = counters.presented.load(Ordering::Acquire);
        let initial_failed = counters.failed.load(Ordering::Acquire);
        let deadline = Instant::now() + timeout;
        if self.validation_event_loop_started.replace(true) {
            while counters.presented.load(Ordering::Acquire) == initial_presented
                && counters.failed.load(Ordering::Acquire) == initial_failed
                && Instant::now() < deadline
            {
                NSRunLoop::mainRunLoop().runUntilDate(&NSDate::dateWithTimeIntervalSinceNow(0.005));
            }
            return;
        }
        let timer_block: RcBlock<dyn Fn(NonNull<NSTimer>)> =
            RcBlock::new(move |timer: NonNull<NSTimer>| {
                let terminal = counters.presented.load(Ordering::Acquire) > initial_presented
                    || counters.failed.load(Ordering::Acquire) > initial_failed
                    || Instant::now() >= deadline;
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

    pub(crate) fn snapshot(&self) -> SurfaceSnapshot {
        SurfaceSnapshot {
            physical_width: self.extent.physical_width(),
            physical_height: self.extent.physical_height(),
            framebuffer_only: self.layer.framebufferOnly(),
            display_sync_enabled: self.layer.displaySyncEnabled(),
            allows_next_drawable_timeout: self.layer.allowsNextDrawableTimeout(),
            maximum_drawable_count: u8::try_from(self.layer.maximumDrawableCount()).unwrap_or(0),
            regular_activation_policy: self.application.activationPolicy()
                == NSApplicationActivationPolicy::Regular,
            display_link_paused: self.display_link.isPaused(),
            visible: self.window.isVisible(),
            callback_count: self.callback_count.load(Ordering::Acquire),
            submission_count: self.counters.submissions.load(Ordering::Acquire),
            direct_present_count: self.counters.direct_presents.load(Ordering::Acquire),
            installed_presented_handler_count: self
                .counters
                .installed_presented_handlers
                .load(Ordering::Acquire),
            presented_count: self.counters.presented.load(Ordering::Acquire),
            last_presented_time_bits: self
                .counters
                .last_presented_time_bits
                .load(Ordering::Acquire),
            skipped_count: self.counters.skipped.load(Ordering::Acquire),
            failed_count: self.counters.failed.load(Ordering::Acquire),
        }
    }

    pub(crate) fn take_error(&self) -> Result<Option<SurfaceError>, SurfaceError> {
        Ok(self
            .driver
            .try_borrow_mut()
            .map_err(|_| SurfaceError::DriverUnavailable)?
            .take_error())
    }

    #[cfg(alpine_native_validation)]
    pub(crate) fn inject_driver_error(&self, error: SurfaceError) {
        if let Ok(mut driver) = self.driver.try_borrow_mut() {
            driver.inject_error(error);
        }
    }

    pub(crate) fn observer(&self) -> SurfaceObserver {
        SurfaceObserver::new(
            Arc::clone(&self.lifecycle),
            Arc::clone(&self.callback_count),
        )
    }
}

#[cfg(alpine_native_validation)]
fn stop_validation_event_loop(application: &NSApplication) {
    application.stop(None);
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

fn require_device(device: Option<Device>) -> Result<Device, SurfaceError> {
    device.ok_or(SurfaceError::NativeUnavailable {
        stage: SurfaceStage::Device,
    })
}

impl Drop for NativeSurface {
    fn drop(&mut self) {
        begin_close_observer_state(&self.lifecycle);
        if let Ok(mut driver) = self.driver.try_borrow_mut() {
            driver.shutdown();
        }
        self.display_link.setPaused(true);
        self.display_link.invalidate();
        #[cfg(alpine_native_validation)]
        if let Some(probe) = &self.validation_probe {
            probe.record_link_invalidation();
        }
        self.display_link.setDelegate(None);
        #[cfg(alpine_native_validation)]
        if let Some(probe) = &self.validation_probe {
            probe.record_delegate_revocation();
        }
        self.window.orderOut(None);
        self.window.close();
        #[cfg(alpine_native_validation)]
        if let Some(probe) = &self.validation_probe {
            probe.record_window_close();
        }
        finish_close_observer_state(&self.lifecycle);
    }
}

fn native_unavailable(stage: SurfaceStage) -> SurfaceError {
    SurfaceError::NativeUnavailable { stage }
}

struct NativeSurfaceBuilder {
    extent: crate::SurfaceExtent,
    callback_count: Arc<AtomicU64>,
    counters: Arc<FrameCounters>,
    backend: Option<MetalBackend>,
    driver: Option<Rc<RefCell<PresentationDriver>>>,
    lifecycle: Arc<AtomicU8>,
    display_link: Option<Retained<CAMetalDisplayLink>>,
    delegate: Option<Retained<DisplayLinkDelegate>>,
    layer: Option<Retained<CAMetalLayer>>,
    view: Option<Retained<NSView>>,
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
        extent: crate::SurfaceExtent,
        lifecycle: Arc<AtomicU8>,
        callback_count: Arc<AtomicU64>,
        #[cfg(alpine_native_validation)] validation_probe: Option<InitializationProbe>,
    ) -> Self {
        Self {
            extent,
            callback_count,
            counters: Arc::new(FrameCounters::default()),
            backend: None,
            driver: None,
            lifecycle,
            display_link: None,
            delegate: None,
            layer: None,
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
        let surface = NativeSurface {
            extent: self.extent,
            callback_count: Arc::clone(&self.callback_count),
            counters: Arc::clone(&self.counters),
            driver: take_owner(&mut self.driver, SurfaceStage::Renderer)?,
            lifecycle: Arc::clone(&self.lifecycle),
            display_link: take_owner(&mut self.display_link, SurfaceStage::DisplayLink)?,
            delegate: take_owner(&mut self.delegate, SurfaceStage::DisplayLink)?,
            layer: take_owner(&mut self.layer, SurfaceStage::Layer)?,
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
            window.orderOut(None);
            window.close();
            #[cfg(alpine_native_validation)]
            if let Some(probe) = &self.validation_probe {
                probe.record_window_close();
            }
        }
        finish_close_observer_state(&self.lifecycle);
    }
}

fn take_owner<T>(owner: &mut Option<T>, stage: SurfaceStage) -> Result<T, SurfaceError> {
    owner.take().ok_or_else(|| native_unavailable(stage))
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
        (SurfaceStage::Layer, 6),
        (SurfaceStage::DisplayLink, 8),
        (SurfaceStage::RunLoop, 8),
    ];

    for (stage, owner_count) in stages {
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

        let expected = expected_owner_counts(owner_count);
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
            u64::from(owner_count == NATIVE_OWNER_KINDS)
        );
        assert_eq!(
            probe.0.delegate_revocations.get(),
            u64::from(owner_count == NATIVE_OWNER_KINDS)
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

    let all_owners = [1; NATIVE_OWNER_KINDS];
    let (acquired, released, active) = probe.counts();
    assert_eq!(acquired, all_owners);
    assert_eq!(released, all_owners);
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
    ] {
        drop(faulty_cleanup.acquire(kind));
    }
    let (_, faulty_releases, faulty_active) = faulty_cleanup.counts();
    assert_eq!(faulty_releases, [0, 0, 0, 1, 0, 0, 1, 1]);
    assert_eq!(faulty_active, [0; NATIVE_OWNER_KINDS]);
    assert_eq!(faulty_cleanup.0.release_order_violations.get(), 3);
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn delegate_counts_callbacks_only_for_a_live_generation() {
        let (lifecycle, callback_count) = new_observer_state();
        let delegate = DisplayLinkDelegate::new(
            Arc::clone(&lifecycle),
            Arc::clone(&callback_count),
            Arc::new(FrameCounters::default()),
            None,
        );
        let link = CAMetalDisplayLink::new();
        let update = CAMetalDisplayLinkUpdate::new();

        // SAFETY: The receiver implements the registered protocol selector,
        // and both concrete arguments remain retained for the complete call.
        let _: () =
            unsafe { msg_send![&*delegate, metalDisplayLink: &*link, needsUpdate: &*update] };
        assert_eq!(callback_count.load(Ordering::Acquire), 1);

        begin_close_observer_state(&lifecycle);
        // SAFETY: This repeats the same correctly typed protocol message after
        // callback admission has been revoked.
        let _: () =
            unsafe { msg_send![&*delegate, metalDisplayLink: &*link, needsUpdate: &*update] };
        assert_eq!(callback_count.load(Ordering::Acquire), 1);
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
    #[cfg(alpine_native_validation)]
    fn idle_driver_shutdown_stops_portable_and_backend_ownership() -> Result<(), SurfaceError> {
        let device = require_device(MTLCreateSystemDefaultDevice())?;
        let backend = platform_spi::new_validation_backend_with_device(device)?;
        let mut driver = PresentationDriver::new(backend)?;

        driver.shutdown();

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
}
