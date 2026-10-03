//! Always-on recorder: rings of frame stages and process samples, reserved
//! once at start and filled only by events and frames the app already
//! handles. It owns no timer or thread, so an idle app records nothing.

use std::{
    cell::RefCell,
    io::{self, Write},
    time::{Duration, Instant},
};

use alpine_platform::PresentationOutcome;

use crate::{EditorSignpost, EditorSignpostStage};

const FRAME_CAPACITY: usize = 4_096;
const SAMPLE_CAPACITY: usize = 3_600;
/// Language-server children one process sample reads; more are counted.
pub const MAX_SAMPLED_CHILDREN: usize = 6;
const MAX_SERVERS: usize = 16;
const SERVER_NAME_BYTES: usize = 64;
const NO_SERVER: u8 = u8::MAX;
const SAMPLE_INTERVAL: Duration = Duration::from_secs(1);
const ABSENT: u32 = u32::MAX;

// Columns after `event_ns` are ns after event receipt, or after submit start
// for a frame no event produced (empty `event`). gpu_observed is when a poll
// saw the GPU finish, an upper bound on completion.
const FRAME_HEADER: &str = concat!(
    "event\trevision\toutcome\tevent_ns\tdispatch_ns\tmutation_ns\tbuild_begin_ns",
    "\tlayout_begin_ns\tlayout_end_ns\tatlas_begin_ns\tatlas_end_ns\tbuild_end_ns",
    "\thandler_end_ns\tsubmit_begin_ns\tsubmit_end_ns\tgpu_observed_ns\ttarget_ns",
    "\ttarget_present_ns\tpresented_ns\trecorded_ns\n",
);
// A child row's server is empty when its name is unknown. The editor row's
// dropped_children counts children past MAX_SAMPLED_CHILDREN, left unread.
const SAMPLE_HEADER: &str = concat!(
    "time_ns\trole\tpid\tserver\tphys_footprint\tcpu_ns",
    "\tinterrupt_wakeups\tidle_wakeups\tsubmissions\tdropped_children\n",
);
const STAGE_COUNT: usize = 16;
pub(crate) const EDITOR_STAGES: usize = 8;
pub(crate) const HANDLER_END: usize = 8;
pub(crate) const SUBMIT_BEGIN: usize = 9;
pub(crate) const SUBMIT_END: usize = 10;
pub(crate) const GPU_OBSERVED: usize = 11;
pub(crate) const TARGET: usize = 12;
pub(crate) const TARGET_PRESENT: usize = 13;
pub(crate) const PRESENTED: usize = 14;
pub(crate) const RECORDED: usize = 15;

/// Editor stage offsets captured while one event dispatched.
pub(crate) type EditorStages = [u32; EDITOR_STAGES];
pub(crate) const NO_STAGES: EditorStages = [ABSENT; EDITOR_STAGES];

const fn editor_column(stage: EditorSignpostStage) -> Option<usize> {
    match stage {
        EditorSignpostStage::EventDispatchBegin => Some(0),
        EditorSignpostStage::StateMutationComplete => Some(1),
        EditorSignpostStage::FrameBuildBegin => Some(2),
        EditorSignpostStage::VisibleLayoutBegin => Some(3),
        EditorSignpostStage::VisibleLayoutComplete => Some(4),
        EditorSignpostStage::AtlasPublicationBegin => Some(5),
        EditorSignpostStage::AtlasPublicationComplete
        | EditorSignpostStage::AtlasPublicationFailed => Some(6),
        EditorSignpostStage::FrameBuildComplete => Some(7),
        EditorSignpostStage::TextSummary
        | EditorSignpostStage::LayoutCacheSummary
        | EditorSignpostStage::GlyphAtlasSummary
        | EditorSignpostStage::FrameBuildFailed
        | EditorSignpostStage::NativeEventHandlerLatency
        | EditorSignpostStage::NativeFrameQueueLatency
        | EditorSignpostStage::NativeSubmissionLatency
        | EditorSignpostStage::NativeGpuTerminalObservedLatency
        | EditorSignpostStage::NativePresentedHandlerLatency
        | EditorSignpostStage::NativeTerminalRecordLatency
        | EditorSignpostStage::NativeDisplayLinkTargetLatency
        | EditorSignpostStage::NativeTargetPresentationLatency
        | EditorSignpostStage::NativeActualPresentationLatency
        | EditorSignpostStage::NativePresentationCallbackLag => None,
    }
}

