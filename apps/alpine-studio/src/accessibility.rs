//! Bounded, revision-synchronized Studio accessibility semantics.

use std::{error::Error, fmt, sync::Arc};

#[cfg(test)]
pub(crate) use alpine_platform_macos::AccessibilityReport;
pub(crate) use alpine_platform_macos::{
    AccessibilityAction, AccessibilityBounds, AccessibilityNode, AccessibilityNodeId,
    AccessibilityRevision, AccessibilityRole, AccessibilitySelection,
    AccessibilitySnapshot as PlatformAccessibilitySnapshot, AccessibilityTextRange,
    MAX_ACCESSIBILITY_NODES,
};
use alpine_platform_macos::{
    AccessibilityActionResult, AccessibilityError as PlatformAccessibilityError,
    AccessibilityOperation, AccessibilityPayload, AccessibilityRequest, AccessibilityResponse,
    AccessibilityText, MAX_ACCESSIBILITY_TEXT_RESPONSE_BYTES,
};
use alpine_text::{BufferSnapshot, ByteOffset, Selection, TextError};
use alpine_text_layout::TextShaper;

use super::{EventEffect, MAX_VISIBLE_DIAGNOSTIC_MARKERS, StudioApp, StudioCommand};

pub(crate) const MAX_ACCESSIBILITY_TEXT_REQUEST_BYTES: usize =
    MAX_ACCESSIBILITY_TEXT_RESPONSE_BYTES;

const WINDOW_NODE: AccessibilityNodeId = AccessibilityNodeId::new(1);
const TAB_LIST_NODE: AccessibilityNodeId = AccessibilityNodeId::new(2);
const EDITOR_NODE: AccessibilityNodeId = AccessibilityNodeId::new(3);
const FILE_TREE_NODE: AccessibilityNodeId = AccessibilityNodeId::new(4);
const FIND_NODE: AccessibilityNodeId = AccessibilityNodeId::new(5);
const QUICK_OPEN_NODE: AccessibilityNodeId = AccessibilityNodeId::new(6);
const PROJECT_SEARCH_NODE: AccessibilityNodeId = AccessibilityNodeId::new(7);
const COMMAND_PALETTE_NODE: AccessibilityNodeId = AccessibilityNodeId::new(8);
const STATUS_NODE: AccessibilityNodeId = AccessibilityNodeId::new(9);
const COMPLETION_NODE: AccessibilityNodeId = AccessibilityNodeId::new(10);
const NAVIGATION_NODE: AccessibilityNodeId = AccessibilityNodeId::new(11);
const SYMBOL_NODE: AccessibilityNodeId = AccessibilityNodeId::new(12);
const WORKSPACE_EDIT_NODE: AccessibilityNodeId = AccessibilityNodeId::new(13);
const TAB_NODE_BASE: u64 = 1_024;
const FILE_ROW_NODE_BASE: u64 = 1 << 20;
const COMMAND_ROW_NODE_BASE: u64 = 2 << 20;
const DIAGNOSTIC_NODE_BASE: u64 = 3 << 20;

#[derive(Clone, Debug)]
pub(crate) struct AccessibilitySnapshot {
    transport: PlatformAccessibilitySnapshot,
    #[cfg(test)]
    text: BufferSnapshot,
}

impl AccessibilitySnapshot {
    #[cfg(test)]
    pub(crate) const fn revision(&self) -> AccessibilityRevision {
        self.transport.revision()
    }

    pub(crate) fn nodes(&self) -> &[AccessibilityNode] {
        self.transport.nodes()
    }

    #[cfg(test)]
    pub(crate) const fn selection(&self) -> AccessibilitySelection {
        self.transport.selection()
    }

    #[cfg(test)]
    pub(crate) const fn text_len_utf16(&self) -> usize {
        self.transport.text_len_utf16()
    }

    #[cfg(test)]
    pub(crate) const fn line_count(&self) -> usize {
        self.transport.line_count()
    }

    #[cfg(test)]
    pub(crate) const fn is_dirty(&self) -> bool {
        self.transport.is_dirty()
    }

    #[cfg(test)]
    pub(crate) const fn report(&self) -> AccessibilityReport {
        self.transport.report()
    }

    #[cfg(test)]
    pub(crate) fn text(&self, range: AccessibilityTextRange) -> Result<String, AccessibilityError> {
        text_from_snapshot(&self.text, range)
    }

    fn into_transport(self) -> PlatformAccessibilitySnapshot {
        self.transport
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum AccessibilityError {
    AllocationFailed,
    ArithmeticOverflow,
    InvalidTree,
    StaleRevision {
        expected: AccessibilityRevision,
        actual: AccessibilityRevision,
    },
    TextRequestTooLarge {
        actual: usize,
        limit: usize,
    },
    Text(TextError),
    Transport(PlatformAccessibilityError),
}

impl AccessibilityError {
    fn into_transport(self) -> PlatformAccessibilityError {
        match self {
            Self::AllocationFailed => PlatformAccessibilityError::AllocationFailed,
            Self::ArithmeticOverflow => PlatformAccessibilityError::ArithmeticOverflow,
            Self::InvalidTree => PlatformAccessibilityError::InvalidTree,
            Self::StaleRevision { expected, actual } => {
                PlatformAccessibilityError::StaleRevision { expected, actual }
            }
            Self::TextRequestTooLarge { actual, limit } => {
                PlatformAccessibilityError::TextResponseTooLarge { actual, limit }
            }
            Self::Text(_) => PlatformAccessibilityError::TextMappingFailed,
            Self::Transport(error) => error,
        }
    }
}

impl fmt::Display for AccessibilityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AllocationFailed => formatter.write_str("accessibility allocation failed"),
            Self::ArithmeticOverflow => formatter.write_str("accessibility arithmetic overflow"),
            Self::InvalidTree => formatter.write_str("accessibility tree is inconsistent"),
            Self::StaleRevision { expected, actual } => write!(
                formatter,
                "stale accessibility revision {actual:?}; expected {expected:?}"
            ),
            Self::TextRequestTooLarge { actual, limit } => write!(
                formatter,
                "accessibility text request {actual} bytes exceeds limit {limit}"
            ),
            Self::Text(error) => write!(formatter, "accessibility text mapping failed: {error}"),
            Self::Transport(error) => write!(formatter, "accessibility transport failed: {error}"),
        }
    }
}

impl Error for AccessibilityError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Text(error) => Some(error),
            Self::Transport(error) => Some(error),
            Self::AllocationFailed
            | Self::ArithmeticOverflow
            | Self::InvalidTree
            | Self::StaleRevision { .. }
            | Self::TextRequestTooLarge { .. } => None,
        }
    }
}

impl From<TextError> for AccessibilityError {
    fn from(error: TextError) -> Self {
        Self::Text(error)
    }
}

impl From<PlatformAccessibilityError> for AccessibilityError {
    fn from(error: PlatformAccessibilityError) -> Self {
        match error {
            PlatformAccessibilityError::AllocationFailed => Self::AllocationFailed,
            PlatformAccessibilityError::ArithmeticOverflow => Self::ArithmeticOverflow,
            PlatformAccessibilityError::InvalidTree => Self::InvalidTree,
            PlatformAccessibilityError::StaleRevision { expected, actual } => {
                Self::StaleRevision { expected, actual }
            }
            PlatformAccessibilityError::TextResponseTooLarge { actual, limit } => {
                Self::TextRequestTooLarge { actual, limit }
            }
            PlatformAccessibilityError::ActionTargetMissing(id) => {
                Self::Transport(PlatformAccessibilityError::ActionTargetMissing(id))
            }
            PlatformAccessibilityError::ActionDisabled(id) => {
                Self::Transport(PlatformAccessibilityError::ActionDisabled(id))
            }
            other => Self::Transport(other),
        }
    }
}

pub(super) fn revision(app: &StudioApp) -> AccessibilityRevision {
    AccessibilityRevision::new(app.runtime_document_revision, app.buffer().revision().get())
        .with_semantic(app.accessibility_semantic_revision)
}

pub(super) fn snapshot(app: &StudioApp) -> Result<AccessibilitySnapshot, AccessibilityError> {
    require_revision(app.accessibility_projection_revision, revision(app))?;
    let nodes = build_nodes(app)?;
    let text = app.buffer().snapshot();
    let selection = selection_from_snapshot(app, &text)?;
    let text_len_utf16 = text.appkit_utf16_of_byte(ByteOffset::new(text.len_bytes()))?;
    let transport = transport_snapshot(app, nodes, selection, text_len_utf16, text.line_count())?;
    Ok(AccessibilitySnapshot {
        transport,
        #[cfg(test)]
        text,
    })
}

fn transport_snapshot(
    app: &StudioApp,
    nodes: Vec<AccessibilityNode>,
    selection: AccessibilitySelection,
    text_len_utf16: usize,
    line_count: usize,
) -> Result<PlatformAccessibilitySnapshot, PlatformAccessibilityError> {
    PlatformAccessibilitySnapshot::new(
        app.accessibility_projection_revision,
        WINDOW_NODE,
        nodes,
        selection,
        text_len_utf16,
        line_count,
        app.document.is_dirty(),
    )
    .map(|snapshot| snapshot.with_editor_composition(app.composition.is_some()))
}

fn selection_from_snapshot(
    app: &StudioApp,
    text: &BufferSnapshot,
) -> Result<AccessibilitySelection, AccessibilityError> {
    Ok(AccessibilitySelection::new(
        text.appkit_utf16_of_byte(app.selection.anchor())?,
        text.appkit_utf16_of_byte(app.selection.head())?,
    ))
}

fn text_from_snapshot(
    text: &BufferSnapshot,
    range: AccessibilityTextRange,
) -> Result<String, AccessibilityError> {
    let end_utf16 = range.end_utf16()?;
    let start = text.byte_of_appkit_utf16(range.start_utf16())?;
    let end = text.byte_of_appkit_utf16(end_utf16)?;
    let byte_count = end
        .get()
        .checked_sub(start.get())
        .ok_or(AccessibilityError::ArithmeticOverflow)?;
    if byte_count > MAX_ACCESSIBILITY_TEXT_REQUEST_BYTES {
        return Err(AccessibilityError::TextRequestTooLarge {
            actual: byte_count,
            limit: MAX_ACCESSIBILITY_TEXT_REQUEST_BYTES,
        });
    }
    text.slice(start.get()..end.get()).map_err(Into::into)
}

