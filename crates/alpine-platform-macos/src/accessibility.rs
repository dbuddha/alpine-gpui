//! Bounded, handle-free accessibility values shared with the native adapter.

use core::{error::Error, fmt, mem};
use std::sync::Arc;

/// Maximum semantic nodes retained by one accessibility snapshot.
pub const MAX_ACCESSIBILITY_NODES: usize = 271;
/// Maximum UTF-8 bytes retained by one node name.
pub const MAX_ACCESSIBILITY_NODE_NAME_BYTES: usize = 4 * 1024;
/// Maximum aggregate UTF-8 name bytes referenced by one snapshot.
pub const MAX_ACCESSIBILITY_NAME_BYTES: usize = 256 * 1024;
/// Maximum UTF-8 bytes returned by one text request.
pub const MAX_ACCESSIBILITY_TEXT_RESPONSE_BYTES: usize = 64 * 1024;
/// Maximum finite view-local coordinate or extent accepted by accessibility.
pub const MAX_ACCESSIBILITY_COORDINATE: f32 = 1_048_576.0;

/// Stable semantic identity independent of native accessibility objects.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AccessibilityNodeId(u64);

impl AccessibilityNodeId {
    /// Creates an identity. Zero is rejected when a node is constructed.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }
    /// Returns the stable integer identity.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// One validated finite rectangle in Alpine view-local coordinates.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AccessibilityBounds {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

impl AccessibilityBounds {
    /// Creates a bounded rectangle without retaining a platform geometry object.
    ///
    /// # Errors
    /// Rejects non-finite, negative, or out-of-domain coordinates and extents.
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Result<Self, AccessibilityError> {
        let values = [x, y, width, height];
        if values.into_iter().any(|value| {
            !value.is_finite() || !(0.0..=MAX_ACCESSIBILITY_COORDINATE).contains(&value)
        }) || x + width > MAX_ACCESSIBILITY_COORDINATE
            || y + height > MAX_ACCESSIBILITY_COORDINATE
        {
            return Err(AccessibilityError::InvalidBounds);
        }
        Ok(Self {
            x: normalized_bits(x),
            y: normalized_bits(y),
            width: normalized_bits(width),
            height: normalized_bits(height),
        })
    }

    /// Returns the view-local x coordinate.
    #[must_use]
    pub const fn x(self) -> f32 {
        f32::from_bits(self.x)
    }
    /// Returns the view-local y coordinate.
    #[must_use]
    pub const fn y(self) -> f32 {
        f32::from_bits(self.y)
    }
    /// Returns the finite width.
    #[must_use]
    pub const fn width(self) -> f32 {
        f32::from_bits(self.width)
    }
    /// Returns the finite height.
    #[must_use]
    pub const fn height(self) -> f32 {
        f32::from_bits(self.height)
    }
}

const fn normalized_bits(value: f32) -> u32 {
    if value == 0.0 {
        0.0_f32.to_bits()
    } else {
        value.to_bits()
    }
}

/// Semantic role vocabulary supported by Alpine Studio v1.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccessibilityRole {
    /// Top-level application window.
    Window,
    /// Container of document tabs.
    TabList,
    /// One selectable document tab.
    Tab,
    /// Navigable and editable code text.
    CodeEditor,
    /// Local workspace file hierarchy.
    FileTree,
    /// Search input.
    SearchField,
    /// Modal or transient command surface.
    Dialog,
    /// Status or announcement value.
    Status,
    /// One actionable row inside a bounded list or outline.
    ListItem,
}

/// One bounded semantic element without a native handle.
#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent semantic state and action capability must remain explicit"
)]
pub struct AccessibilityNode {
    id: AccessibilityNodeId,
    parent: Option<AccessibilityNodeId>,
    role: AccessibilityRole,
    name: Arc<str>,
    focused: bool,
    selected: bool,
    announces: bool,
    bounds: AccessibilityBounds,
    enabled: bool,
    supports_activate: bool,
}

impl AccessibilityNode {
    /// Creates one bounded node.
    ///
    /// # Errors
    /// Rejects zero identity and names beyond the per-node byte ceiling.
    pub fn new(
        id: AccessibilityNodeId,
        parent: Option<AccessibilityNodeId>,
        role: AccessibilityRole,
        name: Arc<str>,
        focused: bool,
        selected: bool,
        announces: bool,
    ) -> Result<Self, AccessibilityError> {
        if id.get() == 0 || parent.is_some_and(|parent| parent.get() == 0) {
            return Err(AccessibilityError::InvalidNodeId);
        }
        if name.len() > MAX_ACCESSIBILITY_NODE_NAME_BYTES {
            return Err(AccessibilityError::NodeNameTooLarge {
                actual: name.len(),
                limit: MAX_ACCESSIBILITY_NODE_NAME_BYTES,
            });
        }
        Ok(Self {
            id,
            parent,
            role,
            name,
            focused,
            selected,
            announces,
            bounds: AccessibilityBounds::new(0.0, 0.0, 0.0, 0.0)?,
            enabled: true,
            supports_activate: false,
        })
    }

    /// Replaces the node's view-local rectangle after validation.
    #[must_use]
    pub fn with_bounds(mut self, bounds: AccessibilityBounds) -> Self {
        self.bounds = bounds;
        self
    }

    /// Marks whether this node admits the closed `Activate` action.
    #[must_use]
    pub const fn with_activate(mut self, enabled: bool) -> Self {
        self.supports_activate = true;
        self.enabled = enabled;
        self
    }

    /// Returns the stable semantic identity.
    #[must_use]
    pub const fn id(&self) -> AccessibilityNodeId {
        self.id
    }
    /// Returns the semantic parent identity.
    #[must_use]
    pub const fn parent(&self) -> Option<AccessibilityNodeId> {
        self.parent
    }
    /// Returns the semantic role.
    #[must_use]
    pub const fn role(&self) -> AccessibilityRole {
        self.role
    }
    /// Returns the bounded label or value.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Retains the bounded semantic name for a post-borrow native payload.
    #[must_use]
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub(crate) fn retained_name(&self) -> Arc<str> {
        Arc::clone(&self.name)
    }
    /// Returns whether this node owns native focus.
    #[must_use]
    pub const fn is_focused(&self) -> bool {
        self.focused
    }
    /// Returns whether this node is selected.
    #[must_use]
    pub const fn is_selected(&self) -> bool {
        self.selected
    }
    /// Returns whether changes to this node should be announced.
    #[must_use]
    pub const fn announces(&self) -> bool {
        self.announces
    }
    /// Returns the finite view-local semantic rectangle.
    #[must_use]
    pub const fn bounds(&self) -> AccessibilityBounds {
        self.bounds
    }
    /// Returns whether the current action target is enabled.
    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }
    /// Returns whether this node admits the closed `Activate` action.
    #[must_use]
    pub const fn supports_activate(&self) -> bool {
        self.supports_activate
    }
}

/// Exact Studio document, buffer, and non-text semantic identity observed by accessibility.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AccessibilityRevision {
    document: u64,
    buffer: u64,
    semantic: u64,
}