/// One frame attempt, from its event's receipt to its terminal record.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FrameRecord {
    event: u64,
    revision: u64,
    event_ns: u64,
    offsets: [u32; STAGE_COUNT],
    outcome: PresentationOutcome,
}

impl FrameRecord {
    pub(crate) const fn new(revision: u64, outcome: PresentationOutcome) -> Self {
        Self {
            event: 0,
            revision,
            event_ns: 0,
            offsets: [ABSENT; STAGE_COUNT],
            outcome,
        }
    }

    /// Adopts the event's sequence and the stages its dispatch recorded.
    pub(crate) fn set_event(&mut self, event: u64, stages: EditorStages) {
        self.event = event;
        self.offsets[..EDITOR_STAGES].copy_from_slice(&stages);
    }

    /// Sets one column from nanoseconds after event receipt.
    pub(crate) fn set(&mut self, column: usize, ns: Option<u64>) {
        if let (Some(slot), Some(ns)) = (self.offsets.get_mut(column), ns) {
            *slot = offset(ns);
        }
    }
}

// Saturates one below ABSENT, so 4294967294 reads as "4.29 s or later".
fn offset(ns: u64) -> u32 {
    u32::try_from(ns).map_or(ABSENT - 1, |ns| ns.min(ABSENT - 1))
}

fn nanos(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

/// CPU time and wakeups of this process since it started.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct TaskUsage {
    pub(crate) cpu_ns: u64,
    pub(crate) interrupt_wakeups: u64,
    pub(crate) idle_wakeups: u64,
}

// Footprints are phys_footprint bytes; zero means the read failed. Servers
// index the recorder's server names.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct ProcessSample {
    time_ns: u64,
    footprint: u64,
    usage: Option<TaskUsage>,
    submissions: u64,
    child_pids: [u32; MAX_SAMPLED_CHILDREN],
    child_servers: [u8; MAX_SAMPLED_CHILDREN],
    child_footprints: [u64; MAX_SAMPLED_CHILDREN],
    dropped_children: u16,
}

// The whole reservation stays under the recorder's 1 MiB ceiling.
const RESERVED_BYTES: usize = FRAME_CAPACITY * size_of::<FrameRecord>()
    + SAMPLE_CAPACITY * size_of::<ProcessSample>()
    + MAX_SERVERS * (size_of::<Box<str>>() + SERVER_NAME_BYTES);
const _: () = assert!(RESERVED_BYTES <= 1 << 20);
const _: () = assert!(MAX_SERVERS < NO_SERVER as usize);

// Each server name once, never evicted. An empty name, one with a control
// character, or one arriving after the table fills is recorded as unknown.
#[derive(Debug)]
struct ServerNames(Vec<Box<str>>);

impl ServerNames {
    fn new() -> Self {
        Self(Vec::with_capacity(MAX_SERVERS))
    }

    fn index(&mut self, name: &str) -> u8 {
        let name = &name[..name.floor_char_boundary(SERVER_NAME_BYTES)];
        if name.is_empty() || name.contains(char::is_control) {
            return NO_SERVER;
        }
        let index = match self.0.iter().position(|known| **known == *name) {
            Some(index) => index,
            None if self.0.len() < MAX_SERVERS => {
                self.0.push(Box::from(name));
                self.0.len() - 1
            }
            None => return NO_SERVER,
        };
        u8::try_from(index).unwrap_or(NO_SERVER)
    }
}

/// The language servers one process sample reads, beyond the editor.
pub struct SampledChildren<'a> {
    names: &'a mut ServerNames,
    pids: [u32; MAX_SAMPLED_CHILDREN],
    servers: [u8; MAX_SAMPLED_CHILDREN],
    len: usize,
    dropped: u16,
}

