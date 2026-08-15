use std::sync::{
    Arc,
    atomic::{AtomicU8, AtomicU64, Ordering},
};

use objc2::{AnyThread, DefinedClass, MainThreadOnly, define_class, msg_send, rc::Retained};
use objc2::{MainThreadMarker, runtime::ProtocolObject};
use objc2_app_kit::{NSApplication, NSBackingStoreType, NSView, NSWindow, NSWindowStyleMask};
use objc2_foundation::{
    NSObject, NSObjectProtocol, NSPoint, NSRect, NSRunLoop, NSRunLoopCommonModes, NSSize, NSString,
};
use objc2_metal::{MTLCreateSystemDefaultDevice, MTLDevice, MTLPixelFormat};
use objc2_quartz_core::{
    CAMetalDisplayLink, CAMetalDisplayLinkDelegate, CAMetalDisplayLinkUpdate, CAMetalLayer,
};

use crate::{
    SURFACE_LIVE, SurfaceDescriptor, SurfaceError, SurfaceObserver, SurfaceSnapshot, SurfaceStage,
    begin_close_observer_state, finish_close_observer_state, new_observer_state,
};

type Device = Retained<ProtocolObject<dyn MTLDevice>>;

#[derive(Debug)]
struct DisplayLinkDelegateIvars {
    lifecycle: Arc<AtomicU8>,
    callback_count: Arc<AtomicU64>,
}

define_class!(
    // SAFETY:
    // - NSObject has no subclassing requirements.
    // - DisplayLinkDelegate has no custom Drop implementation.
    // - Its ivars are thread-safe atomics because display-link callback thread
    //   affinity is intentionally not assumed at this boundary.
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
            _link: &CAMetalDisplayLink,
            _update: &CAMetalDisplayLinkUpdate,
        ) {
            if self.ivars().lifecycle.load(Ordering::Acquire) == SURFACE_LIVE {
                self.ivars().callback_count.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
);

impl DisplayLinkDelegate {
    fn new(lifecycle: Arc<AtomicU8>, callback_count: Arc<AtomicU64>) -> Retained<Self> {
        let allocated = Self::alloc().set_ivars(DisplayLinkDelegateIvars {
            lifecycle,
            callback_count,
        });
        // SAFETY: The message is NSObject's parameterless init initializer and
        // the allocated object already contains fully initialized Rust ivars.
        unsafe { msg_send![super(allocated), init] }
    }
}

pub(crate) struct NativeSurface {
    extent: crate::SurfaceExtent,
    callback_count: Arc<AtomicU64>,
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
}

impl NativeSurface {
    #[allow(
        clippy::too_many_lines,
        reason = "the creation sequence keeps partial native ownership and rollback auditable"
    )]
    pub(crate) fn new(descriptor: &SurfaceDescriptor) -> Result<Self, SurfaceError> {
        let Some(main_thread) = MainThreadMarker::new() else {
            return Err(SurfaceError::NativeUnavailable {
                stage: SurfaceStage::MainThread,
            });
        };

        let application = NSApplication::sharedApplication(main_thread);
        let device = require_device(MTLCreateSystemDefaultDevice())?;

        let extent = descriptor.extent();
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

        let view = NSView::initWithFrame(NSView::alloc(main_thread), frame);
        let layer = CAMetalLayer::layer();
        layer.setDevice(Some(&device));
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
        view.setWantsLayer(true);
        view.setLayer(Some(&layer));
        window.setContentView(Some(&view));

        let (lifecycle, callback_count) = new_observer_state();
        let delegate =
            DisplayLinkDelegate::new(Arc::clone(&lifecycle), Arc::clone(&callback_count));
        let display_link =
            CAMetalDisplayLink::initWithMetalLayer(CAMetalDisplayLink::alloc(), &layer);
        display_link.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        display_link.setPaused(true);
        let run_loop = NSRunLoop::mainRunLoop();
        // SAFETY: Construction is admitted only with MainThreadMarker, this is
        // the process main run loop, and the common mode object is static for
        // the process lifetime. Drop invalidates the link before owners release.
        unsafe { display_link.addToRunLoop_forMode(&run_loop, NSRunLoopCommonModes) };

        Ok(Self {
            extent,
            callback_count,
            lifecycle,
            display_link,
            delegate,
            layer,
            view,
            window,
            device,
            application,
        })
    }

    pub(crate) fn show(&self) {
        self.window.makeKeyAndOrderFront(None);
    }

    pub(crate) fn snapshot(&self) -> SurfaceSnapshot {
        SurfaceSnapshot {
            physical_width: self.extent.physical_width(),
            physical_height: self.extent.physical_height(),
            framebuffer_only: self.layer.framebufferOnly(),
            display_sync_enabled: self.layer.displaySyncEnabled(),
            allows_next_drawable_timeout: self.layer.allowsNextDrawableTimeout(),
            maximum_drawable_count: u8::try_from(self.layer.maximumDrawableCount()).unwrap_or(0),
            display_link_paused: self.display_link.isPaused(),
            visible: self.window.isVisible(),
            callback_count: self.callback_count.load(Ordering::Acquire),
        }
    }

    pub(crate) fn observer(&self) -> SurfaceObserver {
        SurfaceObserver::new(
            Arc::clone(&self.lifecycle),
            Arc::clone(&self.callback_count),
        )
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
        self.display_link.setPaused(true);
        self.display_link.invalidate();
        self.display_link.setDelegate(None);
        self.window.orderOut(None);
        self.window.close();
        finish_close_observer_state(&self.lifecycle);
    }
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
        let delegate =
            DisplayLinkDelegate::new(Arc::clone(&lifecycle), Arc::clone(&callback_count));
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
}
