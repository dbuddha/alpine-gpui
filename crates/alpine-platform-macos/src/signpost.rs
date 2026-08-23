//! Handle-free Alpine Studio points for externally retained Instruments traces.

/// Stable stage vocabulary emitted by the Alpine Studio release hot path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum StudioSignpostStage {
    /// Native event dispatch entered Studio state.
    EventDispatchBegin = 0,
    /// Synchronous Studio state mutation and admission completed.
    StateMutationComplete = 1,
    /// Immutable scene construction began.
    FrameBuildBegin = 2,
    /// Visible editor-line layout began.
    VisibleLayoutBegin = 3,
    /// Visible editor-line layout completed.
    VisibleLayoutComplete = 4,
    /// Shaping and confirmed glyph-rasterization deltas were sampled.
    TextSummary = 5,
    /// Current-frame line-layout cache deltas were sampled.
    LayoutCacheSummary = 6,
    /// Current-frame glyph-atlas lookup and residency deltas were sampled.
    GlyphAtlasSummary = 7,
    /// CPU atlas publication planning began.
    AtlasPublicationBegin = 8,
    /// CPU atlas publication completed.
    AtlasPublicationComplete = 9,
    /// CPU atlas publication failed structurally.
    AtlasPublicationFailed = 10,
    /// Immutable scene construction completed.
    FrameBuildComplete = 11,
    /// Scene construction failed and the fallback path was selected.
    FrameBuildFailed = 12,
    /// Synchronous native event handling completed; `a` is elapsed nanoseconds.
    NativeEventHandlerLatency = 13,
    /// Frame admission waited for the display-link callback; `a` is elapsed nanoseconds.
    NativeFrameQueueLatency = 14,
    /// Native validation, upload, encode, commit, and present completed; `a` is nanoseconds.
    NativeSubmissionLatency = 15,
    /// GPU terminal state reached the main-thread observer; `a` is an upper-bound nanoseconds.
    NativeGpuTerminalObservedLatency = 16,
    /// The drawable presented handler ran; `a` is nanoseconds from event receipt.
    NativePresentedHandlerLatency = 17,
    /// Alpine published terminal frame evidence; `a` is nanoseconds from event receipt.
    NativeTerminalRecordLatency = 18,
}

/// One numeric, revision-correlated point suitable for a dynamic signpost.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StudioSignpost {
    stage: StudioSignpostStage,
    event_timestamp: u64,
    scene_revision: u64,
    document_revision: u64,
    buffer_revision: u64,
    values: [u64; 3],
}

impl StudioSignpost {
    /// Creates one handle-free point without formatting or retaining payloads.
    #[must_use]
    pub const fn new(
        stage: StudioSignpostStage,
        event_timestamp: u64,
        scene_revision: u64,
        document_revision: u64,
        buffer_revision: u64,
        values: [u64; 3],
    ) -> Self {
        Self {
            stage,
            event_timestamp,
            scene_revision,
            document_revision,
            buffer_revision,
            values,
        }
    }

    /// Returns the stable stage identity.
    #[must_use]
    pub const fn stage(self) -> StudioSignpostStage {
        self.stage
    }

    /// Returns the native process-local event sequence.
    #[must_use]
    pub const fn event_timestamp(self) -> u64 {
        self.event_timestamp
    }

    /// Returns the immutable scene revision, or zero for event-only points.
    #[must_use]
    pub const fn scene_revision(self) -> u64 {
        self.scene_revision
    }

    /// Returns the Studio runtime document revision.
    #[must_use]
    pub const fn document_revision(self) -> u64 {
        self.document_revision
    }

    /// Returns the local text-buffer revision.
    #[must_use]
    pub const fn buffer_revision(self) -> u64 {
        self.buffer_revision
    }

    /// Returns the stage-specific numeric values documented by the capture protocol.
    #[must_use]
    pub const fn values(self) -> [u64; 3] {
        self.values
    }

    const fn correlation(self) -> u64 {
        if self.event_timestamp == 0 {
            let scene_revision = if self.scene_revision == 0 {
                1
            } else {
                self.scene_revision
            };
            (1_u64 << 63) | scene_revision
        } else {
            self.event_timestamp
        }
    }
}