fn require_revision(
    expected: AccessibilityRevision,
    actual: AccessibilityRevision,
) -> Result<(), AccessibilityError> {
    if expected != actual {
        return Err(AccessibilityError::StaleRevision { expected, actual });
    }
    Ok(())
}

fn line_for_index_from_snapshot(
    text: &BufferSnapshot,
    index_utf16: usize,
) -> Result<usize, AccessibilityError> {
    let byte = text.byte_of_appkit_utf16(index_utf16)?;
    text.line_of_byte(byte).map_err(Into::into)
}

fn range_for_line_from_snapshot(
    text: &BufferSnapshot,
    line: usize,
) -> Result<AccessibilityTextRange, AccessibilityError> {
    let bytes = text.line_byte_range(line)?;
    let start = text.appkit_utf16_of_byte(ByteOffset::new(bytes.start))?;
    let end = text.appkit_utf16_of_byte(ByteOffset::new(bytes.end))?;
    let length = end
        .checked_sub(start)
        .ok_or(AccessibilityError::ArithmeticOverflow)?;
    Ok(AccessibilityTextRange::new(start, length))
}

fn range_for_index_from_snapshot(
    text: &BufferSnapshot,
    index_utf16: usize,
) -> Result<AccessibilityTextRange, AccessibilityError> {
    let byte = text.byte_of_appkit_utf16(index_utf16)?;
    let bytes = text.grapheme_byte_range_at(byte)?;
    let start = text.appkit_utf16_of_byte(ByteOffset::new(bytes.start))?;
    let end = text.appkit_utf16_of_byte(ByteOffset::new(bytes.end))?;
    let length = end
        .checked_sub(start)
        .ok_or(AccessibilityError::ArithmeticOverflow)?;
    Ok(AccessibilityTextRange::new(start, length))
}

pub(super) fn respond(
    app: &mut StudioApp,
    request: &AccessibilityRequest,
) -> (AccessibilityResponse, EventEffect) {
    let observed = revision(app);
    let (result, effect) = match request.operation() {
        AccessibilityOperation::Snapshot => (
            snapshot(app).map(|value| AccessibilityPayload::Snapshot(value.into_transport())),
            EventEffect::default(),
        ),
        AccessibilityOperation::Text { revision, range } => (
            require_revision(*revision, observed).and_then(|()| {
                let text = text_from_snapshot(&app.buffer().snapshot(), *range)?;
                Ok(AccessibilityPayload::Text(AccessibilityText::new(text)?))
            }),
            EventEffect::default(),
        ),
        AccessibilityOperation::Selection { revision } => (
            require_revision(*revision, observed).and_then(|()| {
                let text = app.buffer().snapshot();
                selection_from_snapshot(app, &text).map(AccessibilityPayload::Selection)
            }),
            EventEffect::default(),
        ),
        AccessibilityOperation::LineForIndex {
            revision,
            index_utf16,
        } => (
            require_revision(*revision, observed).and_then(|()| {
                line_for_index_from_snapshot(&app.buffer().snapshot(), *index_utf16)
                    .map(AccessibilityPayload::Line)
            }),
            EventEffect::default(),
        ),
        AccessibilityOperation::RangeForLine { revision, line } => (
            require_revision(*revision, observed).and_then(|()| {
                range_for_line_from_snapshot(&app.buffer().snapshot(), *line)
                    .map(AccessibilityPayload::Range)
            }),
            EventEffect::default(),
        ),
        AccessibilityOperation::RangeForIndex {
            revision,
            index_utf16,
        } => (
            require_revision(*revision, observed).and_then(|()| {
                range_for_index_from_snapshot(&app.buffer().snapshot(), *index_utf16)
                    .map(AccessibilityPayload::Range)
            }),
            EventEffect::default(),
        ),
        AccessibilityOperation::FirstRectForRange {
            revision: expected,
            range,
            marked_text,
        } => (
            require_revision(*expected, observed).and_then(|()| {
                require_revision(app.accessibility_projection_revision, observed)?;
                text_geometry(app, *range, *marked_text)
            }),
            EventEffect::default(),
        ),
        AccessibilityOperation::IndexForPoint {
            revision: expected,
            point,
        } => (
            require_revision(*expected, observed).and_then(|()| {
                require_revision(app.accessibility_projection_revision, observed)?;
                text_index_at_point(app, *point).map(AccessibilityPayload::Index)
            }),
            EventEffect::default(),
        ),
        AccessibilityOperation::Action(_) if app.workspace_edits.is_publication_pending() => (
            Ok(AccessibilityPayload::Action(
                AccessibilityActionResult::Unchanged,
            )),
            EventEffect::default(),
        ),
        AccessibilityOperation::Action(action) => match apply_action(app, *action) {
            Ok(effect) => {
                let result = if effect.visual_changed {
                    AccessibilityActionResult::Applied
                } else {
                    AccessibilityActionResult::Unchanged
                };
                (Ok(AccessibilityPayload::Action(result)), effect)
            }
            Err(error) => (Err(error), EventEffect::default()),
        },
    };
    (finish_response(request, observed, result), effect)
}

fn geometry_unavailable() -> AccessibilityError {
    PlatformAccessibilityError::InvalidBounds.into()
}

#[cfg(all(test, target_os = "macos", target_arch = "aarch64"))]
#[allow(
    clippy::float_cmp,
    reason = "zero-width carets and exact logical line spacing are protocol requirements"
)]
mod native_text_geometry_tests {
    use super::*;
    use crate::{FONT_FAMILY, LINE_HEIGHT, StudioDocument};
    use alpine_core::Size;
    use alpine_platform_macos::AccessibilityRequestId;
    use alpine_scene::SceneRevision;

    fn app(text: &str) -> Result<StudioApp, Box<dyn Error>> {
        let mut system = alpine_text_layout::CoreTextSystem::new();
        system.register_font(FONT_FAMILY, "Menlo-Regular")?;
        let mut app = StudioApp::from_document(system, StudioDocument::scratch(text), None)?;
        let _ = app.scene(
            SceneRevision::new(1),
            Size::new(600.0, 240.0).ok_or("viewport")?,
        );
        Ok(app)
    }

    fn geometry(
        app: &mut StudioApp,
        start: usize,
        length: usize,
    ) -> Result<(AccessibilityBounds, AccessibilityTextRange), Box<dyn Error>> {
        let request = AccessibilityRequest::first_rect_for_range(
            AccessibilityRequestId::new(99),
            revision(app),
            AccessibilityTextRange::new(start, length),
            false,
        )?;
        let (response, effect) = respond(app, &request);
        assert!(!effect.visual_changed);
        match response.result() {
            Ok(AccessibilityPayload::TextGeometry { bounds, range }) => Ok((*bounds, *range)),
            other => Err(format!("geometry: {other:?}").into()),
        }
    }

    #[test]
    fn native_text_geometry_carets_unicode_lines_and_hits() -> Result<(), Box<dyn Error>> {
        let mut app = app("ab😀e\u{301}\txy\nsecond")?;
        let (start, _) = geometry(&mut app, 0, 0)?;
        let (after_emoji, _) = geometry(&mut app, 4, 0)?;
        assert_eq!(start.width(), 0.0);
        assert_eq!(after_emoji.width(), 0.0);
        assert!(after_emoji.x() > start.x());
        let (selection, actual) = geometry(&mut app, 0, 15)?;
        assert_eq!(actual, AccessibilityTextRange::new(0, 10));
        assert!(selection.width() > 0.0);
        assert_eq!(selection.height(), LINE_HEIGHT);
        let (next, _) = geometry(&mut app, 10, 0)?;
        assert_eq!(next.y() - start.y(), LINE_HEIGHT);
        let point = AccessibilityBounds::new(start.x() + 1.0, start.y() + 5.0, 0.0, 0.0)?;
        assert_eq!(text_index_at_point(&mut app, point)?, 0);
        assert!(
            text_index_at_point(
                &mut app,
                AccessibilityBounds::new(580.0, start.y() + 5.0, 0.0, 0.0)?
            )
            .is_err()
        );
        assert!(text_geometry(&mut app, AccessibilityTextRange::new(3, 0), false).is_err());
        assert!(
            text_geometry(&mut app, AccessibilityTextRange::new(usize::MAX, 1), false).is_err()
        );
        Ok(())
    }

    #[test]
    fn native_character_hits_contain_the_final_cluster() -> Result<(), Box<dyn Error>> {
        for text in ["aZ", "a😀"] {
            let mut app = app(text)?;
            let units = text.encode_utf16().count();
            let (left, _) = geometry(&mut app, 1, 0)?;
            let (right, _) = geometry(&mut app, units, 0)?;
            for fraction in [0.25, 0.75] {
                let point = AccessibilityBounds::new(
                    left.x() + (right.x() - left.x()) * fraction,
                    left.y() + 5.0,
                    0.0,
                    0.0,
                )?;
                assert_eq!(text_index_at_point(&mut app, point)?, 1);
            }
        }
        Ok(())
    }

    #[test]
    fn native_text_geometry_revision_scroll_and_marked_projection() -> Result<(), Box<dyn Error>> {
        let mut app = app("prefix target suffix\nsecond\nthird")?;
        let old_revision = revision(&app);
        app.selection = Selection::new(ByteOffset::new(7), ByteOffset::new(13));
        app.composition = Some(crate::Composition {
            replacement: 7..13,
            text: "漢字".into(),
            selected_start_utf16: 1,
            selected_length_utf16: 0,
        });
        app.advance_accessibility_semantic_revision(true);
        let _ = app.scene(SceneRevision::new(2), app.last_viewport);
        let request = AccessibilityRequest::first_rect_for_range(
            AccessibilityRequestId::new(99),
            old_revision,
            AccessibilityTextRange::new(0, 0),
            false,
        )?;
        assert!(respond(&mut app, &request).0.result().is_err());
        let (anchor, _) = geometry(&mut app, 7, 0)?;
        let marked = text_geometry(&mut app, AccessibilityTextRange::new(8, 0), true)?;
        let AccessibilityPayload::TextGeometry { bounds, range } = marked else {
            return Err("marked geometry".into());
        };
        assert_eq!(range, AccessibilityTextRange::new(8, 0));
        assert!(bounds.x() > anchor.x());
        assert_eq!(bounds.width(), 0.0);
        let suffix = text_geometry(&mut app, AccessibilityTextRange::new(9, 1), true)?;
        assert!(
            matches!(suffix, AccessibilityPayload::TextGeometry { range, .. } if range == AccessibilityTextRange::new(9, 1))
        );
        app.scroll_y = LINE_HEIGHT * 2.0;
        assert!(text_geometry(&mut app, AccessibilityTextRange::new(0, 0), false).is_err());
        Ok(())
    }

