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
}

/// One bounded semantic element without a native handle.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccessibilityNode {
    id: AccessibilityNodeId,
    parent: Option<AccessibilityNodeId>,
    role: AccessibilityRole,
    name: Arc<str>,
    focused: bool,
    selected: bool,
    announces: bool,
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
        })
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
}

/// Exact Studio document and buffer identity observed by accessibility.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AccessibilityRevision {
    document: u64,
    buffer: u64,
}

impl AccessibilityRevision {
    /// Creates an exact semantic revision.
    #[must_use]
    pub const fn new(document: u64, buffer: u64) -> Self {
        Self { document, buffer }
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
    /// Returns the exact observed revision.
    #[must_use]
    pub const fn revision(self) -> AccessibilityRevision {
        match self {
            Self::SetSelection { revision, .. } => revision,
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
            AccessibilityOperation::Action(_) => AccessibilityRequestKind::Action,
        }
    }
    /// Returns the revision required by this operation, if any.
    #[must_use]
    pub const fn revision(&self) -> Option<AccessibilityRevision> {
        match self.operation {
            AccessibilityOperation::Snapshot => None,
            AccessibilityOperation::Text { revision, .. }
            | AccessibilityOperation::Selection { revision } => Some(revision),
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
            Self::DuplicateResponse => formatter.write_str("accessibility response is already set"),
            Self::RequestMismatch => {
                formatter.write_str("accessibility response does not match its request")
            }
        }
    }
}

impl Error for AccessibilityError {}

#[cfg(kani)]
mod verification {
    use super::*;

    #[kani::proof]
    fn checked_utf16_range_end_never_wraps() {
        let start = kani::any::<usize>();
        let length = kani::any::<usize>();
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
    fn snapshot_validates_identity_tree_selection_and_accounting() -> Result<(), AccessibilityError>
    {
        let root = AccessibilityNodeId::new(1);
        let snapshot = AccessibilitySnapshot::new(
            AccessibilityRevision::new(3, 5),
            root,
            vec![node(1, None, true)?, node(2, Some(1), false)?],
            AccessibilitySelection::new(4, 2),
            7,
            1,
            true,
        )?;
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
        let request = AccessibilityRequest::text(
            AccessibilityRequestId::new(13),
            revision,
            AccessibilityTextRange::new(1, 2),
        )?;
        let response = AccessibilityResponse::success(
            &request,
            revision,
            AccessibilityPayload::Text(AccessibilityText::new("ok")?),
        )?;
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
}