/// Process-lifetime dynamic signpost writer with no retained sample storage.
#[derive(Clone, Copy, Debug)]
pub struct StudioSignposts {
    enabled: bool,
}

impl StudioSignposts {
    /// Initializes the static dynamic-tracing category before the event loop starts.
    #[must_use]
    pub fn new() -> Self {
        Self {
            enabled: imp::enabled().0,
        }
    }

    /// Returns whether Instruments enabled dynamic signposts at construction.
    #[must_use]
    pub const fn enabled(self) -> bool {
        self.enabled
    }

    /// Emits one point and returns its correlation when recording is enabled.
    #[must_use]
    pub fn emit(self, point: StudioSignpost) -> Option<u64> {
        if self.enabled {
            Some(imp::emit(point))
        } else {
            None
        }
    }

    #[cfg(any(test, all(target_os = "macos", target_arch = "aarch64")))]
    pub(crate) fn emit_frame_latency(self, evidence: crate::FrameLatencyEvidence) -> u8 {
        if !self.enabled {
            return 0;
        }
        let mut emitted = 0_u8;
        for point in frame_latency_points(evidence).into_iter().flatten() {
            let _correlation = imp::emit(point);
            emitted = emitted.saturating_add(1);
        }
        emitted
    }
}

#[cfg(any(test, all(target_os = "macos", target_arch = "aarch64")))]
fn frame_latency_point(
    stage: StudioSignpostStage,
    evidence: crate::FrameLatencyEvidence,
    duration_ns: u64,
) -> StudioSignpost {
    StudioSignpost::new(
        stage,
        evidence.event_timestamp().get(),
        0,
        0,
        0,
        [duration_ns, 0, 0],
    )
}

#[cfg(any(test, all(target_os = "macos", target_arch = "aarch64")))]
fn frame_latency_points(evidence: crate::FrameLatencyEvidence) -> [Option<StudioSignpost>; 6] {
    [
        Some(frame_latency_point(
            StudioSignpostStage::NativeEventHandlerLatency,
            evidence,
            evidence.event_handler_ns(),
        )),
        evidence.frame_queue_ns().map(|duration_ns| {
            frame_latency_point(
                StudioSignpostStage::NativeFrameQueueLatency,
                evidence,
                duration_ns,
            )
        }),
        evidence.submission_ns().map(|duration_ns| {
            frame_latency_point(
                StudioSignpostStage::NativeSubmissionLatency,
                evidence,
                duration_ns,
            )
        }),
        evidence
            .event_to_gpu_terminal_observed_ns()
            .map(|duration_ns| {
                frame_latency_point(
                    StudioSignpostStage::NativeGpuTerminalObservedLatency,
                    evidence,
                    duration_ns,
                )
            }),
        evidence.event_to_presented_handler_ns().map(|duration_ns| {
            frame_latency_point(
                StudioSignpostStage::NativePresentedHandlerLatency,
                evidence,
                duration_ns,
            )
        }),
        Some(frame_latency_point(
            StudioSignpostStage::NativeTerminalRecordLatency,
            evidence,
            evidence.event_to_terminal_record_ns(),
        )),
    ]
}

struct DynamicTracingState(bool);

impl Default for StudioSignposts {
    fn default() -> Self {
        Self::new()
    }
}

mod imp {
    use super::{DynamicTracingState, StudioSignpost};

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    unsafe extern "C" {
        fn alpine_studio_signposts_enabled() -> bool;
        fn alpine_studio_signpost_emit(
            stage: u8,
            correlation: u64,
            event_timestamp: u64,
            scene_revision: u64,
            document_revision: u64,
            buffer_revision: u64,
            value_a: u64,
            value_b: u64,
            value_c: u64,
        );
    }

    pub(super) fn enabled() -> DynamicTracingState {
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        // SAFETY: The Alpine-owned C shim takes no pointers and initializes its
        // process-lifetime os_log handle through dispatch_once.
        let enabled = unsafe { alpine_studio_signposts_enabled() };
        #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
        let enabled = false;
        DynamicTracingState(enabled)
    }

