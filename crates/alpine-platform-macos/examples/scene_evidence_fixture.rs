//! Bounded native scene control. Screen-session approval is required to run.

#[cfg(any(test, alpine_native_validation))]
mod protocol {
    use std::io::Read;
    #[derive(Debug, PartialEq, Eq)]
    pub enum Command {
        Transition(u32, u64),
        Stop,
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
                for _ in 0..3 {
                    let command = protocol::read(&mut input, &mut transitioned);
                    let stop = !command.as_ref().is_ok_and(|c| *c != Command::Stop);
                    if tx.try_send(command).is_err() || stop {
                        break;
                    }
                }
            });
            while Instant::now() < deadline {
                native_validation::drain_run_loop(|| false, 1, Duration::from_millis(10));
                match rx.try_recv() {
                    Ok(Ok(Command::Transition(window, tick))) => {
                        present(&surface, &control, true, Some((window, tick)))?;
                    }
                    Ok(Ok(Command::Stop)) => return Ok(()),
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
