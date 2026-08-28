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
    use objc2::{MainThreadMarker, rc::Retained};
    use objc2_app_kit::NSApplication;
    use objc2_foundation::{NSDate, NSRunLoop};

    type TestResult<T = ()> = Result<T, Box<dyn Error>>;

    const TITLE: &str = "Alpine native idle qualification";
    const WIDTH: f32 = 320.0;
    const HEIGHT: f32 = 180.0;
    const SETTLEMENT: Duration = Duration::from_millis(150);
    const MAX_FRAME_ATTEMPTS: u64 = 3;

    #[derive(Clone, Copy)]
    struct NativeWindowConfiguration {
        logical_width: f64,
        logical_height: f64,
        scale: f64,
        display_identity: usize,
    }

    pub(super) fn run() -> TestResult {
        let hosted_direct = hosted_direct_mode()?;
        let descriptor = SurfaceDescriptor::new(TITLE, f64::from(WIDTH), f64::from(HEIGHT), 1.0)?;
        let surface = native_validation::new_surface(&descriptor)?;
        surface.show()?;
        pump_main_run_loop(SETTLEMENT);
        establish_visible_presentation(&surface, hosted_direct)?;
        assert_window_state(true, false)?;

        present(&surface, 1, hosted_direct)?;
        assert_quiescent(&surface, "visible")?;

        with_window(|window| window.orderOut(None))?;
        assert!(native_validation::inject_configuration_callback(&surface));
        assert_window_state(false, false)?;
        assert_quiescent(&surface, "hidden")?;

        surface.show()?;
        pump_main_run_loop(SETTLEMENT);
        establish_visible_presentation(&surface, hosted_direct)?;
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
        establish_visible_presentation(&surface, hosted_direct)?;
        assert_window_state(true, false)?;
        assert_quiescent(&surface, "restored-before-control")?;

        for revision in 3..=10 {
            present(&surface, revision, hosted_direct)?;
            assert_quiescent(&surface, &format!("restored-control-{revision}"))?;
        }

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

    fn establish_visible_presentation(surface: &NativeSurface, hosted_direct: bool) -> TestResult {
        assert!(native_validation::inject_configuration_callback(surface));
        if hosted_direct || !surface.snapshot().is_presentation_visible() {
            let configuration = native_window_configuration()?;
            native_validation::inject_surface_configuration(
                surface,
                configuration.logical_width,
                configuration.logical_height,
                configuration.scale,
                configuration.display_identity,
                true,
            )?;
        }
        assert!(
            surface.snapshot().is_presentation_visible(),
            "the hosted setup must establish portable presentation eligibility before requesting a frame"
        );
        Ok(())
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
        let terminal = drain_presented_frame(surface, before, revision, hosted_direct)?;
        let pause_evidence = native_validation::pause_confirmation_evidence(surface);
        assert!(
            terminal.callback_count() > before.callback_count(),
            "every setup revision must reach at least one display-link callback: before={before:?}, terminal={terminal:?}, pause_evidence={pause_evidence:?}"
        );
        let submission_delta = terminal
            .submission_count()
            .checked_sub(before.submission_count())
            .ok_or("submission counter regressed")?;
        let superseded_delta = terminal
            .superseded_count()
            .checked_sub(before.superseded_count())
            .ok_or("superseded counter regressed")?;
        let skipped_delta = terminal
            .skipped_count()
            .checked_sub(before.skipped_count())
            .ok_or("skipped counter regressed")?;
        let expected_submissions = superseded_delta
            .checked_add(skipped_delta)
            .ok_or("expected retry count exhausted")?
            .checked_add(1)
            .ok_or("expected submission count exhausted")?;
        assert_eq!(
            submission_delta, expected_submissions,
            "every setup revision must admit one final submission plus exactly one replacement for each superseded or skipped attempt: before={before:?}, terminal={terminal:?}, pause_evidence={pause_evidence:?}"
        );
        let direct_present_delta = terminal
            .direct_present_count()
            .checked_sub(before.direct_present_count())
            .ok_or("direct-present counter regressed")?;
        assert_eq!(
            direct_present_delta, submission_delta,
            "every admitted submission must issue exactly one direct presentation: before={before:?}, terminal={terminal:?}, pause_evidence={pause_evidence:?}"
        );
        assert_eq!(
            terminal
                .qualified_presented_count()
                .checked_sub(before.qualified_presented_count())
                .ok_or("qualified-presented counter regressed")?,
            1,
            "every setup revision must finish with exactly one current presented attempt: before={before:?}, terminal={terminal:?}, pause_evidence={pause_evidence:?}"
        );
        assert_eq!(
            terminal
                .failed_count()
                .checked_sub(before.failed_count())
                .ok_or("failed counter regressed")?,
            0,
            "a setup revision must not terminalize as failed"
        );
        assert_eq!(
            terminal
                .cancelled_count()
                .checked_sub(before.cancelled_count())
                .ok_or("cancelled counter regressed")?,
            0,
            "a setup revision must not terminalize as cancelled"
        );
        assert_eq!(
            terminal.occupied_frame_slots(),
            0,
            "terminal observation must include completion-owned frame-slot drain: before={before:?}, terminal={terminal:?}, pause_evidence={pause_evidence:?}"
        );
        assert_eq!(
            terminal.submitted_frame_slots(),
            0,
            "terminal observation must release every submitted frame slot: before={before:?}, terminal={terminal:?}, pause_evidence={pause_evidence:?}"
        );
        let expected_pause_confirmations = before_pause_confirmations
            .checked_add(1)
            .ok_or("pause confirmation count exhausted")?;
        await_display_link_paused(surface, SETTLEMENT, expected_pause_confirmations)?;
        let snapshot = surface.snapshot();
        assert!(snapshot.display_link_paused());
        assert_eq!(
            native_validation::pause_confirmation_count(surface),
            expected_pause_confirmations,
            "every setup revision must complete exactly one post-callback pause reaffirmation"
        );
        let pause_evidence = native_validation::pause_confirmation_evidence(surface);
        assert_eq!(pause_evidence.requested(), expected_pause_confirmations);
        assert_eq!(pause_evidence.enqueued(), expected_pause_confirmations);
        assert_eq!(pause_evidence.executed(), expected_pause_confirmations);
        assert_eq!(pause_evidence.eligible(), expected_pause_confirmations);
        assert_eq!(pause_evidence.observed(), expected_pause_confirmations);
        Ok(())
    }

    fn drain_presented_frame(
        surface: &NativeSurface,
        initial: SurfaceSnapshot,
        revision: u64,
        hosted_direct: bool,
    ) -> TestResult<SurfaceSnapshot> {
        let initial_terminals = terminal_outcome_count(initial)?;
        let mut armed_at_submission = None;
        for _ in 0..MAX_FRAME_ATTEMPTS {
            let before_drain = surface.snapshot();
            let submissions = before_drain
                .submission_count()
                .checked_sub(initial.submission_count())
                .ok_or("submission counter regressed during terminal drain")?;
            if submissions > MAX_FRAME_ATTEMPTS {
                return Err(format!(
                    "revision {revision} exceeded the bounded frame-attempt contract: initial={initial:?}, current={before_drain:?}"
                )
                .into());
            }
            if frame_drain_complete(initial, initial_terminals, before_drain)? {
                return Ok(before_drain);
            }
            if hosted_direct && armed_at_submission != Some(before_drain.submission_count()) {
                // Hosted runners can expose callback drawables without a
                // compositor observation. Arm one validation-only observation
                // for each distinct admitted submission, including a
                // replacement after native configuration supersedes an older
                // attempt. This remains non-physical evidence.
                let presented_time = 1.0 + f64::from(u32::try_from(revision)?);
                native_validation::inject_post_commit_observation(surface, None, presented_time)?;
                armed_at_submission = Some(before_drain.submission_count());
            }
            let terminals_before_drain = terminal_outcome_count(before_drain)?;
            native_validation::run_until_frame_terminal(surface, Duration::from_secs(30));
            if let Some(error) = surface.take_error()? {
                return Err(error.into());
            }
            let terminal = surface.snapshot();
            let terminals_after_drain = terminal_outcome_count(terminal)?;
            if frame_drain_complete(initial, initial_terminals, terminal)? {
                return Ok(terminal);
            }
            if terminals_after_drain <= terminals_before_drain {
                return Err(format!(
                    "revision {revision} made no terminal progress within the bounded drain: before={before_drain:?}, after={terminal:?}"
                )
                .into());
            }
        }
        Err(format!(
            "revision {revision} did not terminalize every bounded frame attempt: initial={initial:?}, current={:?}",
            surface.snapshot()
        )
        .into())
    }

    fn frame_drain_complete(
        initial: SurfaceSnapshot,
        initial_terminals: u64,
        current: SurfaceSnapshot,
    ) -> TestResult<bool> {
        let terminal_delta = terminal_outcome_count(current)?
            .checked_sub(initial_terminals)
            .ok_or("terminal counter regressed during frame drain")?;
        let submission_delta = current
            .submission_count()
            .checked_sub(initial.submission_count())
            .ok_or("submission counter regressed during frame drain")?;
        if terminal_delta > submission_delta {
            return Err(format!(
                "frame drain observed more terminal outcomes than submissions: initial={initial:?}, current={current:?}"
            )
            .into());
        }
        let qualified_delta = current
            .qualified_presented_count()
            .checked_sub(initial.qualified_presented_count())
            .ok_or("qualified-presented counter regressed during frame drain")?;
        Ok(submission_delta > 0
            && terminal_delta == submission_delta
            && qualified_delta > 0
            && current.occupied_frame_slots() == 0
            && current.submitted_frame_slots() == 0)
    }

    fn terminal_outcome_count(snapshot: SurfaceSnapshot) -> TestResult<u64> {
        snapshot
            .qualified_presented_count()
            .checked_add(snapshot.superseded_count())
            .and_then(|count| count.checked_add(snapshot.cancelled_count()))
            .and_then(|count| count.checked_add(snapshot.skipped_count()))
            .ok_or_else(|| "terminal outcome count exhausted".into())
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
        let pause_evidence = native_validation::pause_confirmation_evidence(surface);
        let pause_confirmations = pause_evidence.observed();
        assert!(
            after.display_link_paused(),
            "display link did not pause within the settlement bound: pause_evidence={pause_evidence:?}, callbacks_before={}, callbacks_after={}, before={before:?}, after={after:?}",
            before.callback_count(),
            after.callback_count()
        );
        assert!(
            pause_confirmations >= minimum_pause_confirmations,
            "post-callback pause reaffirmation did not complete within the settlement bound: expected_at_least={minimum_pause_confirmations}, pause_evidence={pause_evidence:?}, native_paused={}",
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

    fn native_window_configuration() -> TestResult<NativeWindowConfiguration> {
        let mut configuration = None;
        with_window(|window| {
            let (Some(view), Some(screen)) = (window.contentView(), window.screen()) else {
                return;
            };
            let size = view.bounds().size;
            configuration = Some(NativeWindowConfiguration {
                logical_width: size.width,
                logical_height: size.height,
                scale: window.backingScaleFactor(),
                display_identity: Retained::as_ptr(&screen) as usize,
            });
        })?;
        configuration.ok_or_else(|| "native window configuration was unavailable".into())
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
