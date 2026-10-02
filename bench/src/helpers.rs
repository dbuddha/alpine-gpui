//! The Swift helpers in `bench/helpers/`, their line formats and the
//! processes that run them. Parsers are pure so they are tested without GUI.

use crate::tsv;
use std::fs::File;
use std::io::{BufRead as _, BufReader, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub const WINDOW: &str = "bench-window";
pub const SAMPLE: &str = "bench-sample";
pub const INPUT: &str = "bench-input";
pub const CAPTURE: &str = "bench-capture";
pub const REFERENCE: &str = "bench-reference-appkit";

/// Exit code a helper uses when a privacy permission is missing.
const EXIT_PERMISSION: i32 = 3;
/// Exit code the input helper uses when the target lost the foreground.
const EXIT_ABORTED: i32 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timebase {
    pub numer: u64,
    pub denom: u64,
}

impl Timebase {
    pub fn nanos(self, ticks: u64) -> u64 {
        let value = u128::from(ticks) * u128::from(self.numer) / u128::from(self.denom.max(1));
        u64::try_from(value).unwrap_or(u64::MAX)
    }

    pub fn ticks(self, nanos: u64) -> u64 {
        let value = u128::from(nanos) * u128::from(self.denom) / u128::from(self.numer.max(1));
        u64::try_from(value).unwrap_or(u64::MAX)
    }

    pub fn millis_between(self, start: u64, end: u64) -> f64 {
        if end >= start {
            crate::stats::to_f64(self.nanos(end - start)) / 1e6
        } else {
            -crate::stats::to_f64(self.nanos(start - end)) / 1e6
        }
    }
}

fn field<'a>(fields: &'a [String], index: usize, line: &str) -> Result<&'a str, String> {
    fields
        .get(index)
        .map(String::as_str)
        .ok_or_else(|| format!("short helper line: {line:?}"))
}

fn number<T: std::str::FromStr>(fields: &[String], index: usize, line: &str) -> Result<T, String> {
    let text = field(fields, index, line)?;
    text.parse()
        .map_err(|_| format!("bad number {text:?} in helper line {line:?}"))
}

