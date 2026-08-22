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
            enabled: imp::enabled(),
        }
    }

    /// Returns whether Instruments enabled dynamic signposts at construction.
    #[must_use]
    pub const fn enabled(self) -> bool {
        self.enabled
    }

    /// Emits one point when recording is enabled and otherwise performs no FFI call.
    pub fn emit(self, point: StudioSignpost) {
        if self.enabled {
            imp::emit(point);
        }
    }
}

impl Default for StudioSignposts {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod imp {
    use super::StudioSignpost;

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

    pub(super) fn enabled() -> bool {
        // SAFETY: The Alpine-owned C shim takes no pointers and initializes its
        // process-lifetime os_log handle through dispatch_once.
        unsafe { alpine_studio_signposts_enabled() }
    }

    pub(super) fn emit(point: StudioSignpost) {
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
}

#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
mod imp {
    use super::StudioSignpost;

    pub(super) const fn enabled() -> bool {
        false
    }

    pub(super) const fn emit(_point: StudioSignpost) {}
}

#[cfg(test)]
mod tests {
    use super::*;

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

        if !StudioSignposts::new().enabled() {
            StudioSignposts::new().emit(point);
        }
        StudioSignposts { enabled: true }.emit(point);
    }
}