impl AccessibilityRevision {
    /// Creates an exact semantic revision.
    #[must_use]
    pub const fn new(document: u64, buffer: u64) -> Self {
        Self {
            document,
            buffer,
            semantic: 0,
        }
    }
    /// Adds the exact revision of non-text semantic state.
    #[must_use]
    pub const fn with_semantic(self, semantic: u64) -> Self {
        Self { semantic, ..self }
    }
    /// Returns the Studio document identity.
    #[must_use]
    pub const fn document(self) -> u64 {
        self.document
    }
    /// Returns the text-buffer revision.
    #[must_use]
    pub const fn buffer(self) -> u64 {
        self.buffer
    }
    /// Returns the non-text semantic revision.
    #[must_use]
    pub const fn semantic(self) -> u64 {
        self.semantic
    }
}

/// Global `AppKit` UTF-16 range.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AccessibilityTextRange {
    start_utf16: usize,
    length_utf16: usize,
}

impl AccessibilityTextRange {
    /// Creates a range whose checked end is validated before use.
    #[must_use]
    pub const fn new(start_utf16: usize, length_utf16: usize) -> Self {
        Self {
            start_utf16,
            length_utf16,
        }
    }
    /// Returns the first UTF-16 code-unit offset.
    #[must_use]
    pub const fn start_utf16(self) -> usize {
        self.start_utf16
    }
    /// Returns the UTF-16 code-unit length.
    #[must_use]
    pub const fn length_utf16(self) -> usize {
        self.length_utf16
    }
    /// Returns the checked exclusive end.
    ///
    /// # Errors
    /// Rejects arithmetic overflow.
    pub fn end_utf16(self) -> Result<usize, AccessibilityError> {
        self.start_utf16
            .checked_add(self.length_utf16)
            .ok_or(AccessibilityError::ArithmeticOverflow)
    }
}

/// Directional selection in global `AppKit` UTF-16 coordinates.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AccessibilitySelection {
    anchor_utf16: usize,
    head_utf16: usize,
}

impl AccessibilitySelection {
    /// Creates one directional selection.
    #[must_use]
    pub const fn new(anchor_utf16: usize, head_utf16: usize) -> Self {
        Self {
            anchor_utf16,
            head_utf16,
        }
    }
    /// Returns the directional anchor.
    #[must_use]
    pub const fn anchor_utf16(self) -> usize {
        self.anchor_utf16
    }
    /// Returns the directional head.
    #[must_use]
    pub const fn head_utf16(self) -> usize {
        self.head_utf16
    }
    /// Returns the normalized selected range.
    #[must_use]
    pub fn range(self) -> AccessibilityTextRange {
        let start = self.anchor_utf16.min(self.head_utf16);
        AccessibilityTextRange::new(start, self.anchor_utf16.abs_diff(self.head_utf16))
    }
}

/// Exact retained ownership for one semantic snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AccessibilityReport {
    node_count: usize,
    owned_node_bytes: usize,
    referenced_name_bytes: usize,
}

impl AccessibilityReport {
    /// Returns retained semantic nodes.
    #[must_use]
    pub const fn node_count(self) -> usize {
        self.node_count
    }
    /// Returns bytes owned by the compact node array.
    #[must_use]
    pub const fn owned_node_bytes(self) -> usize {
        self.owned_node_bytes
    }
    /// Returns aggregate referenced UTF-8 name bytes.
    #[must_use]
    pub const fn referenced_name_bytes(self) -> usize {
        self.referenced_name_bytes
    }
    /// Returns the node ceiling.
    #[must_use]
    pub const fn max_nodes(self) -> usize {
        MAX_ACCESSIBILITY_NODES
    }
    /// Returns the per-response text ceiling.
    #[must_use]
    pub const fn max_text_request_bytes(self) -> usize {
        MAX_ACCESSIBILITY_TEXT_RESPONSE_BYTES
    }
}

/// One validated semantic tree and current editor metadata, never document text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccessibilitySnapshot {
    revision: AccessibilityRevision,
    root: AccessibilityNodeId,
    nodes: Box<[AccessibilityNode]>,
    selection: AccessibilitySelection,
    text_len_utf16: usize,
    line_count: usize,
    dirty: bool,
    report: AccessibilityReport,
}

impl AccessibilitySnapshot {
    /// Validates and owns one bounded semantic tree.
    ///
    /// # Errors
    /// Rejects invalid roots, duplicate identities, cycles, missing parents,
    /// invalid selections, multiple focused nodes, and every byte ceiling.
    #[allow(
        clippy::too_many_arguments,
        reason = "all values are independent semantic evidence"
    )]
    pub fn new(
        revision: AccessibilityRevision,
        root: AccessibilityNodeId,
        nodes: Vec<AccessibilityNode>,
        selection: AccessibilitySelection,
        text_len_utf16: usize,
        line_count: usize,
        dirty: bool,
    ) -> Result<Self, AccessibilityError> {
        validate_tree(root, &nodes)?;
        if selection.anchor_utf16() > text_len_utf16 || selection.head_utf16() > text_len_utf16 {
            return Err(AccessibilityError::InvalidSelection { text_len_utf16 });
        }
        let referenced_name_bytes = nodes.iter().try_fold(0_usize, |total, node| {
            total
                .checked_add(node.name().len())
                .ok_or(AccessibilityError::ArithmeticOverflow)
        })?;
        if referenced_name_bytes > MAX_ACCESSIBILITY_NAME_BYTES {
            return Err(AccessibilityError::NameBudgetExceeded {
                actual: referenced_name_bytes,
                limit: MAX_ACCESSIBILITY_NAME_BYTES,
            });
        }
        let owned_node_bytes = nodes
            .len()
            .checked_mul(mem::size_of::<AccessibilityNode>())
            .ok_or(AccessibilityError::ArithmeticOverflow)?;
        let report = AccessibilityReport {
            node_count: nodes.len(),
            owned_node_bytes,
            referenced_name_bytes,
        };
        Ok(Self {
            revision,
            root,
            nodes: nodes.into_boxed_slice(),
            selection,
            text_len_utf16,
            line_count,
            dirty,
            report,
        })
    }

    /// Returns exact document and buffer identity.
    #[must_use]
    pub const fn revision(&self) -> AccessibilityRevision {
        self.revision
    }
    /// Returns the root semantic identity.
    #[must_use]
    pub const fn root(&self) -> AccessibilityNodeId {
        self.root
    }
    /// Returns the bounded semantic nodes.
    #[must_use]
    pub fn nodes(&self) -> &[AccessibilityNode] {
        &self.nodes
    }
    /// Returns the current directional selection.
    #[must_use]
    pub const fn selection(&self) -> AccessibilitySelection {
        self.selection
    }
    /// Returns the complete text length without retaining text.
    #[must_use]
    pub const fn text_len_utf16(&self) -> usize {
        self.text_len_utf16
    }
    /// Returns the logical line count.
    #[must_use]
    pub const fn line_count(&self) -> usize {
        self.line_count
    }
    /// Returns whether the current document is dirty.
    #[must_use]
    pub const fn is_dirty(&self) -> bool {
        self.dirty
    }
    /// Returns exact retained ownership.
    #[must_use]
    pub const fn report(&self) -> AccessibilityReport {
        self.report
    }
}