    pub(super) fn emit(point: StudioSignpost) -> u64 {
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        {
            let values = point.values();
            // SAFETY: Every argument is a copied integer. The stage discriminant is
            // repr(u8), and the C shim neither retains Rust storage nor calls back.
            unsafe {
                alpine_studio_signpost_emit(
                    point.stage() as u8,
                    point.correlation(),
                    point.event_timestamp(),
                    point.scene_revision(),
                    point.document_revision(),
                    point.buffer_revision(),
                    values[0],
                    values[1],
                    values[2],
                );
            }
        }
        point.correlation()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EventTimestamp, FrameLatencyEvidence};

    #[test]
    fn point_identity_and_disabled_contract_are_handle_free() {
        let point = StudioSignpost::new(
            StudioSignpostStage::GlyphAtlasSummary,
            17,
            23,
            29,
            31,
            [37, 41, 43],
        );
        assert_eq!(point.stage(), StudioSignpostStage::GlyphAtlasSummary);
        assert_eq!(point.event_timestamp(), 17);
        assert_eq!(point.scene_revision(), 23);
        assert_eq!(point.document_revision(), 29);
        assert_eq!(point.buffer_revision(), 31);
        assert_eq!(point.values(), [37, 41, 43]);
        assert_eq!(point.correlation(), 17);

        let startup = StudioSignpost::new(StudioSignpostStage::FrameBuildBegin, 0, 5, 1, 1, [0; 3]);
        assert_eq!(startup.correlation(), (1_u64 << 63) | 5);
        let zero_startup =
            StudioSignpost::new(StudioSignpostStage::FrameBuildBegin, 0, 0, 1, 1, [0; 3]);
        assert_eq!(zero_startup.correlation(), (1_u64 << 63) | 1);
        let high_bit_startup = StudioSignpost::new(
            StudioSignpostStage::FrameBuildBegin,
            0,
            1_u64 << 63,
            1,
            1,
            [0; 3],
        );
        assert_eq!(high_bit_startup.correlation(), 1_u64 << 63);

        let disabled = StudioSignposts { enabled: false };
        assert!(!disabled.enabled());
        assert_eq!(disabled.emit(point), None);
        let enabled = StudioSignposts { enabled: true };
        assert!(enabled.enabled());
        assert_eq!(enabled.emit(point), Some(17));

        #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
        assert!(!StudioSignposts::new().enabled());
    }

    #[test]
    fn native_latency_points_preserve_order_values_omissions_and_boundaries() {
        let complete = FrameLatencyEvidence::new(
            EventTimestamp::new(53),
            0,
            Some(59),
            Some(61),
            Some(67),
            Some(71),
            u64::MAX,
        );
        let points = frame_latency_points(complete).map(Option::unwrap);
        assert_eq!(
            points.map(StudioSignpost::stage),
            [
                StudioSignpostStage::NativeEventHandlerLatency,
                StudioSignpostStage::NativeFrameQueueLatency,
                StudioSignpostStage::NativeSubmissionLatency,
                StudioSignpostStage::NativeGpuTerminalObservedLatency,
                StudioSignpostStage::NativePresentedHandlerLatency,
                StudioSignpostStage::NativeTerminalRecordLatency,
            ]
        );
        assert_eq!(points.map(StudioSignpost::event_timestamp), [53; 6]);
        assert_eq!(
            points.map(|point| point.values()[0]),
            [0, 59, 61, 67, 71, u64::MAX]
        );
        assert_eq!(
            StudioSignposts { enabled: true }.emit_frame_latency(complete),
            6
        );
        assert!(points.iter().all(|point| point.scene_revision() == 0
            && point.document_revision() == 0
            && point.buffer_revision() == 0));

        let omitted =
            FrameLatencyEvidence::new(EventTimestamp::new(73), 79, None, None, None, None, 83);
        let [handler, queue, submission, gpu, presented, terminal] = frame_latency_points(omitted);
        assert_eq!(
            handler.map(StudioSignpost::stage),
            Some(StudioSignpostStage::NativeEventHandlerLatency)
        );
        assert_eq!(queue, None);
        assert_eq!(submission, None);
        assert_eq!(gpu, None);
        assert_eq!(presented, None);
        assert_eq!(
            terminal.map(StudioSignpost::stage),
            Some(StudioSignpostStage::NativeTerminalRecordLatency)
        );
        assert_eq!(
            StudioSignposts { enabled: true }.emit_frame_latency(omitted),
            2
        );
        assert_eq!(
            StudioSignposts { enabled: false }.emit_frame_latency(complete),
            0
        );
    }
}