#[derive(Clone, Debug, PartialEq)]
pub struct WindowInfo {
    pub order: usize,
    pub id: u32,
    pub pid: i32,
    pub owner: String,
    pub layer: i64,
    pub alpha: f64,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WindowList {
    pub front_pid: i32,
    pub front_name: String,
    pub windows: Vec<WindowInfo>,
}

pub fn parse_window_list(text: &str) -> Result<WindowList, String> {
    let mut list = WindowList {
        front_pid: -1,
        front_name: String::new(),
        windows: Vec::new(),
    };
    for line in text.lines().filter(|line| !line.is_empty()) {
        let fields = tsv::split(line)?;
        match field(&fields, 0, line)? {
            "front" => {
                list.front_pid = number(&fields, 1, line)?;
                field(&fields, 2, line)?.clone_into(&mut list.front_name);
            }
            "window" => list.windows.push(WindowInfo {
                order: number(&fields, 1, line)?,
                id: number(&fields, 2, line)?,
                pid: number(&fields, 3, line)?,
                owner: field(&fields, 4, line)?.to_owned(),
                layer: number(&fields, 5, line)?,
                alpha: number(&fields, 6, line)?,
                x: number(&fields, 7, line)?,
                y: number(&fields, 8, line)?,
                width: number(&fields, 9, line)?,
                height: number(&fields, 10, line)?,
            }),
            other => return Err(format!("unexpected window line kind {other:?}")),
        }
    }
    Ok(list)
}

/// `now\t<mach>\t<numer>\t<denom>` from `bench-window now`.
pub fn parse_now(text: &str) -> Result<(u64, Timebase), String> {
    let line = text
        .lines()
        .find(|line| line.starts_with("now\t"))
        .ok_or_else(|| format!("no now line in {text:?}"))?;
    let fields = tsv::split(line)?;
    Ok((
        number(&fields, 1, line)?,
        Timebase {
            numer: number(&fields, 2, line)?,
            denom: number(&fields, 3, line)?,
        },
    ))
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Appeared {
    pub mach: u64,
    pub id: u32,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

pub fn parse_appeared(text: &str) -> Result<Appeared, String> {
    let line = text
        .lines()
        .find(|line| line.starts_with("appeared\t"))
        .ok_or_else(|| format!("no appeared line in {text:?}"))?;
    let fields = tsv::split(line)?;
    Ok(Appeared {
        mach: number(&fields, 1, line)?,
        id: number(&fields, 2, line)?,
        x: number(&fields, 3, line)?,
        y: number(&fields, 4, line)?,
        width: number(&fields, 5, line)?,
        height: number(&fields, 6, line)?,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Display {
    pub id: u32,
    pub main: bool,
    pub builtin: bool,
    pub width_pt: u32,
    pub height_pt: u32,
    pub width_px: u32,
    pub height_px: u32,
    pub refresh_hz: u32,
    pub max_fps: u32,
}

pub fn parse_displays(text: &str) -> Result<Vec<Display>, String> {
    text.lines()
        .filter(|line| line.starts_with("display\t"))
        .map(|line| {
            let fields = tsv::split(line)?;
            Ok(Display {
                id: number(&fields, 1, line)?,
                main: field(&fields, 2, line)? == "1",
                builtin: field(&fields, 3, line)? == "1",
                width_pt: number(&fields, 4, line)?,
                height_pt: number(&fields, 5, line)?,
                width_px: number(&fields, 6, line)?,
                height_px: number(&fields, 7, line)?,
                refresh_hz: number(&fields, 8, line)?,
                max_fps: number(&fields, 9, line)?,
            })
        })
        .collect()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Permissions {
    pub accessibility: bool,
    pub post_events: bool,
    pub screen_recording: bool,
}

pub fn parse_permissions(text: &str) -> Result<Permissions, String> {
    let mut permissions = Permissions::default();
    for line in text.lines().filter(|line| line.starts_with("permission\t")) {
        let fields = tsv::split(line)?;
        let granted = field(&fields, 2, line)? == "1";
        match field(&fields, 1, line)? {
            "accessibility" => permissions.accessibility = granted,
            "post-events" => permissions.post_events = granted,
            "screen-recording" => permissions.screen_recording = granted,
            other => return Err(format!("unknown permission {other:?}")),
        }
    }
    Ok(permissions)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetKind {
    Tick,
    Mark,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcRow {
    pub pid: i32,
    pub ppid: i32,
    pub depth: u32,
    pub name: String,
    pub footprint: u64,
    pub lifetime_max_footprint: u64,
    pub user_ns: u64,
    pub system_ns: u64,
    pub child_user_ns: u64,
    pub child_system_ns: u64,
    pub idle_wakeups: u64,
    pub interrupt_wakeups: u64,
    pub child_idle_wakeups: u64,
    pub child_interrupt_wakeups: u64,
    pub instructions: u64,
    pub cycles: u64,
    pub energy_nj: u64,
    pub start_mach: u64,
    pub exited: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SampleSet {
    pub seq: u64,
    pub kind: SetKind,
    pub mach: u64,
    pub rows: Vec<ProcRow>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mark {
    pub seq: u64,
    pub mach: u64,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SamplerLine {
    Timebase(Timebase),
    Mark(Mark),
    Set {
        seq: u64,
        kind: SetKind,
        mach: u64,
        count: usize,
    },
    Proc {
        seq: u64,
        row: ProcRow,
    },
    End {
        reason: String,
    },
}

pub fn parse_sampler_line(line: &str) -> Result<SamplerLine, String> {
    let fields = tsv::split(line)?;
    match field(&fields, 0, line)? {
        "timebase" => Ok(SamplerLine::Timebase(Timebase {
            numer: number(&fields, 1, line)?,
            denom: number(&fields, 2, line)?,
        })),
        "mark" => Ok(SamplerLine::Mark(Mark {
            seq: number(&fields, 1, line)?,
            mach: number(&fields, 2, line)?,
            label: field(&fields, 3, line)?.to_owned(),
        })),
        "set" => Ok(SamplerLine::Set {
            seq: number(&fields, 1, line)?,
            kind: match field(&fields, 2, line)? {
                "tick" => SetKind::Tick,
                "mark" => SetKind::Mark,
                other => return Err(format!("unknown set kind {other:?}")),
            },
            mach: number(&fields, 3, line)?,
            count: number(&fields, 4, line)?,
        }),
        "proc" => Ok(SamplerLine::Proc {
            seq: number(&fields, 1, line)?,
            row: ProcRow {
                pid: number(&fields, 2, line)?,
                ppid: number(&fields, 3, line)?,
                depth: number(&fields, 4, line)?,
                name: field(&fields, 5, line)?.to_owned(),
                footprint: number(&fields, 6, line)?,
                lifetime_max_footprint: number(&fields, 7, line)?,
                user_ns: number(&fields, 8, line)?,
                system_ns: number(&fields, 9, line)?,
                child_user_ns: number(&fields, 10, line)?,
                child_system_ns: number(&fields, 11, line)?,
                idle_wakeups: number(&fields, 12, line)?,
                interrupt_wakeups: number(&fields, 13, line)?,
                child_idle_wakeups: number(&fields, 14, line)?,
                child_interrupt_wakeups: number(&fields, 15, line)?,
                instructions: number(&fields, 16, line)?,
                cycles: number(&fields, 17, line)?,
                energy_nj: number(&fields, 18, line)?,
                start_mach: number(&fields, 19, line)?,
                exited: field(&fields, 20, line)? == "1",
            },
        }),
        "end" => Ok(SamplerLine::End {
            reason: field(&fields, 3, line)?.to_owned(),
        }),
        other => Err(format!("unexpected sampler line kind {other:?}")),
    }
}

/// Everything one sampler run reported, assembled from its lines.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SamplerData {
    pub timebase: Option<Timebase>,
    pub sets: Vec<SampleSet>,
    pub marks: Vec<Mark>,
    pub end: Option<String>,
    pub errors: Vec<String>,
}

impl SamplerData {
    pub fn accept(&mut self, line: &str) {
        match parse_sampler_line(line) {
            Ok(SamplerLine::Timebase(timebase)) => self.timebase = Some(timebase),
            Ok(SamplerLine::Mark(mark)) => self.marks.push(mark),
            Ok(SamplerLine::Set {
                seq,
                kind,
                mach,
                count,
            }) => self.sets.push(SampleSet {
                seq,
                kind,
                mach,
                rows: Vec::with_capacity(count),
            }),
            Ok(SamplerLine::Proc { seq, row }) => match self.sets.last_mut() {
                Some(set) if set.seq == seq => set.rows.push(row),
                _ => self.errors.push(format!("proc row for unknown set {seq}")),
            },
            Ok(SamplerLine::End { reason }) => self.end = Some(reason),
            Err(error) => self.errors.push(error),
        }
    }
}

pub struct Helpers {
    dir: PathBuf,
}

impl Helpers {
    pub fn locate(dir: &Path) -> Result<Self, String> {
        for name in [WINDOW, SAMPLE, INPUT, CAPTURE, REFERENCE] {
            if !dir.join(name).is_file() {
                return Err(format!(
                    "missing {}; build the helpers with bench/helpers/build.sh",
                    dir.join(name).display()
                ));
            }
        }
        Ok(Self {
            dir: dir.to_path_buf(),
        })
    }

    pub fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    fn window(&self, args: &[&str]) -> Result<String, String> {
        crate::procs::output(&self.path(WINDOW), args)
    }

    pub fn window_list(&self) -> Result<WindowList, String> {
        parse_window_list(&self.window(&["list"])?)
    }

    pub fn wait_for_window(&self, pid: i32, timeout_ms: u64) -> Result<Appeared, String> {
        let pid = pid.to_string();
        let timeout = timeout_ms.to_string();
        parse_appeared(&self.window(&["wait", "--pid", &pid, "--timeout-ms", &timeout])?)
    }

    pub fn activate(&self, pid: i32) -> Result<bool, String> {
        let text = self.window(&["activate", "--pid", &pid.to_string()])?;
        Ok(text.trim_end() == "activate\t1")
    }

    pub fn displays(&self) -> Result<Vec<Display>, String> {
        parse_displays(&self.window(&["display"])?)
    }

    /// Preflight only: these calls never show a permission prompt.
    pub fn permissions(&self) -> Result<Permissions, String> {
        parse_permissions(&self.window(&["permissions"])?)
    }

    pub fn spawn_sampler(&self, root: i32, log: &Path) -> Result<Sampler, String> {
        Sampler::spawn(&self.path(SAMPLE), root, log)
    }

    /// Runs an input script to completion; events carry mach post times.
    pub fn run_input(&self, args: &[String], log: &Path) -> Result<Vec<InputEvent>, String> {
        let stderr = File::create(log).map_err(|error| format!("{}: {error}", log.display()))?;
        let output = Command::new(self.path(INPUT))
            .args(args)
            .stdin(Stdio::null())
            .stderr(stderr)
            .output()
            .map_err(|error| format!("{INPUT}: {error}"))?;
        let text = String::from_utf8_lossy(&output.stdout);
        match output.status.code() {
            Some(0) => parse_input_events(&text),
            Some(EXIT_PERMISSION) => Err(format!(
                "{INPUT} needs Accessibility for this terminal; see {}",
                log.display()
            )),
            Some(EXIT_ABORTED) => Err(format!(
                "{INPUT} stopped: the target lost the foreground; see {}",
                log.display()
            )),
            _ => Err(format!(
                "{INPUT} failed ({}); see {}",
                output.status,
                log.display()
            )),
        }
    }

    pub fn spawn_capture(&self, args: &[String], log: &Path) -> Result<Capture, String> {
        Capture::spawn(&self.path(CAPTURE), args, log)
    }
}

pub struct Sampler {
    child: Child,
    stdin: Option<ChildStdin>,
    data: Arc<Mutex<SamplerData>>,
    reader: Option<JoinHandle<()>>,
}

impl Sampler {
    fn spawn(program: &Path, root: i32, log: &Path) -> Result<Self, String> {
        let stderr = File::create(log).map_err(|error| format!("{}: {error}", log.display()))?;
        let mut child = Command::new(program)
            .args(["run", "--root", &root.to_string(), "--interval-ms", "1000"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(stderr)
            .spawn()
            .map_err(|error| format!("{SAMPLE}: {error}"))?;
        let stdout = child.stdout.take().ok_or("sampler has no stdout")?;
        let stdin = child.stdin.take();
        let data = Arc::new(Mutex::new(SamplerData::default()));
        let shared = Arc::clone(&data);
        let reader = thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                shared
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .accept(&line);
            }
        });
        Ok(Self {
            child,
            stdin,
            data,
            reader: Some(reader),
        })
    }

    /// Asks for a boundary sample now and waits for the sampler to echo the
    /// mark, so phases start only once their boundary is on record.
    pub fn mark(&mut self, label: &str) -> Result<Mark, String> {
        let before = self
            .data
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .marks
            .len();
        let stdin = self.stdin.as_mut().ok_or("sampler already finished")?;
        writeln!(stdin, "{label}")
            .and_then(|()| stdin.flush())
            .map_err(|error| format!("sampler mark {label}: {error}"))?;
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            {
                let data = self.data.lock().unwrap_or_else(PoisonError::into_inner);
                if let Some(mark) = data
                    .marks
                    .get(before..)
                    .and_then(|new| new.iter().find(|mark| mark.label == label))
                {
                    return Ok(mark.clone());
                }
                if let Some(reason) = &data.end {
                    return Err(format!("sampler ended ({reason}) before mark {label}"));
                }
            }
            if Instant::now() >= deadline {
                return Err(format!("sampler did not record mark {label}"));
            }
            thread::sleep(Duration::from_millis(5));
        }
    }

    pub fn snapshot(&self) -> SamplerData {
        self.data
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Closes stdin, which ends the sampler, and returns all it reported.
    pub fn finish(mut self) -> Result<SamplerData, String> {
        drop(self.stdin.take());
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
                _ => {
                    let _ = self.child.kill();
                    let _ = self.child.wait();
                    break;
                }
            }
        }
        if let Some(reader) = self.reader.take() {
            reader.join().map_err(|_| "sampler reader panicked")?;
        }
        Ok(self.snapshot())
    }
}

impl Drop for Sampler {
    fn drop(&mut self) {
        if self.reader.is_some() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputEvent {
    pub seq: u64,
    pub mach: u64,
    pub kind: String,
    pub label: String,
}

pub fn parse_input_events(text: &str) -> Result<Vec<InputEvent>, String> {
    text.lines()
        .filter(|line| line.starts_with("event\t"))
        .map(|line| {
            let fields = tsv::split(line)?;
            Ok(InputEvent {
                seq: number(&fields, 1, line)?,
                mach: number(&fields, 2, line)?,
                kind: field(&fields, 3, line)?.to_owned(),
                label: field(&fields, 4, line)?.to_owned(),
            })
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    pub seq: u64,
    pub display_mach: u64,
    pub arrival_mach: u64,
    pub hash: u64,
}

pub fn parse_frame(line: &str) -> Result<Option<Frame>, String> {
    if !line.starts_with("frame\t") {
        return Ok(None);
    }
    let fields = tsv::split(line)?;
    let hash_text = field(&fields, 4, line)?;
    Ok(Some(Frame {
        seq: number(&fields, 1, line)?,
        display_mach: number(&fields, 2, line)?,
        arrival_mach: number(&fields, 3, line)?,
        hash: u64::from_str_radix(hash_text, 16)
            .map_err(|_| format!("bad frame hash {hash_text:?}"))?,
    }))
}

#[derive(Default)]
struct CaptureState {
    ready: bool,
    frames: Vec<Frame>,
    errors: Vec<String>,
}

pub struct Capture {
    child: Child,
    state: Arc<Mutex<CaptureState>>,
    reader: Option<JoinHandle<()>>,
    log: PathBuf,
}

impl Capture {
    fn spawn(program: &Path, args: &[String], log: &Path) -> Result<Self, String> {
        let stderr = File::create(log).map_err(|error| format!("{}: {error}", log.display()))?;
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(stderr)
            .spawn()
            .map_err(|error| format!("{CAPTURE}: {error}"))?;
        let stdout = child.stdout.take().ok_or("capture has no stdout")?;
        let state = Arc::new(Mutex::new(CaptureState::default()));
        let shared = Arc::clone(&state);
        let reader = thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                let mut state = shared.lock().unwrap_or_else(PoisonError::into_inner);
                if line.starts_with("ready\t") {
                    state.ready = true;
                }
                match parse_frame(&line) {
                    Ok(Some(frame)) => state.frames.push(frame),
                    Ok(None) => {}
                    Err(error) => state.errors.push(error),
                }
            }
        });
        Ok(Self {
            child,
            state,
            reader: Some(reader),
            log: log.to_path_buf(),
        })
    }

    /// Waits for the first frame so input never starts before capture.
    pub fn wait_ready(&mut self, timeout: Duration) -> Result<(), String> {
        let deadline = Instant::now() + timeout;
        loop {
            if self
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .ready
            {
                return Ok(());
            }
            if let Ok(Some(status)) = self.child.try_wait() {
                return Err(match status.code() {
                    Some(EXIT_PERMISSION) => format!(
                        "{CAPTURE} needs Screen Recording for this terminal; see {}",
                        self.log.display()
                    ),
                    _ => format!("{CAPTURE} exited ({status}); see {}", self.log.display()),
                });
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "{CAPTURE} delivered no frame; see {}",
                    self.log.display()
                ));
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    /// Waits for the capture's own duration to end and returns its frames.
    pub fn finish(mut self, timeout: Duration) -> Result<Vec<Frame>, String> {
        let deadline = Instant::now() + timeout;
        let status = loop {
            match self.child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
                _ => {
                    let _ = self.child.kill();
                    let _ = self.child.wait();
                    return Err(format!(
                        "{CAPTURE} did not finish; see {}",
                        self.log.display()
                    ));
                }
            }
        };
        if let Some(reader) = self.reader.take() {
            reader.join().map_err(|_| "capture reader panicked")?;
        }
        if !status.success() {
            return Err(format!(
                "{CAPTURE} failed ({status}); see {}",
                self.log.display()
            ));
        }
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(error) = state.errors.first() {
            return Err(format!("{CAPTURE}: {error}"));
        }
        Ok(state.frames.clone())
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        if self.reader.is_some() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        SamplerData, SamplerLine, SetKind, Timebase, parse_appeared, parse_displays, parse_frame,
        parse_input_events, parse_permissions, parse_sampler_line, parse_window_list,
    };

    #[test]
    fn apple_silicon_ticks_convert_to_nanoseconds() {
        let timebase = Timebase {
            numer: 125,
            denom: 3,
        };
        assert_eq!(timebase.nanos(24_000_000), 1_000_000_000);
        assert!((timebase.millis_between(0, 240_000) - 10.0).abs() < 1e-9);
        assert!((timebase.millis_between(240_000, 0) + 10.0).abs() < 1e-9);
    }

    #[test]
    fn window_lists_parse_front_and_windows() -> Result<(), String> {
        let text = "front\t1200\tAlacritty\nwindow\t0\t141833\t4423\tControl Center\t25\t1.0\t1033\t0\t32\t33\nwindow\t10\t40\t1200\tAlacritty\t0\t1.0\t0\t34\t1512\t948\n";
        let list = parse_window_list(text)?;
        assert_eq!(list.front_pid, 1200);
        assert_eq!(list.front_name, "Alacritty");
        assert_eq!(list.windows.len(), 2);
        assert_eq!(list.windows[1].order, 10);
        assert_eq!(list.windows[1].owner, "Alacritty");
        assert!((list.windows[1].height - 948.0).abs() < f64::EPSILON);
        assert!(parse_window_list("bogus\t1\n").is_err());
        assert!(parse_window_list("window\t0\t1\n").is_err());
        Ok(())
    }

    #[test]
    fn appeared_displays_and_permissions_parse() -> Result<(), String> {
        let appeared = parse_appeared("appeared\t83731128219850\t9001\t276\t221\t960\t568\n")?;
        assert_eq!(appeared.id, 9001);
        assert!((appeared.width - 960.0).abs() < f64::EPSILON);
        assert!(parse_appeared("").is_err());
        let displays = parse_displays("display\t1\t1\t1\t1512\t982\t3024\t1964\t120\t120\n")?;
        assert_eq!(displays.len(), 1);
        assert!(displays[0].main && displays[0].builtin);
        assert_eq!(displays[0].max_fps, 120);
        let permissions = parse_permissions(
            "permission\taccessibility\t0\npermission\tpost-events\t1\npermission\tscreen-recording\t0\n",
        )?;
        assert!(!permissions.accessibility && permissions.post_events);
        assert!(!permissions.screen_recording);
        assert!(parse_permissions("permission\tcamera\t1\n").is_err());
        Ok(())
    }

    #[test]
    fn sampler_lines_assemble_into_sets() {
        let lines = [
            "timebase\t125\t3",
            "mark\t1\t1000\tphase:startup:start",
            "set\t1\tmark\t1001\t2",
            "proc\t1\t600\t1\t0\talpine-editor\t31457280\t40000000\t5000000\t1000000\t0\t0\t12\t3\t0\t0\t100\t200\t300\t900\t0",
            "proc\t1\t601\t600\t1\trust-analyzer\t1048576\t1048576\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t950\t0",
            "set\t2\ttick\t2000\t1",
            "proc\t2\t600\t1\t0\talpine-editor\t32505856\t40000000\t6000000\t1000000\t0\t0\t14\t3\t0\t0\t100\t200\t300\t900\t0",
            "end\t2\t3000\tstdin-closed",
        ];
        let mut data = SamplerData::default();
        for line in lines {
            data.accept(line);
        }
        assert!(data.errors.is_empty(), "{:?}", data.errors);
        assert_eq!(
            data.timebase,
            Some(Timebase {
                numer: 125,
                denom: 3
            })
        );
        assert_eq!(data.marks.len(), 1);
        assert_eq!(data.sets.len(), 2);
        assert_eq!(data.sets[0].kind, SetKind::Mark);
        assert_eq!(data.sets[0].rows.len(), 2);
        assert_eq!(data.sets[0].rows[1].depth, 1);
        assert_eq!(data.sets[1].rows[0].footprint, 32_505_856);
        assert_eq!(data.end.as_deref(), Some("stdin-closed"));
    }

    #[test]
    fn sampler_rejects_orphan_rows_and_bad_lines() {
        let mut data = SamplerData::default();
        data.accept("proc\t9\t1\t0\t0\tx\t1\t1\t1\t1\t1\t1\t1\t1\t1\t1\t1\t1\t1\t1\t0");
        data.accept("set\t1\tweird\t0\t0");
        assert_eq!(data.errors.len(), 2);
        assert!(matches!(
            parse_sampler_line("end\t3\t99\troot-exited"),
            Ok(SamplerLine::End { .. })
        ));
    }

    #[test]
    fn input_events_and_frames_parse() -> Result<(), String> {
        let events = parse_input_events(
            "event\t1\t5000\tkey\tt\nevent\t2\t9000\tkey\th\ndone\t9500\t2\ttype\n",
        )?;
        assert_eq!(events.len(), 2);
        assert_eq!(events[1].mach, 9000);
        assert_eq!(events[0].label, "t");
        let frame = parse_frame("frame\t3\t7000\t7100\tdeadbeef")?.ok_or("frame")?;
        assert_eq!(frame.hash, 0xdead_beef);
        assert_eq!(frame.display_mach, 7000);
        assert_eq!(parse_frame("ready\t1")?, None);
        assert!(parse_frame("frame\t1\t2\t3\tzz").is_err());
        Ok(())
    }
}
