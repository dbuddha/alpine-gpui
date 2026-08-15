use crate::{
    SurfaceDescriptor, SurfaceError, SurfaceObserver, SurfaceSnapshot, begin_close_observer_state,
    finish_close_observer_state, new_observer_state,
};
use alpine_core::LinearRgba;
use alpine_platform::PresentationRevision;
use alpine_scene::Scene;

pub(crate) struct NativeSurface;

impl NativeSurface {
    pub(crate) fn new(_descriptor: &SurfaceDescriptor) -> Result<Self, SurfaceError> {
        Err(SurfaceError::UnsupportedPlatform)
    }

    #[allow(
        clippy::unused_self,
        reason = "the unsupported implementation mirrors the native owner contract"
    )]
    pub(crate) const fn show(&self) -> Result<(), SurfaceError> {
        Err(SurfaceError::UnsupportedPlatform)
    }

    #[allow(
        clippy::unused_self,
        reason = "the unsupported implementation mirrors the native owner contract"
    )]
    pub(crate) fn request_frame(
        &self,
        _scene: Scene,
        _clear: LinearRgba,
    ) -> Result<PresentationRevision, SurfaceError> {
        Err(SurfaceError::UnsupportedPlatform)
    }

    #[allow(
        clippy::unused_self,
        reason = "the unsupported implementation mirrors the native owner contract"
    )]
    pub(crate) const fn take_error(&self) -> Result<Option<SurfaceError>, SurfaceError> {
        Err(SurfaceError::UnsupportedPlatform)
    }

    #[allow(
        clippy::unused_self,
        reason = "the unsupported implementation mirrors the native owner contract"
    )]
    pub(crate) const fn snapshot(&self) -> SurfaceSnapshot {
        SurfaceSnapshot {
            physical_width: 0,
            physical_height: 0,
            surface_epoch: 0,
            sized: false,
            presentation_visible: false,
            sdr_color_contract: None,
            extended_dynamic_range: false,
            framebuffer_only: false,
            display_sync_enabled: false,
            allows_next_drawable_timeout: false,
            maximum_drawable_count: 0,
            regular_activation_policy: false,
            display_link_paused: true,
            visible: false,
            callback_count: 0,
            submission_count: 0,
            direct_present_count: 0,
            installed_presented_handler_count: 0,
            presented_count: 0,
            qualified_presented_count: 0,
            superseded_count: 0,
            last_presented_time_bits: 0,
            skipped_count: 0,
            failed_count: 0,
            allocated_bytes: 0,
            current_retained_bytes: 0,
            last_terminal: None,
            last_superseded: None,
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
        assert_eq!(surface.show(), Err(SurfaceError::UnsupportedPlatform));
        assert_eq!(
            surface.snapshot(),
            SurfaceSnapshot {
                physical_width: 0,
                physical_height: 0,
                surface_epoch: 0,
                sized: false,
                presentation_visible: false,
                sdr_color_contract: None,
                extended_dynamic_range: false,
                framebuffer_only: false,
                display_sync_enabled: false,
                allows_next_drawable_timeout: false,
                maximum_drawable_count: 0,
                regular_activation_policy: false,
                display_link_paused: true,
                visible: false,
                callback_count: 0,
                submission_count: 0,
                direct_present_count: 0,
                installed_presented_handler_count: 0,
                presented_count: 0,
                qualified_presented_count: 0,
                superseded_count: 0,
                last_presented_time_bits: 0,
                skipped_count: 0,
                failed_count: 0,
                allocated_bytes: 0,
                current_retained_bytes: 0,
                last_terminal: None,
                last_superseded: None,
            }
        );
        assert_eq!(
            surface.observer().lifecycle(),
            crate::SurfaceLifecycle::Closed
        );
    }
}