fn validate_tree(
    root: AccessibilityNodeId,
    nodes: &[AccessibilityNode],
) -> Result<(), AccessibilityError> {
    if root.get() == 0 || nodes.is_empty() {
        return Err(AccessibilityError::InvalidTree);
    }
    if nodes.len() > MAX_ACCESSIBILITY_NODES {
        return Err(AccessibilityError::TooManyNodes {
            actual: nodes.len(),
            limit: MAX_ACCESSIBILITY_NODES,
        });
    }
    for (index, node) in nodes.iter().enumerate() {
        if nodes[..index].iter().any(|prior| prior.id() == node.id()) {
            return Err(AccessibilityError::DuplicateNodeId(node.id()));
        }
    }
    if nodes.iter().filter(|node| node.id() == root).count() != 1
        || nodes.iter().filter(|node| node.is_focused()).count() > 1
    {
        return Err(AccessibilityError::InvalidTree);
    }
    for node in nodes {
        if node.id() == root {
            if node.parent().is_some() {
                return Err(AccessibilityError::InvalidTree);
            }
            continue;
        }
        let mut parent = node.parent().ok_or(AccessibilityError::InvalidTree)?;
        let mut reached_root = false;
        for _ in 0..nodes.len() {
            if parent == root {
                reached_root = true;
                break;
            }
            let parent_node = nodes
                .iter()
                .find(|candidate| candidate.id() == parent)
                .ok_or(AccessibilityError::InvalidTree)?;
            parent = parent_node
                .parent()
                .ok_or(AccessibilityError::InvalidTree)?;
        }
        if !reached_root {
            return Err(AccessibilityError::InvalidTree);
        }
    }
    Ok(())
}

/// Monotonic identity of one native pull request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AccessibilityRequestId(u64);

impl AccessibilityRequestId {
    /// Creates an identity validated by request construction.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }
    /// Returns the integer identity.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Revision-checked assistive action.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccessibilityAction {
    /// Replaces the current editor selection.
    SetSelection {
        /// Revision observed by assistive technology.
        revision: AccessibilityRevision,
        /// Requested directional selection.
        selection: AccessibilitySelection,
    },
    /// Activates one current bounded semantic node.
    Activate {
        /// Revision observed with the target node.
        revision: AccessibilityRevision,
        /// Exact semantic identity observed by assistive technology.
        node: AccessibilityNodeId,
    },
}

impl AccessibilityAction {
    /// Creates a revision-checked selection action.
    #[must_use]
    pub const fn set_selection(
        revision: AccessibilityRevision,
        anchor_utf16: usize,
        head_utf16: usize,
    ) -> Self {
        Self::SetSelection {
            revision,
            selection: AccessibilitySelection::new(anchor_utf16, head_utf16),
        }
    }
    /// Creates one revision-checked activation.
    #[must_use]
    pub const fn activate(revision: AccessibilityRevision, node: AccessibilityNodeId) -> Self {
        Self::Activate { revision, node }
    }
    /// Returns the exact observed revision.
    #[must_use]
    pub const fn revision(self) -> AccessibilityRevision {
        match self {
            Self::SetSelection { revision, .. } | Self::Activate { revision, .. } => revision,
        }
    }
}

/// Operation carried by one accessibility request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AccessibilityOperation {
    /// Pull the current bounded semantic snapshot.
    Snapshot,
    /// Pull one bounded text range for an exact revision.
    Text {
        /// Exact revision required by the request.
        revision: AccessibilityRevision,
        /// Global UTF-16 range to materialize.
        range: AccessibilityTextRange,
    },
    /// Pull the directional selection for an exact revision.
    Selection {
        /// Exact revision required by the request.
        revision: AccessibilityRevision,
    },
    /// Map one global UTF-16 index to its zero-based logical line.
    LineForIndex {
        /// Exact revision required by the request.
        revision: AccessibilityRevision,
        /// Global `AppKit` UTF-16 index to map.
        index_utf16: usize,
    },
    /// Map one zero-based logical line to its global UTF-16 range.
    RangeForLine {
        /// Exact revision required by the request.
        revision: AccessibilityRevision,
        /// Zero-based logical line to map.
        line: usize,
    },
    /// Map one global UTF-16 index to its containing grapheme range.
    RangeForIndex {
        /// Exact revision required by the request.
        revision: AccessibilityRevision,
        /// Global `AppKit` UTF-16 index to map.
        index_utf16: usize,
    },
    /// Apply one revision-checked action.
    Action(AccessibilityAction),
}

/// Stable operation identity repeated by every response.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccessibilityRequestKind {
    /// Semantic snapshot request.
    Snapshot,
    /// Bounded text request.
    Text,
    /// Directional selection request.
    Selection,
    /// UTF-16 index to logical-line mapping request.
    LineForIndex,
    /// Logical-line to UTF-16 range mapping request.
    RangeForLine,
    /// UTF-16 index to grapheme-range mapping request.
    RangeForIndex,
    /// Revision-checked action request.
    Action,
}

/// One validated synchronous accessibility request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccessibilityRequest {
    id: AccessibilityRequestId,
    operation: AccessibilityOperation,
}