impl SampledChildren<'_> {
    /// Adds a running child and its server's name, such as `rust-analyzer`.
    /// A zero PID is ignored; children past the sixth are counted, not read.
    pub fn push(&mut self, pid: u32, server: &str) {
        if pid == 0 {
            return;
        }
        let slots = (self.pids.get_mut(self.len), self.servers.get_mut(self.len));
        if let (Some(slot), Some(name)) = slots {
            *slot = pid;
            *name = self.names.index(server);
            self.len += 1;
        } else {
            self.dropped = self.dropped.saturating_add(1);
        }
    }
}

/// Capacity reserved once; pushing past it overwrites the oldest item.
#[derive(Debug)]
struct Ring<T> {
    items: Vec<T>,
    capacity: usize,
    next: usize,
}

impl<T: Copy> Ring<T> {
    fn new(capacity: usize) -> Self {
        Self {
            items: Vec::with_capacity(capacity),
            capacity,
            next: 0,
        }
    }

    fn push(&mut self, item: T) {
        if self.items.len() < self.capacity {
            self.items.push(item);
        } else if let Some(slot) = self.items.get_mut(self.next) {
            *slot = item;
        }
        self.next = (self.next + 1).checked_rem(self.capacity).unwrap_or(0);
    }

    fn to_vec(&self) -> Vec<T> {
        let (newest, oldest) = self.items.split_at(self.next.min(self.items.len()));
        [oldest, newest].concat()
    }
}

struct Pending {
    event: u64,
    received_at: Instant,
    stages: EditorStages,
}

struct Recorder {
    origin: Instant,
    origin_ns: u64,
    pending: Option<Pending>,
    frames: Ring<FrameRecord>,
    samples: Ring<ProcessSample>,
    servers: ServerNames,
    last_sample: Option<Instant>,
    submissions: u64,
    work: u64,
}

impl Recorder {
    fn new(origin: Instant, origin_seconds: f64) -> Self {
        Self {
            origin,
            origin_ns: Duration::try_from_secs_f64(origin_seconds).map_or(0, nanos),
            pending: None,
            frames: Ring::new(FRAME_CAPACITY),
            samples: Ring::new(SAMPLE_CAPACITY),
            servers: ServerNames::new(),
            last_sample: None,
            submissions: 0,
            work: 0,
        }
    }

    fn clock_ns(&self, at: Instant) -> u64 {
        let elapsed = nanos(at.saturating_duration_since(self.origin));
        self.origin_ns.saturating_add(elapsed)
    }

    fn stage(&mut self, column: usize, event: u64, now: Instant) {
        let Some(pending) = self
            .pending
            .as_mut()
            .filter(|pending| pending.event == event)
        else {
            return;
        };
        if let Some(slot) = pending.stages.get_mut(column) {
            *slot = offset(nanos(now.saturating_duration_since(pending.received_at)));
            self.work = self.work.saturating_add(1);
        }
    }

    fn take_stages(&mut self, event: u64) -> EditorStages {
        self.pending
            .take()
            .filter(|pending| pending.event == event)
            .map_or(NO_STAGES, |pending| pending.stages)
    }

    fn frame(&mut self, mut frame: FrameRecord, base: Instant, submissions: u8) {
        frame.event_ns = self.clock_ns(base);
        self.submissions = self.submissions.saturating_add(u64::from(submissions));
        self.frames.push(frame);
        self.work = self.work.saturating_add(1);
    }

    fn sample_due(&self, now: Instant) -> bool {
        self.last_sample
            .is_none_or(|last| now.saturating_duration_since(last) >= SAMPLE_INTERVAL)
    }

