//! Bounded scene diagnostics, never physical qualification or latency proof.

use std::{fs::File, io::Read as _, path::Path};

const SCHEMA: &str = "alpine-scene-diagnostic/v1";
const MAX_BYTES: u64 = 2 * 1024 * 1024;
const MAX_FRAMES: u64 = 2048;
const SAMPLES: usize = 135;
const EXPECTED: [u8; 5] = [0, 118, 188, 225, 255];
const TOLERANCE: u8 = 12;

fn read_bounded(path: &Path, limit: u64) -> Result<String, String> {
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|error| format!("{}: {error}", path.display()))?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    if bytes.len() as u64 > limit {
        return Err(format!("{} exceeds {limit} bytes", path.display()));
    }
    String::from_utf8(bytes).map_err(|error| format!("{}: {error}", path.display()))
}

fn fields(line: &str, expected: usize) -> Result<Vec<&str>, String> {
    if line.len() > 1024 {
        return Err("diagnostic record exceeds 1024 bytes".to_owned());
    }
    let fields: Vec<_> = line.split('\t').collect();
    if fields.len() != expected {
        return Err("wrong diagnostic field count".to_owned());
    }
    Ok(fields)
}

fn number(text: &str) -> Result<u64, String> {
    text.parse()
        .map_err(|_| format!("invalid integer {text:?}"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    A,
    B,
    Invalid,
}

fn classify(samples: &[u8; SAMPLES]) -> Result<State, String> {
    for (state, reversed) in [(State::A, false), (State::B, true)] {
        if samples.iter().enumerate().all(|(index, value)| {
            let patch = index / 27;
            let expected = EXPECTED[if reversed { 4 - patch } else { patch }];
            value.abs_diff(expected) <= TOLERANCE
        }) {
            return Ok(state);
        }
    }
    Err("raw samples do not match scene A or B".to_owned())
}

#[derive(Clone, Copy)]
struct Source {
    pid: u64,
    window: u64,
    start: u64,
    width: u64,
    height: u64,
    scale: f64,
}

fn source(line: &str) -> Result<Source, String> {
    let f = fields(line, 17)?;
    if f[0] != "scene-source" || f[1] != SCHEMA || f[16] != "srgb" {
        return Err("missing diagnostic scene source/version".to_owned());
    }
    let width = number(f[4])?;
    let height = number(f[5])?;
    let scale: f64 = f[6].parse().map_err(|_| "invalid scale")?;
    let region_x = number(f[7])?;
    let region_y = number(f[8])?;
    let region_width = number(f[9])?;
    let region_height = number(f[10])?;
    if width == 0
        || height == 0
        || width > 4096
        || height > 4096
        || width * height > 4_194_304
        || !scale.is_finite()
        || !(1.0..=4.0).contains(&scale)
        || region_width < 50
        || region_height < 12
        || region_x
            .checked_add(region_width)
            .is_none_or(|end| end > width)
        || region_y
            .checked_add(region_height)
            .is_none_or(|end| end > height)
    {
        return Err("invalid diagnostic scene geometry".to_owned());
    }
    let source = Source {
        pid: number(f[2])?,
        window: number(f[3])?,
        start: number(f[11])?,
        width,
        height,
        scale,
    };
    if source.pid == 0
        || source.pid > i32::MAX as u64
        || source.window == 0
        || source.window > u64::from(u32::MAX)
        || source.start == 0
        || number(f[12])? == 0
        || number(f[13])? == 0
        || number(f[14])? != MAX_FRAMES
        || !(1..=10_000).contains(&number(f[15])?)
    {
        return Err("invalid diagnostic source identity, clock or bounds".to_owned());
    }
    Ok(source)
}

fn action(text: &str, source: Source) -> Result<u64, String> {
    if text.len() > 256 || text.lines().count() != 1 {
        return Err("exactly one bounded action record is required".to_owned());
    }
    let f = fields(text.lines().next().ok_or("missing action")?, 7)?;
    if f[0] != "scene-action"
        || f[1] != SCHEMA
        || number(f[2])? != source.pid
        || number(f[3])? != source.window
        || f[5] != "A"
        || f[6] != "B"
    {
        return Err("action identity or A-to-B transition does not match source".to_owned());
    }
    let tick = number(f[4])?;
    if tick <= source.start {
        return Err("action must follow capture start".to_owned());
    }
    Ok(tick)
}

fn samples(text: &str) -> Result<[u8; SAMPLES], String> {
    let mut values = [0; SAMPLES];
    let mut count = 0;
    for part in text.split(',') {
        let slot = values.get_mut(count).ok_or("too many raw scene samples")?;
        *slot = part.parse().map_err(|_| "invalid raw scene sample")?;
        count += 1;
    }
    if count != SAMPLES {
        return Err("incomplete raw scene samples".to_owned());
    }
    Ok(values)
}

#[derive(Debug, PartialEq, Eq)]
struct Observation {
    frames: u64,
    first_b: u64,
}

fn transform(f: &[&str], source: Source) -> Result<Vec<f64>, String> {
    let transform: Vec<f64> = f[6..]
        .iter()
        .map(|v| v.parse().map_err(|_| "invalid transform"))
        .collect::<Result<_, _>>()?;
    if transform.iter().any(|v| !v.is_finite())
        || transform[0] != 0.0
        || transform[1] != 0.0
        || transform[4] <= 0.0
        || !(1.0..=4.0).contains(&transform[5])
        || (transform[4] * transform[5] - source.scale).abs() > 0.000_001
        || (transform[2] * transform[5] - crate::stats::to_f64(source.width)).abs() > 0.000_001
        || (transform[3] * transform[5] - crate::stats::to_f64(source.height)).abs() > 0.000_001
    {
        return Err("unsupported diagnostic content transform".to_owned());
    }
    Ok(transform)
}

fn frame_state(f: &[&str], control: Option<&str>) -> Result<State, String> {
    let raw = samples(f[5])?;
    let state = match classify(&raw) {
        Ok(state) => state,
        Err(_) if control == Some("wrong") && raw.iter().all(|v| v.abs_diff(188) <= TOLERANCE) => {
            State::Invalid
        }
        Err(error) => return Err(error),
    };
    if f[4]
        != match state {
            State::A => "A",
            State::B => "B",
            State::Invalid => "invalid",
        }
    {
        return Err("reported scene disagrees with raw samples".to_owned());
    }
    Ok(state)
}

fn inspect(
    capture: &str,
    actions: Option<&str>,
    control: Option<&str>,
    witness: Option<u64>,
) -> Result<Observation, String> {
    if capture.len() as u64 > MAX_BYTES {
        return Err("diagnostic capture byte cap exceeded".to_owned());
    }
    let mut lines = capture.lines();
    let source = source(lines.next().ok_or("empty diagnostic capture")?)?;
    let action = actions.map_or(Ok(u64::MAX), |text| action(text, source))?;
    let mut frames = 0;
    let mut display = 0;
    let mut arrival = source.start;
    let mut baseline = false;
    let mut first_b = None;
    let mut wrong = false;
    let mut ended = false;
    let mut previous_transform: Option<Vec<f64>> = None;
    for line in lines {
        if ended {
            return Err("record after diagnostic end".to_owned());
        }
        if line.starts_with("scene-end\t") {
            let f = fields(line, 3)?;
            if number(f[1])? < arrival || number(f[1])? <= action || number(f[2])? != frames {
                return Err("invalid diagnostic end time or frame count".to_owned());
            }
            ended = true;
            continue;
        }
        let f = fields(line, 12)?;
        let transform = transform(&f, source)?;
        if previous_transform
            .as_ref()
            .is_some_and(|previous| *previous != transform)
        {
            return Err("diagnostic content transform changed".to_owned());
        }
        previous_transform = Some(transform);
        if f[0] != "scene-frame" || frames >= MAX_FRAMES || number(f[1])? != frames + 1 {
            return Err("invalid, duplicate or excessive diagnostic frame".to_owned());
        }
        let next_display = number(f[2])?;
        let next_arrival = number(f[3])?;
        if next_display <= display || next_arrival <= arrival || next_arrival < next_display {
            return Err("zero, regressing or inconsistent diagnostic timestamps".to_owned());
        }
        let state = frame_state(&f, control)?;
        match state {
            State::Invalid if !baseline || next_display <= action => {
                return Err("wrong scene before action/baseline".into());
            }
            State::Invalid => wrong = true,
            State::A if first_b.is_some() || wrong => {
                return Err("scene regressed after B".to_owned());
            }
            State::A => baseline |= next_arrival < action,
            State::B if next_display <= action || !baseline => {
                return Err("B preceded the action or a captured A baseline".to_owned());
            }
            State::B => {
                first_b.get_or_insert(next_display);
            }
        }
        frames += 1;
        display = next_display;
        arrival = next_arrival;
    }
    if actions.is_some() && !ended {
        return Err("diagnostic capture has no terminal record".to_owned());
    }
    if !baseline {
        return Err("no captured A baseline".into());
    }
    if control == Some("stale") && witness.is_none_or(|after| after <= action || display <= after) {
        return Err("stale control needs A displayed after the admission witness".into());
    }
    if let Some(control) = control
        && (first_b.is_some()
            || (control == "wrong" && !wrong)
            || (control != "wrong" && control != "stale"))
    {
        return Err("unexpected negative-control scene outcome".into());
    }
    Ok(Observation {
        frames,
        first_b: if actions.is_some() && control.is_none() {
            first_b.ok_or("no diagnostic A-to-B transition")?
        } else {
            0
        },
    })
}

fn replay(capture: &str, actions: &str) -> Result<Observation, String> {
    inspect(capture, Some(actions), None, None)
}

/// Validates complete baseline records and the supervisor's window mapping.
#[allow(
    clippy::float_cmp,
    reason = "integer window geometry must match exactly"
)]
pub fn baseline(
    capture: &str,
    pid: u32,
    window: u32,
    width: f64,
    height: f64,
    region_x: f64,
    region_y: f64,
) -> Result<(), String> {
    let complete = capture.rsplit_once('\n').map_or("", |(prefix, _)| prefix);
    let first = complete.lines().next().ok_or("waiting for source")?;
    let source = source(first)?;
    let f = fields(first, 17)?;
    if source.pid != u64::from(pid)
        || source.window != u64::from(window)
        || crate::stats::to_f64(source.width) != width
        || crate::stats::to_f64(source.height) != height
        || source.scale != 1.0
        || f[7].parse::<f64>().map_err(|e| e.to_string())? != region_x
        || f[8].parse::<f64>().map_err(|e| e.to_string())? != region_y
        || f[9] != "500"
        || f[10] != "40"
    {
        return Err("capture differs from verified fixture mapping".into());
    }
    inspect(complete, None, None, None).map(|_| ())
}