    #[test]
    fn composed_scene_geometry_and_hits_match_committed_text() -> Result<(), Box<dyn Error>> {
        for (source, replacement, mark) in [
            ("prefix old suffix\nnext", 7..10, "漢😀"),
            ("before\none\ntwo\nafter", 9..13, "x\ny"),
            ("ab old suffix", 3..6, "e\u{301}\tאבג"),
            ("a\rb", 2..2, "\n"),
        ] {
            let mut projected = app(source)?;
            projected.selection = Selection::new(
                ByteOffset::new(replacement.start),
                ByteOffset::new(replacement.end),
            );
            let before_revision = projected.buffer().revision();
            let before_history = projected.buffer().history_snapshot();
            projected.handle_ime(&alpine_platform_macos::ImeEvent::Started);
            projected.handle_ime(&alpine_platform_macos::ImeEvent::Updated {
                text: mark.into(),
                selected_start_utf16: u32::try_from(mark.encode_utf16().count())?,
                selected_length_utf16: 0,
            });
            let scene = projected.try_scene(SceneRevision::new(2), projected.last_viewport)?;
            let mut expected = source.to_owned();
            expected.replace_range(replacement.clone(), mark);
            let mut committed = app(&expected)?;
            let expected_scene =
                committed.try_scene(SceneRevision::new(2), committed.last_viewport)?;
            // Compare actual submitted glyph destination rectangles. Extra source
            // glyphs or an unshifted suffix fail even if the text query is right.
            assert_eq!(
                scene
                    .glyphs()
                    .iter()
                    .map(|glyph| glyph.bounds())
                    .collect::<Vec<_>>(),
                expected_scene
                    .glyphs()
                    .iter()
                    .map(|glyph| glyph.bounds())
                    .collect::<Vec<_>>(),
                "{source:?} -> {mark:?}"
            );
            assert_eq!(projected.buffer().snapshot().text(), source);
            assert_eq!(projected.buffer().revision(), before_revision);
            assert_eq!(projected.buffer().history_snapshot(), before_history);
            for (byte, _) in expected
                .char_indices()
                .chain(std::iter::once((expected.len(), '\0')))
            {
                let index = expected[..byte].encode_utf16().count();
                let actual =
                    text_geometry(&mut projected, AccessibilityTextRange::new(index, 0), true)?;
                let expected_geometry =
                    text_geometry(&mut committed, AccessibilityTextRange::new(index, 0), false)?;
                assert_eq!(actual, expected_geometry, "index {index} of {expected:?}");
            }
            let mark_start = source[..replacement.start].encode_utf16().count();
            let caret_index = mark_start + mark.encode_utf16().count();
            let AccessibilityPayload::TextGeometry { bounds, .. } = text_geometry(
                &mut projected,
                AccessibilityTextRange::new(caret_index, 0),
                true,
            )?
            else {
                return Err("caret geometry".into());
            };
            let caret = projected
                .caret_bounds(
                    &projected.buffer().snapshot(),
                    &projected.rendered_lines.clone(),
                    projected.active_pane_bounds()?.origin().x(),
                )?
                .ok_or("caret")?;
            assert_eq!(caret.origin().x(), bounds.x());
            assert_eq!(caret.origin().y(), bounds.y());
            let pane = committed.active_pane_bounds()?;
            for line in &committed.rendered_lines.clone() {
                for glyph in line.layout.glyphs() {
                    let x = pane.origin().x() + glyph.x() + glyph.advance() * 0.5;
                    let point =
                        AccessibilityBounds::new(x, line.top + LINE_HEIGHT * 0.5, 0.0, 0.0)?;
                    if let Ok(expected_index) = text_index_at_point(&mut committed, point) {
                        assert_eq!(text_index_at_point(&mut projected, point)?, expected_index);
                    }
                }
            }
            projected.cancel_composition();
            let cancelled = projected.try_scene(SceneRevision::new(3), projected.last_viewport)?;
            let mut original = app(source)?;
            let original = original.try_scene(SceneRevision::new(3), original.last_viewport)?;
            assert_eq!(
                cancelled
                    .glyphs()
                    .iter()
                    .map(|glyph| glyph.bounds())
                    .collect::<Vec<_>>(),
                original
                    .glyphs()
                    .iter()
                    .map(|glyph| glyph.bounds())
                    .collect::<Vec<_>>()
            );
        }
        Ok(())
    }

    #[test]
    fn composition_admission_and_cancel_preserve_a_usable_view() -> Result<(), Box<dyn Error>> {
        use alpine_platform_macos::ImeEvent;
        let mut app = app("one line")?;
        let mark = "x\n".repeat(40);
        app.handle_ime(&ImeEvent::Started);
        app.handle_ime(&ImeEvent::Updated {
            text: mark.clone().into(),
            selected_start_utf16: u32::try_from(mark.len())?,
            selected_length_utf16: 0,
        });
        assert!(app.scroll_y > 0.0);
        app.try_scene(SceneRevision::new(2), app.last_viewport)?;
        app.cancel_composition();
        assert_eq!(app.scroll_y, 0.0);
        assert_eq!(app.buffer().snapshot().text(), "one line");
        let limit = alpine_text_layout::DEFAULT_MAX_LINE_BYTES;
        *app.buffer_mut() = crate::Buffer::new(&format!(
            "{}\n{}",
            "a".repeat(limit / 2 + 1),
            "b".repeat(limit / 2 + 1)
        ));
        app.selection = Selection::new(
            ByteOffset::new(limit / 2 + 1),
            ByteOffset::new(limit / 2 + 2),
        );
        app.handle_ime(&ImeEvent::Started);
        assert!(
            app.composition.is_none(),
            "reject an oversized joined line before installing preedit"
        );
        *app.buffer_mut() = crate::Buffer::new(&"x".repeat(limit - 8));
        app.selection = Selection::caret(ByteOffset::new(0));
        app.handle_ime(&ImeEvent::Started);
        app.handle_ime(&ImeEvent::Updated {
            text: "ok".into(),
            selected_start_utf16: 2,
            selected_length_utf16: 0,
        });
        assert!(app.composition.is_some());
        app.handle_ime(&ImeEvent::Updated {
            text: "too much text".into(),
            selected_start_utf16: 13,
            selected_length_utf16: 0,
        });
        assert!(
            app.composition.is_none(),
            "revoke ownership instead of displaying the older mark"
        );
        Ok(())
    }

    #[test]
    fn marked_bidi_selection_does_not_fill_unselected_visual_gaps() -> Result<(), Box<dyn Error>> {
        use alpine_platform_macos::ImeEvent;
        let mut app = app("")?;
        let text = "abc אבג def";
        app.handle_ime(&ImeEvent::Started);
        app.handle_ime(&ImeEvent::Updated {
            text: text.into(),
            selected_start_utf16: 1,
            selected_length_utf16: 5,
        });
        let scene = app.try_scene(SceneRevision::new(2), app.last_viewport)?;
        let origin = app.active_pane_bounds()?.origin();
        let selected: Vec<_> = scene
            .quads()
            .iter()
            .filter(|quad| quad.color() == app.settings.active().theme.selection)
            .map(|quad| quad.bounds())
            .collect();
        assert_eq!(
            selected.len(),
            2,
            "selection spans two separated visual runs"
        );
        for glyph in app.rendered_lines[0].layout.glyphs() {
            if glyph.advance() <= 0.0 {
                continue;
            }
            let midpoint = origin.x() + glyph.x() + glyph.advance() * 0.5;
            let highlighted = selected.iter().any(|rect| {
                midpoint >= rect.origin().x() && midpoint < rect.origin().x() + rect.size().width()
            });
            assert_eq!(
                highlighted,
                (1..6).contains(&glyph.source_utf16()),
                "glyph {glyph:?}"
            );
        }
        Ok(())
    }

    #[test]
    fn composition_crlf_caret_and_committed_suffix_have_visible_geometry()
    -> Result<(), Box<dyn Error>> {
        use alpine_platform_macos::ImeEvent;
        let mut app = app("abc")?;
        app.selection = Selection::caret(ByteOffset::new(1));
        app.handle_ime(&ImeEvent::Started);
        app.handle_ime(&ImeEvent::Updated {
            text: "a\r\nb".into(),
            selected_start_utf16: 2,
            selected_length_utf16: 0,
        });
        app.try_scene(SceneRevision::new(2), app.last_viewport)?;
        let source_suffix = text_geometry(&mut app, AccessibilityTextRange::new(1, 1), false)?;
        let projected_suffix = text_geometry(&mut app, AccessibilityTextRange::new(5, 1), true)?;
        let (
            AccessibilityPayload::TextGeometry {
                bounds: source,
                range,
            },
            AccessibilityPayload::TextGeometry {
                bounds: projected, ..
            },
        ) = (source_suffix, projected_suffix)
        else {
            return Err("suffix geometry".into());
        };
        assert_eq!(source, projected);
        assert_eq!(range, AccessibilityTextRange::new(1, 1));
        let caret = app
            .caret_bounds(
                &app.buffer().snapshot(),
                &app.rendered_lines.clone(),
                app.active_pane_bounds()?.origin().x(),
            )?
            .ok_or("caret")?;
        let AccessibilityPayload::TextGeometry { bounds, .. } =
            text_geometry(&mut app, AccessibilityTextRange::new(3, 0), true)?
        else {
            return Err("caret geometry".into());
        };
        assert_eq!(caret.origin().x(), bounds.x());
        assert_eq!(caret.origin().y(), bounds.y());
        Ok(())
    }
}

