//! Pure measurement logic: phase windows from sampler marks, per-phase
//! footprint, CPU and wakeups, keypress-to-screen matching, frame intervals,
//! window visibility and the quiet detector. Nothing here touches the system.

use crate::helpers::{
    Appeared, Frame, InputEvent, Mark, SampleSet, SetKind, Timebase, WindowInfo, WindowList,
};
use crate::stats::{self, Summary, to_f64};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq)]
pub struct Metric {
    pub phase: String,
    pub name: String,
    pub value: f64,
    pub unit: &'static str,
}

impl Metric {
    pub fn new(phase: &str, name: impl Into<String>, value: f64, unit: &'static str) -> Self {
        Self {
            phase: phase.to_owned(),
            name: name.into(),
            value,
            unit,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PhaseWindow {
    pub name: String,
    pub start_seq: u64,
    pub end_seq: u64,
    pub start_mach: u64,
    pub end_mach: u64,
}

pub fn phase_label(name: &str, edge: &str) -> String {
    format!("phase:{name}:{edge}")
}

/// Pairs `phase:<name>:start` with the next `phase:<name>:end`. Phases do
/// not nest, so another start before the end is an error.
pub fn phase_windows(marks: &[Mark]) -> Result<Vec<PhaseWindow>, String> {
    let mut windows = Vec::new();
    let mut open: Option<(&str, &Mark)> = None;
    for mark in marks {
        let Some(rest) = mark.label.strip_prefix("phase:") else {
            continue;
        };
        let (name, edge) = rest
            .rsplit_once(':')
            .ok_or_else(|| format!("malformed mark {:?}", mark.label))?;
        match (edge, open) {
            ("start", None) => open = Some((name, mark)),
            ("end", Some((open_name, start))) if open_name == name => {
                windows.push(PhaseWindow {
                    name: name.to_owned(),
                    start_seq: start.seq,
                    end_seq: mark.seq,
                    start_mach: start.mach,
                    end_mach: mark.mach,
                });
                open = None;
            }
            _ => return Err(format!("unexpected mark {:?}", mark.label)),
        }
    }
    if let Some((name, _)) = open {
        return Err(format!("phase {name} never ended"));
    }
    Ok(windows)
}

/// CPU of the live tree plus everything its members reaped: each process
/// counts once, in itself while alive and in its parent once reaped.
pub fn tree_cpu_ns(set: &SampleSet) -> u64 {
    set.rows
        .iter()
        .map(|row| row.user_ns + row.system_ns + row.child_user_ns + row.child_system_ns)
        .sum()
}

pub fn tree_idle_wakeups(set: &SampleSet) -> u64 {
    set.rows
        .iter()
        .map(|row| row.idle_wakeups + row.child_idle_wakeups)
        .sum()
}

pub fn tree_interrupt_wakeups(set: &SampleSet) -> u64 {
    set.rows
        .iter()
        .map(|row| row.interrupt_wakeups + row.child_interrupt_wakeups)
        .sum()
}

fn app_footprint(set: &SampleSet) -> Option<u64> {
    set.rows
        .iter()
        .find(|row| row.depth == 0 && !row.exited)
        .map(|row| row.footprint)
}

fn tree_footprint(set: &SampleSet) -> u64 {
    set.rows
        .iter()
        .filter(|row| !row.exited)
        .map(|row| row.footprint)
        .sum()
}

/// Energy of processes alive at the end; a process that exits mid-phase
/// takes its energy with it, so this undercounts short-lived children.
fn energy_nj(start: &SampleSet, end: &SampleSet) -> u64 {
    end.rows
        .iter()
        .map(|row| {
            let before = start
                .rows
                .iter()
                .find(|old| old.pid == row.pid && old.start_mach == row.start_mach)
                .map_or(0, |old| old.energy_nj);
            row.energy_nj.saturating_sub(before)
        })
        .sum()
}

/// The largest per-sample footprint of each child name, summing processes
/// that share a name (several `rustc`, for example) within one sample.
fn child_maxima<'a>(sets: &[&'a SampleSet]) -> BTreeMap<&'a str, u64> {
    let mut maxima: BTreeMap<&str, u64> = BTreeMap::new();
    for set in sets {
        let mut by_name: BTreeMap<&str, u64> = BTreeMap::new();
        for row in set.rows.iter().filter(|row| row.depth > 0 && !row.exited) {
            *by_name.entry(row.name.as_str()).or_default() += row.footprint;
        }
        for (name, bytes) in by_name {
            let best = maxima.entry(name).or_default();
            *best = (*best).max(bytes);
        }
    }
    maxima
}

/// Every child process name any sample saw under the app, first seen first;
/// a no-server trial needs this empty.
pub fn child_names(sets: &[SampleSet]) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for row in sets.iter().flat_map(|set| &set.rows) {
        if row.depth > 0 && !names.contains(&row.name) {
            names.push(row.name.clone());
        }
    }
    names
}

/// CPU the app reaped from exited children, from its last sample, so a child
/// that started and exited between 1 Hz samples still shows.
pub fn reaped_child_cpu_ns(sets: &[SampleSet]) -> u64 {
    sets.iter()
        .rev()
        .flat_map(|set| &set.rows)
        .find(|row| row.depth == 0)
        .map_or(0, |row| {
            row.child_user_ns.saturating_add(row.child_system_ns)
        })
}

/// Why a no-server trial is void: a child some sample saw, or child CPU the
/// app reaped between samples. `None` means no child process ran.
pub fn child_process_evidence(sets: &[SampleSet]) -> Option<String> {
    let names = child_names(sets);
    if !names.is_empty() {
        return Some(format!("the app started {}", names.join(", ")));
    }
    let reaped = reaped_child_cpu_ns(sets);
    (reaped > 0).then(|| format!("the app reaped {reaped} ns of CPU from exited children"))
}

fn set_by_seq(sets: &[SampleSet], seq: u64) -> Result<&SampleSet, String> {
    sets.iter()
        .find(|set| set.seq == seq)
        .ok_or_else(|| format!("no sample set {seq}"))
}

fn percentiles(phase: &str, prefix: &str, values: &[f64], metrics: &mut Vec<Metric>) {
    for (suffix, quantile) in [("p50", 0.5), ("p95", 0.95), ("max", 1.0)] {
        if let Some(value) = stats::nearest_rank(values, quantile) {
            metrics.push(Metric::new(
                phase,
                format!("{prefix}_{suffix}"),
                value,
                "bytes",
            ));
        }
    }
}

/// Footprint percentiles use the 1 Hz ticks inside the phase; CPU, wakeups
/// and energy are exact deltas between the phase's boundary samples, or
/// from process start (all zero) when `from_launch` is set.
pub fn phase_metrics(
    window: &PhaseWindow,
    sets: &[SampleSet],
    timebase: Timebase,
    from_launch: bool,
) -> Result<Vec<Metric>, String> {
    let phase = window.name.as_str();
    let marked = set_by_seq(sets, window.start_seq)?;
    let end = set_by_seq(sets, window.end_seq)?;
    let launch;
    let start = if from_launch {
        let started = sets
            .iter()
            .flat_map(|set| set.rows.iter())
            .find(|row| row.depth == 0)
            .map_or(marked.mach, |row| row.start_mach);
        launch = SampleSet {
            seq: 0,
            kind: SetKind::Mark,
            mach: started,
            rows: Vec::new(),
        };
        &launch
    } else {
        marked
    };
    let mut inside: Vec<&SampleSet> = sets
        .iter()
        .filter(|set| set.kind == SetKind::Tick && set.mach >= start.mach && set.mach <= end.mach)
        .collect();
    let samples = inside.len();
    if inside.is_empty() {
        inside = vec![start, end];
    }
    let mut metrics = Vec::new();
    let duration_ms = timebase.millis_between(start.mach, end.mach);
    metrics.push(Metric::new(phase, "duration", duration_ms, "ms"));
    metrics.push(Metric::new(
        phase,
        "samples",
        to_f64(samples as u64),
        "count",
    ));
    let app: Vec<f64> = inside
        .iter()
        .filter_map(|set| app_footprint(set))
        .map(to_f64)
        .collect();
    let tree: Vec<f64> = inside
        .iter()
        .map(|set| to_f64(tree_footprint(set)))
        .collect();
    percentiles(phase, "footprint_app", &app, &mut metrics);
    percentiles(phase, "footprint_tree", &tree, &mut metrics);
    for (name, bytes) in child_maxima(&inside) {
        let metric = format!("footprint_child_max:{name}");
        metrics.push(Metric::new(phase, metric, to_f64(bytes), "bytes"));
    }
    let cpu_ms = to_f64(tree_cpu_ns(end).saturating_sub(tree_cpu_ns(start))) / 1e6;
    let seconds = duration_ms / 1e3;
    metrics.push(Metric::new(phase, "cpu_time", cpu_ms, "ms"));
    let idle = to_f64(tree_idle_wakeups(end).saturating_sub(tree_idle_wakeups(start)));
    let interrupt =
        to_f64(tree_interrupt_wakeups(end).saturating_sub(tree_interrupt_wakeups(start)));
    metrics.push(Metric::new(phase, "idle_wakeups", idle, "count"));
    metrics.push(Metric::new(phase, "interrupt_wakeups", interrupt, "count"));
    if seconds > 0.0 {
        metrics.push(Metric::new(
            phase,
            "cpu_core_pct",
            cpu_ms / duration_ms * 100.0,
            "%",
        ));
        metrics.push(Metric::new(
            phase,
            "idle_wakeups_rate",
            idle / seconds,
            "per_s",
        ));
        metrics.push(Metric::new(
            phase,
            "interrupt_wakeups_rate",
            interrupt / seconds,
            "per_s",
        ));
    }
    metrics.push(Metric::new(
        phase,
        "energy",
        to_f64(energy_nj(start, end)) / 1e6,
        "mJ",
    ));
    Ok(metrics)
}