    fn sample(
        &mut self,
        now: Instant,
        children: impl FnOnce(&mut SampledChildren<'_>),
        footprint: impl Fn(u32) -> Option<u64>,
        usage: Option<TaskUsage>,
    ) {
        let mut sampled = SampledChildren {
            names: &mut self.servers,
            pids: [0; MAX_SAMPLED_CHILDREN],
            servers: [NO_SERVER; MAX_SAMPLED_CHILDREN],
            len: 0,
            dropped: 0,
        };
        children(&mut sampled);
        let SampledChildren {
            pids,
            servers,
            dropped,
            ..
        } = sampled;
        let mut sample = ProcessSample {
            time_ns: self.clock_ns(now),
            footprint: footprint(std::process::id()).unwrap_or(0),
            usage,
            submissions: self.submissions,
            child_pids: pids,
            child_servers: servers,
            child_footprints: [0; MAX_SAMPLED_CHILDREN],
            dropped_children: dropped,
        };
        let reads = sample.child_footprints.iter_mut().zip(pids);
        for (bytes, pid) in reads.filter(|(_, pid)| *pid != 0) {
            *bytes = footprint(pid).unwrap_or(0);
        }
        self.samples.push(sample);
        self.last_sample = Some(now);
        self.work = self.work.saturating_add(1);
    }
}

thread_local! {
    static RECORDER: RefCell<Option<Recorder>> = const { RefCell::new(None) };
}

fn with_recorder<T>(apply: impl FnOnce(&mut Recorder) -> T) -> Option<T> {
    RECORDER
        .try_with(|cell| {
            let mut slot = cell.try_borrow_mut().ok()?;
            slot.as_mut().map(apply)
        })
        .ok()
        .flatten()
}

/// Starts this thread's recorder once; `origin_seconds` is `origin` on the
/// uptime clock that `CACurrentMediaTime` and display-link timestamps use.
pub(crate) fn start(origin: Instant, origin_seconds: f64) {
    let _ = RECORDER.try_with(|cell| {
        if let Ok(mut slot) = cell.try_borrow_mut()
            && slot.is_none()
        {
            *slot = Some(Recorder::new(origin, origin_seconds));
        }
    });
}

pub(crate) fn begin_event(event: u64, received_at: Instant) {
    let _ = with_recorder(|recorder| {
        recorder.pending = Some(Pending {
            event,
            received_at,
            stages: NO_STAGES,
        });
    });
}

pub(crate) fn take_stages(event: u64) -> EditorStages {
    with_recorder(|recorder| recorder.take_stages(event)).unwrap_or(NO_STAGES)
}

pub(crate) fn record_stage(point: EditorSignpost) {
    if let Some(column) = editor_column(point.stage()) {
        let _ = with_recorder(|recorder| {
            recorder.stage(column, point.event_timestamp(), Instant::now());
        });
    }
}

pub(crate) fn record_frame(frame: FrameRecord, base: Instant, submissions: u8) {
    let _ = with_recorder(|recorder| recorder.frame(frame, base, submissions));
}

/// Samples this process and up to six children, at most once a second.
/// `children` adds each running language server; it runs only when a
/// sample is due. Call it from event handling, so idle takes no samples.
pub fn sample_processes(children: impl FnOnce(&mut SampledChildren<'_>)) {
    let _ = with_recorder(|recorder| {
        let now = Instant::now();
        if !recorder.sample_due(now) {
            return;
        }
        let usage = crate::implementation::task_usage();
        let footprint = crate::implementation::phys_footprint;
        recorder.sample(now, children, footprint, usage);
    });
}

/// Starts the calling thread's recorder for downstream tests.
#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub fn start_recorder_for_test() {
    start(Instant::now(), 0.0);
}

/// The newest 4,096 frames and 3,600 process samples, oldest first. Frame
/// times after `event_ns`, on the uptime clock, are nanoseconds after it;
/// an empty TSV field is absent evidence, never zero.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RecorderSnapshot {
    frames: Vec<FrameRecord>,
    samples: Vec<ProcessSample>,
    servers: Vec<Box<str>>,
    work: u64,
}

impl RecorderSnapshot {
    /// Copies the calling thread's recorder; empty on a thread without one.
    /// The main thread's recorder starts with its first native surface.
    #[must_use]
    pub fn capture() -> Self {
        with_recorder(|recorder| Self::of(recorder)).unwrap_or_default()
    }

    fn of(recorder: &Recorder) -> Self {
        Self {
            frames: recorder.frames.to_vec(),
            samples: recorder.samples.to_vec(),
            servers: recorder.servers.0.clone(),
            work: recorder.work,
        }
    }

    fn server(&self, index: u8) -> &str {
        self.servers.get(usize::from(index)).map_or("", |name| name)
    }

