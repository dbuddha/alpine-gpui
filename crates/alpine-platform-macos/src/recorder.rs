//! Always-on recorder: a ring of frame stages reserved once at start and
//! filled only by events and frames the app already handles. It owns no
//! timer or thread, so an idle app records nothing.

use std::{
    cell::RefCell,
    io::{self, Write},
    time::{Duration, Instant},
};

use alpine_platform::PresentationOutcome;

use crate::{EditorSignpost, EditorSignpostStage};

const FRAME_CAPACITY: usize = 4_096;
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

// The whole reservation stays under the recorder's 1 MiB ceiling.
const RESERVED_BYTES: usize = FRAME_CAPACITY * size_of::<FrameRecord>();
const _: () = assert!(RESERVED_BYTES <= 1 << 20);

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
    work: u64,
}

impl Recorder {
    fn new(origin: Instant, origin_seconds: f64) -> Self {
        Self {
            origin,
            origin_ns: Duration::try_from_secs_f64(origin_seconds).map_or(0, nanos),
            pending: None,
            frames: Ring::new(FRAME_CAPACITY),
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

    fn frame(&mut self, mut frame: FrameRecord, base: Instant) {
        frame.event_ns = self.clock_ns(base);
        self.frames.push(frame);
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

pub(crate) fn record_frame(frame: FrameRecord, base: Instant) {
    let _ = with_recorder(|recorder| recorder.frame(frame, base));
}

/// Starts the calling thread's recorder for downstream tests.
#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub fn start_recorder_for_test() {
    start(Instant::now(), 0.0);
}

/// The newest 4,096 frame attempts, oldest first. Times after `event_ns`,
/// on the uptime clock, are nanoseconds after it; an empty TSV field is
/// absent evidence, never zero.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RecorderSnapshot {
    frames: Vec<FrameRecord>,
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
            work: recorder.work,
        }
    }

    /// Returns the number of retained frame records.
    #[must_use]
    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }

    /// Returns the stage points and frames recorded since start.
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
    use std::error::Error;

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
    fn recorder_reserves_its_ring_once_under_one_mebibyte() {
        let origin = Instant::now();
        let mut recorder = Recorder::new(origin, 0.0);
        let frames = &recorder.frames.items;
        let reserved = (frames.capacity(), frames.as_ptr());
        assert!(reserved.0 >= FRAME_CAPACITY);
        assert!(reserved.0 * size_of::<FrameRecord>() <= 1 << 20);
        let frame = FrameRecord::new(1, PresentationOutcome::Presented);
        for _ in 0..FRAME_CAPACITY + 7 {
            recorder.frame(frame, origin);
        }
        let frames = &recorder.frames.items;
        assert_eq!((frames.capacity(), frames.as_ptr()), reserved);
        assert_eq!(
            RecorderSnapshot::of(&recorder).frame_count(),
            FRAME_CAPACITY
        );
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
        recorder.frame(frame, nanos_after(origin, 10));
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
        recorder.frame(frame, origin);
        let frames = tsv(|out| RecorderSnapshot::of(&recorder).write_frames_tsv(out))?;
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
        );
        let busy = RecorderSnapshot::capture();
        assert_eq!((busy.work(), busy.frame_count()), (2, 1));
        assert_eq!(RecorderSnapshot::capture(), busy);
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
        println!("reserved ring bytes: {RESERVED_BYTES}");
    }
}