/// Window and lifetime metrics for the whole trial.
pub fn trial_metrics(
    phase: &str,
    sets: &[SampleSet],
    appeared: &Appeared,
    footprint_tool: Option<u64>,
    timebase: Timebase,
) -> Vec<Metric> {
    let mut metrics = Vec::new();
    let root = sets
        .iter()
        .flat_map(|set| set.rows.iter())
        .find(|row| row.depth == 0);
    if let Some(root) = root.filter(|root| appeared.mach >= root.start_mach) {
        let launch = timebase.millis_between(root.start_mach, appeared.mach);
        metrics.push(Metric::new(phase, "launch_to_window", launch, "ms"));
    }
    metrics.push(Metric::new(phase, "window_width", appeared.width, "pt"));
    metrics.push(Metric::new(phase, "window_height", appeared.height, "pt"));
    let peak = sets
        .iter()
        .rev()
        .flat_map(|set| set.rows.iter())
        .find(|row| row.depth == 0)
        .map(|row| row.lifetime_max_footprint);
    if let Some(peak) = peak {
        metrics.push(Metric::new(
            phase,
            "footprint_app_lifetime_peak",
            to_f64(peak),
            "bytes",
        ));
    }
    if let Some(bytes) = footprint_tool {
        metrics.push(Metric::new(
            phase,
            "footprint_tool_end",
            to_f64(bytes),
            "bytes",
        ));
    }
    metrics
}