fn text_geometry(
    app: &mut StudioApp,
    requested: AccessibilityTextRange,
    marked_text: bool,
) -> Result<AccessibilityPayload, AccessibilityError> {
    let source = app.buffer().snapshot();
    let projection = app
        .composition
        .as_ref()
        .map(|composition| crate::composition::Projection::new(source.clone(), composition))
        .transpose()
        .map_err(|_| geometry_unavailable())?;
    let mut range = requested;
    if let Some(projection) = &projection {
        if !marked_text {
            let start = projection
                .source_to_display(range.start_utf16(), range.length_utf16() > 0)
                .ok_or_else(geometry_unavailable)?;
            let end = projection
                .source_to_display(range.end_utf16()?, false)
                .ok_or_else(geometry_unavailable)?;
            // Committed text hidden by the preedit has no visible rectangle.
            let mark = projection.mark();
            if start < mark.end && end > mark.start && range.length_utf16() != 0 {
                return Err(geometry_unavailable());
            }
            range = AccessibilityTextRange::new(
                start,
                end.checked_sub(start).ok_or_else(geometry_unavailable)?,
            );
        }
        projection
            .line_at_utf16(range.end_utf16()?)
            .map_err(|_| geometry_unavailable())?;
        let line = projection
            .line_at_utf16(range.start_utf16())
            .map_err(|_| geometry_unavailable())?;
        let mut result = line_text_geometry(
            app,
            line.snapshot,
            line.local,
            line.display,
            line.base_utf16,
            range,
        )?;
        if !marked_text && let AccessibilityPayload::TextGeometry { range, .. } = &mut result {
            *range = AccessibilityTextRange::new(requested.start_utf16(), range.length_utf16());
        }
        return Ok(result);
    }
    source.byte_of_appkit_utf16(range.end_utf16()?)?;
    let byte = source.byte_of_appkit_utf16(range.start_utf16())?;
    let line = source.line_of_byte(byte)?;
    let base = source.appkit_utf16_of_byte(ByteOffset::new(source.line_byte_range(line)?.start))?;
    line_text_geometry(app, &source, line, line, base, range)
}

fn line_text_geometry(
    app: &mut StudioApp,
    snapshot: &BufferSnapshot,
    local_line: usize,
    display_line: usize,
    base_utf16: usize,
    range: AccessibilityTextRange,
) -> Result<AccessibilityPayload, AccessibilityError> {
    let line_bytes = snapshot.line_byte_range(local_line)?;
    if line_bytes.len() > alpine_text_layout::DEFAULT_MAX_LINE_BYTES {
        return Err(geometry_unavailable());
    }
    let pane = app
        .active_pane_bounds()
        .map_err(|_| geometry_unavailable())?;
    let top =
        pane.origin().y() + super::usize_as_f32(display_line) * super::LINE_HEIGHT - app.scroll_y;
    if top + super::LINE_HEIGHT <= pane.origin().y()
        || top >= pane.origin().y() + pane.size().height()
    {
        return Err(geometry_unavailable());
    }
    let text = snapshot.slice(line_bytes)?;
    let line_units = text.encode_utf16().count();
    let actual_end_utf16 = range.end_utf16()?.min(base_utf16 + line_units);
    let content = text.trim_end_matches(['\r', '\n']);
    let units = content.encode_utf16().count();
    let start = range
        .start_utf16()
        .checked_sub(base_utf16)
        .ok_or_else(geometry_unavailable)?;
    let end = actual_end_utf16 - base_utf16;
    let start_u32 = u32::try_from(start).map_err(|_| geometry_unavailable())?;
    let end_u32 = u32::try_from(end).map_err(|_| geometry_unavailable())?;
    super::byte_at_utf16(&text, start_u32).ok_or_else(geometry_unavailable)?;
    super::byte_at_utf16(&text, end_u32).ok_or_else(geometry_unavailable)?;
    let start = start.min(units);
    let end = end.min(units);
    let font = app.resolved_font().map_err(|_| geometry_unavailable())?;
    let x0 = app
        .text_system
        .caret_offset(content, font, start)
        .map_err(|_| geometry_unavailable())?;
    let x1 = app
        .text_system
        .caret_offset(content, font, end)
        .map_err(|_| geometry_unavailable())?;
    let mut left = x0.min(x1);
    let mut right = x0.max(x1);
    if start != end {
        // A logical range may cover discontiguous visual runs. Include their
        // glyph advances rather than assuming monotonically increasing indices.
        let layout = app
            .text_system
            .shape(content, font)
            .map_err(|_| geometry_unavailable())?;
        for glyph in layout.glyphs() {
            let index =
                usize::try_from(glyph.source_utf16()).map_err(|_| geometry_unavailable())?;
            if index >= start && index < end {
                left = left.min(glyph.x());
                right = right.max(glyph.x() + glyph.advance());
            }
        }
    }
    let actual =
        AccessibilityTextRange::new(range.start_utf16(), actual_end_utf16 - range.start_utf16());
    clipped_geometry(
        app,
        actual,
        pane.origin().x() + left,
        top,
        right - left,
        super::LINE_HEIGHT,
    )
}

fn clipped_geometry(
    app: &StudioApp,
    range: AccessibilityTextRange,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
) -> Result<AccessibilityPayload, AccessibilityError> {
    let pane = app
        .active_pane_bounds()
        .map_err(|_| geometry_unavailable())?;
    let left = x.max(pane.origin().x());
    let top = y.max(pane.origin().y());
    let right = (x + width).min(pane.origin().x() + pane.size().width());
    let bottom = (y + height).min(pane.origin().y() + pane.size().height());
    if right < left || bottom <= top {
        return Err(geometry_unavailable());
    }
    Ok(AccessibilityPayload::TextGeometry {
        range,
        bounds: AccessibilityBounds::new(left, top, right - left, bottom - top)?,
    })
}

fn text_index_at_point(
    app: &mut StudioApp,
    point: AccessibilityBounds,
) -> Result<usize, AccessibilityError> {
    let pane = app
        .active_pane_bounds()
        .map_err(|_| geometry_unavailable())?;
    if point.x() < pane.origin().x()
        || point.x() >= pane.origin().x() + pane.size().width()
        || point.y() < pane.origin().y()
        || point.y() >= pane.origin().y() + pane.size().height()
    {
        return Err(geometry_unavailable());
    }
    let line = super::floor_f32_to_usize(
        (point.y() - pane.origin().y() + app.scroll_y) / super::LINE_HEIGHT,
    )
    .ok_or_else(geometry_unavailable)?;
    let snapshot = app.buffer().snapshot();
    let projection = app
        .composition
        .as_ref()
        .map(|composition| crate::composition::Projection::new(snapshot.clone(), composition))
        .transpose()
        .map_err(|_| geometry_unavailable())?;
    let projected_line = projection
        .as_ref()
        .map(|projection| projection.line(line))
        .transpose()
        .map_err(|_| geometry_unavailable())?;
    let (snapshot, local) = projected_line
        .as_ref()
        .map_or((&snapshot, line), |line| (line.snapshot, line.local));
    let bytes = snapshot.line_byte_range(local)?;
    if bytes.len() > alpine_text_layout::DEFAULT_MAX_LINE_BYTES {
        return Err(geometry_unavailable());
    }
    let base = if let Some(line) = &projected_line {
        line.base_utf16
    } else {
        snapshot.appkit_utf16_of_byte(ByteOffset::new(bytes.start))?
    };
    let text = snapshot.slice(bytes)?;
    let font = app.resolved_font().map_err(|_| geometry_unavailable())?;
    let local = app
        .text_system
        .index_at_x(
            text.trim_end_matches(['\r', '\n']),
            font,
            point.x() - pane.origin().x(),
        )
        .map_err(|_| geometry_unavailable())?
        .ok_or_else(geometry_unavailable)?;
    base.checked_add(local)
        .ok_or(AccessibilityError::ArithmeticOverflow)
}

fn finish_response(
    request: &AccessibilityRequest,
    observed: AccessibilityRevision,
    result: Result<AccessibilityPayload, AccessibilityError>,
) -> AccessibilityResponse {
    match result {
        Ok(payload) => AccessibilityResponse::success(request, observed, payload)
            .unwrap_or_else(|error| AccessibilityResponse::failure(request, observed, error)),
        Err(error) => AccessibilityResponse::failure(request, observed, error.into_transport()),
    }
}

fn build_nodes(app: &StudioApp) -> Result<Vec<AccessibilityNode>, AccessibilityError> {
    let overlays = [
        app.file_tree.is_visible(),
        app.find.is_open(),
        app.quick_open.is_open(),
        app.project_search.is_open(),
        app.command_palette.is_open(),
        app.rust_diagnostics
            .completion_is_open(app.language_identity()),
        app.rust_diagnostics
            .navigation_is_open(app.language_identity()),
        app.rust_diagnostics
            .symbols_are_open(app.language_identity()),
        app.workspace_edits.is_open(),
    ];
    let node_count = required_node_count(app.tabs.len(), overlays, app.local_status.is_some())?;
    let focus_owner = focus_owner(app);
    let mut nodes = Vec::new();
    nodes
        .try_reserve_exact(node_count)
        .map_err(|_| AccessibilityError::AllocationFailed)?;
    nodes.push(window_node(app)?);
    nodes.push(tab_list_node(app)?);
    push_tabs(app, &mut nodes)?;
    let active_name = app
        .tabs
        .label(app.tabs.active_index())
        .ok_or(AccessibilityError::InvalidTree)?;
    let editor = editor_node(app, active_name, focus_owner == Some(EDITOR_NODE))?;
    nodes.push(editor);
    push_overlays(app, focus_owner, &mut nodes)?;
    push_file_rows(app, &mut nodes)?;
    push_command_rows(app, &mut nodes)?;
    push_diagnostics(app, &mut nodes)?;
    if let Some(status) = app.local_status.as_ref() {
        nodes.push(status_node(app, status.message())?);
    }
    let focused_count = nodes.iter().filter(|value| value.is_focused()).count();
    validate_tree_shape(nodes.len(), nodes.len(), focused_count, app.focused)?;
    Ok(nodes)
}