impl AccessibilityRequest {
    fn new(
        id: AccessibilityRequestId,
        operation: AccessibilityOperation,
    ) -> Result<Self, AccessibilityError> {
        if id.get() == 0 {
            return Err(AccessibilityError::InvalidRequestId);
        }
        if let AccessibilityOperation::Text { range, .. } = operation {
            let _end = range.end_utf16()?;
            if range.length_utf16() > MAX_ACCESSIBILITY_TEXT_RESPONSE_BYTES {
                return Err(AccessibilityError::TextResponseTooLarge {
                    actual: range.length_utf16(),
                    limit: MAX_ACCESSIBILITY_TEXT_RESPONSE_BYTES,
                });
            }
        }
        Ok(Self { id, operation })
    }
    /// Creates a semantic snapshot pull.
    ///
    /// # Errors
    ///
    /// Rejects zero request identity.
    pub fn snapshot(id: AccessibilityRequestId) -> Result<Self, AccessibilityError> {
        Self::new(id, AccessibilityOperation::Snapshot)
    }
    /// Creates a bounded text pull.
    ///
    /// # Errors
    ///
    /// Rejects zero request identity, overflowing ranges, and ranges whose
    /// UTF-16 length already exceeds the text response ceiling.
    pub fn text(
        id: AccessibilityRequestId,
        revision: AccessibilityRevision,
        range: AccessibilityTextRange,
    ) -> Result<Self, AccessibilityError> {
        Self::new(id, AccessibilityOperation::Text { revision, range })
    }
    /// Creates a selection pull.
    ///
    /// # Errors
    ///
    /// Rejects zero request identity.
    pub fn selection(
        id: AccessibilityRequestId,
        revision: AccessibilityRevision,
    ) -> Result<Self, AccessibilityError> {
        Self::new(id, AccessibilityOperation::Selection { revision })
    }
    /// Creates a revision-checked UTF-16 index to logical-line mapping.
    ///
    /// # Errors
    ///
    /// Rejects zero request identity.
    pub fn line_for_index(
        id: AccessibilityRequestId,
        revision: AccessibilityRevision,
        index_utf16: usize,
    ) -> Result<Self, AccessibilityError> {
        Self::new(
            id,
            AccessibilityOperation::LineForIndex {
                revision,
                index_utf16,
            },
        )
    }
    /// Creates a revision-checked logical-line to UTF-16 range mapping.
    ///
    /// # Errors
    ///
    /// Rejects zero request identity.
    pub fn range_for_line(
        id: AccessibilityRequestId,
        revision: AccessibilityRevision,
        line: usize,
    ) -> Result<Self, AccessibilityError> {
        Self::new(id, AccessibilityOperation::RangeForLine { revision, line })
    }
    /// Creates a revision-checked UTF-16 index to grapheme-range mapping.
    ///
    /// # Errors
    ///
    /// Rejects zero request identity.
    pub fn range_for_index(
        id: AccessibilityRequestId,
        revision: AccessibilityRevision,
        index_utf16: usize,
    ) -> Result<Self, AccessibilityError> {
        Self::new(
            id,
            AccessibilityOperation::RangeForIndex {
                revision,
                index_utf16,
            },
        )
    }
    /// Creates a revision-checked action request.
    ///
    /// # Errors
    ///
    /// Rejects zero request identity.
    pub fn action(
        id: AccessibilityRequestId,
        action: AccessibilityAction,
    ) -> Result<Self, AccessibilityError> {
        Self::new(id, AccessibilityOperation::Action(action))
    }
    /// Returns request identity.
    #[must_use]
    pub const fn id(&self) -> AccessibilityRequestId {
        self.id
    }
    /// Returns the operation.
    #[must_use]
    pub const fn operation(&self) -> &AccessibilityOperation {
        &self.operation
    }
    /// Returns stable operation identity.
    #[must_use]
    pub const fn kind(&self) -> AccessibilityRequestKind {
        match self.operation {
            AccessibilityOperation::Snapshot => AccessibilityRequestKind::Snapshot,
            AccessibilityOperation::Text { .. } => AccessibilityRequestKind::Text,
            AccessibilityOperation::Selection { .. } => AccessibilityRequestKind::Selection,
            AccessibilityOperation::LineForIndex { .. } => AccessibilityRequestKind::LineForIndex,
            AccessibilityOperation::RangeForLine { .. } => AccessibilityRequestKind::RangeForLine,
            AccessibilityOperation::RangeForIndex { .. } => AccessibilityRequestKind::RangeForIndex,
            AccessibilityOperation::Action(_) => AccessibilityRequestKind::Action,
        }
    }
    /// Returns the revision required by this operation, if any.
    #[must_use]
    pub const fn revision(&self) -> Option<AccessibilityRevision> {
        match self.operation {
            AccessibilityOperation::Snapshot => None,
            AccessibilityOperation::Text { revision, .. }
            | AccessibilityOperation::Selection { revision }
            | AccessibilityOperation::LineForIndex { revision, .. }
            | AccessibilityOperation::RangeForLine { revision, .. }
            | AccessibilityOperation::RangeForIndex { revision, .. } => Some(revision),
            AccessibilityOperation::Action(action) => Some(action.revision()),
        }
    }
}

/// Bounded text returned independently from a semantic snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccessibilityText(Box<str>);

impl AccessibilityText {
    /// Owns text only when its UTF-8 byte length is within the response ceiling.
    ///
    /// # Errors
    ///
    /// Rejects text beyond [`MAX_ACCESSIBILITY_TEXT_RESPONSE_BYTES`].
    pub fn new(text: impl Into<Box<str>>) -> Result<Self, AccessibilityError> {
        let text = text.into();
        if text.len() > MAX_ACCESSIBILITY_TEXT_RESPONSE_BYTES {
            return Err(AccessibilityError::TextResponseTooLarge {
                actual: text.len(),
                limit: MAX_ACCESSIBILITY_TEXT_RESPONSE_BYTES,
            });
        }
        Ok(Self(text))
    }
    /// Returns bounded UTF-8 text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Terminal result of one accessibility action.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccessibilityActionResult {
    /// The accepted action changed observable state.
    Applied,
    /// The accepted action was already reflected by current state.
    Unchanged,
}

/// Typed payload whose variant must match request identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AccessibilityPayload {
    /// Current bounded semantic tree without document text.
    Snapshot(AccessibilitySnapshot),
    /// Separately bounded UTF-8 text.
    Text(AccessibilityText),
    /// Current directional selection.
    Selection(AccessibilitySelection),
    /// Zero-based logical line.
    Line(usize),
    /// Global UTF-16 range.
    Range(AccessibilityTextRange),
    /// Terminal action result.
    Action(AccessibilityActionResult),
}

impl AccessibilityPayload {
    const fn matches(kind: AccessibilityRequestKind, payload: &Self) -> bool {
        matches!(
            (kind, payload),
            (AccessibilityRequestKind::Snapshot, Self::Snapshot(_))
                | (AccessibilityRequestKind::Text, Self::Text(_))
                | (AccessibilityRequestKind::Selection, Self::Selection(_))
                | (AccessibilityRequestKind::LineForIndex, Self::Line(_))
                | (
                    AccessibilityRequestKind::RangeForLine
                        | AccessibilityRequestKind::RangeForIndex,
                    Self::Range(_),
                )
                | (AccessibilityRequestKind::Action, Self::Action(_))
        )
    }
}

/// One exact response to one accessibility request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccessibilityResponse {
    request_id: AccessibilityRequestId,
    kind: AccessibilityRequestKind,
    requested_revision: Option<AccessibilityRevision>,
    observed_revision: AccessibilityRevision,
    result: Result<AccessibilityPayload, AccessibilityError>,
}

impl AccessibilityResponse {
    /// Creates a successful response after kind and revision validation.
    ///
    /// # Errors
    ///
    /// Rejects stale observed revision and payload kind mismatch.
    pub fn success(
        request: &AccessibilityRequest,
        observed_revision: AccessibilityRevision,
        payload: AccessibilityPayload,
    ) -> Result<Self, AccessibilityError> {
        if let Some(expected) = request.revision()
            && expected != observed_revision
        {
            return Err(AccessibilityError::StaleRevision {
                expected,
                actual: observed_revision,
            });
        }
        if !AccessibilityPayload::matches(request.kind(), &payload) {
            return Err(AccessibilityError::RequestMismatch);
        }
        Ok(Self {
            request_id: request.id(),
            kind: request.kind(),
            requested_revision: request.revision(),
            observed_revision,
            result: Ok(payload),
        })
    }
    /// Creates a structural failure preserving exact request identity.
    #[must_use]
    pub const fn failure(
        request: &AccessibilityRequest,
        observed_revision: AccessibilityRevision,
        error: AccessibilityError,
    ) -> Self {
        Self {
            request_id: request.id(),
            kind: request.kind(),
            requested_revision: request.revision(),
            observed_revision,
            result: Err(error),
        }
    }
    /// Validates response identity against the originating request.
    ///
    /// # Errors
    ///
    /// Rejects request ID, kind, or requested-revision mismatch.
    pub fn validate_for(&self, request: &AccessibilityRequest) -> Result<(), AccessibilityError> {
        if self.request_id != request.id()
            || self.kind != request.kind()
            || self.requested_revision != request.revision()
        {
            return Err(AccessibilityError::RequestMismatch);
        }
        Ok(())
    }
    /// Returns request identity.
    #[must_use]
    pub const fn request_id(&self) -> AccessibilityRequestId {
        self.request_id
    }
    /// Returns operation identity.
    #[must_use]
    pub const fn kind(&self) -> AccessibilityRequestKind {
        self.kind
    }
    /// Returns the revision required by the request.
    #[must_use]
    pub const fn requested_revision(&self) -> Option<AccessibilityRevision> {
        self.requested_revision
    }
    /// Returns the exact revision observed by Studio.
    #[must_use]
    pub const fn observed_revision(&self) -> AccessibilityRevision {
        self.observed_revision
    }
    /// Returns the typed terminal result.
    pub const fn result(&self) -> &Result<AccessibilityPayload, AccessibilityError> {
        &self.result
    }
}

