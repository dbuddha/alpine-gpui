//! Hosted structural qualification for native idle-state admission.

#[cfg(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    validation::run()
}

#[cfg(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64"))]
mod validation {
    use std::{
        error::Error,
        ffi::OsStr,
        time::{Duration, Instant},
    };

    use alpine_core::{LinearRgba, Point, Rect, Size};
    use alpine_platform_macos::{
        NativeSurface, SurfaceDescriptor, SurfaceSnapshot, native_validation,
    };
    use alpine_scene::{Primitive, SceneBuilder, SceneRevision};
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;
    use objc2_foundation::{NSDate, NSRunLoop};

    type TestResult<T = ()> = Result<T, Box<dyn Error>>;

    const TITLE: &str = "Alpine native idle qualification";
    const WIDTH: f32 = 320.0;
    const HEIGHT: f32 = 180.0;
    const SETTLEMENT: Duration = Duration::from_millis(150);

    pub(super) fn run() -> TestResult {
        let hosted_direct = hosted_direct_mode()?;
        let descriptor = SurfaceDescriptor::new(TITLE, f64::from(WIDTH), f64::from(HEIGHT), 1.0)?;
        let surface = native_validation::new_surface(&descriptor)?;
        surface.show()?;

        present(&surface, 1, hosted_direct)?;
        assert_quiescent(&surface, "visible")?;

        with_window(|window| window.orderOut(None))?;
        assert!(native_validation::inject_configuration_callback(&surface));
        assert_window_state(false, false)?;
        assert_quiescent(&surface, "hidden")?;

        surface.show()?;
        pump_main_run_loop(SETTLEMENT);
        assert!(native_validation::inject_configuration_callback(&surface));
        assert_window_state(true, false)?;
        present(&surface, 2, hosted_direct)?;

        with_window(|window| window.miniaturize(None))?;
        pump_main_run_loop(SETTLEMENT);
        assert!(native_validation::inject_configuration_callback(&surface));
        assert_window_state(false, true)?;
        assert_quiescent(&surface, "minimized")?;

        with_window(|window| window.deminiaturize(None))?;
        surface.show()?;
        pump_main_run_loop(SETTLEMENT);
        assert!(native_validation::inject_configuration_callback(&surface));
        assert_window_state(true, false)?;
        assert_quiescent(&surface, "restored-before-control")?;

        let before_control = surface.snapshot();
        present(&surface, 3, hosted_direct)?;
        let after_control = surface.snapshot();
        assert_eq!(
            after_control.submission_count(),
            before_control.submission_count() + 1,
            "the invalidation control must admit exactly one submission"
        );
        assert_eq!(
            after_control.direct_present_count(),
            before_control.direct_present_count() + 1,
            "the invalidation control must issue exactly one direct presentation"
        );
        assert_quiescent(&surface, "restored-control")?;

        let evidence = native_validation::close_with_owner_evidence(surface)?;
        assert_eq!(evidence.active(), [0; 10]);
        assert_eq!(evidence.release_order_violations(), 0);
        Ok(())
    }

    fn hosted_direct_mode() -> TestResult<bool> {
        match std::env::var_os("ALPINE_PRESENTATION_EVIDENCE_MODE") {
            None => Ok(false),
            Some(mode) if mode == OsStr::new("hosted-direct") => Ok(true),
            Some(_) => Err("unsupported presentation evidence mode".into()),
        }
    }

    fn present(surface: &NativeSurface, revision: u64, hosted_direct: bool) -> TestResult {
        let before = surface.snapshot();
        let before_pause_confirmations = native_validation::pause_confirmation_count(surface);
        let viewport = Size::new(WIDTH, HEIGHT).ok_or("valid idle viewport")?;
        let bounds = Rect::new(Point::new(0.0, 0.0).ok_or("valid idle origin")?, viewport);
        let color = LinearRgba::new(0.08, 0.12, 0.16, 1.0).ok_or("valid idle color")?;
        let mut builder = SceneBuilder::new(SceneRevision::new(revision), viewport);
        builder.push(Primitive::Quad { bounds, color });
        let clear = LinearRgba::new(0.0, 0.0, 0.0, 1.0).ok_or("valid idle clear")?;
        assert_eq!(
            surface.request_frame(builder.finish(), clear)?.get(),
            revision
        );
        if hosted_direct {
            // A hosted runner can supply a callback drawable without ever
            // reporting compositor presentation. The validation-only
            // observation terminalizes real submitted work so this test can
            // qualify idle ownership. Only the independently counted direct
            // present call is evidence; this observation is not physical
            // presentation evidence.
            let presented_time = 1.0 + f64::from(u32::try_from(revision)?);
            native_validation::inject_post_commit_observation(surface, None, presented_time)?;
        }
        native_validation::run_until_frame_terminal(surface, Duration::from_secs(30));
        if let Some(error) = surface.take_error()? {
            return Err(error.into());
        }
        let expected_pause_confirmations = before_pause_confirmations
            .checked_add(1)
            .ok_or("pause confirmation count exhausted")?;
        await_display_link_paused(surface, SETTLEMENT, expected_pause_confirmations)?;
        let snapshot = surface.snapshot();
        assert!(
            snapshot.submission_count() > before.submission_count(),
            "every setup revision must admit at least one submission"
        );
        assert!(
            snapshot.direct_present_count() > before.direct_present_count(),
            "every setup revision must issue at least one direct presentation"
        );
        assert_eq!(snapshot.occupied_frame_slots(), 0);
        assert_eq!(snapshot.submitted_frame_slots(), 0);
        assert!(snapshot.display_link_paused());
        assert_eq!(
            native_validation::pause_confirmation_count(surface),
            expected_pause_confirmations,
            "every setup revision must complete exactly one post-callback pause reaffirmation"
        );
        Ok(())
    }