fn required_node_count(
    tab_count: usize,
    overlays: [bool; 9],
    has_status: bool,
) -> Result<usize, AccessibilityError> {
    let overlay_count = overlays
        .into_iter()
        .try_fold(0_usize, |count, present| {
            count.checked_add(usize::from(present))
        })
        .ok_or(AccessibilityError::ArithmeticOverflow)?;
    let node_count = 3_usize
        .checked_add(tab_count)
        .and_then(|count| count.checked_add(overlay_count))
        .and_then(|count| count.checked_add(usize::from(has_status)))
        .ok_or(AccessibilityError::ArithmeticOverflow)?;
    if node_count > MAX_ACCESSIBILITY_NODES {
        return Err(AccessibilityError::InvalidTree);
    }
    Ok(node_count)
}

fn validate_tree_shape(
    actual_nodes: usize,
    expected_nodes: usize,
    focused_nodes: usize,
    app_focused: bool,
) -> Result<(), AccessibilityError> {
    if actual_nodes != expected_nodes || focused_nodes != usize::from(app_focused) {
        return Err(AccessibilityError::InvalidTree);
    }
    Ok(())
}

fn focus_owner(app: &StudioApp) -> Option<AccessibilityNodeId> {
    if !app.focused {
        None
    } else if app.command_palette.is_open() {
        Some(COMMAND_PALETTE_NODE)
    } else if app.workspace_edits.is_open() {
        Some(WORKSPACE_EDIT_NODE)
    } else if app
        .rust_diagnostics
        .symbols_are_open(app.language_identity())
    {
        Some(SYMBOL_NODE)
    } else if app
        .rust_diagnostics
        .navigation_is_open(app.language_identity())
    {
        Some(NAVIGATION_NODE)
    } else if app
        .rust_diagnostics
        .completion_is_open(app.language_identity())
    {
        Some(COMPLETION_NODE)
    } else if app.project_search.is_open() {
        Some(PROJECT_SEARCH_NODE)
    } else if app.quick_open.is_open() {
        Some(QUICK_OPEN_NODE)
    } else if app.find.is_open() {
        Some(FIND_NODE)
    } else if app.file_tree.is_focused() {
        Some(FILE_TREE_NODE)
    } else {
        Some(EDITOR_NODE)
    }
}

fn push_tabs(
    app: &StudioApp,
    nodes: &mut Vec<AccessibilityNode>,
) -> Result<(), AccessibilityError> {
    for index in 0..app.tabs.len() {
        let tab = app
            .tabs
            .id_at(index)
            .ok_or(AccessibilityError::InvalidTree)?;
        let id = AccessibilityNodeId::new(
            TAB_NODE_BASE
                .checked_add(tab.0)
                .ok_or(AccessibilityError::ArithmeticOverflow)?,
        );
        let label = app
            .tabs
            .label(index)
            .ok_or(AccessibilityError::InvalidTree)?;
        let tab = tab_node(app, id, label, index, index == app.tabs.active_index())?;
        nodes.push(tab);
    }
    Ok(())
}

fn push_overlays(
    app: &StudioApp,
    focus_owner: Option<AccessibilityNodeId>,
    nodes: &mut Vec<AccessibilityNode>,
) -> Result<(), AccessibilityError> {
    let overlays = [
        (
            app.file_tree.is_visible(),
            FILE_TREE_NODE,
            AccessibilityRole::FileTree,
            "Files",
        ),
        (
            app.find.is_open(),
            FIND_NODE,
            AccessibilityRole::SearchField,
            "Find in document",
        ),
        (
            app.quick_open.is_open(),
            QUICK_OPEN_NODE,
            AccessibilityRole::Dialog,
            "Quick open",
        ),
        (
            app.project_search.is_open(),
            PROJECT_SEARCH_NODE,
            AccessibilityRole::Dialog,
            "Project search",
        ),
        (
            app.command_palette.is_open(),
            COMMAND_PALETTE_NODE,
            AccessibilityRole::Dialog,
            "Command palette",
        ),
    ];
    for (present, id, role, name) in overlays {
        let focused = focus_owner == Some(id);
        push_conditional_node(app, nodes, present, id, role, name, focused)?;
    }
    if let Some(label) = app
        .rust_diagnostics
        .completion_accessibility_label(app.language_identity())
    {
        let focused = focus_owner == Some(COMPLETION_NODE);
        nodes.push(completion_node(app, label, focused)?);
    }
    if let Some(label) = app
        .rust_diagnostics
        .navigation_accessibility_label(app.language_identity())
    {
        let focused = focus_owner == Some(NAVIGATION_NODE);
        let activate = app
            .rust_diagnostics
            .navigation_has_target(app.language_identity());
        nodes.push(navigation_node(app, label, focused, activate)?);
    }
    if let Some(label) = app
        .rust_diagnostics
        .symbol_accessibility_label(app.language_identity())
    {
        let focused = focus_owner == Some(SYMBOL_NODE);
        let activate = app
            .rust_diagnostics
            .selected_symbol_location(app.language_identity())
            .is_some();
        nodes.push(symbol_node(app, label, focused, activate)?);
    }
    if let Some(label) = app.workspace_edits.accessibility_label() {
        let focused = focus_owner == Some(WORKSPACE_EDIT_NODE);
        nodes.push(workspace_edit_node(app, label, focused)?);
    }
    Ok(())
}

pub(super) fn apply_action(
    app: &mut StudioApp,
    action: AccessibilityAction,
) -> Result<EventEffect, AccessibilityError> {
    let actual = revision(app);
    match action {
        AccessibilityAction::SetSelection {
            revision: expected,
            selection,
        } => {
            if expected != actual {
                return Err(AccessibilityError::StaleRevision { expected, actual });
            }
            let text = app.buffer().snapshot();
            let anchor = text.byte_of_appkit_utf16(selection.anchor_utf16())?;
            let head = text.byte_of_appkit_utf16(selection.head_utf16())?;
            Ok(app.set_selection(Selection::new(anchor, head)))
        }
        AccessibilityAction::Activate {
            revision: expected,
            node: target,
        } => {
            require_revision(expected, actual)?;
            let current = snapshot(app)?;
            let node = current
                .nodes()
                .iter()
                .find(|node| node.id() == target)
                .ok_or(PlatformAccessibilityError::ActionTargetMissing(target))?;
            if !node.supports_activate() || !node.is_enabled() {
                return Err(PlatformAccessibilityError::ActionDisabled(target).into());
            }
            activate_node(app, target, node.parent())
        }
    }
}

fn activate_node(
    app: &mut StudioApp,
    target: AccessibilityNodeId,
    parent: Option<AccessibilityNodeId>,
) -> Result<EventEffect, AccessibilityError> {
    if target == NAVIGATION_NODE {
        return Ok(app.apply_selected_navigation());
    }
    if target == SYMBOL_NODE {
        return Ok(app.apply_selected_symbol());
    }
    match parent {
        Some(TAB_LIST_NODE) => {
            for index in 0..app.tabs.len() {
                if app
                    .tabs
                    .id_at(index)
                    .is_some_and(|tab| TAB_NODE_BASE.checked_add(tab.0) == Some(target.get()))
                {
                    let focus = app
                        .file_tree
                        .unfocus()
                        .then(EventEffect::visual)
                        .unwrap_or_default();
                    return Ok(focus.merge(
                        app.activate_document_tab(index)
                            .unwrap_or_else(|error| app.record_workspace_error(&error)),
                    ));
                }
            }
        }
        Some(FILE_TREE_NODE) => {
            let index = target
                .get()
                .checked_sub(FILE_ROW_NODE_BASE)
                .and_then(|value| usize::try_from(value).ok())
                .ok_or(AccessibilityError::ArithmeticOverflow)?;
            return Ok(match app.file_tree.activate_row(index) {
                Ok(action) => app.apply_file_tree_action(action),
                Err(error) => app.record_file_tree_error(&error),
            });
        }
        Some(COMMAND_PALETTE_NODE) => {
            let rows = app
                .command_palette
                .visible_commands()
                .map_err(|_| AccessibilityError::InvalidTree)?;
            if let Some(row) = rows
                .into_iter()
                .find(|row| command_node_id(row.command) == target)
            {
                let context = app.command_context();
                return Ok(match app.command_palette.execute(row.command, context) {
                    Ok(command) => EventEffect::visual().merge(app.dispatch_command(command)),
                    Err(error) => app.record_command_palette_error(&error),
                });
            }
        }
        Some(EDITOR_NODE) => return activate_diagnostic(app, target),
        None | Some(_) => {}
    }
    Err(PlatformAccessibilityError::ActionTargetMissing(target).into())
}

#[allow(
    clippy::too_many_arguments,
    reason = "semantic axes and bounded action state remain explicit evidence"
)]
fn node(
    id: AccessibilityNodeId,
    parent: Option<AccessibilityNodeId>,
    role: AccessibilityRole,
    name: Arc<str>,
    focused: bool,
    selected: bool,
    announces: bool,
    bounds: AccessibilityBounds,
    activate: Option<bool>,
) -> Result<AccessibilityNode, AccessibilityError> {
    let node = AccessibilityNode::new(id, parent, role, name, focused, selected, announces)
        .map_err(AccessibilityError::from)?
        .with_bounds(bounds);
    Ok(match activate {
        Some(enabled) => node.with_activate(enabled),
        None => node,
    })
}

fn bounds(
    x: f32,
    y: f32,
    width: f32,
    height: f32,
) -> Result<AccessibilityBounds, AccessibilityError> {
    AccessibilityBounds::new(x.max(0.0), y.max(0.0), width.max(0.0), height.max(0.0))
        .map_err(Into::into)
}

fn tab_list_bounds(
    viewport_width: f32,
    sidebar: f32,
) -> Result<AccessibilityBounds, AccessibilityError> {
    bounds(
        sidebar,
        0.0,
        (viewport_width - sidebar).max(0.0),
        super::TAB_BAR_HEIGHT,
    )
}

fn tab_bounds(
    viewport_width: f32,
    sidebar: f32,
    index: usize,
    scroll_x: f32,
) -> Result<AccessibilityBounds, AccessibilityError> {
    let left = sidebar + super::usize_as_f32(index) * super::TAB_WIDTH - scroll_x;
    let clipped_left = left.max(sidebar).min(viewport_width);
    let clipped_right = (left + super::TAB_WIDTH).max(sidebar).min(viewport_width);
    bounds(
        clipped_left,
        0.0,
        (clipped_right - clipped_left).max(0.0),
        super::TAB_BAR_HEIGHT,
    )
}