/// Structural accessibility protocol failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AccessibilityError {
    /// Fallible owned allocation failed.
    AllocationFailed,
    /// Checked arithmetic overflowed.
    ArithmeticOverflow,
    /// Request identity zero is reserved.
    InvalidRequestId,
    /// Node identity zero is reserved.
    InvalidNodeId,
    /// A semantic identity occurred more than once.
    DuplicateNodeId(AccessibilityNodeId),
    /// Root, parent, cycle, or focus invariants failed.
    InvalidTree,
    /// Node count exceeded the fixed ceiling.
    TooManyNodes {
        /// Rejected node count.
        actual: usize,
        /// Fixed node ceiling.
        limit: usize,
    },
    /// One node name exceeded its fixed byte ceiling.
    NodeNameTooLarge {
        /// Rejected UTF-8 bytes.
        actual: usize,
        /// Fixed per-name ceiling.
        limit: usize,
    },
    /// Aggregate referenced names exceeded the snapshot budget.
    NameBudgetExceeded {
        /// Rejected aggregate bytes.
        actual: usize,
        /// Fixed aggregate ceiling.
        limit: usize,
    },
    /// A selection endpoint exceeded current text.
    InvalidSelection {
        /// Current global UTF-16 text length.
        text_len_utf16: usize,
    },
    /// A text response exceeded its fixed byte ceiling.
    TextResponseTooLarge {
        /// Rejected bytes or preflight UTF-16 units.
        actual: usize,
        /// Fixed response ceiling.
        limit: usize,
    },
    /// Requested document or buffer identity is stale.
    StaleRevision {
        /// Revision required by the request.
        expected: AccessibilityRevision,
        /// Current Studio revision.
        actual: AccessibilityRevision,
    },
    /// Checked byte or UTF-16 mapping failed.
    TextMappingFailed,
    /// A requested semantic action target is absent from the current tree.
    ActionTargetMissing(AccessibilityNodeId),
    /// A current semantic node does not admit activation.
    ActionDisabled(AccessibilityNodeId),
    /// A semantic rectangle was non-finite, negative, or outside the fixed domain.
    InvalidBounds,
    /// A response was already installed for the dispatch.
    DuplicateResponse,
    /// Request identity, operation, revision, or payload kind did not match.
    RequestMismatch,
}

impl fmt::Display for AccessibilityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AllocationFailed => formatter.write_str("accessibility allocation failed"),
            Self::ArithmeticOverflow => formatter.write_str("accessibility arithmetic overflow"),
            Self::InvalidRequestId => formatter.write_str("accessibility request identity is zero"),
            Self::InvalidNodeId => formatter.write_str("accessibility node identity is zero"),
            Self::DuplicateNodeId(id) => {
                write!(formatter, "duplicate accessibility node {}", id.get())
            }
            Self::InvalidTree => formatter.write_str("accessibility tree is inconsistent"),
            Self::TooManyNodes { actual, limit } => write!(
                formatter,
                "accessibility tree has {actual} nodes; limit is {limit}"
            ),
            Self::NodeNameTooLarge { actual, limit } => write!(
                formatter,
                "accessibility node name has {actual} bytes; limit is {limit}"
            ),
            Self::NameBudgetExceeded { actual, limit } => write!(
                formatter,
                "accessibility names retain {actual} bytes; limit is {limit}"
            ),
            Self::InvalidSelection { text_len_utf16 } => write!(
                formatter,
                "accessibility selection exceeds UTF-16 text length {text_len_utf16}"
            ),
            Self::TextResponseTooLarge { actual, limit } => write!(
                formatter,
                "accessibility text response has {actual} bytes; limit is {limit}"
            ),
            Self::StaleRevision { expected, actual } => write!(
                formatter,
                "stale accessibility revision {expected:?}; current is {actual:?}"
            ),
            Self::TextMappingFailed => formatter.write_str("accessibility text mapping failed"),
            Self::ActionTargetMissing(id) => {
                write!(
                    formatter,
                    "accessibility action target {} is missing",
                    id.get()
                )
            }
            Self::ActionDisabled(id) => {
                write!(
                    formatter,
                    "accessibility action target {} is disabled",
                    id.get()
                )
            }
            Self::InvalidBounds => formatter.write_str("accessibility bounds are invalid"),
            Self::DuplicateResponse => formatter.write_str("accessibility response is already set"),
            Self::RequestMismatch => {
                formatter.write_str("accessibility response does not match its request")
            }
        }
    }
}

impl Error for AccessibilityError {}

#[cfg(test)]
mod bounded_action_tests {
    use super::*;

    #[test]
    fn semantic_revision_axis_is_exact_and_independent() {
        let base = AccessibilityRevision::new(7, 11);
        assert_eq!(base.document(), 7);
        assert_eq!(base.buffer(), 11);
        assert_eq!(base.semantic(), 0);
        let semantic = base.with_semantic(13);
        assert_eq!(semantic.document(), 7);
        assert_eq!(semantic.buffer(), 11);
        assert_eq!(semantic.semantic(), 13);
        assert_ne!(semantic, base);
    }

    #[test]
    fn bounds_and_activation_values_fail_closed() -> Result<(), AccessibilityError> {
        let normalized_zero = AccessibilityBounds::new(-0.0, 2.0, 30.0, 40.0)?;
        assert_eq!(normalized_zero.x().to_bits(), 0.0_f32.to_bits());
        let bounds = AccessibilityBounds::new(1.5, 2.0, 30.0, 40.0)?;
        assert_eq!(
            (bounds.x(), bounds.y(), bounds.width(), bounds.height()),
            (1.5, 2.0, 30.0, 40.0)
        );
        assert_eq!(
            AccessibilityBounds::new(f32::NAN, 0.0, 1.0, 1.0),
            Err(AccessibilityError::InvalidBounds)
        );
        assert_eq!(
            AccessibilityBounds::new(MAX_ACCESSIBILITY_COORDINATE, 0.0, 1.0, 1.0),
            Err(AccessibilityError::InvalidBounds)
        );
        let revision = AccessibilityRevision::new(7, 11);
        let id = AccessibilityNodeId::new(13);
        let action = AccessibilityAction::activate(revision, id);
        assert_eq!(action.revision(), revision);
        assert_eq!(action, AccessibilityAction::Activate { revision, node: id });
        let default_node = AccessibilityNode::new(
            id,
            None,
            AccessibilityRole::ListItem,
            "row".into(),
            false,
            false,
            false,
        )
        .unwrap_or_else(|_| unreachable!());
        assert!(default_node.is_enabled());
        assert!(!default_node.supports_activate());
        let node = default_node.with_bounds(bounds).with_activate(false);
        assert!(node.supports_activate());
        assert!(!node.is_enabled());
        assert_eq!(node.bounds(), bounds);
        assert_eq!(
            AccessibilityError::ActionTargetMissing(id).to_string(),
            "accessibility action target 13 is missing"
        );
        assert_eq!(
            AccessibilityError::ActionDisabled(id).to_string(),
            "accessibility action target 13 is disabled"
        );
        assert_eq!(
            AccessibilityError::InvalidBounds.to_string(),
            "accessibility bounds are invalid"
        );
        Ok(())
    }
}