/// Requires the command clock to use the capture timebase.
pub fn clock_matches(
    capture: &str,
    tick: u64,
    timebase: crate::helpers::Timebase,
) -> Result<(), String> {
    let line = capture.lines().next().ok_or("missing capture clock")?;
    let source = source(line)?;
    let f = fields(line, 17)?;
    if tick <= source.start || number(f[12])? != timebase.numer || number(f[13])? != timebase.denom
    {
        return Err("action clock differs from capture clock".into());
    }
    Ok(())
}

/// Validates the whole negative trace before accepting a semantic control.
pub fn negative(
    capture: &Path,
    actions: &Path,
    control: &str,
    witness: Option<u64>,
) -> Result<(), String> {
    inspect(
        &read_bounded(capture, MAX_BYTES)?,
        Some(&read_bounded(actions, 256)?),
        Some(control),
        witness,
    )
    .map(|_| ())
}

/// Checks trace consistency only; the action file is not independent proof.
pub fn run(capture: &Path, actions: &Path) -> Result<String, String> {
    let observation = replay(
        &read_bounded(capture, MAX_BYTES)?,
        &read_bounded(actions, 256)?,
    )?;
    Ok(format!(
        "diagnostic scene consistency only: {} frames, first B display tick {}; \
         physical qualification=false; typing qualification=false; action provenance unverified",
        observation.frames, observation.first_b
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: &str = "scene-source\talpine-scene-diagnostic/v1\t123\t456\t100\t40\t1\t0\t0\t100\t40\t10\t1\t1\t2048\t1000\tsrgb\n";
    const ACTION: &str = "scene-action\talpine-scene-diagnostic/v1\t123\t456\t30\tA\tB\n";

    fn raw(state: State) -> [u8; SAMPLES] {
        std::array::from_fn(|index| {
            EXPECTED[if state == State::A {
                index / 27
            } else {
                4 - index / 27
            }]
        })
    }

    fn frame(seq: u64, display: u64, arrival: u64, state: State) -> String {
        let label = if state == State::A { "A" } else { "B" };
        let samples = raw(state)
            .iter()
            .map(u8::to_string)
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "scene-frame\t{seq}\t{display}\t{arrival}\t{label}\t{samples}\t0\t0\t100\t40\t1\t1\n"
        )
    }

    fn trace(frames: &str, count: u64) -> String {
        format!("{SOURCE}{frames}scene-end\t100\t{count}\n")
    }

    fn valid() -> String {
        trace(
            &(frame(1, 20, 21, State::A) + &frame(2, 40, 41, State::B)),
            2,
        )
    }

    #[test]
    fn recognizes_both_patterns_and_tolerance_boundary() -> Result<(), String> {
        assert_eq!(classify(&raw(State::A))?, State::A);
        assert_eq!(classify(&raw(State::B))?, State::B);
        let mut samples = raw(State::A);
        samples[27] += TOLERANCE;
        assert_eq!(classify(&samples)?, State::A);
        samples[27] += 1;
        assert!(classify(&samples).is_err());
        for wrong in [[0, 181, 223, 241, 255], [0, 188, 118, 225, 255], [0; 5]] {
            assert!(classify(&std::array::from_fn(|i| wrong[i / 27])).is_err());
        }
        samples = raw(State::A);
        samples[28] = 200;
        assert!(classify(&samples).is_err());
        Ok(())
    }

    #[test]
    fn accepts_only_an_observed_single_transition() -> Result<(), String> {
        assert_eq!(
            replay(&valid(), ACTION)?,
            Observation {
                frames: 2,
                first_b: 40
            }
        );
        let late =
            frame(1, 20, 21, State::A) + &frame(2, 40, 41, State::A) + &frame(3, 70, 71, State::B);
        assert_eq!(replay(&trace(&late, 3), ACTION)?.first_b, 70);
        Ok(())
    }

    #[test]
    fn accepts_a_static_baseline_displayed_before_capture_started() -> Result<(), String> {
        let captured = frame(1, 5, 21, State::A) + &frame(2, 40, 41, State::B);
        assert_eq!(replay(&trace(&captured, 2), ACTION)?.first_b, 40);
        let zero = frame(1, 0, 21, State::A) + &frame(2, 40, 41, State::B);
        assert!(replay(&trace(&zero, 2), ACTION).is_err());
        Ok(())
    }

    #[test]
    fn rejects_stale_early_missing_and_regressed_states() {
        for frames in [
            frame(1, 20, 21, State::B),
            frame(1, 40, 41, State::B),
            frame(1, 20, 21, State::A) + &frame(2, 40, 41, State::A),
            frame(1, 20, 31, State::A) + &frame(2, 40, 41, State::B),
            frame(1, 20, 21, State::A) + &frame(2, 30, 31, State::B),
            frame(1, 20, 21, State::A) + &frame(2, 40, 41, State::B) + &frame(3, 60, 61, State::A),
        ] {
            let count = frames.lines().count() as u64;
            assert!(replay(&trace(&frames, count), ACTION).is_err(), "{frames}");
        }
    }

    #[test]
    fn rejects_multiple_actions_and_wrong_source() {
        for action in [
            ACTION.repeat(2),
            ACTION.replace("123", "124"),
            ACTION.replace("456", "457"),
            ACTION.replace("30", "0"),
            ACTION.replace("30", "10"),
            ACTION.replace("\tA\tB", "\tB\tA"),
        ] {
            assert!(replay(&valid(), &action).is_err());
        }
    }

    #[test]
    fn rejects_bad_clocks_sequences_samples_and_truncation() {
        for frame in [
            frame(2, 40, 39, State::B),
            frame(2, 0, 41, State::B),
            frame(2, 20, 41, State::B),
            frame(2, 40, 21, State::B),
            frame(1, 40, 41, State::B),
            frame(2, 40, 41, State::B).replace("\tB\t", "\tA\t"),
        ] {
            assert!(
                replay(
                    &trace(&(super::tests::frame(1, 20, 21, State::A) + &frame), 2),
                    ACTION
                )
                .is_err()
            );
        }
        for text in [
            valid().replace("scene-end\t100\t2\n", ""),
            valid() + "extra\n",
            valid().replace("scene-end\t100\t2", "scene-end\t40\t2"),
            valid().replace("scene-end\t100\t2", "scene-end\t100\t3"),
            valid().replace(",255", ",256"),
            valid().replace(",255", ""),
            valid().replace("\tA\t0,", "\tA\t0,0,"),
        ] {
            assert!(replay(&text, ACTION).is_err());
        }
    }

    #[test]
    fn rejects_invalid_metadata() {
        for source in [
            SOURCE.replace("\t1\t0\t0", "\tNaN\t0\t0"),
            SOURCE.replacen("\t100\t40\t1", "\t99\t40\t1", 1),
            SOURCE.replace("\t0\t0\t100", "\t1\t0\t100"),
            SOURCE.replace("\t2048\t", "\t4096\t"),
            SOURCE.replace("\t10\t1\t1", "\t0\t1\t1"),
            SOURCE.replace("diagnostic/v1", "diagnostic/v2"),
        ] {
            assert!(
                replay(&valid().replacen(SOURCE, &source, 1), ACTION).is_err(),
                "{source}"
            );
        }
    }

    #[test]
    fn enforces_frame_and_byte_caps_before_admission() -> Result<(), String> {
        let mut frames = frame(1, 20, 21, State::A);
        for seq in 2..=MAX_FRAMES {
            frames += &frame(seq, 40 + seq * 2, 41 + seq * 2, State::B);
        }
        let at_cap = format!("{SOURCE}{frames}scene-end\t10000\t{MAX_FRAMES}\n");
        assert_eq!(replay(&at_cap, ACTION)?.frames, MAX_FRAMES);
        frames += &frame(MAX_FRAMES + 1, 5000, 5001, State::B);
        let text = format!("{SOURCE}{frames}scene-end\t10000\t{}\n", MAX_FRAMES + 1);
        assert!(replay(&text, ACTION).is_err());
        assert!(replay(&"x".repeat(2 * 1024 * 1024 + 1), ACTION).is_err());
        assert!(replay(&valid(), &"x".repeat(257)).is_err());
        Ok(())
    }

    #[test]
    fn rejects_changed_or_unqualified_geometry() {
        let second = frame(2, 40, 41, State::B);
        for changed in [
            second.replace("\t0\t0\t100\t40\t1\t1", "\t1\t0\t100\t40\t1\t1"),
            second.replace("\t0\t0\t100\t40\t1\t1", "\t0\t0\t50\t20\t0.5\t2"),
            second.replace("\t0\t0\t100\t40\t1\t1", "\t0\t0\t100\t40\tNaN\t1"),
        ] {
            assert!(replay(&trace(&(frame(1, 20, 21, State::A) + &changed), 2), ACTION).is_err());
        }
        assert!(replay(&valid().replace("\tsrgb", "\tdisplay-p3"), ACTION).is_err());
        assert!(replay(&(SOURCE.to_owned() + &"x".repeat(1025)), ACTION).is_err());
    }

    #[test]
    fn stale_needs_a_display_sample_after_action() {
        let before = frame(1, 20, 21, State::A);
        for (rows, count) in [
            (before.clone(), 1),
            (before.clone() + &frame(2, 30, 41, State::A), 2),
        ] {
            assert!(inspect(&trace(&rows, count), Some(ACTION), Some("stale"), Some(35)).is_err());
        }
    }
    #[test]
    fn queued_a_after_command_does_not_prove_display_after_admission() -> Result<(), String> {
        let before = frame(1, 20, 21, State::A);
        let queued = trace(&(before.clone() + &frame(2, 34, 41, State::A)), 2);
        for witness in [None, Some(30), Some(34), Some(35)] {
            assert!(inspect(&queued, Some(ACTION), Some("stale"), witness).is_err());
        }
        let after = trace(&(before + &frame(2, 36, 41, State::A)), 2);
        inspect(&after, Some(ACTION), Some("stale"), Some(35))?;
        Ok(())
    }
    #[test]
    fn negative_controls_validate_the_whole_trace() -> Result<(), String> {
        let stale = trace(
            &(frame(1, 20, 21, State::A) + &frame(2, 40, 41, State::A)),
            2,
        );
        inspect(&stale, Some(ACTION), Some("stale"), Some(35))?;
        let wrong_samples = [188; SAMPLES]
            .iter()
            .map(u8::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let wrong =
            format!("scene-frame\t2\t40\t41\tinvalid\t{wrong_samples}\t0\t0\t100\t40\t1\t1\n");
        let good = trace(&(frame(1, 20, 21, State::A) + &wrong), 2);
        inspect(&good, Some(ACTION), Some("wrong"), None)?;
        for bad in [
            good.replace("scene-end\t100\t2", "scene-end\t100\t3"),
            good.replace("scene-end\t100\t2\n", ""),
            good.replace("\t40\t41\tinvalid", "\t40\t39\tinvalid"),
            good.replace("188", "0"),
            trace(
                &(frame(1, 20, 21, State::A) + &wrong + &frame(3, 60, 61, State::A)),
                3,
            ),
        ] {
            assert!(inspect(&bad, Some(ACTION), Some("wrong"), None).is_err());
        }
        assert!(inspect(&valid(), Some(ACTION), Some("wrong"), None).is_err());
        assert!(inspect(&valid(), Some(ACTION), Some("stale"), Some(35)).is_err());
        assert!(
            inspect(
                &stale.replace("scene-end\t100\t2\n", ""),
                Some(ACTION),
                Some("stale"),
                Some(35)
            )
            .is_err()
        );
        Ok(())
    }
    #[test]
    fn live_baseline_requires_raw_a_and_verified_window_mapping() -> Result<(), String> {
        let source = "scene-source\talpine-scene-diagnostic/v1\t123\t456\t500\t328\t1\t0\t158\t500\t40\t10\t1\t1\t2048\t8000\tsrgb\n";
        let frame =
            frame(1, 5, 21, State::A).replace("\t0\t0\t100\t40\t1\t1", "\t0\t0\t500\t328\t1\t1");
        let good = source.to_owned() + &frame;
        baseline(&good, 123, 456, 500.0, 328.0, 0.0, 158.0)?;
        baseline(
            &(good.clone() + "scene-frame\t2"),
            123,
            456,
            500.0,
            328.0,
            0.0,
            158.0,
        )?;
        for (pid, window, height, y) in [
            (124, 456, 328.0, 158.0),
            (123, 457, 328.0, 158.0),
            (123, 456, 329.0, 158.0),
            (123, 456, 328.0, 159.0),
        ] {
            assert!(baseline(&good, pid, window, 500.0, height, 0.0, y).is_err());
        }
        assert!(
            baseline(
                &good.replace("\tA\t", "\tB\t"),
                123,
                456,
                500.0,
                328.0,
                0.0,
                158.0
            )
            .is_err()
        );
        assert!(baseline(source, 123, 456, 500.0, 328.0, 0.0, 158.0).is_err());
        Ok(())
    }

    #[test]
    fn bordered_capture_requires_exact_outer_and_strip_mapping() -> Result<(), String> {
        let source = "scene-source\talpine-scene-diagnostic/v1\t123\t456\t502\t332\t1\t1\t146\t500\t40\t10\t1\t1\t2048\t8000\tsrgb\n";
        let frame =
            frame(1, 5, 21, State::A).replace("\t0\t0\t100\t40\t1\t1", "\t0\t0\t502\t332\t1\t1");
        let good = source.to_owned() + &frame;
        baseline(&good, 123, 456, 502.0, 332.0, 1.0, 146.0)?;
        for (width, height, x, y) in [
            (500.0, 332.0, 1.0, 146.0),
            (502.0, 330.0, 1.0, 146.0),
            (502.0, 332.0, 0.0, 146.0),
            (502.0, 332.0, 1.0, 147.0),
        ] {
            assert!(baseline(&good, 123, 456, width, height, x, y).is_err());
        }
        for bad in [
            good.replacen("\t1\t146\t500\t40", "\t0\t146\t500\t40", 1),
            good.replacen("\t1\t146\t500\t40", "\t1\t147\t500\t40", 1),
            good.replace("\t0\t0\t502\t332", "\t0\t0\t500\t332"),
        ] {
            assert!(baseline(&bad, 123, 456, 502.0, 332.0, 1.0, 146.0).is_err());
        }
        Ok(())
    }

    #[test]
    fn supervisor_clock_must_match_capture() -> Result<(), String> {
        let timebase = crate::helpers::Timebase { numer: 1, denom: 1 };
        clock_matches(SOURCE, 30, timebase)?;
        assert!(clock_matches(SOURCE, 10, timebase).is_err());
        assert!(
            clock_matches(SOURCE, 30, crate::helpers::Timebase { numer: 2, denom: 1 }).is_err()
        );
        assert!(
            clock_matches(SOURCE, 30, crate::helpers::Timebase { numer: 1, denom: 0 }).is_err()
        );
        Ok(())
    }

    #[test]
    fn files_round_trip_without_a_qualification_claim() -> Result<(), String> {
        let dir = crate::test_dir("scene-evidence")?;
        let capture = dir.join("capture.tsv");
        let action = dir.join("action.tsv");
        std::fs::write(&capture, valid()).map_err(|e| e.to_string())?;
        std::fs::write(&action, ACTION).map_err(|e| e.to_string())?;
        let result = run(&capture, &action)?;
        assert!(result.contains("physical qualification=false"));
        assert!(result.contains("action provenance unverified"));
        std::fs::write(&capture, "x".repeat(2 * 1024 * 1024 + 1)).map_err(|e| e.to_string())?;
        assert!(run(&capture, &action).is_err());
        std::fs::write(&capture, valid()).map_err(|e| e.to_string())?;
        std::fs::write(&action, "x".repeat(257)).map_err(|e| e.to_string())?;
        assert!(run(&capture, &action).is_err());
        Ok(())
    }
}
