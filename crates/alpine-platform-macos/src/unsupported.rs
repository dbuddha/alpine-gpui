use crate::{
    SurfaceDescriptor, SurfaceError, SurfaceObserver, SurfaceSnapshot, begin_close_observer_state,
    finish_close_observer_state, new_observer_state,
};

pub(crate) struct NativeSurface;

impl NativeSurface {
    pub(crate) fn new(_descriptor: &SurfaceDescriptor) -> Result<Self, SurfaceError> {
        Err(SurfaceError::UnsupportedPlatform)
    }

    #[allow(
        clippy::unused_self,
        reason = "the unsupported implementation mirrors the native owner contract"
    )]
    pub(crate) const fn show(&self) {}

    #[allow(
        clippy::unused_self,
        reason = "the unsupported implementation mirrors the native owner contract"
    )]
    pub(crate) const fn snapshot(&self) -> SurfaceSnapshot {
        SurfaceSnapshot {
            physical_width: 0,
            physical_height: 0,
            framebuffer_only: false,
            display_sync_enabled: false,
            allows_next_drawable_timeout: false,
            maximum_drawable_count: 0,
            display_link_paused: true,
            visible: false,
            callback_count: 0,
        }
    }

    #[allow(
        clippy::unused_self,
        reason = "the unsupported implementation mirrors the native owner contract"
    )]
    pub(crate) fn observer(&self) -> SurfaceObserver {
        let (lifecycle, callback_count) = new_observer_state();
        begin_close_observer_state(&lifecycle);
        finish_close_observer_state(&lifecycle);
        SurfaceObserver::new(lifecycle, callback_count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_construction_is_rejected_without_side_effects() -> Result<(), SurfaceError> {
        let descriptor = SurfaceDescriptor::new("Alpine", 640.0, 480.0, 1.0)?;
        assert!(matches!(
            NativeSurface::new(&descriptor),
            Err(SurfaceError::UnsupportedPlatform)
        ));
        Ok(())
    }

    #[test]
    fn inert_snapshot_discloses_no_native_configuration() {
        let surface = NativeSurface;
        surface.show();
        assert_eq!(
            surface.snapshot(),
            SurfaceSnapshot {
                physical_width: 0,
                physical_height: 0,
                framebuffer_only: false,
                display_sync_enabled: false,
                allows_next_drawable_timeout: false,
                maximum_drawable_count: 0,
                display_link_paused: true,
                visible: false,
                callback_count: 0,
            }
        );
        assert_eq!(
            surface.observer().lifecycle(),
            crate::SurfaceLifecycle::Closed
        );
    }
}