#[cfg(kani)]
#[kani::proof]
fn activation_revision_and_target_identity_are_preserved() {
    let document = kani::any::<u64>();
    let buffer = kani::any::<u64>();
    let target = kani::any::<u64>();
    kani::assume(target != 0);
    kani::cover!(target == 1, "minimum target identity");
    kani::cover!(document == 0 && buffer == 0, "zero revision identity");
    let revision = AccessibilityRevision::new(document, buffer);
    let node = AccessibilityNodeId::new(target);
    let action = AccessibilityAction::activate(revision, node);
    assert_eq!(action.revision(), revision);
    assert_eq!(action, AccessibilityAction::Activate { revision, node });
}

#[cfg(test)]
mod mapping_protocol_tests {
    use super::*;

    #[test]
    fn mapping_request_kinds_and_payloads_are_exact() -> Result<(), AccessibilityError> {
        let revision = AccessibilityRevision::new(11, 13);
        let line_request =
            AccessibilityRequest::line_for_index(AccessibilityRequestId::new(17), revision, 19)?;
        let line_response =
            AccessibilityResponse::success(&line_request, revision, AccessibilityPayload::Line(2))?;
        assert_eq!(line_request.kind(), AccessibilityRequestKind::LineForIndex);
        assert_eq!(line_request.revision(), Some(revision));
        assert_eq!(line_response.validate_for(&line_request), Ok(()));

        let line_range_request =
            AccessibilityRequest::range_for_line(AccessibilityRequestId::new(18), revision, 2)?;
        let range = AccessibilityTextRange::new(7, 5);
        let line_range_response = AccessibilityResponse::success(
            &line_range_request,
            revision,
            AccessibilityPayload::Range(range),
        );
        assert_eq!(
            line_range_request.kind(),
            AccessibilityRequestKind::RangeForLine
        );
        assert!(matches!(
            &line_range_response,
            Ok(response) if response.validate_for(&line_range_request) == Ok(())
        ));

        let grapheme_request =
            AccessibilityRequest::range_for_index(AccessibilityRequestId::new(19), revision, 8)?;
        assert_eq!(
            grapheme_request.kind(),
            AccessibilityRequestKind::RangeForIndex
        );
        assert_eq!(
            AccessibilityResponse::success(
                &grapheme_request,
                revision,
                AccessibilityPayload::Line(2),
            ),
            Err(AccessibilityError::RequestMismatch)
        );
        assert!(matches!(
            &line_range_response,
            Ok(response)
                if response.validate_for(&grapheme_request)
                    == Err(AccessibilityError::RequestMismatch)
        ));
        Ok(())
    }
}

#[cfg(kani)]
mod verification {
    use super::*;