fn status_bounds(
    viewport_width: f32,
    viewport_height: f32,
) -> Result<AccessibilityBounds, AccessibilityError> {
    bounds(
        0.0,
        (viewport_height - super::TREE_ROW_HEIGHT).max(0.0),
        viewport_width,
        super::TREE_ROW_HEIGHT,
    )
}

fn overlay_bounds(
    viewport_width: f32,
    viewport_height: f32,
    sidebar: f32,
    id: AccessibilityNodeId,
) -> Result<AccessibilityBounds, AccessibilityError> {
    let width = match id {
        FIND_NODE => super::FIND_BAR_WIDTH,
        PROJECT_SEARCH_NODE => super::PROJECT_SEARCH_WIDTH,
        FILE_TREE_NODE => sidebar,
        _ => super::QUICK_OPEN_WIDTH,
    }
    .min(viewport_width);
    let is_file_tree = id == FILE_TREE_NODE;
    let height = if is_file_tree {
        viewport_height
    } else {
        (viewport_height - super::TAB_BAR_HEIGHT).min(360.0)
    };
    let x = if is_file_tree {
        0.0
    } else {
        (viewport_width - width) * 0.5
    };
    let y = if is_file_tree {
        0.0
    } else {
        super::TAB_BAR_HEIGHT + super::CONTENT_INSET
    };
    bounds(x, y, width, height)
}

const fn should_project_file_rows(visible: bool, active: bool) -> bool {
    visible && active
}

fn first_visible_file_row(scroll_y: f32) -> usize {
    super::floor_f32_to_usize(scroll_y / super::TREE_ROW_HEIGHT).unwrap_or(0)
}

fn file_row_bounds(
    viewport_height: f32,
    sidebar: f32,
    row_index: usize,
    scroll_y: f32,
) -> Result<AccessibilityBounds, AccessibilityError> {
    let top =
        super::CONTENT_INSET + super::usize_as_f32(row_index) * super::TREE_ROW_HEIGHT - scroll_y;
    let clipped_top = top.max(0.0).min(viewport_height);
    let clipped_bottom = (top + super::TREE_ROW_HEIGHT).max(0.0).min(viewport_height);
    bounds(
        0.0,
        clipped_top,
        sidebar,
        (clipped_bottom - clipped_top).max(0.0),
    )
}

fn command_row_bounds(
    viewport_width: f32,
    visible_index: usize,
) -> Result<AccessibilityBounds, AccessibilityError> {
    let width = super::COMMAND_PALETTE_WIDTH.min(viewport_width);
    let left = (viewport_width - width) * 0.5;
    let first_top =
        super::TAB_BAR_HEIGHT + super::CONTENT_INSET + super::COMMAND_PALETTE_QUERY_HEIGHT;
    let top = first_top + super::usize_as_f32(visible_index) * super::COMMAND_PALETTE_ROW_HEIGHT;
    bounds(left, top, width, super::COMMAND_PALETTE_ROW_HEIGHT)
}

fn window_node(app: &StudioApp) -> Result<AccessibilityNode, AccessibilityError> {
    let width = app.last_viewport.width();
    let height = app.last_viewport.height();
    let node_bounds = bounds(0.0, 0.0, width, height)?;
    node(
        WINDOW_NODE,
        None,
        AccessibilityRole::Window,
        Arc::from("Alpine Studio"),
        false,
        false,
        false,
        node_bounds,
        None,
    )
}

fn tab_list_node(app: &StudioApp) -> Result<AccessibilityNode, AccessibilityError> {
    let left = app.sidebar_width(app.last_viewport);
    let node_bounds = tab_list_bounds(app.last_viewport.width(), left)?;
    node(
        TAB_LIST_NODE,
        Some(WINDOW_NODE),
        AccessibilityRole::TabList,
        Arc::from("Open documents"),
        false,
        false,
        false,
        node_bounds,
        None,
    )
}

fn editor_node(
    app: &StudioApp,
    name: Arc<str>,
    focused: bool,
) -> Result<AccessibilityNode, AccessibilityError> {
    let rect = app
        .active_pane_bounds()
        .map_err(|_| AccessibilityError::InvalidTree)?;
    let x = rect.origin().x();
    let y = rect.origin().y();
    let width = rect.size().width();
    let height = rect.size().height();
    let node_bounds = bounds(x, y, width, height)?;
    node(
        EDITOR_NODE,
        Some(WINDOW_NODE),
        AccessibilityRole::CodeEditor,
        name,
        focused,
        false,
        false,
        node_bounds,
        None,
    )
}

fn tab_node(
    app: &StudioApp,
    id: AccessibilityNodeId,
    name: Arc<str>,
    index: usize,
    selected: bool,
) -> Result<AccessibilityNode, AccessibilityError> {
    let sidebar = app.sidebar_width(app.last_viewport);
    let node_bounds = tab_bounds(app.last_viewport.width(), sidebar, index, app.tab_scroll_x)?;
    node(
        id,
        Some(TAB_LIST_NODE),
        AccessibilityRole::Tab,
        name,
        false,
        selected,
        false,
        node_bounds,
        Some(true),
    )
}

fn status_node(app: &StudioApp, message: &str) -> Result<AccessibilityNode, AccessibilityError> {
    let node_bounds = status_bounds(app.last_viewport.width(), app.last_viewport.height())?;
    node(
        STATUS_NODE,
        Some(WINDOW_NODE),
        AccessibilityRole::Status,
        Arc::from(message),
        false,
        false,
        true,
        node_bounds,
        None,
    )
}

fn overlay_node(
    app: &StudioApp,
    id: AccessibilityNodeId,
    role: AccessibilityRole,
    name: &'static str,
    focused: bool,
) -> Result<AccessibilityNode, AccessibilityError> {
    let viewport = app.last_viewport;
    let sidebar = app.sidebar_width(viewport);
    let node_bounds = overlay_bounds(viewport.width(), viewport.height(), sidebar, id)?;
    node(
        id,
        Some(WINDOW_NODE),
        role,
        Arc::from(name),
        focused,
        false,
        false,
        node_bounds,
        None,
    )
}

fn completion_node(
    app: &StudioApp,
    name: Arc<str>,
    focused: bool,
) -> Result<AccessibilityNode, AccessibilityError> {
    let editor = app
        .active_pane_bounds()
        .map_err(|_| AccessibilityError::InvalidTree)?;
    let x = editor.origin().x();
    let y = editor.origin().y();
    let width = editor.size().width().min(520.0);
    let height = 240.0_f32.min(editor.size().height());
    let node_bounds = bounds(x, y, width, height)?;
    node(
        COMPLETION_NODE,
        Some(WINDOW_NODE),
        AccessibilityRole::Dialog,
        name,
        focused,
        true,
        true,
        node_bounds,
        None,
    )
}

fn navigation_node(
    app: &StudioApp,
    name: Arc<str>,
    focused: bool,
    activate: bool,
) -> Result<AccessibilityNode, AccessibilityError> {
    let editor = app
        .active_pane_bounds()
        .map_err(|_| AccessibilityError::InvalidTree)?;
    let width = editor.size().width().min(520.0);
    let height = 264.0_f32.min(editor.size().height());
    let node_bounds = bounds(editor.origin().x(), editor.origin().y(), width, height)?;
    node(
        NAVIGATION_NODE,
        Some(WINDOW_NODE),
        AccessibilityRole::Dialog,
        name,
        focused,
        true,
        true,
        node_bounds,
        activate.then_some(true),
    )
}

fn symbol_node(
    app: &StudioApp,
    name: Arc<str>,
    focused: bool,
    activate: bool,
) -> Result<AccessibilityNode, AccessibilityError> {
    let editor = app
        .active_pane_bounds()
        .map_err(|_| AccessibilityError::InvalidTree)?;
    let width = editor.size().width().min(520.0);
    let height = 286.0_f32.min(editor.size().height());
    let node_bounds = bounds(editor.origin().x(), editor.origin().y(), width, height)?;
    node(
        SYMBOL_NODE,
        Some(WINDOW_NODE),
        AccessibilityRole::Dialog,
        name,
        focused,
        true,
        true,
        node_bounds,
        activate.then_some(true),
    )
}

fn workspace_edit_node(
    app: &StudioApp,
    name: Arc<str>,
    focused: bool,
) -> Result<AccessibilityNode, AccessibilityError> {
    let editor = app
        .active_pane_bounds()
        .map_err(|_| AccessibilityError::InvalidTree)?;
    let width = editor.size().width().min(520.0);
    let height = 264.0_f32.min(editor.size().height());
    let node_bounds = bounds(editor.origin().x(), editor.origin().y(), width, height)?;
    node(
        WORKSPACE_EDIT_NODE,
        Some(WINDOW_NODE),
        AccessibilityRole::Dialog,
        name,
        focused,
        true,
        true,
        node_bounds,
        None,
    )
}

fn push_conditional_node(
    app: &StudioApp,
    nodes: &mut Vec<AccessibilityNode>,
    present: bool,
    id: AccessibilityNodeId,
    role: AccessibilityRole,
    name: &'static str,
    focused: bool,
) -> Result<(), AccessibilityError> {
    if present {
        nodes.push(overlay_node(app, id, role, name, focused)?);
    }
    Ok(())
}

fn push_file_rows(
    app: &StudioApp,
    nodes: &mut Vec<AccessibilityNode>,
) -> Result<(), AccessibilityError> {
    if !should_project_file_rows(app.file_tree.is_visible(), app.file_tree.is_active()) {
        return Ok(());
    }
    let first = first_visible_file_row(app.workspace_scroll_y);
    let rows = app
        .file_tree
        .visible_rows(first, app.visible_tree_rows(), super::TREE_OVERSCAN_ROWS)
        .map_err(|_| AccessibilityError::InvalidTree)?;
    for row in rows {
        let id = AccessibilityNodeId::new(
            FILE_ROW_NODE_BASE
                .checked_add(
                    u64::try_from(row.index).map_err(|_| AccessibilityError::ArithmeticOverflow)?,
                )
                .ok_or(AccessibilityError::ArithmeticOverflow)?,
        );
        let width = app.sidebar_width(app.last_viewport);
        let viewport_height = app.last_viewport.height();
        let scroll_y = app.workspace_scroll_y;
        let row_bounds = file_row_bounds(viewport_height, width, row.index, scroll_y)?;
        let row_result = node(
            id,
            Some(FILE_TREE_NODE),
            AccessibilityRole::ListItem,
            Arc::clone(&row.path),
            false,
            row.selected,
            false,
            row_bounds,
            Some(true),
        );
        let row_node = row_result?;
        nodes.push(row_node);
    }
    Ok(())
}