    /// Returns the number of retained frame records.
    #[must_use]
    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }

    /// Returns the number of retained process samples.
    #[must_use]
    pub fn sample_count(&self) -> usize {
        self.samples.len()
    }

    /// Returns the stage points, frames and samples recorded since start.
    #[must_use]
    pub const fn work(&self) -> u64 {
        self.work
    }

    /// Writes a header and one TSV row per frame attempt.
    /// # Errors
    /// Returns the first error from `out`.
    pub fn write_frames_tsv(&self, out: &mut impl Write) -> io::Result<()> {
        out.write_all(FRAME_HEADER.as_bytes())?;
        for frame in &self.frames {
            // Event sequences start at 1, so 0 marks a frame no event produced.
            field_first(out, (frame.event != 0).then_some(frame.event))?;
            write!(
                out,
                "\t{}\t{}\t{}",
                frame.revision,
                outcome_name(frame.outcome),
                frame.event_ns
            )?;
            for offset in frame.offsets {
                field(out, (offset != ABSENT).then_some(u64::from(offset)))?;
            }
            out.write_all(b"\n")?;
        }
        Ok(())
    }

    /// Writes a header and one TSV row per process per sample.
    /// # Errors
    /// Returns the first error from `out`.
    pub fn write_samples_tsv(&self, out: &mut impl Write) -> io::Result<()> {
        out.write_all(SAMPLE_HEADER.as_bytes())?;
        let editor = std::process::id();
        for sample in &self.samples {
            write!(out, "{}\teditor\t{editor}\t", sample.time_ns)?;
            field(out, (sample.footprint != 0).then_some(sample.footprint))?;
            field(out, sample.usage.map(|usage| usage.cpu_ns))?;
            field(out, sample.usage.map(|usage| usage.interrupt_wakeups))?;
            field(out, sample.usage.map(|usage| usage.idle_wakeups))?;
            let (submissions, dropped) = (sample.submissions, sample.dropped_children);
            writeln!(out, "\t{submissions}\t{dropped}")?;
            let children = sample.child_pids.iter().zip(sample.child_servers);
            let children = children.zip(sample.child_footprints);
            for ((pid, server), bytes) in children.filter(|((pid, _), _)| **pid != 0) {
                let server = self.server(server);
                write!(out, "{}\tlanguage-server\t{pid}\t{server}", sample.time_ns)?;
                field(out, (bytes != 0).then_some(bytes))?;
                out.write_all(b"\t\t\t\t\t\n")?;
            }
        }
        Ok(())
    }
}

fn field_first(out: &mut impl Write, value: Option<u64>) -> io::Result<()> {
    match value {
        Some(value) => write!(out, "{value}"),
        None => Ok(()),
    }
}

fn field(out: &mut impl Write, value: Option<u64>) -> io::Result<()> {
    match value {
        Some(value) => write!(out, "\t{value}"),
        None => out.write_all(b"\t"),
    }
}