    #[cfg_attr(test, mutants::skip)] // This cfg(kani) proof is executed by the dedicated Kani gate.
    #[kani::proof]
    fn checked_utf16_range_end_never_wraps() {
        let start = kani::any::<usize>();
        let length = kani::any::<usize>();
        kani::cover!(start == 0 && length == 0, "empty origin range");
        kani::cover!(
            start == usize::MAX && length == 1,
            "checked overflow boundary"
        );
        let range = AccessibilityTextRange::new(start, length);
        match range.end_utf16() {
            Ok(end) => {
                assert!(end >= start);
                assert_eq!(end, start + length);
            }
            Err(AccessibilityError::ArithmeticOverflow) => {
                assert!(start.checked_add(length).is_none())
            }
            Err(_) => unreachable!(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(
        id: u64,
        parent: Option<u64>,
        focused: bool,
    ) -> Result<AccessibilityNode, AccessibilityError> {
        AccessibilityNode::new(
            AccessibilityNodeId::new(id),
            parent.map(AccessibilityNodeId::new),
            AccessibilityRole::Window,
            Arc::from("node"),
            focused,
            false,
            false,
        )
    }

    #[test]
    fn retained_name_preserves_exact_shared_storage() -> Result<(), AccessibilityError> {
        let node = node(1, None, true)?;
        let retained = node.retained_name();
        assert_eq!(&*retained, "node");
        assert!(Arc::ptr_eq(&retained, &node.name));
        Ok(())
    }

    #[test]
    fn snapshot_validates_identity_tree_selection_and_accounting() -> Result<(), AccessibilityError>
    {
        let root = AccessibilityNodeId::new(1);
        let snapshot_result = AccessibilitySnapshot::new(
            AccessibilityRevision::new(3, 5),
            root,
            vec![node(1, None, true)?, node(2, Some(1), false)?],
            AccessibilitySelection::new(4, 2),
            7,
            1,
            true,
        );
        let snapshot = snapshot_result?;
        assert_eq!(snapshot.nodes().len(), 2);
        assert_eq!(snapshot.report().referenced_name_bytes(), 8);
        assert_eq!(
            snapshot.selection().range(),
            AccessibilityTextRange::new(2, 2)
        );
        assert_eq!(
            AccessibilitySnapshot::new(
                snapshot.revision(),
                root,
                vec![node(1, None, false)?, node(1, Some(1), false)?],
                AccessibilitySelection::new(0, 0),
                0,
                1,
                false,
            ),
            Err(AccessibilityError::DuplicateNodeId(root))
        );
        Ok(())
    }

    #[test]
    fn request_and_response_identity_fail_closed() -> Result<(), AccessibilityError> {
        let revision = AccessibilityRevision::new(7, 11);
        let request_result = AccessibilityRequest::text(
            AccessibilityRequestId::new(13),
            revision,
            AccessibilityTextRange::new(1, 2),
        );
        let request = request_result?;
        let text = AccessibilityText::new("ok")?;
        let response_result =
            AccessibilityResponse::success(&request, revision, AccessibilityPayload::Text(text));
        let response = response_result?;
        assert_eq!(response.validate_for(&request), Ok(()));
        let other = AccessibilityRequest::selection(AccessibilityRequestId::new(14), revision)?;
        assert_eq!(
            response.validate_for(&other),
            Err(AccessibilityError::RequestMismatch)
        );
        assert_eq!(
            AccessibilityResponse::success(
                &request,
                AccessibilityRevision::new(7, 12),
                AccessibilityPayload::Text(AccessibilityText::new("ok")?),
            ),
            Err(AccessibilityError::StaleRevision {
                expected: revision,
                actual: AccessibilityRevision::new(7, 12),
            })
        );
        Ok(())
    }

    #[test]
    fn byte_and_arithmetic_ceilings_are_exact() {
        assert_eq!(
            AccessibilityRequest::snapshot(AccessibilityRequestId::new(0)),
            Err(AccessibilityError::InvalidRequestId)
        );
        assert_eq!(
            AccessibilityTextRange::new(usize::MAX, 1).end_utf16(),
            Err(AccessibilityError::ArithmeticOverflow)
        );
        assert!(AccessibilityText::new("x".repeat(MAX_ACCESSIBILITY_TEXT_RESPONSE_BYTES)).is_ok());
        assert_eq!(
            AccessibilityText::new("x".repeat(MAX_ACCESSIBILITY_TEXT_RESPONSE_BYTES + 1)),
            Err(AccessibilityError::TextResponseTooLarge {
                actual: MAX_ACCESSIBILITY_TEXT_RESPONSE_BYTES + 1,
                limit: MAX_ACCESSIBILITY_TEXT_RESPONSE_BYTES,
            })
        );
    }

    fn flat_tree(
        count: usize,
        name_bytes: usize,
    ) -> Result<Vec<AccessibilityNode>, AccessibilityError> {
        let name: Arc<str> = "x".repeat(name_bytes).into();
        (1..=count)
            .map(|id| {
                AccessibilityNode::new(
                    AccessibilityNodeId::new(id as u64),
                    (id != 1).then_some(AccessibilityNodeId::new(1)),
                    AccessibilityRole::CodeEditor,
                    Arc::clone(&name),
                    id == 1,
                    id == 2,
                    id == 3,
                )
            })
            .collect()
    }

    #[test]
    fn public_values_preserve_every_axis_and_ceiling() -> Result<(), AccessibilityError> {
        let root = AccessibilityNodeId::new(1);
        let child_result = AccessibilityNode::new(
            AccessibilityNodeId::new(2),
            Some(root),
            AccessibilityRole::Tab,
            Arc::from("selected tab"),
            true,
            true,
            true,
        );
        let child = child_result?;
        assert_eq!(root.get(), 1);
        assert_eq!(child.id().get(), 2);
        assert_eq!(child.parent(), Some(root));
        assert_eq!(child.role(), AccessibilityRole::Tab);
        assert_eq!(child.name(), "selected tab");
        assert!(child.is_focused());
        assert!(child.is_selected());
        assert!(child.announces());
        assert_eq!(
            AccessibilityNode::new(
                AccessibilityNodeId::new(0),
                None,
                AccessibilityRole::Window,
                Arc::from("invalid"),
                false,
                false,
                false,
            ),
            Err(AccessibilityError::InvalidNodeId)
        );
        assert_eq!(
            AccessibilityNode::new(
                AccessibilityNodeId::new(2),
                Some(AccessibilityNodeId::new(0)),
                AccessibilityRole::Window,
                Arc::from("invalid"),
                false,
                false,
                false,
            ),
            Err(AccessibilityError::InvalidNodeId)
        );
        assert!(matches!(
            AccessibilityNode::new(
                AccessibilityNodeId::new(2),
                Some(root),
                AccessibilityRole::Window,
                "x".repeat(MAX_ACCESSIBILITY_NODE_NAME_BYTES + 1).into(),
                false,
                false,
                false,
            ),
            Err(AccessibilityError::NodeNameTooLarge { actual, limit })
                if actual == MAX_ACCESSIBILITY_NODE_NAME_BYTES + 1
                    && limit == MAX_ACCESSIBILITY_NODE_NAME_BYTES
        ));

        let revision = AccessibilityRevision::new(7, 11);
        assert_eq!(revision.document(), 7);
        assert_eq!(revision.buffer(), 11);
        let range = AccessibilityTextRange::new(3, 5);
        assert_eq!(range.start_utf16(), 3);
        assert_eq!(range.length_utf16(), 5);
        assert_eq!(range.end_utf16(), Ok(8));
        let selection = AccessibilitySelection::new(8, 3);
        assert_eq!(selection.anchor_utf16(), 8);
        assert_eq!(selection.head_utf16(), 3);
        assert_eq!(selection.range(), range);

        let snapshot_result = AccessibilitySnapshot::new(
            revision,
            root,
            vec![node(1, None, false)?, child],
            selection,
            8,
            2,
            true,
        );
        let snapshot = snapshot_result?;
        assert_eq!(snapshot.root(), root);
        assert_eq!(snapshot.text_len_utf16(), 8);
        assert_eq!(snapshot.line_count(), 2);
        assert!(snapshot.is_dirty());
        assert_eq!(snapshot.report().node_count(), 2);
        assert_eq!(
            snapshot.report().owned_node_bytes(),
            2 * mem::size_of::<AccessibilityNode>()
        );
        assert_eq!(snapshot.report().max_nodes(), MAX_ACCESSIBILITY_NODES);
        assert_eq!(
            snapshot.report().max_text_request_bytes(),
            MAX_ACCESSIBILITY_TEXT_RESPONSE_BYTES
        );
        Ok(())
    }

    #[test]
    fn exact_limits_and_false_observer_axes_are_discriminating() -> Result<(), AccessibilityError> {
        assert_eq!(MAX_ACCESSIBILITY_NODE_NAME_BYTES, 4_096);
        assert_eq!(MAX_ACCESSIBILITY_NAME_BYTES, 262_144);
        assert_eq!(MAX_ACCESSIBILITY_TEXT_RESPONSE_BYTES, 65_536);
        let root = AccessibilityNodeId::new(1);
        let inactive = node(2, Some(1), false)?;
        assert!(!inactive.is_selected());
        assert!(!inactive.announces());
        assert_eq!(
            AccessibilitySnapshot::new(
                AccessibilityRevision::new(7, 11),
                root,
                vec![node(1, None, false)?, inactive],
                AccessibilitySelection::new(0, 0),
                0,
                1,
                false,
            )
            .map(|snapshot| snapshot.is_dirty()),
            Ok(false)
        );
        Ok(())
    }

    #[test]
    fn malformed_trees_and_budgets_fail_before_publication() -> Result<(), AccessibilityError> {
        let revision = AccessibilityRevision::new(1, 1);
        let root = AccessibilityNodeId::new(1);
        let empty_selection = AccessibilitySelection::new(0, 0);
        assert_eq!(
            AccessibilitySnapshot::new(revision, root, Vec::new(), empty_selection, 0, 1, false,),
            Err(AccessibilityError::InvalidTree)
        );
        assert!(matches!(
            AccessibilitySnapshot::new(
                revision,
                root,
                flat_tree(MAX_ACCESSIBILITY_NODES + 1, 1)?,
                empty_selection,
                0,
                1,
                false,
            ),
            Err(AccessibilityError::TooManyNodes { actual, limit })
                if actual == MAX_ACCESSIBILITY_NODES + 1 && limit == MAX_ACCESSIBILITY_NODES
        ));
        assert_eq!(
            AccessibilitySnapshot::new(
                revision,
                AccessibilityNodeId::new(0),
                flat_tree(MAX_ACCESSIBILITY_NODES + 1, 1)?,
                empty_selection,
                0,
                1,
                false,
            ),
            Err(AccessibilityError::InvalidTree)
        );
        assert!(matches!(
            AccessibilitySnapshot::new(
                revision,
                root,
                flat_tree(MAX_ACCESSIBILITY_NODES, 1_000)?,
                empty_selection,
                0,
                1,
                false,
            ),
            Err(AccessibilityError::NameBudgetExceeded { actual, limit })
                if actual == MAX_ACCESSIBILITY_NODES * 1_000
                    && limit == MAX_ACCESSIBILITY_NAME_BYTES
        ));
        assert_eq!(
            AccessibilitySnapshot::new(
                revision,
                root,
                flat_tree(1, 1)?,
                AccessibilitySelection::new(1, 0),
                0,
                1,
                false,
            ),
            Err(AccessibilityError::InvalidSelection { text_len_utf16: 0 })
        );

        let root_with_parent = vec![node(1, Some(2), false)?, node(2, Some(1), false)?];
        let missing_parent = vec![node(1, None, false)?, node(2, Some(9), false)?];
        let cycle = vec![
            node(1, None, false)?,
            node(2, Some(3), false)?,
            node(3, Some(2), false)?,
        ];
        let multiple_focus = vec![node(1, None, true)?, node(2, Some(1), true)?];
        for nodes in [root_with_parent, missing_parent, cycle, multiple_focus] {
            assert_eq!(
                AccessibilitySnapshot::new(revision, root, nodes, empty_selection, 0, 1, false,),
                Err(AccessibilityError::InvalidTree)
            );
        }
        assert!(
            AccessibilitySnapshot::new(
                revision,
                root,
                vec![
                    node(1, None, false)?,
                    node(2, Some(1), false)?,
                    node(3, Some(2), false)?,
                ],
                empty_selection,
                0,
                1,
                false,
            )
            .is_ok()
        );
        Ok(())
    }

    #[test]
    fn requests_responses_and_diagnostics_preserve_identity() -> Result<(), AccessibilityError> {
        let revision = AccessibilityRevision::new(3, 5);
        let snapshot_request = AccessibilityRequest::snapshot(AccessibilityRequestId::new(1))?;
        assert_eq!(snapshot_request.id().get(), 1);
        assert_eq!(snapshot_request.kind(), AccessibilityRequestKind::Snapshot);
        assert_eq!(snapshot_request.revision(), None);
        assert!(matches!(
            snapshot_request.operation(),
            AccessibilityOperation::Snapshot
        ));

        let selection_request =
            AccessibilityRequest::selection(AccessibilityRequestId::new(2), revision)?;
        assert_eq!(
            selection_request.kind(),
            AccessibilityRequestKind::Selection
        );
        assert_eq!(selection_request.revision(), Some(revision));
        let action = AccessibilityAction::set_selection(revision, 4, 2);
        assert_eq!(action.revision(), revision);
        let action_request = AccessibilityRequest::action(AccessibilityRequestId::new(3), action)?;
        assert_eq!(action_request.kind(), AccessibilityRequestKind::Action);
        assert!(matches!(
            action_request.operation(),
            AccessibilityOperation::Action(AccessibilityAction::SetSelection { selection, .. })
                if selection.anchor_utf16() == 4 && selection.head_utf16() == 2
        ));
        assert!(matches!(
            AccessibilityRequest::text(
                AccessibilityRequestId::new(4),
                revision,
                AccessibilityTextRange::new(0, MAX_ACCESSIBILITY_TEXT_RESPONSE_BYTES + 1),
            ),
            Err(AccessibilityError::TextResponseTooLarge { actual, limit })
                if actual == MAX_ACCESSIBILITY_TEXT_RESPONSE_BYTES + 1
                    && limit == MAX_ACCESSIBILITY_TEXT_RESPONSE_BYTES
        ));

        let text = AccessibilityText::new("bounded")?;
        assert_eq!(text.as_str(), "bounded");
        let response_result = AccessibilityResponse::success(
            &selection_request,
            revision,
            AccessibilityPayload::Selection(AccessibilitySelection::new(1, 2)),
        );
        let response = response_result?;
        assert_eq!(response.request_id().get(), 2);
        assert_eq!(response.kind(), AccessibilityRequestKind::Selection);
        assert_eq!(response.requested_revision(), Some(revision));
        assert_eq!(response.observed_revision(), revision);
        assert!(response.result().is_ok());
        assert_eq!(response.validate_for(&selection_request), Ok(()));
        let wrong_id = AccessibilityRequest::selection(AccessibilityRequestId::new(20), revision)?;
        assert_eq!(
            response.validate_for(&wrong_id),
            Err(AccessibilityError::RequestMismatch)
        );
        let wrong_kind = AccessibilityRequest::text(
            AccessibilityRequestId::new(2),
            revision,
            AccessibilityTextRange::new(0, 0),
        );
        assert_eq!(
            wrong_kind
                .as_ref()
                .map(|request| response.validate_for(request)),
            Ok(Err(AccessibilityError::RequestMismatch))
        );
        let wrong_revision = AccessibilityRequest::selection(
            AccessibilityRequestId::new(2),
            AccessibilityRevision::new(3, 6),
        );
        assert_eq!(
            wrong_revision
                .as_ref()
                .map(|request| response.validate_for(request)),
            Ok(Err(AccessibilityError::RequestMismatch))
        );
        assert_eq!(
            AccessibilityResponse::success(
                &selection_request,
                revision,
                AccessibilityPayload::Action(AccessibilityActionResult::Applied),
            ),
            Err(AccessibilityError::RequestMismatch)
        );
        let failure = AccessibilityResponse::failure(
            &action_request,
            revision,
            AccessibilityError::DuplicateResponse,
        );
        assert_eq!(
            failure.result(),
            &Err(AccessibilityError::DuplicateResponse)
        );

        Ok(())
    }

    #[test]
    fn diagnostics_are_stable_and_nonempty() {
        let revision = AccessibilityRevision::new(3, 5);
        let diagnostics = [
            AccessibilityError::AllocationFailed,
            AccessibilityError::ArithmeticOverflow,
            AccessibilityError::InvalidRequestId,
            AccessibilityError::InvalidNodeId,
            AccessibilityError::DuplicateNodeId(AccessibilityNodeId::new(9)),
            AccessibilityError::InvalidTree,
            AccessibilityError::TooManyNodes {
                actual: 2,
                limit: 1,
            },
            AccessibilityError::NodeNameTooLarge {
                actual: 2,
                limit: 1,
            },
            AccessibilityError::NameBudgetExceeded {
                actual: 2,
                limit: 1,
            },
            AccessibilityError::InvalidSelection { text_len_utf16: 1 },
            AccessibilityError::TextResponseTooLarge {
                actual: 2,
                limit: 1,
            },
            AccessibilityError::StaleRevision {
                expected: revision,
                actual: AccessibilityRevision::new(3, 6),
            },
            AccessibilityError::TextMappingFailed,
            AccessibilityError::DuplicateResponse,
            AccessibilityError::RequestMismatch,
        ];
        for error in diagnostics {
            assert!(!error.to_string().is_empty());
        }
    }
}