fn command_node_id(command: StudioCommand) -> AccessibilityNodeId {
    AccessibilityNodeId::new(COMMAND_ROW_NODE_BASE + u64::from(command as u8))
}

fn push_command_rows(
    app: &StudioApp,
    nodes: &mut Vec<AccessibilityNode>,
) -> Result<(), AccessibilityError> {
    if !app.command_palette.is_open() {
        return Ok(());
    }
    let rows = app
        .command_palette
        .visible_commands()
        .map_err(|_| AccessibilityError::InvalidTree)?;
    for (visible, row) in rows.into_iter().enumerate() {
        let row_bounds = command_row_bounds(app.last_viewport.width(), visible)?;
        let row_result = node(
            command_node_id(row.command),
            Some(COMMAND_PALETTE_NODE),
            AccessibilityRole::ListItem,
            Arc::from(row.title),
            false,
            row.selected,
            false,
            row_bounds,
            Some(true),
        );
        let row_node = row_result?;
        nodes.push(row_node);
    }
    Ok(())
}

fn diagnostic_node_id(
    line: usize,
    ordinal: usize,
) -> Result<AccessibilityNodeId, AccessibilityError> {
    let index = line
        .checked_mul(MAX_VISIBLE_DIAGNOSTIC_MARKERS)
        .and_then(|value| value.checked_add(ordinal))
        .ok_or(AccessibilityError::ArithmeticOverflow)?;
    Ok(AccessibilityNodeId::new(
        DIAGNOSTIC_NODE_BASE
            .checked_add(u64::try_from(index).map_err(|_| AccessibilityError::ArithmeticOverflow)?)
            .ok_or(AccessibilityError::ArithmeticOverflow)?,
    ))
}

fn push_diagnostics(
    app: &StudioApp,
    nodes: &mut Vec<AccessibilityNode>,
) -> Result<(), AccessibilityError> {
    let pane = app
        .active_pane_bounds()
        .map_err(|_| AccessibilityError::InvalidTree)?;
    let mut remaining = MAX_VISIBLE_DIAGNOSTIC_MARKERS;
    let mut rendered_index = 0_usize;
    loop {
        if remaining == 0 {
            break;
        }
        let Some(rendered) = app.rendered_lines.get(rendered_index) else {
            break;
        };
        let mut ordinal = 0_usize;
        let visit = app.rust_diagnostics.for_each_marker(
            app.language_identity(),
            rendered.line,
            remaining,
            |marker| {
                let rect = super::diagnostic_underline_bounds(
                    pane.origin().x(),
                    rendered.baseline,
                    &rendered.layout,
                    marker,
                )
                .map_err(|_| AccessibilityError::InvalidTree)?;
                let severity = marker.severity.map_or("diagnostic".to_owned(), |value| {
                    format!("diagnostic severity {value}")
                });
                let x = rect.origin().x();
                let y = rect.origin().y();
                let width = rect.size().width();
                let height = rect.size().height();
                let marker_bounds = bounds(x, y, width, height)?;
                let marker_result = node(
                    diagnostic_node_id(rendered.line, ordinal)?,
                    Some(EDITOR_NODE),
                    AccessibilityRole::ListItem,
                    Arc::from(format!(
                        "{severity} on line {}",
                        rendered.line.saturating_add(1)
                    )),
                    false,
                    false,
                    false,
                    marker_bounds,
                    Some(true),
                );
                let marker_node = marker_result?;
                nodes.push(marker_node);
                ordinal = ordinal.saturating_add(1);
                Ok::<(), AccessibilityError>(())
            },
        );
        visit?;
        remaining = remaining.saturating_sub(ordinal);
        rendered_index = rendered_index.saturating_add(1);
    }
    Ok(())
}

