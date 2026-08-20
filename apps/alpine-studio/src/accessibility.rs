//! Bounded, revision-synchronized Studio accessibility semantics.

use std::{error::Error, fmt, sync::Arc};

#[cfg(test)]
pub(crate) use alpine_platform_macos::AccessibilityReport;
pub(crate) use alpine_platform_macos::{
    AccessibilityAction, AccessibilityNode, AccessibilityNodeId, AccessibilityRevision,
    AccessibilityRole, AccessibilitySelection,
    AccessibilitySnapshot as PlatformAccessibilitySnapshot, AccessibilityTextRange,
    MAX_ACCESSIBILITY_NODES,
};
use alpine_platform_macos::{
    AccessibilityActionResult, AccessibilityError as PlatformAccessibilityError,
    AccessibilityOperation, AccessibilityPayload, AccessibilityRequest, AccessibilityResponse,
    AccessibilityText, MAX_ACCESSIBILITY_TEXT_RESPONSE_BYTES,
};
use alpine_text::{BufferSnapshot, ByteOffset, Selection, TextError};

use super::{EventEffect, StudioApp};

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
const TAB_NODE_BASE: u64 = 1_024;

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

    #[cfg(test)]
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
            other => Self::Transport(other),
        }
    }
}

pub(super) fn revision(app: &StudioApp) -> AccessibilityRevision {
    AccessibilityRevision::new(app.runtime_document_revision, app.buffer().revision().get())
}

pub(super) fn snapshot(app: &StudioApp) -> Result<AccessibilitySnapshot, AccessibilityError> {
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
        revision(app),
        WINDOW_NODE,
        nodes,
        selection,
        text_len_utf16,
        line_count,
        app.document.is_dirty(),
    )
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
    let byte = text.byte_of_appkit_utf16(index_utf16)?.get();
    if byte == text.len_bytes() {
        return text
            .line_count()
            .checked_sub(1)
            .ok_or(AccessibilityError::InvalidTree);
    }
    let mut low = 0_usize;
    let mut high = text.line_count();
    while low < high {
        let middle = low + (high - low) / 2;
        let range = text.line_byte_range(middle)?;
        if byte < range.start {
            high = middle;
        } else if byte >= range.end {
            low = middle + 1;
        } else {
            return Ok(middle);
        }
    }
    Err(AccessibilityError::InvalidTree)
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
    ];
    let node_count = required_node_count(app.tabs.len(), overlays, app.local_status.is_some())?;
    let focus_owner = focus_owner(app);
    let mut nodes = Vec::new();
    nodes
        .try_reserve_exact(node_count)
        .map_err(|_| AccessibilityError::AllocationFailed)?;
    nodes.push(window_node()?);
    nodes.push(tab_list_node()?);
    push_tabs(app, &mut nodes)?;
    let active_name = app
        .tabs
        .label(app.tabs.active_index())
        .ok_or(AccessibilityError::InvalidTree)?;
    nodes.push(editor_node(active_name, focus_owner == Some(EDITOR_NODE))?);
    push_overlays(app, focus_owner, &mut nodes)?;
    if let Some(status) = app.local_status.as_ref() {
        nodes.push(status_node(status.message())?);
    }
    let focused_count = nodes.iter().filter(|value| value.is_focused()).count();
    validate_tree_shape(nodes.len(), node_count, focused_count, app.focused)?;
    Ok(nodes)
}

fn required_node_count(
    tab_count: usize,
    overlays: [bool; 6],
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
        nodes.push(tab_node(id, label, index == app.tabs.active_index())?);
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
        push_conditional_node(nodes, present, id, role, name, focused)?;
    }
    if let Some(label) = app
        .rust_diagnostics
        .completion_accessibility_label(app.language_identity())
    {
        let focused = focus_owner == Some(COMPLETION_NODE);
        nodes.push(completion_node(label, focused)?);
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
    }
}

fn node(
    id: AccessibilityNodeId,
    parent: Option<AccessibilityNodeId>,
    role: AccessibilityRole,
    name: Arc<str>,
    focused: bool,
    selected: bool,
    announces: bool,
) -> Result<AccessibilityNode, AccessibilityError> {
    AccessibilityNode::new(id, parent, role, name, focused, selected, announces).map_err(Into::into)
}

fn window_node() -> Result<AccessibilityNode, AccessibilityError> {
    node(
        WINDOW_NODE,
        None,
        AccessibilityRole::Window,
        Arc::from("Alpine Studio"),
        false,
        false,
        false,
    )
}

fn tab_list_node() -> Result<AccessibilityNode, AccessibilityError> {
    node(
        TAB_LIST_NODE,
        Some(WINDOW_NODE),
        AccessibilityRole::TabList,
        Arc::from("Open documents"),
        false,
        false,
        false,
    )
}

fn editor_node(name: Arc<str>, focused: bool) -> Result<AccessibilityNode, AccessibilityError> {
    node(
        EDITOR_NODE,
        Some(WINDOW_NODE),
        AccessibilityRole::CodeEditor,
        name,
        focused,
        false,
        false,
    )
}

fn tab_node(
    id: AccessibilityNodeId,
    name: Arc<str>,
    selected: bool,
) -> Result<AccessibilityNode, AccessibilityError> {
    node(
        id,
        Some(TAB_LIST_NODE),
        AccessibilityRole::Tab,
        name,
        false,
        selected,
        false,
    )
}

fn status_node(message: &str) -> Result<AccessibilityNode, AccessibilityError> {
    node(
        STATUS_NODE,
        Some(WINDOW_NODE),
        AccessibilityRole::Status,
        Arc::from(message),
        false,
        false,
        true,
    )
}

fn overlay_node(
    id: AccessibilityNodeId,
    role: AccessibilityRole,
    name: &'static str,
    focused: bool,
) -> Result<AccessibilityNode, AccessibilityError> {
    node(
        id,
        Some(WINDOW_NODE),
        role,
        Arc::from(name),
        focused,
        false,
        false,
    )
}

fn completion_node(name: Arc<str>, focused: bool) -> Result<AccessibilityNode, AccessibilityError> {
    node(
        COMPLETION_NODE,
        Some(WINDOW_NODE),
        AccessibilityRole::Dialog,
        name,
        focused,
        true,
        true,
    )
}

fn push_conditional_node(
    nodes: &mut Vec<AccessibilityNode>,
    present: bool,
    id: AccessibilityNodeId,
    role: AccessibilityRole,
    name: &'static str,
    focused: bool,
) -> Result<(), AccessibilityError> {
    if present {
        nodes.push(overlay_node(id, role, name, focused)?);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_count_and_tree_shape_boundaries_are_exact() {
        assert_eq!(required_node_count(0, [false; 6], false), Ok(3));
        assert_eq!(
            required_node_count(261, [true; 6], true),
            Ok(MAX_ACCESSIBILITY_NODES)
        );
        assert_eq!(
            required_node_count(262, [true; 6], true),
            Err(AccessibilityError::InvalidTree)
        );
        assert_eq!(
            required_node_count(usize::MAX, [false; 6], false),
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
        let expected = AccessibilityRevision::new(3, 5);
        assert_eq!(require_revision(expected, expected), Ok(()));
        let actual = AccessibilityRevision::new(3, 6);
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