/// Frames whose region hash differs from the frame before. The first frame
/// is the baseline and never counts as a change.
fn changed_frames(frames: &[Frame]) -> Vec<Frame> {
    let mut sorted = frames.to_vec();
    sorted.sort_by_key(|frame| (frame.display_mach, frame.seq));
    sorted
        .windows(2)
        .filter(|pair| pair[0].hash != pair[1].hash)
        .map(|pair| pair[1])
        .collect()
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct KeyLatencies {
    pub measured_ms: Vec<f64>,
    pub late: usize,
    pub missed: usize,
    pub ambiguous: usize,
    pub invalid_timestamps: bool,
    pub record_limit_exceeded: bool,
}

// This bounds the sort and maps above nine minutes at 120 Hz; key workloads
// currently post 100 events over 25 seconds plus a short capture tail.
const MAX_KEY_RECORDS: usize = 65_536;

/// Hash changes are diagnostic, never proof of the intended text or caret.
/// One unresolved interval makes every later match ambiguous: a delayed
/// response can otherwise look like a fast answer to a later key.
pub fn key_latencies(
    events: &[InputEvent],
    frames: &[Frame],
    timebase: Timebase,
    spacing_ticks: u64,
) -> KeyLatencies {
    let mut result = KeyLatencies::default();
    if frames.len() > MAX_KEY_RECORDS || events.len() > MAX_KEY_RECORDS {
        result.record_limit_exceeded = true;
        result.ambiguous = events.len();
        return result;
    }
    let changes = changed_frames(frames);
    let mut by_sequence = BTreeMap::new();
    let mut by_display = BTreeMap::new();
    let bad_frames = frames.iter().any(|frame| {
        frame.display_mach == 0
            || frame.arrival_mach < frame.display_mach
            || by_sequence
                .insert(frame.seq, (frame.display_mach, frame.hash))
                .is_some_and(|prior| prior != (frame.display_mach, frame.hash))
            || by_display
                .insert(frame.display_mach, frame.hash)
                .is_some_and(|hash| hash != frame.hash)
    });
    result.invalid_timestamps = spacing_ticks == 0
        || timebase.numer == 0
        || timebase.denom == 0
        || bad_frames
        || events.iter().any(|event| event.mach == 0)
        || events
            .windows(2)
            .any(|pair| pair[0].mach >= pair[1].mach || pair[0].seq >= pair[1].seq)
        || events
            .last()
            .is_some_and(|event| event.mach.checked_add(spacing_ticks).is_none());
    if result.invalid_timestamps {
        result.ambiguous = events.len();
        return result;
    }
    let mut ambiguous = events.first().is_none_or(|first| {
        !frames.iter().any(|frame| frame.display_mach < first.mach)
            || changes.iter().any(|frame| frame.display_mach < first.mach)
    });
    for (index, event) in events.iter().enumerate() {
        let window_end = events
            .get(index + 1)
            .map_or(event.mach.saturating_add(spacing_ticks), |next| next.mach);
        let response = changes
            .iter()
            .find(|frame| frame.display_mach >= event.mach)
            .map(|frame| frame.display_mach);
        match response {
            Some(display) if display > event.mach && display < window_end && !ambiguous => result
                .measured_ms
                .push(timebase.millis_between(event.mach, display)),
            other => {
                ambiguous = true;
                match other {
                    None => result.missed += 1,
                    Some(display) if display >= window_end => result.late += 1,
                    Some(_) => result.ambiguous += 1,
                }
            }
        }
    }
    result
}

/// The captured band in window points as (x, y, width, height): the width
/// less 32 pt of scroll bar, from 30% down to 40 pt above the bottom, so
/// title, tab and status bars and scroll-bar fades never count as changes.
pub fn capture_region(width: f64, height: f64) -> (f64, f64, f64, f64) {
    let top = (height * 0.3).round();
    let bottom = (height - 40.0).max(top + 1.0);
    (0.0, top, (width - 32.0).max(1.0), bottom - top)
}

/// Intervals between consecutive changed frames displayed in the window.
pub fn frame_intervals(frames: &[Frame], start: u64, end: u64, timebase: Timebase) -> Vec<f64> {
    let changes: Vec<Frame> = changed_frames(frames)
        .into_iter()
        .filter(|frame| frame.display_mach >= start && frame.display_mach <= end)
        .collect();
    changes
        .windows(2)
        .map(|pair| timebase.millis_between(pair[0].display_mach, pair[1].display_mach))
        .collect()
}

/// How many different region hashes the frames shown in `start..=end` have.
/// Scrolling text changes the band on nearly every frame; a scroll that never
/// landed left 2 to 10 states on 2026-10-03, against thousands for a real one.
pub fn distinct_frames(frames: &[Frame], start: u64, end: u64) -> usize {
    frames
        .iter()
        .filter(|frame| frame.display_mach >= start && frame.display_mach <= end)
        .map(|frame| frame.hash)
        .collect::<BTreeSet<u64>>()
        .len()
}

/// The scroll phase's frame metrics for the frames shown in `span`: intervals
/// between changed frames, and how many distinct window states there were.
pub fn scroll_metrics(
    phase: &str,
    frames: &[Frame],
    span: (u64, u64),
    (timebase, refresh_hz): (Timebase, u32),
) -> Vec<Metric> {
    let intervals = frame_intervals(frames, span.0, span.1, timebase);
    let mut metrics = interval_metrics(phase, &intervals, refresh_hz);
    let distinct = distinct_frames(frames, span.0, span.1);
    metrics.push(Metric::new(
        phase,
        "frame_distinct",
        to_f64(distinct as u64),
        "count",
    ));
    metrics
}

/// The span an input phase is judged over, in mach ticks: its first posted
/// event to 100 ms after its last. Frames outside it are not its own.
pub fn input_span(events: &[InputEvent], timebase: Timebase) -> Option<(u64, u64)> {
    let (first, last) = (events.first()?, events.last()?);
    Some((
        first.mach,
        last.mach.saturating_add(timebase.ticks(100_000_000)),
    ))
}

/// The fewest distinct window states a scroll of `events` wheel events may
/// leave: one per 50 events, at least 20, and never more than the events.
pub fn min_scroll_states(events: usize) -> usize {
    (events / 50).max(20).min(events)
}

/// Why a scroll did not scroll: it posted nothing, or its span shows too few
/// distinct states. The count is the one `scroll_metrics` reports.
pub fn scroll_refusal(
    events: &[InputEvent],
    frames: &[Frame],
    timebase: Timebase,
) -> Option<String> {
    let Some(span) = input_span(events, timebase) else {
        return Some("the helper posted no scroll events; the scroll did not land".to_owned());
    };
    let distinct = distinct_frames(frames, span.0, span.1);
    let needed = min_scroll_states(events.len());
    (distinct < needed).then(|| {
        format!(
            "{} scroll events left the window in {distinct} distinct states, \
             fewer than the {needed} a working scroll leaves; the scroll did not land",
            events.len()
        )
    })
}

/// The fewest distinct band states `keys` presses may leave: half, at least 10,
/// never more than the keys. A late key still changes the band, so slow passes.
pub fn min_key_states(keys: usize) -> usize {
    (keys / 2).max(10).min(keys)
}

/// A diagnostic warning about absent or sparse band changes. Slow or dropped
/// responses remain measured outcomes, never replaceable environment failures.
pub fn key_band_warning(
    events: &[InputEvent],
    frames: &[Frame],
    timebase: Timebase,
) -> Option<String> {
    let Some(span) = input_span(events, timebase) else {
        return Some("the helper posted no keys; the keys did not land".to_owned());
    };
    let distinct = distinct_frames(frames, span.0, span.1);
    let needed = min_key_states(events.len());
    let plural = if distinct == 1 { "" } else { "s" };
    (distinct < needed).then(|| {
        format!(
            "{} keys left the band in {distinct} distinct state{plural}, \
             fewer than the {needed} required",
            events.len()
        )
    })
}

/// The first sighting's time and id with the frame read after the last phase,
/// since the first sighting can land in the window-open animation. The reason
/// names a frame more than a point from the requested one.
pub fn settle_frame(
    first: Appeared,
    window: &WindowInfo,
    want: (u32, u32),
) -> (Appeared, Option<String>) {
    let settled = Appeared {
        x: window.x,
        y: window.y,
        width: window.width,
        height: window.height,
        ..first
    };
    let off = (window.width - f64::from(want.0)).abs() > 1.0
        || (window.height - f64::from(want.1)).abs() > 1.0;
    let reason = off.then(|| {
        format!(
            "the window is {}x{} pt, not the requested {}x{} pt",
            window.width, window.height, want.0, want.1
        )
    });
    (settled, reason)
}

pub fn latency_metrics(phase: &str, latencies: &KeyLatencies, sent: usize) -> Vec<Metric> {
    let complete = sent > 0
        && latencies.measured_ms.len() == sent
        && latencies.late + latencies.missed + latencies.ambiguous == 0
        && !latencies.invalid_timestamps
        && !latencies.record_limit_exceeded;
    let mut metrics = vec![
        Metric::new(phase, "keys_sent", to_f64(sent as u64), "count"),
        Metric::new(
            phase,
            "keys_hash_timed",
            to_f64(latencies.measured_ms.len() as u64),
            "count",
        ),
        Metric::new(phase, "keys_late", to_f64(latencies.late as u64), "count"),
        Metric::new(
            phase,
            "keys_missed",
            to_f64(latencies.missed as u64),
            "count",
        ),
        Metric::new(
            phase,
            "keys_ambiguous",
            to_f64(latencies.ambiguous as u64),
            "count",
        ),
        Metric::new(
            phase,
            "key_timestamps_valid",
            f64::from(!latencies.invalid_timestamps && !latencies.record_limit_exceeded),
            "bool",
        ),
        Metric::new(
            phase,
            "key_hash_timing_complete",
            f64::from(complete),
            "bool",
        ),
        Metric::new(phase, "key_semantic_qualified", 0.0, "bool"),
        Metric::new(
            phase,
            "key_analysis_within_limit",
            f64::from(!latencies.record_limit_exceeded),
            "bool",
        ),
    ];
    if !complete {
        return metrics;
    }
    for (suffix, quantile) in [("p50", 0.5), ("p95", 0.95), ("max", 1.0)] {
        if let Some(value) = stats::nearest_rank(&latencies.measured_ms, quantile) {
            metrics.push(Metric::new(
                phase,
                format!("key_hash_change_{suffix}"),
                value,
                "ms",
            ));
        }
    }
    metrics
}

/// `late_pct` counts intervals longer than 1.5 refresh periods.
pub fn interval_metrics(phase: &str, intervals: &[f64], refresh_hz: u32) -> Vec<Metric> {
    let mut metrics = vec![Metric::new(
        phase,
        "frame_intervals",
        to_f64(intervals.len() as u64),
        "count",
    )];
    for (suffix, quantile) in [("p50", 0.5), ("p95", 0.95), ("max", 1.0)] {
        if let Some(value) = stats::nearest_rank(intervals, quantile) {
            metrics.push(Metric::new(
                phase,
                format!("frame_interval_{suffix}"),
                value,
                "ms",
            ));
        }
    }
    if !intervals.is_empty() && refresh_hz > 0 {
        let threshold = 1.5 * 1_000.0 / f64::from(refresh_hz);
        let late = intervals
            .iter()
            .filter(|interval| **interval > threshold)
            .count();
        let pct = to_f64(late as u64) / to_f64(intervals.len() as u64) * 100.0;
        metrics.push(Metric::new(phase, "frame_late_pct", pct, "%"));
    }
    metrics
}

#[derive(Clone, Debug, PartialEq)]
pub enum Visibility {
    Visible(WindowInfo),
    NoWindow,
    NotFrontmost {
        pid: i32,
        name: String,
    },
    Occluded {
        window: WindowInfo,
        by: Vec<WindowInfo>,
    },
}

impl Visibility {
    pub fn describe(&self) -> String {
        match self {
            Self::Visible(window) => format!("window {} is visible", window.id),
            Self::NoWindow => "the app has no on-screen window".to_owned(),
            Self::NotFrontmost { pid, name } => {
                format!("the app is not frontmost ({name}, pid {pid}, is)")
            }
            Self::Occluded { window, by } => {
                let owners: Vec<String> = by
                    .iter()
                    .map(|other| {
                        format!("{} (pid {}, layer {})", other.owner, other.pid, other.layer)
                    })
                    .collect();
                format!("window {} is covered by {}", window.id, owners.join(", "))
            }
        }
    }
}

fn overlap_area(a: &WindowInfo, b: &WindowInfo) -> f64 {
    let width = (a.x + a.width).min(b.x + b.width) - a.x.max(b.x);
    let height = (a.y + a.height).min(b.y + b.height) - a.y.max(b.y);
    if width > 0.0 && height > 0.0 {
        width * height
    } else {
        0.0
    }
}

// macOS draws the pointer as a Window Server window at this fixed level; it
// never hides content, so it must not count as covering the target.
const CURSOR_WINDOW_LEVEL: i64 = 2_147_483_630;

fn is_pointer(window: &WindowInfo) -> bool {
    window.layer == CURSOR_WINDOW_LEVEL && window.owner == "Window Server"
}

/// The target's frontmost normal window must exist, its app must be active,
/// and no other app's visible window may overlap it from above.
pub fn visibility(list: &WindowList, pid: i32, min_side: f64) -> Visibility {
    let Some(target) = list.windows.iter().find(|window| {
        window.pid == pid
            && window.layer == 0
            && window.alpha > 0.0
            && window.width >= min_side
            && window.height >= min_side
    }) else {
        return Visibility::NoWindow;
    };
    if list.front_pid != pid {
        return Visibility::NotFrontmost {
            pid: list.front_pid,
            name: list.front_name.clone(),
        };
    }
    let by: Vec<WindowInfo> = list
        .windows
        .iter()
        .filter(|window| {
            window.order < target.order
                && window.pid != pid
                && window.alpha > 0.0
                && !is_pointer(window)
                && overlap_area(window, target) > 0.0
        })
        .cloned()
        .collect();
    if by.is_empty() {
        Visibility::Visible(target.clone())
    } else {
        Visibility::Occluded {
            window: target.clone(),
            by,
        }
    }
}

/// `(elapsed ns, cumulative tree CPU ns)` for every set from `since_seq`.
pub fn cpu_points(sets: &[SampleSet], since_seq: u64, timebase: Timebase) -> Vec<(u64, u64)> {
    let mut selected = sets.iter().filter(|set| set.seq >= since_seq);
    let Some(first) = selected.next() else {
        return Vec::new();
    };
    std::iter::once(first)
        .chain(selected)
        .map(|set| {
            (
                timebase.nanos(set.mach.saturating_sub(first.mach)),
                tree_cpu_ns(set),
            )
        })
        .collect()
}

/// The first time at which the trailing `window_ns` used less than
/// `quiet_core_pct` of one core.
pub fn quiet_since(points: &[(u64, u64)], quiet_core_pct: f64, window_ns: u64) -> Option<u64> {
    for (index, (time, cpu)) in points.iter().enumerate() {
        let earlier = points
            .get(..index)?
            .iter()
            .rev()
            .find(|(start, _)| time.saturating_sub(*start) >= window_ns);
        if let Some((start, start_cpu)) = earlier {
            let span = to_f64(time - start);
            let used = to_f64(cpu.saturating_sub(*start_cpu));
            if span > 0.0 && used / span * 100.0 < quiet_core_pct {
                return Some(*time);
            }
        }
    }
    None
}

#[derive(Clone, Debug, PartialEq)]
pub struct SummaryRow {
    pub phase: String,
    pub name: String,
    pub unit: &'static str,
    pub summary: Summary,
}

/// One row per (phase, metric) across trials, in first-seen order.
pub fn summarize(metrics: &[Metric]) -> Vec<SummaryRow> {
    let mut order: Vec<(String, String, &'static str)> = Vec::new();
    let mut values: BTreeMap<(String, String), Vec<f64>> = BTreeMap::new();
    for metric in metrics {
        let key = (metric.phase.clone(), metric.name.clone());
        if !values.contains_key(&key) {
            order.push((metric.phase.clone(), metric.name.clone(), metric.unit));
        }
        values.entry(key).or_default().push(metric.value);
    }
    order
        .into_iter()
        .filter_map(|(phase, name, unit)| {
            let samples = values.get(&(phase.clone(), name.clone()))?;
            Some(SummaryRow {
                summary: stats::summarize(samples)?,
                phase,
                name,
                unit,
            })
        })
        .collect()
}

pub const CHILD_FOOTPRINT: &str = "footprint_child_max:";

/// Like `summarize` over valid trials, but a child name that a tick caught
/// in some trials only (a short-lived `rustc`) counts as 0 bytes in a trial
/// that ran the phase without it, so every row has one value per trial.
pub fn summarize_trials(trials: &[Vec<Metric>]) -> Vec<SummaryRow> {
    let mut children: Vec<(&str, &str, &'static str)> = Vec::new();
    for metric in trials.iter().flatten() {
        let key = (metric.phase.as_str(), metric.name.as_str(), metric.unit);
        if metric.name.starts_with(CHILD_FOOTPRINT) && !children.contains(&key) {
            children.push(key);
        }
    }
    let mut filled: Vec<Metric> = Vec::new();
    for trial in trials {
        filled.extend(trial.iter().cloned());
        for (phase, name, unit) in &children {
            let ran_phase = trial.iter().any(|metric| metric.phase == *phase);
            let has_child = trial
                .iter()
                .any(|metric| metric.phase == *phase && metric.name == *name);
            if ran_phase && !has_child {
                filled.push(Metric::new(phase, *name, 0.0, unit));
            }
        }
    }
    summarize(&filled)
        .into_iter()
        .filter(|row| {
            let qualification = if row.name.starts_with("key_hash_change_") {
                "key_hash_timing_complete"
            } else if row.name.starts_with("key_to_screen_") {
                "key_semantic_qualified"
            } else {
                return true;
            };
            trials.iter().all(|trial| {
                trial
                    .iter()
                    .any(|metric| metric.phase == row.phase && metric.name == row.name)
                    && trial.iter().any(|metric| {
                        metric.phase == row.phase
                            && metric.name == qualification
                            && metric.value.total_cmp(&1.0).is_eq()
                    })
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        KeyLatencies, MAX_KEY_RECORDS, Metric, PhaseWindow, Visibility, capture_region,
        child_names, child_process_evidence, cpu_points, distinct_frames, frame_intervals,
        input_span, interval_metrics, key_band_warning, key_latencies, latency_metrics,
        min_key_states, min_scroll_states, phase_label, phase_metrics, phase_windows, quiet_since,
        reaped_child_cpu_ns, scroll_metrics, scroll_refusal, settle_frame, summarize,
        summarize_trials, trial_metrics, visibility,
    };
    use crate::helpers::{
        Appeared, Frame, InputEvent, Mark, ProcRow, SampleSet, SetKind, Timebase, WindowInfo,
        WindowList, parse_frame, parse_input_events,
    };

    /// One tick per nanosecond keeps the arithmetic readable.
    const UNIT: Timebase = Timebase { numer: 1, denom: 1 };
    const SECOND: u64 = 1_000_000_000;

    fn mark(seq: u64, mach: u64, label: &str) -> Mark {
        Mark {
            seq,
            mach,
            label: label.to_owned(),
        }
    }

    fn row(pid: i32, depth: u32, name: &str, footprint: u64, cpu_ns: u64) -> ProcRow {
        ProcRow {
            pid,
            ppid: if depth == 0 { 1 } else { 600 },
            depth,
            name: name.to_owned(),
            footprint,
            lifetime_max_footprint: footprint * 2,
            user_ns: cpu_ns,
            system_ns: 0,
            child_user_ns: 0,
            child_system_ns: 0,
            idle_wakeups: cpu_ns / 1_000_000,
            interrupt_wakeups: 1,
            child_idle_wakeups: 0,
            child_interrupt_wakeups: 0,
            instructions: 0,
            cycles: 0,
            energy_nj: cpu_ns,
            start_mach: 0,
            exited: false,
        }
    }

    fn set(seq: u64, kind: SetKind, mach: u64, rows: Vec<ProcRow>) -> SampleSet {
        SampleSet {
            seq,
            kind,
            mach,
            rows,
        }
    }

    fn value(metrics: &[Metric], name: &str) -> Option<f64> {
        metrics
            .iter()
            .find(|metric| metric.name == name)
            .map(|metric| metric.value)
    }

    #[test]
    fn child_names_list_each_child_any_sample_saw_once() {
        let alone = [set(
            1,
            SetKind::Tick,
            10,
            vec![row(600, 0, "alpine-editor", 1, 1)],
        )];
        assert!(child_names(&alone).is_empty());
        let with_server = [
            alone[0].clone(),
            set(
                2,
                SetKind::Tick,
                20,
                vec![
                    row(600, 0, "alpine-editor", 1, 1),
                    row(700, 1, "clangd", 1, 1),
                    row(701, 2, "clangd", 1, 1),
                ],
            ),
            set(3, SetKind::Mark, 30, vec![row(702, 1, "xcrun", 1, 1)]),
        ];
        assert_eq!(child_names(&with_server), ["clangd", "xcrun"]);
    }

    #[test]
    fn a_child_that_came_and_went_between_samples_still_voids() {
        let quiet = [set(
            1,
            SetKind::Tick,
            10,
            vec![row(600, 0, "alpine-editor", 1, 1)],
        )];
        assert_eq!(reaped_child_cpu_ns(&quiet), 0);
        assert_eq!(child_process_evidence(&quiet), None);
        let mut reaped = row(600, 0, "alpine-editor", 1, 1);
        reaped.child_user_ns = 2_000_000;
        let between = [quiet[0].clone(), set(2, SetKind::Mark, 20, vec![reaped])];
        assert_eq!(reaped_child_cpu_ns(&between), 2_000_000);
        let evidence = child_process_evidence(&between).unwrap_or_default();
        assert!(evidence.contains("reaped 2000000 ns"), "{evidence}");
        let seen = [set(
            1,
            SetKind::Tick,
            10,
            vec![
                row(600, 0, "alpine-editor", 1, 1),
                row(700, 1, "clangd", 1, 1),
            ],
        )];
        assert_eq!(
            child_process_evidence(&seen).as_deref(),
            Some("the app started clangd")
        );
    }

    #[test]
    fn marks_pair_into_non_nested_phases() -> Result<(), String> {
        let marks = vec![
            mark(1, 10, &phase_label("startup", "start")),
            mark(4, 40, &phase_label("startup", "end")),
            mark(5, 50, "unrelated"),
            mark(6, 60, &phase_label("idle", "start")),
            mark(9, 90, &phase_label("idle", "end")),
        ];
        let windows = phase_windows(&marks)?;
        assert_eq!(windows.len(), 2);
        assert_eq!(
            windows[1],
            PhaseWindow {
                name: "idle".to_owned(),
                start_seq: 6,
                end_seq: 9,
                start_mach: 60,
                end_mach: 90,
            }
        );
        assert!(
            phase_windows(&[mark(1, 1, "phase:a:start"), mark(2, 2, "phase:b:start")]).is_err()
        );
        assert!(phase_windows(&[mark(1, 1, "phase:a:start")]).is_err());
        assert!(phase_windows(&[mark(1, 1, "phase:a:end")]).is_err());
        Ok(())
    }

    #[test]
    fn phase_metrics_use_ticks_for_footprint_and_boundaries_for_cpu() -> Result<(), String> {
        let mib = 1_048_576;
        let bytes = |count: u64| crate::stats::to_f64(count * mib);
        let sets = vec![
            set(
                1,
                SetKind::Mark,
                0,
                vec![row(600, 0, "alpine-editor", 30 * mib, 0)],
            ),
            set(
                2,
                SetKind::Tick,
                SECOND,
                vec![row(600, 0, "alpine-editor", 40 * mib, 5_000_000)],
            ),
            set(
                3,
                SetKind::Tick,
                2 * SECOND,
                vec![
                    row(600, 0, "alpine-editor", 50 * mib, 8_000_000),
                    row(601, 1, "rust-analyzer", 100 * mib, 1_000_000),
                ],
            ),
            set(
                4,
                SetKind::Mark,
                3 * SECOND,
                vec![
                    row(600, 0, "alpine-editor", 45 * mib, 10_000_000),
                    row(601, 1, "rust-analyzer", 120 * mib, 2_000_000),
                ],
            ),
        ];
        let window = PhaseWindow {
            name: "idle".to_owned(),
            start_seq: 1,
            end_seq: 4,
            start_mach: 0,
            end_mach: 3 * SECOND,
        };
        let metrics = phase_metrics(&window, &sets, UNIT, false)?;
        assert!(metrics.iter().all(|metric| metric.phase == "idle"));
        assert_eq!(value(&metrics, "samples"), Some(2.0));
        assert_eq!(value(&metrics, "duration"), Some(3_000.0));
        assert_eq!(value(&metrics, "footprint_app_p50"), Some(bytes(40)));
        assert_eq!(value(&metrics, "footprint_app_max"), Some(bytes(50)));
        assert_eq!(value(&metrics, "footprint_tree_max"), Some(bytes(150)));
        assert_eq!(
            value(&metrics, "footprint_child_max:rust-analyzer"),
            Some(bytes(100))
        );
        assert_eq!(value(&metrics, "cpu_time"), Some(12.0));
        let pct = value(&metrics, "cpu_core_pct").ok_or("cpu pct")?;
        assert!((pct - 0.4).abs() < 1e-9, "{pct}");
        assert_eq!(value(&metrics, "idle_wakeups"), Some(12.0));
        assert_eq!(value(&metrics, "interrupt_wakeups"), Some(1.0));
        assert_eq!(value(&metrics, "energy"), Some(12.0));
        Ok(())
    }

    #[test]
    fn reaped_children_stay_counted_through_the_parent() -> Result<(), String> {
        let mut parent_after = row(600, 0, "zed", 10, 4_000_000);
        parent_after.child_user_ns = 3_000_000;
        let sets = vec![
            set(
                1,
                SetKind::Mark,
                0,
                vec![
                    row(600, 0, "zed", 10, 1_000_000),
                    row(700, 1, "zsh", 1, 2_000_000),
                ],
            ),
            set(2, SetKind::Mark, SECOND, vec![parent_after]),
        ];
        let window = PhaseWindow {
            name: "startup".to_owned(),
            start_seq: 1,
            end_seq: 2,
            start_mach: 0,
            end_mach: SECOND,
        };
        let metrics = phase_metrics(&window, &sets, UNIT, false)?;
        assert_eq!(value(&metrics, "cpu_time"), Some(4.0));
        assert_eq!(value(&metrics, "samples"), Some(0.0));
        Ok(())
    }

    #[test]
    fn startup_counts_from_process_start() -> Result<(), String> {
        let mut early = row(600, 0, "alpine-editor", 10, 2_000_000);
        early.start_mach = SECOND / 2;
        let mut late = row(600, 0, "alpine-editor", 20, 9_000_000);
        late.start_mach = SECOND / 2;
        let sets = vec![
            set(1, SetKind::Tick, SECOND, vec![early.clone()]),
            set(2, SetKind::Mark, SECOND + 10, vec![early]),
            set(3, SetKind::Mark, 3 * SECOND, vec![late]),
        ];
        let window = PhaseWindow {
            name: "startup".to_owned(),
            start_seq: 2,
            end_seq: 3,
            start_mach: SECOND + 10,
            end_mach: 3 * SECOND,
        };
        let marked = phase_metrics(&window, &sets, UNIT, false)?;
        let launched = phase_metrics(&window, &sets, UNIT, true)?;
        assert_eq!(value(&marked, "cpu_time"), Some(7.0));
        assert_eq!(value(&launched, "cpu_time"), Some(9.0));
        assert_eq!(value(&launched, "duration"), Some(2_500.0));
        assert_eq!(value(&launched, "samples"), Some(1.0));
        Ok(())
    }

    #[test]
    fn trial_metrics_report_launch_window_and_peaks() {
        let sets = vec![set(
            1,
            SetKind::Tick,
            5,
            vec![row(600, 0, "alpine-editor", 64, 0)],
        )];
        let appeared = Appeared {
            mach: 250_000_000,
            id: 1,
            x: 0.0,
            y: 0.0,
            width: 960.0,
            height: 568.0,
        };
        let metrics = trial_metrics("trial", &sets, &appeared, Some(70), UNIT);
        assert_eq!(value(&metrics, "launch_to_window"), Some(250.0));
        assert_eq!(value(&metrics, "window_height"), Some(568.0));
        assert_eq!(value(&metrics, "footprint_app_lifetime_peak"), Some(128.0));
        assert_eq!(value(&metrics, "footprint_tool_end"), Some(70.0));
    }

    fn frame(seq: u64, display: u64, hash: u64) -> Frame {
        Frame {
            seq,
            display_mach: display,
            arrival_mach: display + 1,
            hash,
        }
    }

    fn key(seq: u64, mach: u64) -> InputEvent {
        InputEvent {
            seq,
            mach,
            kind: "key".to_owned(),
            label: "a".to_owned(),
        }
    }

    const MS: u64 = 1_000_000;

    #[test]
    fn four_hundred_ms_responses_never_alias_to_one_hundred_fifty_ms() {
        let events: Vec<_> = (0..6).map(|i| key(i + 1, (100 + i * 250) * MS)).collect();
        let mut frames = vec![frame(1, 1, 0)];
        frames.extend(
            events
                .iter()
                .map(|event| frame(event.seq + 1, event.mach + 400 * MS, event.seq)),
        );
        let latencies = key_latencies(&events, &frames, UNIT, 250 * MS);
        assert!(latencies.measured_ms.is_empty(), "{latencies:?}");
    }

    fn responses(events: &[InputEvent], delay: u64) -> Vec<Frame> {
        let mut frames = vec![frame(0, 1, 0)];
        frames.extend(
            events
                .iter()
                .map(|event| frame(event.seq, event.mach + delay, event.seq)),
        );
        frames
    }

    fn assert_unqualified(latencies: &KeyLatencies, sent: usize, complete: bool) {
        let metrics = latency_metrics("typing", latencies, sent);
        assert_eq!(value(&metrics, "key_semantic_qualified"), Some(0.0));
        assert_eq!(
            value(&metrics, "key_hash_timing_complete"),
            Some(f64::from(complete))
        );
        for suffix in ["p50", "p95", "max"] {
            assert_eq!(value(&metrics, &format!("key_to_screen_{suffix}")), None);
            assert_eq!(
                value(&metrics, &format!("key_hash_change_{suffix}")).is_some(),
                complete
            );
        }
        assert_eq!(
            latencies.measured_ms.len() + latencies.late + latencies.missed + latencies.ambiguous,
            sent
        );
    }

    #[test]
    fn delay_sweep_is_complete_only_inside_every_interval() {
        let events = key_run(8);
        for delay in [0, 1, 8, 100, 249, 250, 400, 500, 750] {
            let latencies = key_latencies(&events, &responses(&events, delay * MS), UNIT, 250 * MS);
            let complete = delay > 0 && delay < 250;
            assert_eq!(
                latencies.measured_ms.len(),
                if complete { 8 } else { 0 },
                "{delay}"
            );
            assert_unqualified(&latencies, 8, complete);
        }
    }

    #[test]
    fn dropping_first_middle_or_last_key_suppresses_all_percentiles() {
        let events = key_run(100);
        for dropped in [0, 49, 99] {
            let mut frames = responses(&events, 30 * MS);
            frames.remove(dropped + 1);
            let latencies = key_latencies(&events, &frames, UNIT, 250 * MS);
            assert_eq!(latencies.measured_ms.len(), dropped);
            assert_unqualified(&latencies, 100, false);
            if dropped == 99 {
                assert_eq!(latencies.missed, 1);
            } else {
                assert_eq!(latencies.late, 1);
                assert_eq!(latencies.ambiguous, 99 - dropped);
            }
        }
    }

    #[test]
    fn coalesced_responses_cannot_restore_identity() {
        let events = key_run(6);
        let mut frames = responses(&events, 30 * MS);
        frames.remove(1);
        frames.remove(2);
        frames.remove(3);
        let latencies = key_latencies(&events, &frames, UNIT, 250 * MS);
        assert_eq!((latencies.late, latencies.ambiguous), (3, 3));
        assert_unqualified(&latencies, 6, false);
    }

    #[test]
    fn closely_spaced_inputs_still_require_disjoint_response_windows() {
        let events = vec![key(1, MS), key(2, 2 * MS), key(3, 3 * MS)];
        let fast = key_latencies(&events, &responses(&events, MS / 2), UNIT, MS);
        assert_eq!(fast.measured_ms, vec![0.5; 3]);
        assert_unqualified(&fast, 3, true);
        let slow = key_latencies(&events, &responses(&events, MS * 3 / 2), UNIT, MS);
        assert_unqualified(&slow, 3, false);
        assert!(slow.measured_ms.is_empty());
    }

    #[test]
    fn capture_arrival_reordering_and_identical_duplicates_do_not_change_display_timing() {
        let events = key_run(3);
        let expected = key_latencies(&events, &responses(&events, 30 * MS), UNIT, 250 * MS);
        let mut frames = responses(&events, 30 * MS);
        frames[1].arrival_mach += 1_000 * MS;
        frames.push(frames[1]);
        frames.reverse();
        assert_eq!(key_latencies(&events, &frames, UNIT, 250 * MS), expected);
    }

    #[test]
    fn malformed_event_or_capture_timestamps_fail_closed() {
        let events = key_run(3);
        let frames = responses(&events, 30 * MS);
        let mut bad_events = vec![events.clone(); 4];
        bad_events[0][1].mach = bad_events[0][0].mach;
        bad_events[1].reverse();
        bad_events[2][1].seq = bad_events[2][0].seq;
        bad_events[3][2].mach = u64::MAX;
        for events in bad_events {
            let result = key_latencies(&events, &frames, UNIT, 250 * MS);
            assert!(result.invalid_timestamps);
            assert_unqualified(&result, 3, false);
        }
        let mut bad_frames = vec![frames.clone(); 3];
        bad_frames[0][1].arrival_mach = 1;
        bad_frames[1][2].seq = bad_frames[1][1].seq;
        bad_frames[2][2].display_mach = bad_frames[2][1].display_mach;
        for frames in bad_frames {
            let result = key_latencies(&events, &frames, UNIT, 250 * MS);
            assert!(result.invalid_timestamps);
            assert_unqualified(&result, 3, false);
        }
        for (clock, spacing) in [
            (Timebase { numer: 0, denom: 1 }, MS),
            (Timebase { numer: 1, denom: 0 }, MS),
            (UNIT, 0),
        ] {
            assert!(key_latencies(&events, &frames, clock, spacing).invalid_timestamps);
        }
    }

    #[test]
    fn absent_or_unstable_pre_input_baselines_are_ambiguous() {
        let events = key_run(3);
        let frames = responses(&events, 30 * MS);
        let missing = key_latencies(&events, &frames[1..], UNIT, 250 * MS);
        assert_unqualified(&missing, 3, false);
        let mut unstable = frames;
        unstable.push(frame(100, 500 * MS, 99));
        let result = key_latencies(&events, &unstable, UNIT, 250 * MS);
        assert_unqualified(&result, 3, false);
        assert!(result.measured_ms.is_empty());
    }

    #[test]
    fn missing_display_time_sentinel_cannot_be_a_pre_input_baseline() {
        let events = key_run(3);
        let frames = responses(&events, 30 * MS);
        let mut missing_display = frames.clone();
        missing_display[0].display_mach = 0;
        let mut missing_arrival = frames.clone();
        missing_arrival[0].arrival_mach = 0;
        for capture in [missing_display, missing_arrival] {
            let latencies = key_latencies(&events, &capture, UNIT, 250 * MS);
            assert!(latencies.invalid_timestamps);
            assert_unqualified(&latencies, 3, false);
        }
        let mut missing_event = events;
        missing_event[0].mach = 0;
        let latencies = key_latencies(&missing_event, &frames, UNIT, 250 * MS);
        assert!(latencies.invalid_timestamps);
        assert_unqualified(&latencies, 3, false);
    }

    #[test]
    fn key_analysis_refuses_records_above_its_memory_bound() {
        let events = key_run(1);
        let mut frames = vec![frame(0, 1, 0); MAX_KEY_RECORDS];
        frames[MAX_KEY_RECORDS - 1] = frame(1, events[0].mach + 30 * MS, 1);
        let at_limit = key_latencies(&events, &frames, UNIT, 250 * MS);
        assert!(!at_limit.record_limit_exceeded);
        assert_unqualified(&at_limit, 1, true);
        frames.push(frames[0]);
        let above_limit = key_latencies(&events, &frames, UNIT, 250 * MS);
        assert!(above_limit.record_limit_exceeded);
        assert_unqualified(&above_limit, 1, false);
        assert_eq!(
            value(
                &latency_metrics("typing", &above_limit, 1),
                "key_timestamps_valid"
            ),
            Some(0.0)
        );
        let events = vec![events[0].clone(); MAX_KEY_RECORDS + 1];
        assert!(key_latencies(&events, &[], UNIT, 250 * MS).record_limit_exceeded);
    }

    #[test]
    fn no_input_and_blink_controls_never_become_screen_latency() {
        let events = key_run(4);
        let mut blink = responses(&events, 30 * MS);
        for frame in &mut blink {
            frame.hash = frame.seq % 2;
        }
        let latencies = key_latencies(&events, &blink, UNIT, 250 * MS);
        assert_unqualified(&latencies, 4, true);
        assert_unqualified(&key_latencies(&[], &blink, UNIT, 250 * MS), 0, false);
        let unchanged = [frame(1, 1, 0), frame(2, 5_000 * MS, 0)];
        let latencies = key_latencies(&events, &unchanged, UNIT, 250 * MS);
        assert_eq!(latencies.missed, 4);
        assert_unqualified(&latencies, 4, false);
    }

    #[test]
    fn a_missing_response_prevents_fast_later_trials_from_restoring_summary_percentiles() {
        let events = key_run(100);
        let frames = responses(&events, 30 * MS);
        let complete = key_latencies(&events, &frames, UNIT, 250 * MS);
        let incomplete = key_latencies(&events, &frames[..100], UNIT, 250 * MS);
        let mut trials = vec![latency_metrics("typing", &incomplete, 100)];
        trials.extend((0..10).map(|_| latency_metrics("typing", &complete, 100)));
        let rows = summarize_trials(&trials);
        assert!(
            !rows
                .iter()
                .any(|row| row.name.starts_with("key_hash_change_"))
        );
        assert!(
            rows.iter().any(|row| row.name == "keys_missed"
                && row.summary.n == 11
                && row.summary.max > 0.0)
        );
        for (prefix, qualification) in [
            ("key_hash_change_", "key_hash_timing_complete"),
            ("key_to_screen_", "key_semantic_qualified"),
        ] {
            let complete = vec![
                Metric::new("caret", format!("{prefix}p95"), 30.0, "ms"),
                Metric::new("caret", qualification, 1.0, "bool"),
            ];
            let mut trials = vec![complete.clone(), complete];
            assert!(
                summarize_trials(&trials)
                    .iter()
                    .any(|row| row.name.starts_with(prefix))
            );
            trials[0].pop();
            assert!(
                !summarize_trials(&trials)
                    .iter()
                    .any(|row| row.name.starts_with(prefix))
            );
            trials[0].clear();
            assert!(
                !summarize_trials(&trials)
                    .iter()
                    .any(|row| row.name.starts_with(prefix))
            );
        }
    }

    #[test]
    fn semantic_summary_can_report_a_verified_response_after_the_next_input() {
        let metrics = vec![
            Metric::new("typing", "key_to_screen_p95", 400.0, "ms"),
            Metric::new("typing", "key_semantic_qualified", 1.0, "bool"),
            Metric::new("typing", "key_hash_timing_complete", 0.0, "bool"),
        ];
        let rows = summarize_trials(&vec![metrics; 10]);
        assert!(rows.iter().any(|row| row.name == "key_to_screen_p95"
            && row.summary.n == 10
            && row.summary.mean.total_cmp(&400.0).is_eq()));
    }

    #[test]
    fn keys_are_timed_by_the_first_change_inside_their_own_window() {
        let frames = vec![
            frame(1, 1, 1),
            frame(2, 10 * MS, 1),
            frame(3, 128 * MS, 2),
            frame(4, 360 * MS, 2),
            frame(5, 392 * MS, 3),
        ];
        let events = vec![key(1, 100 * MS), key(2, 350 * MS)];
        let latencies = key_latencies(&events, &frames, UNIT, 250 * MS);
        assert_eq!(
            latencies,
            KeyLatencies {
                measured_ms: vec![28.0, 42.0],
                late: 0,
                missed: 0,
                ambiguous: 0,
                invalid_timestamps: false,
                record_limit_exceeded: false,
            }
        );
    }

    #[test]
    fn a_response_after_the_next_key_is_late_and_makes_that_key_ambiguous() {
        // Key 1's change shows at 400, after key 2 at 350: key 1 is late,
        // and key 2's first change (400) may be key 1's, so it is ambiguous.
        let frames = vec![frame(1, 1, 1), frame(2, 400 * MS, 2), frame(3, 420 * MS, 3)];
        let events = vec![key(1, 100 * MS), key(2, 350 * MS), key(3, 600 * MS)];
        let latencies = key_latencies(&events, &frames, UNIT, 250 * MS);
        assert!(latencies.measured_ms.is_empty());
        assert_eq!(latencies.late, 1);
        assert_eq!(latencies.ambiguous, 1);
        assert_eq!(latencies.missed, 1);
    }

    #[test]
    fn timing_never_resumes_after_an_unresolved_interval() {
        let frames = vec![
            frame(1, 1, 1),
            frame(2, 400 * MS, 2),
            frame(3, 630 * MS, 3),
            frame(4, 880 * MS, 4),
        ];
        let events = vec![
            key(1, 100 * MS),
            key(2, 350 * MS),
            key(3, 600 * MS),
            key(4, 850 * MS),
        ];
        let latencies = key_latencies(&events, &frames, UNIT, 250 * MS);
        assert!(latencies.measured_ms.is_empty());
        assert_eq!(latencies.late, 1);
        assert_eq!(latencies.ambiguous, 3);
        assert_eq!(latencies.missed, 0);
    }

    #[test]
    fn the_last_key_gets_one_spacing_to_respond() {
        let frames = vec![frame(1, 1, 1), frame(2, 400 * MS, 2)];
        let events = vec![key(1, 100 * MS)];
        assert_eq!(key_latencies(&events, &frames, UNIT, 250 * MS).late, 1);
        let wider = key_latencies(&events, &frames, UNIT, 400 * MS);
        assert_eq!(wider.measured_ms, vec![300.0]);
    }

    #[test]
    fn the_capture_band_skips_chrome_and_scroll_bars() {
        assert_eq!(capture_region(960.0, 572.0), (0.0, 172.0, 928.0, 360.0));
        let (_, y, width, height) = capture_region(10.0, 10.0);
        assert!(width >= 1.0 && height >= 1.0 && y >= 0.0);
    }

    #[test]
    fn frame_intervals_only_count_changes_inside_the_phase() {
        let ms = 1_000_000;
        let frames = vec![
            frame(1, 0, 1),
            frame(2, 8 * ms, 2),
            frame(3, 16 * ms, 3),
            frame(4, 24 * ms, 3),
            frame(5, 41 * ms, 4),
            frame(6, 900 * ms, 5),
        ];
        let intervals = frame_intervals(&frames, 5 * ms, 100 * ms, UNIT);
        assert_eq!(intervals, vec![8.0, 25.0]);
        let metrics = interval_metrics("scroll", &intervals, 120);
        assert_eq!(value(&metrics, "frame_interval_max"), Some(25.0));
        assert_eq!(value(&metrics, "frame_late_pct"), Some(50.0));
        assert_eq!(value(&metrics, "frame_intervals"), Some(2.0));
    }

    /// `count` wheel events `step` ticks apart, the first at tick `first`.
    fn scroll_events(count: u64, first: u64, step: u64) -> Vec<InputEvent> {
        (0..count)
            .map(|index| InputEvent {
                seq: index + 1,
                mach: first + index * step,
                kind: "scroll".to_owned(),
                label: "-66".to_owned(),
            })
            .collect()
    }

    /// Frames every `step` ticks from `from` to `until`, each hashed by time.
    fn frames_between(from: u64, until: u64, step: u64, hash: impl Fn(u64) -> u64) -> Vec<Frame> {
        (0..=(until - from) / step)
            .map(|index| from + index * step)
            .map(|display| frame(display / step, display, hash(display)))
            .collect()
    }

    /// The 2026-10-03 workload: 3,334 wheel events 8 ms apart from t = 1 s.
    fn the_scroll_workload() -> Vec<InputEvent> {
        scroll_events(3_334, 1_000 * MS, 8 * MS)
    }

    /// The shape every 2026-10-03 scroll trial had: capture starts before the
    /// first event, one state for a moment, a second for a second, then the
    /// first again. Nothing moved.
    fn frames_of_a_scroll_that_moved_nothing() -> Vec<Frame> {
        frames_between(860 * MS, 30_000 * MS, 8 * MS, |time| {
            u64::from((870 * MS..1_870 * MS).contains(&time))
        })
    }

    #[test]
    fn scroll_metrics_report_intervals_and_distinct_states() {
        let frames = vec![
            frame(1, 0, 1),
            frame(2, 8 * MS, 2),
            frame(3, 16 * MS, 3),
            frame(4, 24 * MS, 3),
            frame(5, 41 * MS, 4),
            frame(6, 900 * MS, 5),
        ];
        let metrics = scroll_metrics("scroll", &frames, (5 * MS, 100 * MS), (UNIT, 120));
        assert_eq!(value(&metrics, "frame_intervals"), Some(2.0));
        assert_eq!(value(&metrics, "frame_distinct"), Some(3.0));
    }

    #[test]
    fn the_input_span_runs_from_the_first_event_to_100_ms_after_the_last() {
        let events = scroll_events(3, 5 * MS, 8 * MS);
        assert_eq!(
            input_span(&events, UNIT),
            Some((5 * MS, 21 * MS + 100 * MS))
        );
        // The dev Mac's 24 MHz timebase: 100 ms is 2,400,000 ticks.
        let mac = Timebase {
            numer: 125,
            denom: 3,
        };
        assert_eq!(
            input_span(&events, mac),
            Some((5 * MS, 21 * MS + 2_400_000))
        );
        assert_eq!(input_span(&[], UNIT), None);
    }

    #[test]
    fn a_scroll_that_never_landed_is_refused_with_its_counts() {
        let events = the_scroll_workload();
        let frames = frames_of_a_scroll_that_moved_nothing();
        let reason = scroll_refusal(&events, &frames, UNIT).unwrap_or_default();
        assert!(reason.contains("3334 scroll events"), "{reason}");
        assert!(reason.contains("2 distinct states"), "{reason}");
        assert!(reason.contains("fewer than the 66"), "{reason}");
    }

    #[test]
    fn idle_toggling_is_not_scrolling() {
        // Zed's late alternation: ten states over the whole capture.
        let frames = frames_between(860 * MS, 30_000 * MS, 8 * MS, |time| (time / (8 * MS)) % 10);
        let reason = scroll_refusal(&the_scroll_workload(), &frames, UNIT).unwrap_or_default();
        assert!(reason.contains("10 distinct states"), "{reason}");
    }

    #[test]
    fn a_landed_scroll_is_not_refused() {
        let frames = frames_between(860 * MS, 30_000 * MS, 8 * MS, |time| time / (8 * MS));
        assert_eq!(scroll_refusal(&the_scroll_workload(), &frames, UNIT), None);
    }

    #[test]
    fn only_frames_inside_the_scroll_span_count() {
        let events = the_scroll_workload();
        let (first, last) = input_span(&events, UNIT).unwrap_or_default();
        // Two states during the scroll, and many before and after it.
        let during = frames_between(first, last, 8 * MS, |time| (time / (8 * MS)) % 2);
        let before = frames_between(0, first - MS, 5 * MS, |time| 10_000 + time / MS);
        let after = frames_between(last + MS, last + 400 * MS, 5 * MS, |time| {
            20_000 + time / MS
        });
        let frames = [before, during, after].concat();
        assert_eq!(distinct_frames(&frames, 0, u64::MAX), 2 + 200 + 80);
        let reason = scroll_refusal(&events, &frames, UNIT).unwrap_or_default();
        assert!(reason.contains("2 distinct states"), "{reason}");
        // Both ends of the span are inclusive.
        let edges = [
            frame(1, first - 1, 1),
            frame(2, first, 2),
            frame(3, last, 3),
            frame(4, last + 1, 4),
        ];
        assert_eq!(distinct_frames(&edges, first, last), 2);
    }

    #[test]
    fn the_refusal_and_the_metric_count_the_same_states() {
        let events = the_scroll_workload();
        let (first, last) = input_span(&events, UNIT).unwrap_or_default();
        let during = frames_between(first, last, 8 * MS, |time| (time / (8 * MS)) % 3);
        let after = frames_between(last + MS, last + 400 * MS, 5 * MS, |time| {
            20_000 + time / MS
        });
        let frames = [during, after].concat();
        let metrics = scroll_metrics("scroll", &frames, (first, last), (UNIT, 120));
        let distinct = value(&metrics, "frame_distinct");
        assert_eq!(distinct, Some(3.0));
        let reason = scroll_refusal(&events, &frames, UNIT).unwrap_or_default();
        let row = format!("{} distinct states", distinct.unwrap_or_default());
        assert!(reason.contains(&row), "{reason} vs the row {row}");
    }

    #[test]
    fn a_scroll_that_posted_nothing_is_refused() {
        let frames = frames_between(0, 30_000 * MS, 8 * MS, |time| time / (8 * MS));
        let reason = scroll_refusal(&[], &frames, UNIT).unwrap_or_default();
        assert!(reason.contains("posted no scroll events"), "{reason}");
    }

    #[test]
    fn the_scroll_minimum_scales_with_events_and_has_a_floor() {
        assert_eq!(min_scroll_states(3_334), 66);
        assert_eq!(min_scroll_states(1_000), 20);
        assert_eq!(min_scroll_states(100), 20);
        // The floor never asks for more states than there were events.
        assert_eq!(min_scroll_states(20), 20);
        assert_eq!(min_scroll_states(19), 19);
        assert_eq!(min_scroll_states(5), 5);
        assert_eq!(min_scroll_states(0), 0);
        let workload = the_scroll_workload();
        let states = |count: u64| {
            frames_between(1_000 * MS, 30_000 * MS, 8 * MS, |t| (t / (8 * MS)) % count)
        };
        assert_eq!(scroll_refusal(&workload, &states(66), UNIT), None);
        assert!(scroll_refusal(&workload, &states(65), UNIT).is_some());
        // Five events can show five states: five pass and four do not.
        let few = scroll_events(5, 10 * MS, MS);
        let tiny = |count: u64| frames_between(10 * MS, 20 * MS, MS, |t| (t / MS) % count);
        assert_eq!(scroll_refusal(&few, &tiny(5), UNIT), None);
        assert!(scroll_refusal(&few, &tiny(4), UNIT).is_some());
    }

    /// `count` key presses 250 ms apart, the first at t = 1 s.
    fn key_run(count: u64) -> Vec<InputEvent> {
        (0..count)
            .map(|index| key(index + 1, 1_000 * MS + index * 250 * MS))
            .collect()
    }

    /// How many of the typing workload's keys an app has answered by `time`
    /// when it answers each `delay` ticks after the press.
    fn keys_answered(time: u64, delay: u64) -> u64 {
        time.checked_sub(1_000 * MS + delay)
            .map_or(0, |since| (since / (250 * MS) + 1).min(100))
    }

    #[test]
    fn the_key_minimum_is_half_the_keys_with_a_floor() {
        assert_eq!(min_key_states(100), 50);
        assert_eq!(min_key_states(101), 50);
        assert_eq!(min_key_states(1_000), 500);
        assert_eq!(min_key_states(22), 11);
        assert_eq!(min_key_states(21), 10);
        assert_eq!(min_key_states(20), 10);
        // The floor never asks for more states than there were keys.
        assert_eq!(min_key_states(10), 10);
        assert_eq!(min_key_states(9), 9);
        assert_eq!(min_key_states(1), 1);
        assert_eq!(min_key_states(0), 0);
        let keys = key_run(100);
        let states = |count: u64| {
            frames_between(1_000 * MS, 30_000 * MS, 50 * MS, |t| {
                (t / (50 * MS)) % count
            })
        };
        assert_eq!(key_band_warning(&keys, &states(50), UNIT), None);
        let reason = key_band_warning(&keys, &states(49), UNIT).unwrap_or_default();
        assert_eq!(
            reason,
            "100 keys left the band in 49 distinct states, fewer than the 50 required"
        );
        // Ten keys can show ten states: ten pass and nine do not.
        let few = key_run(10);
        let tiny = |count: u64| {
            frames_between(1_000 * MS, 3_300 * MS, 25 * MS, |t| (t / (25 * MS)) % count)
        };
        assert_eq!(key_band_warning(&few, &tiny(10), UNIT), None);
        assert!(key_band_warning(&few, &tiny(9), UNIT).is_some());
    }

    #[test]
    fn keys_that_left_the_band_unchanged_are_refused_with_their_counts() {
        // The band never moved, so its one state was the whole span.
        let frames = frames_between(860 * MS, 30_000 * MS, 8 * MS, |_| 7);
        let reason = key_band_warning(&key_run(100), &frames, UNIT).unwrap_or_default();
        assert_eq!(
            reason,
            "100 keys left the band in 1 distinct state, fewer than the 50 required"
        );
    }

    #[test]
    fn a_late_change_after_the_span_is_not_the_keys() {
        // One change 1.03 s after the last key: two states in the capture, one
        // in the span.
        let last_key = 1_000 * MS + 99 * 250 * MS;
        let frames = frames_between(860 * MS, 30_000 * MS, 8 * MS, |time| {
            u64::from(time > last_key + 1_030 * MS)
        });
        assert_eq!(distinct_frames(&frames, 0, u64::MAX), 2);
        let reason = key_band_warning(&key_run(100), &frames, UNIT).unwrap_or_default();
        assert!(reason.contains("in 1 distinct state,"), "{reason}");
    }

    #[test]
    fn a_blinking_caret_alone_is_not_typing() {
        // Two states switching twice a second: dozens of changes, but typing
        // adds a new state per key, so only states count.
        let frames = frames_between(860 * MS, 30_000 * MS, 8 * MS, |t| (t / (500 * MS)) % 2);
        let keys = key_run(100);
        let (first, last) = input_span(&keys, UNIT).unwrap_or_default();
        assert!(frame_intervals(&frames, first, last, UNIT).len() >= 40);
        let reason = key_band_warning(&keys, &frames, UNIT).unwrap_or_default();
        assert!(reason.contains("in 2 distinct states,"), "{reason}");
    }

    #[test]
    fn a_band_that_changed_with_every_key_is_not_refused() {
        let frames = frames_between(860 * MS, 30_000 * MS, 8 * MS, |t| keys_answered(t, 30 * MS));
        assert_eq!(key_band_warning(&key_run(100), &frames, UNIT), None);
    }

    #[test]
    fn a_slow_app_is_not_refused_until_most_keys_trail_the_span() {
        let keys = key_run(100);
        let (first, last) = input_span(&keys, UNIT).unwrap_or_default();
        let answered_after =
            |delay: u64| frames_between(860 * MS, 40_000 * MS, 8 * MS, |t| keys_answered(t, delay));
        let verdict = |delay: u64| key_band_warning(&keys, &answered_after(delay), UNIT);
        // Answers 400 ms late all land in the span but the last two keys'.
        assert_eq!(
            distinct_frames(&answered_after(400 * MS), first, last),
            1 + 98
        );
        assert_eq!(verdict(400 * MS), None);
        // Twelve seconds late still shows 52 of the 100 keys; thirteen, 48.
        assert_eq!(verdict(12_000 * MS), None);
        let reason = verdict(13_000 * MS).unwrap_or_default();
        assert!(reason.contains("in 49 distinct states,"), "{reason}");
    }

    #[test]
    fn only_frames_inside_the_key_span_count() {
        let keys = key_run(100);
        let (first, last) = input_span(&keys, UNIT).unwrap_or_default();
        // Three states while the keys went in, and many before and after.
        let during = frames_between(first, last, 8 * MS, |time| (time / (400 * MS)) % 3);
        let before = frames_between(0, first - MS, 5 * MS, |time| 10_000 + time / MS);
        let after = frames_between(last + MS, last + 400 * MS, 5 * MS, |time| {
            20_000 + time / MS
        });
        let frames = [before, during, after].concat();
        let reason = key_band_warning(&keys, &frames, UNIT).unwrap_or_default();
        assert!(reason.contains("in 3 distinct states,"), "{reason}");
        // The refusal names the count `distinct_frames` gives over the span.
        assert_eq!(distinct_frames(&frames, first, last), 3);
    }

    #[test]
    fn keys_that_posted_nothing_are_refused() {
        let frames = frames_between(860 * MS, 30_000 * MS, 8 * MS, |t| keys_answered(t, 30 * MS));
        let reason = key_band_warning(&[], &frames, UNIT).unwrap_or_default();
        assert_eq!(reason, "the helper posted no keys; the keys did not land");
    }

    /// An excerpt of a run's `events.tsv` or `frames.tsv` as the helper printed
    /// it: no header, no run, trial or phase columns, `tag` first on each line.
    fn helper_lines(excerpt: &str, tag: &str) -> Vec<String> {
        excerpt
            .lines()
            .skip(1)
            .filter_map(|row| row.splitn(4, '\t').nth(3))
            .map(|rest| format!("{tag}\t{rest}"))
            .collect()
    }

    /// A run's keys and frames from its excerpts, through the helper parsers.
    fn replay(keys: &str, frames: &str) -> Result<(Vec<InputEvent>, Vec<Frame>), String> {
        let keys = parse_input_events(&helper_lines(keys, "event").join("\n"))?;
        let frames = helper_lines(frames, "frame")
            .iter()
            .filter_map(|line| parse_frame(line).transpose())
            .collect::<Result<_, _>>()?;
        Ok((keys, frames))
    }

    /// The dev Mac's timebase, which the real runs were captured on.
    const MAC: Timebase = Timebase {
        numer: 125,
        denom: 3,
    };

    // Each excerpt keeps all 100 keys and the frames that begin a new state or
    // the span; every dropped frame repeats the hash of one kept before it.
    #[test]
    fn real_typing_runs_replay_two_pass_and_zed_is_refused() -> Result<(), String> {
        let appkit = replay(
            include_str!("../tests/fixtures/typing-appkit-events.tsv"),
            include_str!("../tests/fixtures/typing-appkit-frames.tsv"),
        )?;
        let alpine = replay(
            include_str!("../tests/fixtures/typing-alpine-events.tsv"),
            include_str!("../tests/fixtures/typing-alpine-frames.tsv"),
        )?;
        let zed = replay(
            include_str!("../tests/fixtures/typing-zed-events.tsv"),
            include_str!("../tests/fixtures/typing-zed-frames.tsv"),
        )?;
        let zed_reason = "100 keys left the band in 1 distinct state, fewer than the 50 required";
        // Span states, keys (timed, late) and verdict, as each run recorded.
        let runs = [
            ("appkit", appkit, 116, (100, 0), None),
            ("alpine", alpine, 97, (96, 4), None),
            ("zed", zed, 1, (0, 100), Some(zed_reason)),
        ];
        for (name, (keys, frames), states, (timed, late), refusal) in runs {
            let (first, last) = input_span(&keys, MAC).unwrap_or_default();
            assert_eq!(keys.len(), 100, "{name}");
            assert_eq!(distinct_frames(&frames, first, last), states, "{name}");
            let latencies = key_latencies(&keys, &frames, MAC, MAC.ticks(250 * MS));
            assert_eq!(
                (latencies.measured_ms.len(), latencies.late),
                (timed, late),
                "{name}"
            );
            assert_eq!(
                key_band_warning(&keys, &frames, MAC).as_deref(),
                refusal,
                "{name}"
            );
        }
        Ok(())
    }

    fn opening_frame() -> Appeared {
        // The reference editor's first sighting on 2026-10-03.
        Appeared {
            mach: 7,
            id: 5,
            x: 284.0,
            y: 133.0,
            width: 944.0,
            height: 562.0,
        }
    }

    #[test]
    fn the_settled_frame_replaces_the_opening_frame_but_not_the_sighting() {
        let settled_window = window(0, 9, 0, (276.0, 128.0, 960.0, 572.0));
        let (settled, reason) = settle_frame(opening_frame(), &settled_window, (960, 572));
        assert_eq!((settled.mach, settled.id), (7, 5));
        assert_eq!(
            (settled.x, settled.y, settled.width, settled.height),
            (276.0, 128.0, 960.0, 572.0)
        );
        assert_eq!(reason, None);
    }

    #[test]
    fn a_settled_frame_off_the_request_names_the_size_seen() {
        let zed = window(0, 9, 0, (13.0, 41.0, 1486.0, 933.0));
        let (settled, reason) = settle_frame(opening_frame(), &zed, (960, 572));
        assert_eq!((settled.width, settled.height), (1486.0, 933.0));
        let reason = reason.unwrap_or_default();
        assert!(reason.contains("1486x933"), "{reason}");
        assert!(reason.contains("requested 960x572"), "{reason}");
        // The opening-animation frame would have failed the same gate.
        let opening = window(0, 9, 0, (284.0, 133.0, 944.0, 562.0));
        assert!(
            settle_frame(opening_frame(), &opening, (960, 572))
                .1
                .is_some()
        );
    }

    #[test]
    fn the_frame_gate_allows_a_point_of_rounding_and_no_more() {
        let off_by = |dx: f64, dy: f64| {
            let window = window(0, 9, 0, (0.0, 0.0, 960.0 + dx, 572.0 + dy));
            settle_frame(opening_frame(), &window, (960, 572)).1
        };
        assert_eq!(off_by(1.0, -1.0), None);
        assert!(off_by(2.0, 0.0).is_some());
        assert!(off_by(0.0, -2.0).is_some());
    }

    fn window(order: usize, pid: i32, layer: i64, rect: (f64, f64, f64, f64)) -> WindowInfo {
        WindowInfo {
            order,
            id: u32::try_from(order).unwrap_or(0) + 100,
            pid,
            owner: format!("app{pid}"),
            layer,
            alpha: 1.0,
            x: rect.0,
            y: rect.1,
            width: rect.2,
            height: rect.3,
        }
    }

    fn list(front: i32, windows: Vec<WindowInfo>) -> WindowList {
        WindowList {
            front_pid: front,
            front_name: format!("app{front}"),
            windows,
        }
    }

    #[test]
    fn visibility_requires_frontmost_and_no_overlap_from_above() {
        let menu_bar = window(0, 600, 24, (0.0, 0.0, 1512.0, 33.0));
        let target = window(2, 7, 0, (276.0, 221.0, 960.0, 568.0));
        let behind = window(3, 8, 0, (0.0, 33.0, 1512.0, 949.0));
        let own_popup = window(1, 7, 3, (300.0, 300.0, 200.0, 100.0));
        let visible = list(
            7,
            vec![menu_bar.clone(), own_popup, target.clone(), behind.clone()],
        );
        assert_eq!(
            visibility(&visible, 7, 100.0),
            Visibility::Visible(target.clone())
        );
        let not_front = list(8, vec![target.clone(), behind.clone()]);
        assert!(matches!(
            visibility(&not_front, 7, 100.0),
            Visibility::NotFrontmost { pid: 8, .. }
        ));
        let cover = window(1, 9, 0, (500.0, 500.0, 400.0, 400.0));
        let covered = list(7, vec![menu_bar, cover, target, behind]);
        let Visibility::Occluded { by, .. } = visibility(&covered, 7, 100.0) else {
            unreachable!("expected occlusion");
        };
        assert_eq!(by.len(), 1);
        assert_eq!(by[0].pid, 9);
        assert_eq!(visibility(&list(7, vec![]), 7, 100.0), Visibility::NoWindow);
        let tiny = list(7, vec![window(0, 7, 0, (0.0, 0.0, 50.0, 50.0))]);
        assert_eq!(visibility(&tiny, 7, 100.0), Visibility::NoWindow);
    }

    #[test]
    fn the_pointer_over_the_window_does_not_count_as_cover() {
        let target = window(2, 7, 0, (276.0, 221.0, 960.0, 568.0));
        let mut cursor = window(
            0,
            600,
            super::CURSOR_WINDOW_LEVEL,
            (756.0, 466.0, 28.0, 40.0),
        );
        cursor.owner = "Window Server".to_owned();
        let pointed = list(7, vec![cursor.clone(), target.clone()]);
        assert!(matches!(
            visibility(&pointed, 7, 100.0),
            Visibility::Visible(_)
        ));
        cursor.owner = "app600".to_owned();
        let covered = list(7, vec![cursor, target]);
        assert!(matches!(
            visibility(&covered, 7, 100.0),
            Visibility::Occluded { .. }
        ));
    }

    #[test]
    fn notification_banners_and_other_overlays_count_as_cover() {
        let target = window(2, 7, 0, (276.0, 221.0, 960.0, 568.0));
        let mut banner = window(0, 400, 23, (1100.0, 40.0, 380.0, 300.0));
        banner.owner = "NotificationCenter".to_owned();
        let covered = list(7, vec![banner.clone(), target.clone()]);
        let Visibility::Occluded { by, .. } = visibility(&covered, 7, 100.0) else {
            unreachable!("a banner over the window is cover");
        };
        assert_eq!(by[0].layer, 23);
        let mut clear = banner.clone();
        clear.alpha = 0.0;
        let transparent = list(7, vec![clear, target.clone()]);
        assert!(matches!(
            visibility(&transparent, 7, 100.0),
            Visibility::Visible(_)
        ));
        let mut aside = banner;
        aside.x = 1240.0;
        let apart = list(7, vec![aside, target]);
        assert!(matches!(
            visibility(&apart, 7, 100.0),
            Visibility::Visible(_)
        ));
    }

    #[test]
    fn quiet_detection_needs_a_full_quiet_window() {
        let ms = 1_000_000;
        let points: Vec<(u64, u64)> = vec![
            (0, 0),
            (SECOND, 900 * ms),
            (2 * SECOND, 1_800 * ms),
            (3 * SECOND, 1_810 * ms),
            (4 * SECOND, 1_820 * ms),
            (5 * SECOND, 1_830 * ms),
        ];
        assert_eq!(quiet_since(&points, 5.0, 2 * SECOND), Some(4 * SECOND));
        assert_eq!(quiet_since(&points, 0.5, 2 * SECOND), None);
        assert_eq!(quiet_since(&points[..2], 5.0, 2 * SECOND), None);
    }

    #[test]
    fn cpu_points_start_at_the_requested_set() {
        let sets = vec![
            set(1, SetKind::Tick, 10, vec![row(1, 0, "a", 1, 5)]),
            set(2, SetKind::Mark, 30, vec![row(1, 0, "a", 1, 7)]),
            set(3, SetKind::Tick, 50, vec![row(1, 0, "a", 1, 9)]),
        ];
        assert_eq!(cpu_points(&sets, 2, UNIT), vec![(0, 7), (20, 9)]);
        assert!(cpu_points(&sets, 9, UNIT).is_empty());
    }

    #[test]
    fn summaries_group_by_phase_and_metric_in_order() {
        let metrics = vec![
            Metric::new("idle", "cpu_time", 10.0, "ms"),
            Metric::new("idle", "footprint_app_max", 5.0, "bytes"),
            Metric::new("idle", "cpu_time", 20.0, "ms"),
            Metric::new("startup", "cpu_time", 99.0, "ms"),
        ];
        let rows = summarize(&metrics);
        assert_eq!(rows.len(), 3);
        assert_eq!(
            (rows[0].phase.as_str(), rows[0].name.as_str()),
            ("idle", "cpu_time")
        );
        assert_eq!(rows[0].summary.n, 2);
        assert!((rows[0].summary.mean - 15.0).abs() < 1e-9);
        assert_eq!(rows[2].phase, "startup");
    }

    #[test]
    fn transient_children_count_as_zero_in_trials_without_them() {
        let child = "footprint_child_max:rustc";
        let trials = vec![
            vec![
                Metric::new("indexing", "cpu_time", 10.0, "ms"),
                Metric::new("indexing", child, 300.0, "bytes"),
            ],
            vec![Metric::new("indexing", "cpu_time", 12.0, "ms")],
            vec![Metric::new("startup", "cpu_time", 50.0, "ms")],
        ];
        let rows = summarize_trials(&trials);
        let rustc = rows
            .iter()
            .find(|row| row.name == child)
            .map(|row| (row.summary.n, row.summary.mean));
        // Two trials ran `indexing`; the third never reached it.
        assert_eq!(rustc, Some((2, 150.0)));
        let cpu = rows
            .iter()
            .find(|row| row.phase == "indexing" && row.name == "cpu_time")
            .map(|row| row.summary.n);
        assert_eq!(cpu, Some(2));
    }
}