fn activate_diagnostic(
    app: &mut StudioApp,
    target: AccessibilityNodeId,
) -> Result<EventEffect, AccessibilityError> {
    let encoded = target
        .get()
        .checked_sub(DIAGNOSTIC_NODE_BASE)
        .ok_or(AccessibilityError::ArithmeticOverflow)?;
    let index = usize::try_from(encoded).map_err(|_| AccessibilityError::ArithmeticOverflow)?;
    let line = index / MAX_VISIBLE_DIAGNOSTIC_MARKERS;
    let wanted = index % MAX_VISIBLE_DIAGNOSTIC_MARKERS;
    let mut marker = None;
    let mut ordinal = 0_usize;
    let visit = app.rust_diagnostics.for_each_marker(
        app.language_identity(),
        line,
        wanted.saturating_add(1),
        |candidate| {
            if ordinal == wanted {
                marker = Some(candidate);
            }
            ordinal = ordinal.saturating_add(1);
            Ok::<(), AccessibilityError>(())
        },
    );
    visit?;
    let marker = marker.ok_or(PlatformAccessibilityError::ActionTargetMissing(target))?;
    let text = app.buffer().snapshot();
    let line_range = text.line_byte_range(line)?;
    let line_start_utf16 = text.appkit_utf16_of_byte(ByteOffset::new(line_range.start))?;
    let start_utf16 = line_start_utf16
        .checked_add(marker.start_utf16 as usize)
        .ok_or(AccessibilityError::ArithmeticOverflow)?;
    let end_utf16 = line_start_utf16
        .checked_add(marker.end_utf16.unwrap_or(marker.start_utf16) as usize)
        .ok_or(AccessibilityError::ArithmeticOverflow)?;
    let start = text.byte_of_appkit_utf16(start_utf16)?;
    let end = text.byte_of_appkit_utf16(end_utf16)?;
    let focus = app
        .file_tree
        .unfocus()
        .then(EventEffect::visual)
        .unwrap_or_default();
    Ok(focus.merge(app.set_selection(Selection::new(start, end))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_helper_guards_are_structured() -> Result<(), Box<dyn Error>> {
        let app_result = StudioApp::from_document(
            crate::tests::TestTextSystem,
            crate::StudioDocument::scratch("text"),
            None,
        );
        let mut app = app_result?;
        assert!(matches!(
            bounds(f32::INFINITY, 0.0, 1.0, 1.0),
            Err(AccessibilityError::Transport(
                PlatformAccessibilityError::InvalidBounds
            ))
        ));
        assert!(matches!(
            diagnostic_node_id(usize::MAX, usize::MAX),
            Err(AccessibilityError::ArithmeticOverflow)
        ));
        assert!(matches!(
            activate_node(&mut app, AccessibilityNodeId::new(1), None),
            Err(AccessibilityError::Transport(
                PlatformAccessibilityError::ActionTargetMissing(_)
            ))
        ));
        assert!(matches!(
            activate_node(
                &mut app,
                AccessibilityNodeId::new(FILE_ROW_NODE_BASE),
                Some(FILE_TREE_NODE),
            ),
            Ok(EventEffect {
                visual_changed: true,
                ..
            })
        ));
        assert!(matches!(
            activate_diagnostic(&mut app, AccessibilityNodeId::new(1)),
            Err(AccessibilityError::ArithmeticOverflow)
        ));
        assert!(matches!(
            activate_diagnostic(&mut app, AccessibilityNodeId::new(DIAGNOSTIC_NODE_BASE),),
            Err(AccessibilityError::Transport(
                PlatformAccessibilityError::ActionTargetMissing(_)
            ))
        ));
        let all_commands = crate::commands::CommandContext {
            can_save: true,
            can_close_tab: true,
            can_navigate_back: true,
            can_navigate_forward: true,
            has_workspace: true,
            can_split_right: true,
            can_split_down: true,
            can_close_pane: true,
            can_complete: true,
        };
        assert!(app.command_palette.open(all_commands)?);
        let save = command_node_id(StudioCommand::SaveFile);
        assert!(activate_node(&mut app, save, Some(COMMAND_PALETTE_NODE)).is_ok());
        let absent = AccessibilityNodeId::new(u64::MAX);
        assert!(matches!(
            activate_node(&mut app, absent, Some(COMMAND_PALETTE_NODE)),
            Err(AccessibilityError::Transport(
                PlatformAccessibilityError::ActionTargetMissing(id)
            )) if id == absent
        ));
        Ok(())
    }

    #[test]
    fn node_count_and_tree_shape_boundaries_are_exact() {
        assert_eq!(required_node_count(0, [false; 9], false), Ok(3));
        assert_eq!(
            required_node_count(258, [true; 9], true),
            Ok(MAX_ACCESSIBILITY_NODES)
        );
        assert_eq!(
            required_node_count(259, [true; 9], true),
            Err(AccessibilityError::InvalidTree)
        );
        assert_eq!(
            required_node_count(usize::MAX, [false; 9], false),
            Err(AccessibilityError::ArithmeticOverflow)
        );
        assert_eq!(validate_tree_shape(4, 4, 1, true), Ok(()));
        assert_eq!(
            validate_tree_shape(4, 4, 0, true),
            Err(AccessibilityError::InvalidTree)
        );
    }

    #[test]
    fn diagnostics_preserve_text_and_transport_sources() {
        let text = TextError::InvalidUtf16Boundary { offset: 2 };
        assert!(AccessibilityError::Text(text).source().is_some());
        assert!(
            AccessibilityError::Transport(PlatformAccessibilityError::InvalidTree)
                .source()
                .is_some()
        );
        assert!(AccessibilityError::InvalidTree.source().is_none());
    }

    #[test]
    fn revision_admission_is_exact() {
        let expected = AccessibilityRevision::new(3, 5).with_semantic(7);
        assert_eq!(expected.document(), 3);
        assert_eq!(expected.buffer(), 5);
        assert_eq!(expected.semantic(), 7);
        assert_eq!(require_revision(expected, expected), Ok(()));
        let actual = AccessibilityRevision::new(3, 5).with_semantic(8);
        assert_eq!(
            require_revision(expected, actual),
            Err(AccessibilityError::StaleRevision { expected, actual })
        );
        let actual = AccessibilityRevision::new(3, 6).with_semantic(7);
        assert_eq!(
            require_revision(expected, actual),
            Err(AccessibilityError::StaleRevision { expected, actual })
        );
    }

    #[test]
    fn transport_conversion_and_response_finalization_are_discriminating()
    -> Result<(), Box<dyn Error>> {
        let expected = AccessibilityRevision::new(3, 5);
        let actual = AccessibilityRevision::new(7, 11);
        let text = TextError::InvalidUtf16Boundary { offset: 2 };
        let conversions = [
            (
                AccessibilityError::AllocationFailed,
                PlatformAccessibilityError::AllocationFailed,
            ),
            (
                AccessibilityError::ArithmeticOverflow,
                PlatformAccessibilityError::ArithmeticOverflow,
            ),
            (
                AccessibilityError::InvalidTree,
                PlatformAccessibilityError::InvalidTree,
            ),
            (
                AccessibilityError::StaleRevision { expected, actual },
                PlatformAccessibilityError::StaleRevision { expected, actual },
            ),
            (
                AccessibilityError::TextRequestTooLarge {
                    actual: 65_537,
                    limit: 65_536,
                },
                PlatformAccessibilityError::TextResponseTooLarge {
                    actual: 65_537,
                    limit: 65_536,
                },
            ),
            (
                AccessibilityError::Text(text.clone()),
                PlatformAccessibilityError::TextMappingFailed,
            ),
            (
                AccessibilityError::Transport(PlatformAccessibilityError::RequestMismatch),
                PlatformAccessibilityError::RequestMismatch,
            ),
        ];
        for (local, platform) in conversions {
            assert!(!local.to_string().is_empty());
            assert_eq!(local.into_transport(), platform);
        }

        let reverse = [
            PlatformAccessibilityError::AllocationFailed,
            PlatformAccessibilityError::ArithmeticOverflow,
            PlatformAccessibilityError::InvalidTree,
            PlatformAccessibilityError::StaleRevision { expected, actual },
            PlatformAccessibilityError::TextResponseTooLarge {
                actual: 65_537,
                limit: 65_536,
            },
            PlatformAccessibilityError::RequestMismatch,
        ];
        for platform in reverse {
            assert!(!AccessibilityError::from(platform).to_string().is_empty());
        }

        let request_result =
            AccessibilityRequest::snapshot(alpine_platform_macos::AccessibilityRequestId::new(1));
        let request = request_result?;
        let text_result = AccessibilityText::new("wrong kind");
        let text = text_result?;
        let response = finish_response(&request, actual, Ok(AccessibilityPayload::Text(text)));
        assert_eq!(
            response.result(),
            &Err(PlatformAccessibilityError::RequestMismatch)
        );
        let failure = finish_response(&request, actual, Err(AccessibilityError::InvalidTree));
        assert_eq!(
            failure.result(),
            &Err(PlatformAccessibilityError::InvalidTree)
        );
        Ok(())
    }
}

#[cfg(test)]
mod bounded_mapping_tests {
    use super::*;
    use alpine_text::Buffer;

    #[test]
    fn line_and_grapheme_mappings_cover_unicode_and_line_boundaries()
    -> Result<(), AccessibilityError> {
        let snapshot = Buffer::new("a\r\n😀e\u{301}\n").snapshot();
        assert_eq!(line_for_index_from_snapshot(&snapshot, 0)?, 0);
        assert_eq!(line_for_index_from_snapshot(&snapshot, 2)?, 0);
        assert_eq!(line_for_index_from_snapshot(&snapshot, 3)?, 1);
        assert_eq!(line_for_index_from_snapshot(&snapshot, 8)?, 2);
        assert_eq!(
            range_for_line_from_snapshot(&snapshot, 0)?,
            AccessibilityTextRange::new(0, 3)
        );
        assert_eq!(
            range_for_line_from_snapshot(&snapshot, 1)?,
            AccessibilityTextRange::new(3, 5)
        );
        assert_eq!(
            range_for_line_from_snapshot(&snapshot, 2)?,
            AccessibilityTextRange::new(8, 0)
        );
        assert_eq!(
            range_for_index_from_snapshot(&snapshot, 3)?,
            AccessibilityTextRange::new(3, 2)
        );
        assert_eq!(
            range_for_index_from_snapshot(&snapshot, 5)?,
            AccessibilityTextRange::new(5, 2)
        );
        assert_eq!(
            range_for_index_from_snapshot(&snapshot, 6)?,
            AccessibilityTextRange::new(5, 2)
        );
        assert_eq!(
            range_for_index_from_snapshot(&snapshot, 8)?,
            AccessibilityTextRange::new(8, 0)
        );
        assert!(matches!(
            range_for_index_from_snapshot(&snapshot, 4),
            Err(AccessibilityError::Text(
                TextError::InvalidUtf16Boundary { .. }
            ))
        ));
        assert!(matches!(
            range_for_line_from_snapshot(&snapshot, 3),
            Err(AccessibilityError::Text(TextError::LineOutOfBounds { .. }))
        ));
        Ok(())
    }
}

#[cfg(test)]
mod semantic_geometry_tests {
    use super::*;

    fn values(bounds: AccessibilityBounds) -> (f32, f32, f32, f32) {
        (bounds.x(), bounds.y(), bounds.width(), bounds.height())
    }

    #[test]
    fn semantic_container_geometry_is_exact() -> Result<(), AccessibilityError> {
        let viewport_width = 840.0;
        let viewport_height = 240.0;
        let sidebar = 200.0;

        assert_eq!(
            values(tab_list_bounds(viewport_width, sidebar)?),
            (sidebar, 0.0, 640.0, super::super::TAB_BAR_HEIGHT)
        );

        let tab_index = 2;
        let tab_scroll = 37.0;
        let tab_left =
            sidebar + super::super::usize_as_f32(tab_index) * super::super::TAB_WIDTH - tab_scroll;
        assert_eq!(
            values(tab_bounds(viewport_width, sidebar, tab_index, tab_scroll,)?),
            (
                tab_left,
                0.0,
                super::super::TAB_WIDTH,
                super::super::TAB_BAR_HEIGHT,
            )
        );

        assert_eq!(
            values(status_bounds(viewport_width, viewport_height)?),
            (
                0.0,
                viewport_height - super::super::TREE_ROW_HEIGHT,
                viewport_width,
                super::super::TREE_ROW_HEIGHT,
            )
        );

        let overlay_height = (viewport_height - super::super::TAB_BAR_HEIGHT).min(360.0);
        let overlay_y = super::super::TAB_BAR_HEIGHT + super::super::CONTENT_INSET;
        let find_width = super::super::FIND_BAR_WIDTH.min(viewport_width);
        let find = overlay_bounds(viewport_width, viewport_height, sidebar, FIND_NODE)?;
        assert_eq!(
            values(find),
            (
                (viewport_width - find_width) * 0.5,
                overlay_y,
                find_width,
                overlay_height,
            )
        );
        let search_width = super::super::PROJECT_SEARCH_WIDTH.min(viewport_width);
        let id = PROJECT_SEARCH_NODE;
        let search = overlay_bounds(viewport_width, viewport_height, sidebar, id)?;
        assert_eq!(
            values(search),
            (
                (viewport_width - search_width) * 0.5,
                overlay_y,
                search_width,
                overlay_height,
            )
        );
        let file_tree = overlay_bounds(viewport_width, viewport_height, sidebar, FILE_TREE_NODE)?;
        assert_eq!(values(file_tree), (0.0, 0.0, sidebar, viewport_height));
        let quick_width = super::super::QUICK_OPEN_WIDTH.min(viewport_width);
        let quick = overlay_bounds(viewport_width, viewport_height, sidebar, QUICK_OPEN_NODE)?;
        assert_eq!(
            values(quick),
            (
                (viewport_width - quick_width) * 0.5,
                overlay_y,
                quick_width,
                overlay_height,
            )
        );
        Ok(())
    }

    #[test]
    fn semantic_row_geometry_is_exact() -> Result<(), AccessibilityError> {
        let viewport_width = 840.0;
        let viewport_height = 540.0;
        let sidebar = 200.0;
        let row_index = 3;
        let scroll_y = 50.0;
        let row_top = super::super::CONTENT_INSET
            + super::super::usize_as_f32(row_index) * super::super::TREE_ROW_HEIGHT
            - scroll_y;
        let row = file_row_bounds(viewport_height, sidebar, row_index, scroll_y)?;
        assert_eq!(
            values(row),
            (0.0, row_top, sidebar, super::super::TREE_ROW_HEIGHT)
        );

        let visible_index = 2;
        let command_width = super::super::COMMAND_PALETTE_WIDTH.min(viewport_width);
        let command_top = super::super::TAB_BAR_HEIGHT
            + super::super::CONTENT_INSET
            + super::super::COMMAND_PALETTE_QUERY_HEIGHT
            + super::super::usize_as_f32(visible_index) * super::super::COMMAND_PALETTE_ROW_HEIGHT;
        assert_eq!(
            values(command_row_bounds(viewport_width, visible_index)?),
            (
                (viewport_width - command_width) * 0.5,
                command_top,
                command_width,
                super::super::COMMAND_PALETTE_ROW_HEIGHT,
            )
        );
        Ok(())
    }

    #[test]
    fn row_admission_scroll_and_command_identity_are_exact() {
        assert!(should_project_file_rows(true, true));
        assert!(!should_project_file_rows(true, false));
        assert!(!should_project_file_rows(false, true));
        assert!(!should_project_file_rows(false, false));

        let scroll = super::super::TREE_ROW_HEIGHT * 2.0 + 1.0;
        assert_eq!(first_visible_file_row(scroll), 2);

        let close = StudioCommand::CloseTab;
        assert_eq!(
            command_node_id(close).get(),
            COMMAND_ROW_NODE_BASE + u64::from(close as u8)
        );
    }
}
