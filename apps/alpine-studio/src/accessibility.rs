//! Bounded, revision-synchronized Studio accessibility semantics.

#![expect(
    dead_code,
    reason = "the reviewed native adapter will consume this safe semantic boundary in the next Task #130 slice"
)]

use std::{error::Error, fmt, mem, sync::Arc};

use alpine_text::{BufferSnapshot, ByteOffset, Selection, TextError};

use super::{EventEffect, StudioApp};

pub(crate) const MAX_ACCESSIBILITY_NODES: usize = 270;
pub(crate) const MAX_ACCESSIBILITY_TEXT_REQUEST_BYTES: usize = 65_536;

const WINDOW_NODE: AccessibilityNodeId = AccessibilityNodeId(1);
const TAB_LIST_NODE: AccessibilityNodeId = AccessibilityNodeId(2);
const EDITOR_NODE: AccessibilityNodeId = AccessibilityNodeId(3);
const FILE_TREE_NODE: AccessibilityNodeId = AccessibilityNodeId(4);
const FIND_NODE: AccessibilityNodeId = AccessibilityNodeId(5);
const QUICK_OPEN_NODE: AccessibilityNodeId = AccessibilityNodeId(6);
const PROJECT_SEARCH_NODE: AccessibilityNodeId = AccessibilityNodeId(7);
const COMMAND_PALETTE_NODE: AccessibilityNodeId = AccessibilityNodeId(8);
const STATUS_NODE: AccessibilityNodeId = AccessibilityNodeId(9);
const TAB_NODE_BASE: u64 = 1_024;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct AccessibilityNodeId(u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AccessibilityRole {
    Window,
    TabList,
    Tab,
    CodeEditor,
    FileTree,
    SearchField,
    Dialog,
    Status,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AccessibilityNode {
    id: AccessibilityNodeId,
    parent: Option<AccessibilityNodeId>,
    role: AccessibilityRole,
    name: Arc<str>,
    focused: bool,
    selected: bool,
    announces: bool,
}

impl AccessibilityNode {
    pub(crate) const fn id(&self) -> AccessibilityNodeId {
        self.id
    }

    pub(crate) const fn parent(&self) -> Option<AccessibilityNodeId> {
        self.parent
    }

    pub(crate) const fn role(&self) -> AccessibilityRole {
        self.role
    }

    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) const fn is_focused(&self) -> bool {
        self.focused
    }

    pub(crate) const fn is_selected(&self) -> bool {
        self.selected
    }

    pub(crate) const fn announces(&self) -> bool {
        self.announces
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AccessibilityRevision {
    document: u64,
    buffer: u64,
}

impl AccessibilityRevision {
    const fn new(document: u64, buffer: u64) -> Self {
        Self { document, buffer }
    }

    pub(crate) const fn document(self) -> u64 {
        self.document
    }

    pub(crate) const fn buffer(self) -> u64 {
        self.buffer
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AccessibilityTextRange {
    start_utf16: usize,
    length_utf16: usize,
}

impl AccessibilityTextRange {
    pub(crate) const fn new(start_utf16: usize, length_utf16: usize) -> Self {
        Self {
            start_utf16,
            length_utf16,
        }
    }

    pub(crate) const fn start_utf16(self) -> usize {
        self.start_utf16
    }

    pub(crate) const fn length_utf16(self) -> usize {
        self.length_utf16
    }

    fn end_utf16(self) -> Result<usize, AccessibilityError> {
        self.start_utf16
            .checked_add(self.length_utf16)
            .ok_or(AccessibilityError::ArithmeticOverflow)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AccessibilitySelection {
    anchor_utf16: usize,
    head_utf16: usize,
}

impl AccessibilitySelection {
    pub(crate) const fn anchor_utf16(self) -> usize {
        self.anchor_utf16
    }

    pub(crate) const fn head_utf16(self) -> usize {
        self.head_utf16
    }

    pub(crate) fn range(self) -> AccessibilityTextRange {
        let start = self.anchor_utf16.min(self.head_utf16);
        AccessibilityTextRange::new(start, self.anchor_utf16.abs_diff(self.head_utf16))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AccessibilityReport {
    node_count: usize,
    owned_node_bytes: usize,
    referenced_name_bytes: usize,
    max_nodes: usize,
    max_text_request_bytes: usize,
}

impl AccessibilityReport {
    pub(crate) const fn node_count(self) -> usize {
        self.node_count
    }

    pub(crate) const fn owned_node_bytes(self) -> usize {
        self.owned_node_bytes
    }

    pub(crate) const fn referenced_name_bytes(self) -> usize {
        self.referenced_name_bytes
    }

    pub(crate) const fn max_nodes(self) -> usize {
        self.max_nodes
    }

    pub(crate) const fn max_text_request_bytes(self) -> usize {
        self.max_text_request_bytes
    }
}

#[derive(Clone, Debug)]
pub(crate) struct AccessibilitySnapshot {
    revision: AccessibilityRevision,
    nodes: Vec<AccessibilityNode>,
    text: BufferSnapshot,
    selection: AccessibilitySelection,
    text_len_utf16: usize,
    line_count: usize,
    dirty: bool,
    report: AccessibilityReport,
}

impl AccessibilitySnapshot {
    pub(crate) const fn revision(&self) -> AccessibilityRevision {
        self.revision
    }

    pub(crate) fn nodes(&self) -> &[AccessibilityNode] {
        &self.nodes
    }

    pub(crate) const fn selection(&self) -> AccessibilitySelection {
        self.selection
    }

    pub(crate) const fn text_len_utf16(&self) -> usize {
        self.text_len_utf16
    }

    pub(crate) const fn line_count(&self) -> usize {
        self.line_count
    }

    pub(crate) const fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub(crate) const fn report(&self) -> AccessibilityReport {
        self.report
    }

    pub(crate) fn text(&self, range: AccessibilityTextRange) -> Result<String, AccessibilityError> {
        let end_utf16 = range.end_utf16()?;
        let start = self.text.byte_of_appkit_utf16(range.start_utf16())?;
        let end = self.text.byte_of_appkit_utf16(end_utf16)?;
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
        self.text.slice(start.get()..end.get()).map_err(Into::into)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AccessibilityAction {
    SetSelection {
        revision: AccessibilityRevision,
        anchor_utf16: usize,
        head_utf16: usize,
    },
}

impl AccessibilityAction {
    pub(crate) const fn set_selection(
        revision: AccessibilityRevision,
        anchor_utf16: usize,
        head_utf16: usize,
    ) -> Self {
        Self::SetSelection {
            revision,
            anchor_utf16,
            head_utf16,
        }
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
        }
    }
}

impl Error for AccessibilityError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Text(error) => Some(error),
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

pub(super) fn revision(app: &StudioApp) -> AccessibilityRevision {
    AccessibilityRevision::new(app.runtime_document_revision, app.buffer().revision().get())
}

pub(super) fn snapshot(app: &StudioApp) -> Result<AccessibilitySnapshot, AccessibilityError> {
    let nodes = build_nodes(app)?;
    let node_count = nodes.len();
    let text = app.buffer().snapshot();
    let selection = AccessibilitySelection {
        anchor_utf16: text.appkit_utf16_of_byte(app.selection.anchor())?,
        head_utf16: text.appkit_utf16_of_byte(app.selection.head())?,
    };
    let text_len_utf16 = text.appkit_utf16_of_byte(ByteOffset::new(text.len_bytes()))?;
    let referenced_name_bytes = nodes.iter().try_fold(0_usize, |total, node| {
        total
            .checked_add(node.name.len())
            .ok_or(AccessibilityError::ArithmeticOverflow)
    })?;
    let owned_node_bytes = nodes
        .capacity()
        .checked_mul(mem::size_of::<AccessibilityNode>())
        .ok_or(AccessibilityError::ArithmeticOverflow)?;
    let report = AccessibilityReport {
        node_count,
        owned_node_bytes,
        referenced_name_bytes,
        max_nodes: MAX_ACCESSIBILITY_NODES,
        max_text_request_bytes: MAX_ACCESSIBILITY_TEXT_REQUEST_BYTES,
    };
    Ok(AccessibilitySnapshot {
        revision: revision(app),
        nodes,
        line_count: text.line_count(),
        text,
        selection,
        text_len_utf16,
        dirty: app.document.is_dirty(),
        report,
    })
}

fn build_nodes(app: &StudioApp) -> Result<Vec<AccessibilityNode>, AccessibilityError> {
    let overlays = [
        app.file_tree.is_visible(),
        app.find.is_open(),
        app.quick_open.is_open(),
        app.project_search.is_open(),
        app.command_palette.is_open(),
    ];
    let node_count = required_node_count(app.tabs.len(), overlays, app.local_status.is_some())?;

    let focus_owner = focus_owner(app);
    let mut nodes = Vec::new();
    nodes
        .try_reserve_exact(node_count)
        .map_err(|_| AccessibilityError::AllocationFailed)?;
    nodes.push(node(
        WINDOW_NODE,
        None,
        AccessibilityRole::Window,
        Arc::from("Alpine Studio"),
        false,
        false,
        false,
    ));
    nodes.push(node(
        TAB_LIST_NODE,
        Some(WINDOW_NODE),
        AccessibilityRole::TabList,
        Arc::from("Open documents"),
        false,
        false,
        false,
    ));
    push_tabs(app, &mut nodes)?;
    let active_name = app
        .tabs
        .label(app.tabs.active_index())
        .ok_or(AccessibilityError::InvalidTree)?;
    nodes.push(node(
        EDITOR_NODE,
        Some(WINDOW_NODE),
        AccessibilityRole::CodeEditor,
        active_name,
        focus_owner == Some(EDITOR_NODE),
        false,
        false,
    ));
    push_overlays(app, focus_owner, &mut nodes);
    if let Some(status) = app.local_status.as_ref() {
        nodes.push(node(
            STATUS_NODE,
            Some(WINDOW_NODE),
            AccessibilityRole::Status,
            Arc::from(status.message()),
            false,
            false,
            true,
        ));
    }
    let focused_count = nodes.iter().filter(|node| node.is_focused()).count();
    validate_tree_shape(nodes.len(), node_count, focused_count, app.focused)?;
    Ok(nodes)
}

fn required_node_count(
    tab_count: usize,
    overlays: [bool; 5],
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
        return None;
    }
    if app.command_palette.is_open() {
        Some(COMMAND_PALETTE_NODE)
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
        let id = AccessibilityNodeId(
            TAB_NODE_BASE
                .checked_add(tab.0)
                .ok_or(AccessibilityError::ArithmeticOverflow)?,
        );
        nodes.push(node(
            id,
            Some(TAB_LIST_NODE),
            AccessibilityRole::Tab,
            app.tabs
                .label(index)
                .ok_or(AccessibilityError::InvalidTree)?,
            false,
            index == app.tabs.active_index(),
            false,
        ));
    }
    Ok(())
}

fn push_overlays(
    app: &StudioApp,
    focus_owner: Option<AccessibilityNodeId>,
    nodes: &mut Vec<AccessibilityNode>,
) {
    push_conditional_node(
        nodes,
        app.file_tree.is_visible(),
        FILE_TREE_NODE,
        AccessibilityRole::FileTree,
        "Files",
        focus_owner == Some(FILE_TREE_NODE),
    );
    push_conditional_node(
        nodes,
        app.find.is_open(),
        FIND_NODE,
        AccessibilityRole::SearchField,
        "Find in document",
        focus_owner == Some(FIND_NODE),
    );
    push_conditional_node(
        nodes,
        app.quick_open.is_open(),
        QUICK_OPEN_NODE,
        AccessibilityRole::Dialog,
        "Quick open",
        focus_owner == Some(QUICK_OPEN_NODE),
    );
    push_conditional_node(
        nodes,
        app.project_search.is_open(),
        PROJECT_SEARCH_NODE,
        AccessibilityRole::Dialog,
        "Project search",
        focus_owner == Some(PROJECT_SEARCH_NODE),
    );
    push_conditional_node(
        nodes,
        app.command_palette.is_open(),
        COMMAND_PALETTE_NODE,
        AccessibilityRole::Dialog,
        "Command palette",
        focus_owner == Some(COMMAND_PALETTE_NODE),
    );
}

pub(super) fn apply_action(
    app: &mut StudioApp,
    action: AccessibilityAction,
) -> Result<EventEffect, AccessibilityError> {
    let actual = revision(app);
    match action {
        AccessibilityAction::SetSelection {
            revision: expected,
            anchor_utf16,
            head_utf16,
        } => {
            if expected != actual {
                return Err(AccessibilityError::StaleRevision { expected, actual });
            }
            let text = app.buffer().snapshot();
            let anchor = text.byte_of_appkit_utf16(anchor_utf16)?;
            let head = text.byte_of_appkit_utf16(head_utf16)?;
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
) -> AccessibilityNode {
    AccessibilityNode {
        id,
        parent,
        role,
        name,
        focused,
        selected,
        announces,
    }
}

fn push_conditional_node(
    nodes: &mut Vec<AccessibilityNode>,
    present: bool,
    id: AccessibilityNodeId,
    role: AccessibilityRole,
    name: &'static str,
    focused: bool,
) {
    if present {
        nodes.push(node(
            id,
            Some(WINDOW_NODE),
            role,
            Arc::from(name),
            focused,
            false,
            false,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observers_preserve_non_identity_values_and_exact_flags() {
        let selected = node(
            AccessibilityNodeId(42),
            Some(WINDOW_NODE),
            AccessibilityRole::Tab,
            Arc::from("selected"),
            false,
            true,
            false,
        );
        let announced = node(
            AccessibilityNodeId(43),
            Some(WINDOW_NODE),
            AccessibilityRole::Status,
            Arc::from("announced"),
            false,
            false,
            true,
        );
        assert!(selected.is_selected());
        assert!(!selected.announces());
        assert!(!announced.is_selected());
        assert!(announced.announces());

        let revision = AccessibilityRevision::new(7, 11);
        assert_eq!(revision.document(), 7);
        assert_eq!(revision.buffer(), 11);
        let selection = AccessibilitySelection {
            anchor_utf16: 9,
            head_utf16: 2,
        };
        assert_eq!(selection.anchor_utf16(), 9);
        assert_eq!(selection.head_utf16(), 2);
        assert_eq!(selection.range(), AccessibilityTextRange::new(2, 7));

        let report = AccessibilityReport {
            node_count: 5,
            owned_node_bytes: 640,
            referenced_name_bytes: 37,
            max_nodes: 270,
            max_text_request_bytes: 65_536,
        };
        assert_eq!(report.node_count(), 5);
        assert_eq!(report.owned_node_bytes(), 640);
        assert_eq!(report.referenced_name_bytes(), 37);
    }

    #[test]
    fn node_count_and_tree_shape_boundaries_are_exact() {
        assert_eq!(required_node_count(0, [false; 5], false), Ok(3));
        for overlay in 0..5 {
            let mut overlays = [false; 5];
            overlays[overlay] = true;
            assert_eq!(required_node_count(0, overlays, false), Ok(4));
        }
        assert_eq!(
            required_node_count(261, [true; 5], true),
            Ok(MAX_ACCESSIBILITY_NODES)
        );
        assert_eq!(
            required_node_count(262, [true; 5], true),
            Err(AccessibilityError::InvalidTree)
        );
        assert_eq!(
            required_node_count(usize::MAX, [false; 5], false),
            Err(AccessibilityError::ArithmeticOverflow)
        );
        assert_eq!(validate_tree_shape(4, 4, 1, true), Ok(()));
        assert_eq!(
            validate_tree_shape(3, 4, 1, true),
            Err(AccessibilityError::InvalidTree)
        );
        assert_eq!(
            validate_tree_shape(4, 4, 0, true),
            Err(AccessibilityError::InvalidTree)
        );
    }

    #[test]
    fn diagnostics_and_error_sources_are_exact() {
        let expected = AccessibilityRevision::new(3, 5);
        let actual = AccessibilityRevision::new(7, 11);
        let text = TextError::InvalidUtf16Boundary { offset: 2 };
        let diagnostics = [
            (
                AccessibilityError::AllocationFailed,
                "accessibility allocation failed".to_owned(),
            ),
            (
                AccessibilityError::ArithmeticOverflow,
                "accessibility arithmetic overflow".to_owned(),
            ),
            (
                AccessibilityError::InvalidTree,
                "accessibility tree is inconsistent".to_owned(),
            ),
            (
                AccessibilityError::StaleRevision { expected, actual },
                format!("stale accessibility revision {actual:?}; expected {expected:?}"),
            ),
            (
                AccessibilityError::TextRequestTooLarge {
                    actual: 65_537,
                    limit: 65_536,
                },
                "accessibility text request 65537 bytes exceeds limit 65536".to_owned(),
            ),
            (
                AccessibilityError::Text(text.clone()),
                format!("accessibility text mapping failed: {text}"),
            ),
        ];
        for (error, message) in diagnostics {
            assert_eq!(error.to_string(), message);
        }
        assert!(AccessibilityError::Text(text).source().is_some());
        assert!(AccessibilityError::InvalidTree.source().is_none());
    }
}
