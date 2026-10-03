//! Pure measurement logic: phase windows from sampler marks, per-phase
//! footprint, CPU and wakeups, keypress-to-screen matching, frame intervals,
//! window visibility and the quiet detector. Nothing here touches the system.

use crate::helpers::{
    Appeared, Frame, InputEvent, Mark, SampleSet, SetKind, Timebase, WindowInfo, WindowList,
};
use crate::stats::{self, Summary, to_f64};
use std::collections::BTreeMap;

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
}

/// A key is timed only if the first changed frame after it shows before the
/// next key (last key: within `spacing_ticks`) and the previous key's did too;
/// else it is late (the change may be a neighbor's) or missed (no change).
pub fn key_latencies(
    events: &[InputEvent],
    frames: &[Frame],
    timebase: Timebase,
    spacing_ticks: u64,
) -> KeyLatencies {
    let changes = changed_frames(frames);
    let mut sorted: Vec<&InputEvent> = events.iter().collect();
    sorted.sort_by_key(|event| event.mach);
    let mut result = KeyLatencies::default();
    let mut previous_in_window = true;
    for (index, event) in sorted.iter().enumerate() {
        let window_end = sorted
            .get(index + 1)
            .map_or(event.mach.saturating_add(spacing_ticks), |next| next.mach);
        let response = changes
            .iter()
            .find(|frame| frame.display_mach > event.mach)
            .map(|frame| frame.display_mach);
        let in_window = response.is_some_and(|display| display < window_end);
        match response {
            None => result.missed += 1,
            Some(display) if in_window && previous_in_window => result
                .measured_ms
                .push(timebase.millis_between(event.mach, display)),
            Some(_) => result.late += 1,
        }
        previous_in_window = in_window;
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

pub fn latency_metrics(phase: &str, latencies: &KeyLatencies, sent: usize) -> Vec<Metric> {
    let mut metrics = vec![
        Metric::new(phase, "keys_sent", to_f64(sent as u64), "count"),
        Metric::new(
            phase,
            "keys_timed",
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
    ];
    for (suffix, quantile) in [("p50", 0.5), ("p95", 0.95), ("max", 1.0)] {
        if let Some(value) = stats::nearest_rank(&latencies.measured_ms, quantile) {
            metrics.push(Metric::new(
                phase,
                format!("key_to_screen_{suffix}"),
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

#[cfg(test)]
mod tests {
    use super::{
        KeyLatencies, Metric, PhaseWindow, Visibility, capture_region, cpu_points, frame_intervals,
        interval_metrics, key_latencies, phase_label, phase_metrics, phase_windows, quiet_since,
        summarize, trial_metrics, visibility,
    };
    use crate::helpers::{
        Appeared, Frame, InputEvent, Mark, ProcRow, SampleSet, SetKind, Timebase, WindowInfo,
        WindowList,
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
    fn keys_are_timed_by_the_first_change_inside_their_own_window() {
        let frames = vec![
            frame(1, 0, 1),
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
            }
        );
    }

    #[test]
    fn a_response_after_the_next_key_is_late_and_so_is_that_key() {
        // Key 1's change shows at 400, after key 2 at 350: key 1 is late,
        // and key 2's first change (400) may be key 1's, so it is late too.
        let frames = vec![frame(1, 0, 1), frame(2, 400 * MS, 2), frame(3, 420 * MS, 3)];
        let events = vec![key(1, 100 * MS), key(2, 350 * MS), key(3, 600 * MS)];
        let latencies = key_latencies(&events, &frames, UNIT, 250 * MS);
        assert!(latencies.measured_ms.is_empty());
        assert_eq!(latencies.late, 2);
        assert_eq!(latencies.missed, 1);
    }

    #[test]
    fn timing_resumes_once_a_key_is_answered_in_its_window() {
        let frames = vec![
            frame(1, 0, 1),
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
        assert_eq!(latencies.measured_ms, vec![30.0, 30.0]);
        assert_eq!(latencies.late, 2);
        assert_eq!(latencies.missed, 0);
    }

    #[test]
    fn the_last_key_gets_one_spacing_to_respond() {
        let frames = vec![frame(1, 0, 1), frame(2, 400 * MS, 2)];
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
}
