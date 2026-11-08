//! Bounded native scene control. Screen-session approval is required to run.

#[cfg(any(test, alpine_native_validation))]
mod protocol {
    use std::io::Read;
    #[derive(Debug, PartialEq, Eq)]
    pub enum Command {
        Transition(u32, u64),
        Stop,
        InspectStop(Vec<u32>),
        Sharing(u8, u32, u32),
    }
    pub fn read(reader: &mut impl Read, transitioned: &mut bool) -> Result<Command, String> {
        let mut line = Vec::with_capacity(96);
        for byte in reader.bytes().take(97) {
            let byte = byte.map_err(|e| e.to_string())?;
            if byte == b'\n' {
                let text = std::str::from_utf8(&line).map_err(|e| e.to_string())?;
                if text == "stop" {
                    return Ok(Command::Stop);
                }
                let parts: Vec<_> = text.split(' ').collect();
                if let ["sharing", seq, main, child] = parts.as_slice() {
                    let seq: u8 = seq.parse().map_err(|_| "bad inspection sequence")?;
                    let main: u32 = main.parse().map_err(|_| "bad main ID")?;
                    let child: u32 = child.parse().map_err(|_| "bad child ID")?;
                    if !(1..=32).contains(&seq) || main == 0 || child == 0 || main == child {
                        return Err("invalid sharing inspection request".into());
                    }
                    return Ok(Command::Sharing(seq, main, child));
                }
                if parts[0] == "inspect-stop" {
                    if parts.len() > 5 {
                        return Err("inspection exceeds four window IDs".into());
                    }
                    let mut ids = Vec::with_capacity(4);
                    for raw in &parts[1..] {
                        let id = raw.parse::<u32>().map_err(|e| e.to_string())?;
                        if id == 0 || ids.contains(&id) {
                            return Err("invalid inspection window ID".into());
                        }
                        ids.push(id);
                    }
                    return Ok(Command::InspectStop(ids));
                }
                if parts.len() != 3 || parts[0] != "transition" || *transitioned {
                    return Err("expected one transition WINDOW_ID MACH_TICK, then stop".into());
                }
                let window = parts[1].parse::<u32>().map_err(|e| e.to_string())?;
                let tick = parts[2].parse::<u64>().map_err(|e| e.to_string())?;
                if window == 0 || tick == 0 {
                    return Err("zero action identity or tick".into());
                }
                *transitioned = true;
                return Ok(Command::Transition(window, tick));
            }
            line.push(byte);
        }
        Err("command truncated or exceeds 96 bytes".into())
    }
    pub fn levels(control: &str, transitioned: bool) -> Result<[f32; 5], String> {
        let mut levels = [0.0, 0.18, 0.5, 0.75, 1.0];
        match (control, transitioned) {
            ("positive", true) => levels.reverse(),
            ("wrong", true) => levels = [0.5; 5],
            ("positive" | "stale" | "wrong", _) => (),
            _ => return Err("expected positive, stale or wrong control".into()),
        }
        Ok(levels)
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn bounded_single_action() -> Result<(), String> {
            let mut state = false;
            assert_eq!(
                read(&mut &b"transition 4 5\n"[..], &mut state)?,
                Command::Transition(4, 5)
            );
            assert!(read(&mut &b"transition 4 6\n"[..], &mut state).is_err());
            assert_eq!(read(&mut &b"stop\n"[..], &mut state)?, Command::Stop);
            for line in [
                "transition 0 5\n",
                "transition 4 0\n",
                "transition 4 5",
                "transition 4 18446744073709551616\n",
                "stop extra\n",
            ] {
                assert!(read(&mut line.as_bytes(), &mut false).is_err());
            }
            assert!(read(&mut &[b'a'; 98][..], &mut false).is_err());
            Ok(())
        }
        #[test]
        fn inspection_is_bounded_and_available_before_or_after_action() -> Result<(), String> {
            for mut changed in [false, true] {
                assert_eq!(
                    read(&mut &b"inspect-stop 12 34\n"[..], &mut changed)?,
                    Command::InspectStop(vec![12, 34])
                );
            }
            assert_eq!(
                read(&mut &b"inspect-stop\n"[..], &mut false)?,
                Command::InspectStop(Vec::new())
            );
            for bad in [
                "inspect-stop 0\n",
                "inspect-stop 4 4\n",
                "inspect-stop 1 2 3 4 5\n",
                "inspect-stop 4294967296\n",
                "inspect-stop 4",
            ] {
                assert!(read(&mut bad.as_bytes(), &mut false).is_err());
            }
            Ok(())
        }
        #[test]
        fn repeated_sharing_requests_allow_transition_and_termination() -> Result<(), String> {
            let mut input = &b"sharing 1 12 34\ntransition 12 99\nsharing 2 12 34\nstop\n"[..];
            let mut changed = false;
            for expected in [
                Command::Sharing(1, 12, 34),
                Command::Transition(12, 99),
                Command::Sharing(2, 12, 34),
                Command::Stop,
            ] {
                assert_eq!(read(&mut input, &mut changed)?, expected);
            }
            for invalid in [
                "sharing 33 12 34\n",
                "sharing 0 12 34\n",
                "sharing 1 12 12\n",
                "sharing 1 0 34\n",
                "sharing 1 12 0\n",
            ] {
                assert!(read(&mut invalid.as_bytes(), &mut false).is_err());
            }
            Ok(())
        }
        #[test]
        fn independent_controls() -> Result<(), String> {
            let baseline = levels("positive", false)?;
            for name in ["positive", "stale", "wrong"] {
                assert_eq!(levels(name, false)?, baseline);
            }
            assert_eq!(levels("positive", true)?, [1.0, 0.75, 0.5, 0.18, 0.0]);
            assert_eq!(levels("stale", true)?, baseline);
            assert_eq!(levels("wrong", true)?, [0.5; 5]);
            assert!(levels("unknown", true).is_err());
            Ok(())
        }
    }
}