    fn await_display_link_paused(
        surface: &NativeSurface,
        timeout: Duration,
        minimum_pause_confirmations: u64,
    ) -> TestResult {
        let before = surface.snapshot();
        let deadline = Instant::now() + timeout;
        while {
            let snapshot = surface.snapshot();
            (!snapshot.display_link_paused()
                || native_validation::pause_confirmation_count(surface)
                    < minimum_pause_confirmations)
                && Instant::now() < deadline
        } {
            pump_main_run_loop(Duration::from_millis(5));
        }
        let after = surface.snapshot();
        let pause_confirmations = native_validation::pause_confirmation_count(surface);
        assert!(
            after.display_link_paused(),
            "display link did not pause within the settlement bound: confirmations={pause_confirmations}, callbacks_before={}, callbacks_after={}",
            before.callback_count(),
            after.callback_count()
        );
        assert!(
            pause_confirmations >= minimum_pause_confirmations,
            "post-callback pause reaffirmation did not complete within the settlement bound: expected_at_least={minimum_pause_confirmations}, actual={pause_confirmations}, native_paused={}",
            after.display_link_paused()
        );
        assert_eq!(
            after.submission_count(),
            before.submission_count(),
            "pause observation admitted an extra submission"
        );
        assert_eq!(
            after.direct_present_count(),
            before.direct_present_count(),
            "pause observation issued an extra direct presentation"
        );
        assert_eq!(after.occupied_frame_slots(), 0);
        assert_eq!(after.submitted_frame_slots(), 0);
        Ok(())
    }

    fn assert_quiescent(surface: &NativeSurface, state: &str) -> TestResult {
        pump_main_run_loop(SETTLEMENT);
        let before = surface.snapshot();
        pump_main_run_loop(SETTLEMENT);
        let after = surface.snapshot();
        assert_unchanged(before, after, state);
        Ok(())
    }

    fn assert_unchanged(before: SurfaceSnapshot, after: SurfaceSnapshot, state: &str) {
        assert_eq!(
            after.callback_count(),
            before.callback_count(),
            "{state} callback drift"
        );
        assert_eq!(
            after.submission_count(),
            before.submission_count(),
            "{state} submission drift"
        );
        assert_eq!(
            after.direct_present_count(),
            before.direct_present_count(),
            "{state} direct-presentation drift"
        );
        assert_eq!(
            after.occupied_frame_slots(),
            0,
            "{state} occupied frame slots"
        );
        assert_eq!(
            after.submitted_frame_slots(),
            0,
            "{state} submitted frame slots"
        );
        assert!(
            after.display_link_paused(),
            "{state} display link must be paused"
        );
    }

    fn with_window(operation: impl FnOnce(&objc2_app_kit::NSWindow)) -> TestResult {
        let marker =
            MainThreadMarker::new().ok_or("native idle test must run on the main thread")?;
        let application = NSApplication::sharedApplication(marker);
        let windows = application.windows();
        let window = windows
            .iter()
            .find(|window| window.title().to_string() == TITLE)
            .ok_or("native idle AppKit window was not found")?;
        operation(&window);
        Ok(())
    }

    fn assert_window_state(visible: bool, miniaturized: bool) -> TestResult {
        with_window(|window| {
            assert_eq!(window.isVisible(), visible);
            assert_eq!(window.isMiniaturized(), miniaturized);
        })
    }

    fn pump_main_run_loop(duration: Duration) {
        let deadline = NSDate::dateWithTimeIntervalSinceNow(duration.as_secs_f64());
        NSRunLoop::mainRunLoop().runUntilDate(&deadline);
    }
}

#[cfg(not(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64")))]
fn main() {}