const fn outcome_name(outcome: PresentationOutcome) -> &'static str {
    match outcome {
        PresentationOutcome::None => "none",
        PresentationOutcome::Presented => "presented",
        PresentationOutcome::Superseded => "superseded",
        PresentationOutcome::Cancelled => "cancelled",
        PresentationOutcome::Failed => "failed",
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, error::Error};

    use super::*;

    fn tsv(write: impl FnOnce(&mut Vec<u8>) -> io::Result<()>) -> Result<String, Box<dyn Error>> {
        let mut out = Vec::new();
        write(&mut out)?;
        Ok(String::from_utf8(out)?)
    }

    fn nanos_after(origin: Instant, ns: u64) -> Instant {
        origin + Duration::from_nanos(ns)
    }

    #[test]
    fn rings_wrap_in_place_without_reallocating() {
        let mut ring = Ring::new(4);
        ring.push(0_u64);
        ring.push(1);
        assert_eq!(ring.to_vec(), [0, 1]);
        let reserved = (ring.items.capacity(), ring.items.as_ptr());
        for value in 2..11 {
            ring.push(value);
        }
        assert_eq!(ring.to_vec(), [7, 8, 9, 10]);
        assert_eq!(ring.items.len(), 4);
        assert_eq!((ring.items.capacity(), ring.items.as_ptr()), reserved);
    }

    #[test]
    fn recorder_reserves_its_rings_once_under_one_mebibyte() {
        let origin = Instant::now();
        let mut recorder = Recorder::new(origin, 0.0);
        let reserved = [
            (
                recorder.frames.items.capacity(),
                recorder.frames.items.as_ptr().addr(),
            ),
            (
                recorder.samples.items.capacity(),
                recorder.samples.items.as_ptr().addr(),
            ),
        ];
        assert!(reserved[0].0 >= FRAME_CAPACITY && reserved[1].0 >= SAMPLE_CAPACITY);
        let bytes =
            reserved[0].0 * size_of::<FrameRecord>() + reserved[1].0 * size_of::<ProcessSample>();
        assert!(bytes <= 1 << 20);
        let frame = FrameRecord::new(1, PresentationOutcome::Presented);
        for _ in 0..FRAME_CAPACITY + 7 {
            recorder.frame(frame, origin, 1);
        }
        let mut now = origin;
        for _ in 0..SAMPLE_CAPACITY + 7 {
            recorder.sample(now, |_| {}, |_| Some(1), None);
            now += SAMPLE_INTERVAL;
        }
        let after = [
            (
                recorder.frames.items.capacity(),
                recorder.frames.items.as_ptr().addr(),
            ),
            (
                recorder.samples.items.capacity(),
                recorder.samples.items.as_ptr().addr(),
            ),
        ];
        assert_eq!(after, reserved);
        let snapshot = RecorderSnapshot::of(&recorder);
        assert_eq!(snapshot.frame_count(), FRAME_CAPACITY);
        assert_eq!(snapshot.sample_count(), SAMPLE_CAPACITY);
        let submitted = u64::try_from(FRAME_CAPACITY + 7).unwrap_or(0);
        assert_eq!(recorder.submissions, submitted);
    }

    #[test]
    fn stages_join_only_their_event_and_frames_use_the_uptime_clock() -> Result<(), Box<dyn Error>>
    {
        let origin = Instant::now();
        let mut recorder = Recorder::new(origin, 2.0);
        recorder.stage(0, 7, origin);
        recorder.pending = Some(Pending {
            event: 7,
            received_at: nanos_after(origin, 10),
            stages: NO_STAGES,
        });
        recorder.stage(0, 7, nanos_after(origin, 15));
        recorder.stage(1, 8, nanos_after(origin, 20));
        recorder.stage(7, 7, nanos_after(origin, 40));
        assert_eq!(recorder.work, 2);
        let stages = recorder.take_stages(7);
        let mut expected = NO_STAGES;
        expected[0] = 5;
        expected[7] = 30;
        assert_eq!(stages, expected);
        assert_eq!(recorder.take_stages(7), NO_STAGES);

        let mut frame = FrameRecord::new(3, PresentationOutcome::Superseded);
        frame.set_event(7, stages);
        frame.set(TARGET, Some(1_000));
        frame.set(PRESENTED, None);
        frame.set(RECORDED, Some(u64::MAX));
        frame.set(STAGE_COUNT, Some(1));
        recorder.frame(frame, nanos_after(origin, 10), 1);
        let mut offsets = [""; STAGE_COUNT];
        offsets[0] = "5";
        offsets[7] = "30";
        offsets[TARGET] = "1000";
        offsets[RECORDED] = "4294967294";
        let row = format!("7\t3\tsuperseded\t2000000010\t{}", offsets.join("\t"));
        let rows = tsv(|out| RecorderSnapshot::of(&recorder).write_frames_tsv(out))?;
        assert_eq!(rows.lines().nth(1), Some(row.as_str()));
        assert_eq!(recorder.work, 3);
        Ok(())
    }

    #[test]
    fn tsv_header_names_every_column_and_leaves_absent_fields_empty() -> Result<(), Box<dyn Error>>
    {
        let origin = Instant::now();
        let mut recorder = Recorder::new(origin, 0.0);
        let frame = FrameRecord::new(9, PresentationOutcome::Cancelled);
        recorder.frame(frame, origin, 0);
        let usage = TaskUsage {
            cpu_ns: 11,
            interrupt_wakeups: 12,
            idle_wakeups: 13,
        };
        let footprint = |pid: u32| (pid != 42).then_some(4_096);
        let children = |children: &mut SampledChildren<'_>| {
            children.push(41, "rust-analyzer");
            children.push(42, "");
        };
        recorder.sample(nanos_after(origin, 5), children, footprint, Some(usage));
        recorder.sample(nanos_after(origin, 6), |_| {}, |_| None, None);
        let snapshot = RecorderSnapshot::of(&recorder);
        let frames = tsv(|out| snapshot.write_frames_tsv(out))?;
        let mut lines = frames.lines();
        let header = lines.next().ok_or("frame header")?;
        let columns: Vec<&str> = header.split('\t').collect();
        assert_eq!(columns.len(), 4 + STAGE_COUNT);
        assert_eq!(
            columns[..5],
            ["event", "revision", "outcome", "event_ns", "dispatch_ns"]
        );
        assert_eq!(columns.get(4 + EDITOR_STAGES - 1), Some(&"build_end_ns"));
        assert_eq!(columns.get(4 + HANDLER_END), Some(&"handler_end_ns"));
        assert_eq!(columns.get(4 + GPU_OBSERVED), Some(&"gpu_observed_ns"));
        assert_eq!(columns.get(4 + TARGET), Some(&"target_ns"));
        assert_eq!(columns.get(4 + RECORDED), Some(&"recorded_ns"));
        let row = format!("\t9\tcancelled\t0{}", "\t".repeat(STAGE_COUNT));
        assert_eq!(lines.next(), Some(row.as_str()));
        assert_eq!(lines.next(), None);

        let editor = std::process::id();
        let samples = tsv(|out| snapshot.write_samples_tsv(out))?;
        let expected = [
            SAMPLE_HEADER.trim_end().to_owned(),
            format!("5\teditor\t{editor}\t\t4096\t11\t12\t13\t0\t0"),
            "5\tlanguage-server\t41\trust-analyzer\t4096\t\t\t\t\t".to_owned(),
            "5\tlanguage-server\t42\t\t\t\t\t\t\t".to_owned(),
            format!("6\teditor\t{editor}\t\t\t\t\t\t0\t0"),
        ];
        assert_eq!(samples.lines().collect::<Vec<_>>(), expected);
        Ok(())
    }

    #[test]
    fn a_sample_counts_the_children_past_its_limit() -> Result<(), Box<dyn Error>> {
        let origin = Instant::now();
        let mut recorder = Recorder::new(origin, 0.0);
        let children = |children: &mut SampledChildren<'_>| {
            for pid in 1..=8 {
                children.push(pid, "server");
            }
        };
        recorder.sample(origin, children, |_| Some(1), None);
        let samples = tsv(|out| RecorderSnapshot::of(&recorder).write_samples_tsv(out))?;
        let mut lines = samples.lines();
        let header: Vec<&str> = lines.next().ok_or("header")?.split('\t').collect();
        let column = header
            .iter()
            .position(|name| *name == "dropped_children")
            .ok_or("dropped_children column")?;
        let editor: Vec<&str> = lines.next().ok_or("editor row")?.split('\t').collect();
        assert_eq!(editor.get(column), Some(&"2"));
        let rows: Vec<&str> = lines.collect();
        assert_eq!(rows.len(), 6);
        assert!(
            rows.iter()
                .all(|row| row.split('\t').nth(column) == Some(""))
        );
        Ok(())
    }

    #[test]
    fn server_names_are_kept_once_in_a_bounded_tsv_safe_table() -> Result<(), Box<dyn Error>> {
        let mut names = ServerNames::new();
        let reserved = (names.0.capacity(), names.0.as_ptr().addr());
        assert_eq!(names.index("rust-analyzer"), 0);
        assert_eq!(names.index("clangd"), 1);
        assert_eq!(names.index("rust-analyzer"), 0);
        for unknown in ["", "two\twords", "line\nbreak"] {
            assert_eq!(names.index(unknown), NO_SERVER);
        }
        // Truncation keeps whole characters: 21 three-byte euros fit in 64.
        assert_eq!(names.index(&"\u{20ac}".repeat(SERVER_NAME_BYTES)), 2);
        assert_eq!(names.0.get(2).map(|name| name.len()), Some(63));
        for index in 3..MAX_SERVERS {
            assert_eq!(
                names.index(&format!("server-{index}")),
                u8::try_from(index)?
            );
        }
        assert_eq!(names.index("one-too-many"), NO_SERVER);
        assert_eq!(names.index("clangd"), 1);
        assert_eq!((names.0.capacity(), names.0.as_ptr().addr()), reserved);
        Ok(())
    }

    #[test]
    fn idle_threads_record_nothing_until_an_event_dispatches() {
        let point = EditorSignpost::new(EditorSignpostStage::FrameBuildBegin, 9, 1, 1, 1, [0; 3]);
        record_stage(point);
        assert_eq!(RecorderSnapshot::capture(), RecorderSnapshot::default());

        start_recorder_for_test();
        record_stage(point);
        assert_eq!(take_stages(9), NO_STAGES);
        assert_eq!(RecorderSnapshot::capture().work(), 0);
        begin_event(9, Instant::now());
        record_stage(point);
        record_stage(EditorSignpost::new(
            EditorSignpostStage::TextSummary,
            9,
            1,
            1,
            1,
            [0; 3],
        ));
        assert_ne!(take_stages(9)[2], ABSENT);
        record_frame(
            FrameRecord::new(1, PresentationOutcome::Presented),
            Instant::now(),
            1,
        );
        let busy = RecorderSnapshot::capture();
        assert_eq!((busy.work(), busy.frame_count()), (2, 1));
        assert_eq!(RecorderSnapshot::capture(), busy);
    }

    #[test]
    fn events_sample_at_most_once_a_second_and_idle_samples_nothing() -> Result<(), Box<dyn Error>>
    {
        let calls = Cell::new(0_u32);
        let children = |children: &mut SampledChildren<'_>| {
            calls.set(calls.get() + 1);
            children.push(std::process::id(), "self");
        };
        sample_processes(children);
        assert_eq!((RecorderSnapshot::capture().work(), calls.get()), (0, 0));
        start_recorder_for_test();
        let started = Instant::now();
        for _ in 0..3 {
            sample_processes(children);
        }
        // A stalled runner may sample again a second later, but never sooner.
        let allowed = 1 + started.elapsed().as_secs();
        let sampled = RecorderSnapshot::capture();
        let count = u64::try_from(sampled.sample_count())?;
        assert!((1..=allowed).contains(&count));
        assert_eq!((sampled.work(), u64::from(calls.get())), (count, count));
        assert_eq!(RecorderSnapshot::capture(), sampled);

        let origin = Instant::now();
        let mut recorder = Recorder::new(origin, 0.0);
        assert!(recorder.sample_due(origin));
        recorder.sample(origin, |_| {}, |_| None, None);
        assert!(!recorder.sample_due(nanos_after(origin, 999_999_999)));
        assert!(recorder.sample_due(origin + SAMPLE_INTERVAL));
        Ok(())
    }

    #[test]
    #[ignore = "local microbenchmark; run with --release -- --ignored --nocapture"]
    fn stage_point_cost() {
        start_recorder_for_test();
        let signposts = crate::EditorSignposts::for_test(false, false);
        let point = EditorSignpost::new(EditorSignpostStage::FrameBuildBegin, 1, 1, 1, 1, [0; 3]);
        let rounds = 1_000_000_u32;
        begin_event(1, Instant::now());
        let started = Instant::now();
        for _ in 0..rounds {
            let _ = signposts.emit(std::hint::black_box(point));
        }
        println!("stage point: {:?}", started.elapsed() / rounds);
        let started = Instant::now();
        for _ in 0..1_000 {
            let _ = crate::implementation::phys_footprint(std::process::id());
            let _ = crate::implementation::task_usage();
        }
        println!("editor usage read: {:?}", started.elapsed() / 1_000);
        println!("reserved ring bytes: {RESERVED_BYTES}");
    }
}