#[cfg(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64"))]
mod native {
    use super::protocol::{self, Command};
    use alpine_core::{LinearRgba, Point, Rect, Size};
    use alpine_platform_macos::{NativeSurface, SurfaceDescriptor, native_validation};
    use alpine_scene::{Primitive, Scene, SceneBuilder, SceneRevision};
    use std::{
        io::Write as _,
        sync::mpsc,
        time::{Duration, Instant},
    };
    type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
    fn inspect_windows(ids: &[u32]) -> Result {
        let mtm = objc2::MainThreadMarker::new().ok_or("inspection requires main thread")?;
        let app = objc2_app_kit::NSApplication::sharedApplication(mtm);
        let mut resolved = 0;
        for id in ids {
            let Some(window) = app.windowWithWindowNumber(isize::try_from(*id)?) else {
                println!("scene-inspect\tcg_id={id}\tstatus=unresolved");
                continue;
            };
            let number = window.windowNumber();
            if u32::try_from(number) != Ok(*id) {
                println!("scene-inspect\tcg_id={id}\tstatus=unresolved\tappkit_id={number}");
                continue;
            }
            let object: &objc2::runtime::AnyObject = &window;
            let bytes = object.class().name().to_bytes();
            let class = if bytes.len() <= 96 && bytes.iter().all(u8::is_ascii_graphic) {
                std::str::from_utf8(bytes)?
            } else {
                "unresolved"
            };
            let parent = window.parentWindow().map(|parent| parent.windowNumber());
            let frame = window.frame();
            let content = window
                .contentView()
                .map(|view| (view.frame(), view.bounds()));
            println!(
                "scene-inspect\tcg_id={id}\tappkit_id={number}\tclass={class}\tparent_id={parent:?}\tframe_appkit_screen={frame:?}\tcontent_frame_and_bounds_local={content:?}"
            );
            resolved += 1;
        }
        println!(
            "scene-inspect-end\trequested={}\tresolved={resolved}",
            ids.len()
        );
        std::io::stdout().flush()?;
        Ok(())
    }
    fn cg_rect(rect: objc2_foundation::NSRect, primary_height: f64) -> String {
        format!(
            "{},{},{},{}",
            rect.origin.x,
            primary_height - (rect.origin.y + rect.size.height),
            rect.size.width,
            rect.size.height
        )
    }
    #[cfg(test)]
    mod geometry_tests {
        #[test]
        fn observed_appkit_frames_convert_to_cg_global_points() {
            use objc2_foundation::{NSPoint, NSRect, NSSize};
            for (origin, size, expected) in [
                ((506.0, 308.0), (500.0, 332.0), "506,342,500,332"),
                ((512.0, 614.0), (66.0, 20.0), "512,348,66,20"),
                ((506.0, 308.0), (500.0, 300.0), "506,374,500,300"),
            ] {
                assert_eq!(
                    super::cg_rect(
                        NSRect::new(
                            NSPoint::new(origin.0, origin.1),
                            NSSize::new(size.0, size.1)
                        ),
                        982.0
                    ),
                    expected
                );
            }
        }
    }
    fn sharing_geometry(main: u32, child: u32) -> Result<String> {
        let mtm = objc2::MainThreadMarker::new().ok_or("main thread required")?;
        let app = objc2_app_kit::NSApplication::sharedApplication(mtm);
        let lookup = |id| -> Result<_> {
            let window = app
                .windowWithWindowNumber(isize::try_from(id)?)
                .ok_or("unresolved window")?;
            if u32::try_from(window.windowNumber())? != id {
                return Err("window identity mismatch".into());
            }
            Ok(window)
        };
        let main = lookup(main)?;
        let child = lookup(child)?;
        let view = main.contentView().ok_or("unresolved content view")?;
        let content = main.convertRectToScreen(view.convertRect_toView(view.bounds(), None));
        let screen = objc2_app_kit::NSScreen::screens(mtm)
            .firstObject()
            .ok_or("missing primary screen")?
            .frame();
        if screen.origin.x != 0.0
            || screen.origin.y != 0.0
            || !screen.size.height.is_finite()
            || !(1.0..=65536.0).contains(&screen.size.height)
        {
            return Err("unsupported primary coordinate system".into());
        }
        let object: &objc2::runtime::AnyObject = &child;
        let class = object.class().name().to_bytes();
        if class.len() > 96 || !class.iter().all(u8::is_ascii_graphic) {
            return Err("unresolved class".into());
        }
        Ok(format!(
            "{}\t{}\t{}\t{}\t{}",
            std::str::from_utf8(class)?,
            child
                .parentWindow()
                .ok_or("unresolved parent")?
                .windowNumber(),
            cg_rect(main.frame(), screen.size.height),
            cg_rect(content, screen.size.height),
            cg_rect(child.frame(), screen.size.height)
        ))
    }
    fn chart(control: &str, changed: bool) -> Result<Scene> {
        let mut builder = SceneBuilder::new(
            SceneRevision::new(if changed { 2 } else { 1 }),
            Size::new(500.0, 300.0).ok_or("size")?,
        );
        for (index, level) in protocol::levels(control, changed)?.into_iter().enumerate() {
            builder.push(Primitive::Quad {
                bounds: Rect::new(
                    Point::new(f32::from(u16::try_from(index)?) * 100.0, 0.0).ok_or("origin")?,
                    Size::new(100.0, 300.0).ok_or("patch")?,
                ),
                color: LinearRgba::new(level, level, level, 1.0).ok_or("color")?,
            });
        }
        Ok(builder.finish())
    }
    fn present(
        surface: &NativeSurface,
        control: &str,
        changed: bool,
        action: Option<(u32, u64)>,
    ) -> Result {
        let previous = surface.snapshot();
        let before = previous.submission_count();
        surface.request_frame(
            chart(control, changed)?,
            LinearRgba::new(0.0, 0.0, 0.0, 1.0).ok_or("clear")?,
        )?;
        if let Some((window, tick)) = action {
            println!(
                "scene-action\talpine-scene-diagnostic/v1\t{}\t{window}\t{tick}\tA\tB",
                std::process::id()
            );
            std::io::stdout().flush()?;
        }
        native_validation::run_until_frame_terminal(surface, Duration::from_secs(3));
        let s = surface.snapshot();
        let error = surface.take_error()?;
        println!(
            "scene-native\tchanged={changed}\tnew_submissions={}\tqualified={}\tpresented_bits={}\toccupied={}\tsubmitted={}\tretained={}\tpaused={}\tterminal={:?}\terror={:?}",
            s.submission_count().saturating_sub(before),
            s.qualified_presented_count(),
            s.last_presented_time_bits(),
            s.occupied_frame_slots(),
            s.submitted_frame_slots(),
            s.current_retained_bytes(),
            s.display_link_paused(),
            s.last_terminal(),
            error
        );
        std::io::stdout().flush()?;
        if let Some(error) = error {
            return Err(error.into());
        }
        let terminal = s.last_terminal().ok_or("missing terminal outcome")?;
        if terminal.requested_revision().get() != if changed { 2 } else { 1 }
            || previous
                .last_terminal()
                .is_some_and(|old| old.attempt() >= terminal.attempt())
        {
            return Err("terminal did not advance to requested scene".into());
        }
        if s.submission_count() != before + 1
            || s.occupied_frame_slots() != 0
            || s.submitted_frame_slots() != 0
            || s.current_retained_bytes() != 0
            || !s.display_link_paused()
        {
            return Err("native submission or drain failed; see scene-native".into());
        }
        Ok(())
    }
    pub fn run() -> Result {
        let revision =
            option_env!("ALPINE_SCENE_REVISION").ok_or("build requires ALPINE_SCENE_REVISION")?;
        if revision.len() != 40 || !revision.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("invalid fixture revision stamp".into());
        }
        println!("scene-build\t{revision}");
        let control = std::env::args()
            .nth(1)
            .ok_or("positive|stale|wrong required")?;
        protocol::levels(&control, false)?;
        let deadline = Instant::now() + Duration::from_secs(30);
        let surface = native_validation::new_surface(&SurfaceDescriptor::new(
            "Alpine scene diagnostic",
            500.0,
            300.0,
            1.0,
        )?)?;
        let result = (|| -> Result {
            let config = native_validation::move_window_to_screen(&surface, 0)?;
            surface.show()?;
            present(&surface, &control, false, None)?;
            let s = surface.snapshot();
            println!(
                "scene-ready\t{}\t500\t300\t{}\t{}\t{}",
                std::process::id(),
                config.backing_scale,
                s.physical_width(),
                s.physical_height()
            );
            std::io::stdout().flush()?;
            let (tx, rx) = mpsc::sync_channel(2);
            std::thread::spawn(move || {
                let mut transitioned = false;
                let mut input = std::io::stdin().lock();
                for _ in 0..34 {
                    let command = protocol::read(&mut input, &mut transitioned);
                    let stop = !matches!(
                        command,
                        Ok(Command::Transition(_, _) | Command::Sharing(_, _, _))
                    );
                    if tx.try_send(command).is_err() || stop {
                        break;
                    }
                }
            });
            let mut next_inspection = 1;
            while Instant::now() < deadline {
                native_validation::drain_run_loop(|| false, 1, Duration::from_millis(10));
                match rx.try_recv() {
                    Ok(Ok(Command::Transition(window, tick))) => {
                        present(&surface, &control, true, Some((window, tick)))?;
                    }
                    Ok(Ok(Command::Sharing(seq, main, child))) => {
                        if seq != next_inspection {
                            return Err("unexpected inspection sequence".into());
                        }
                        next_inspection += 1;
                        let proof =
                            sharing_geometry(main, child).unwrap_or_else(|_| "unresolved".into());
                        println!(
                            "scene-sharing\t{seq}\t{}\t{main}\t{child}\t{proof}",
                            std::process::id()
                        );
                        std::io::stdout().flush()?;
                    }
                    Ok(Ok(Command::Stop)) => return Ok(()),
                    Ok(Ok(Command::InspectStop(ids))) => return inspect_windows(&ids),
                    Ok(Err(error)) => return Err(error.into()),
                    Err(mpsc::TryRecvError::Disconnected) => {
                        return Err("command stream ended before stop".into());
                    }
                    Err(mpsc::TryRecvError::Empty) => (),
                }
            }
            Err("fixture absolute lifetime expired".into())
        })();
        let owners = native_validation::close_with_owner_evidence(surface)?;
        let counts = [1, 1, 1, 1, 1, 1, 1, 1, 1, 0];
        let exact = owners.acquired() == counts
            && owners.released() == counts
            && owners.active() == [0; 10]
            && owners.run_loop_registrations() == 1
            && owners.link_invalidations() == 1
            && owners.early_link_invalidations() == 0
            && owners.delegate_revocations() == 1
            && owners.window_closes() == 1
            && owners.pasteboard_releases() == 0
            && owners.release_order_violations() == 0;
        println!("scene-cleanup\texact={exact}\t{owners:?}");
        if !exact {
            return Err("owner teardown mismatch".into());
        }
        result
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64"))]
    return native::run();
    #[cfg(not(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64")))]
    Err("requires --cfg alpine_native_validation on Apple Silicon macOS".into())
}
