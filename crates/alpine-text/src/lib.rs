//! Checked local text storage and one-file editing for Alpine Studio.
//!
//! Byte offsets are canonical. Third-party rope values never cross this crate's
//! public boundary, and every externally supplied coordinate is validated before
//! it reaches the rope.

use ropey::Rope;
use std::collections::VecDeque;
use std::ffi::OsString;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use unicode_segmentation::UnicodeSegmentation;

#[cfg(kani)]
mod proofs;

const DEFAULT_HISTORY_ENTRIES: usize = 1_024;
const DEFAULT_HISTORY_BYTES: usize = 64 * 1_024 * 1_024;
const TEMPORARY_FILE_ATTEMPTS: u64 = 32;
static TEMPORARY_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(1);

/// A monotonically increasing local buffer revision.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct BufferRevision(u64);

impl BufferRevision {
    /// The initial revision of a newly opened buffer.
    pub const INITIAL: Self = Self(0);

    /// Returns the numeric revision.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    fn next(self) -> Result<Self, TextError> {
        self.0
            .checked_add(1)
            .map(Self)
            .ok_or(TextError::RevisionExhausted)
    }
}

/// A canonical UTF-8 byte offset in a buffer.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ByteOffset(usize);

impl ByteOffset {
    /// Creates an offset. The receiving snapshot validates its bounds and
    /// character-boundary identity.
    #[must_use]
    pub const fn new(value: usize) -> Self {
        Self(value)
    }

    /// Returns the underlying byte count.
    #[must_use]
    pub const fn get(self) -> usize {
        self.0
    }
}

/// A zero-based line and UTF-8 byte column.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct LineColumn {
    line: usize,
    byte_column: usize,
}

impl LineColumn {
    /// Creates a line-column coordinate.
    #[must_use]
    pub const fn new(line: usize, byte_column: usize) -> Self {
        Self { line, byte_column }
    }

    /// Returns the zero-based line.
    #[must_use]
    pub const fn line(self) -> usize {
        self.line
    }

    /// Returns the UTF-8 byte column.
    #[must_use]
    pub const fn byte_column(self) -> usize {
        self.byte_column
    }
}

/// A zero-based LSP position using UTF-16 code units for the column.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct LspPosition {
    line: usize,
    utf16_column: usize,
}

impl LspPosition {
    /// Creates an LSP position.
    #[must_use]
    pub const fn new(line: usize, utf16_column: usize) -> Self {
        Self { line, utf16_column }
    }

    /// Returns the zero-based line.
    #[must_use]
    pub const fn line(self) -> usize {
        self.line
    }

    /// Returns the UTF-16 code-unit column.
    #[must_use]
    pub const fn utf16_column(self) -> usize {
        self.utf16_column
    }
}

/// One directional selection in canonical byte coordinates.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct Selection {
    anchor: ByteOffset,
    head: ByteOffset,
}

impl Selection {
    /// Creates a directional selection.
    #[must_use]
    pub const fn new(anchor: ByteOffset, head: ByteOffset) -> Self {
        Self { anchor, head }
    }

    /// Creates a collapsed caret.
    #[must_use]
    pub const fn caret(offset: ByteOffset) -> Self {
        Self::new(offset, offset)
    }

    /// Returns the fixed end.
    #[must_use]
    pub const fn anchor(self) -> ByteOffset {
        self.anchor
    }

    /// Returns the moving end.
    #[must_use]
    pub const fn head(self) -> ByteOffset {
        self.head
    }

    /// Returns the ordered byte range.
    #[must_use]
    pub fn range(self) -> Range<usize> {
        self.anchor.0.min(self.head.0)..self.anchor.0.max(self.head.0)
    }
}

/// A non-empty, deterministically ordered set of selections.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SelectionSet {
    selections: Vec<Selection>,
}

impl SelectionSet {
    /// Creates, sorts, and deduplicates a non-empty selection set.
    ///
    /// # Errors
    ///
    /// Returns [`TextError::EmptySelectionSet`] when no selection is supplied.
    pub fn new(mut selections: Vec<Selection>) -> Result<Self, TextError> {
        if selections.is_empty() {
            return Err(TextError::EmptySelectionSet);
        }
        selections.sort_unstable_by_key(|selection| {
            let range = selection.range();
            (range.start, range.end, selection.anchor.0, selection.head.0)
        });
        selections.dedup();
        Ok(Self { selections })
    }

    /// Creates one caret.
    #[must_use]
    pub fn caret(offset: ByteOffset) -> Self {
        Self {
            selections: vec![Selection::caret(offset)],
        }
    }

    /// Returns the selections in deterministic document order.
    #[must_use]
    pub fn as_slice(&self) -> &[Selection] {
        &self.selections
    }

    fn transformed(&self, edits: &[Edit]) -> Self {
        let selections = self
            .selections
            .iter()
            .map(|selection| {
                Selection::new(
                    ByteOffset(transform_offset(selection.anchor.0, edits)),
                    ByteOffset(transform_offset(selection.head.0, edits)),
                )
            })
            .collect();
        Self { selections }
    }
}

/// Bounded undo and redo retention policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HistoryLimits {
    max_entries: usize,
    max_bytes: usize,
}

impl HistoryLimits {
    /// Creates a policy. Zero disables history retention for that dimension.
    #[must_use]
    pub const fn new(max_entries: usize, max_bytes: usize) -> Self {
        Self {
            max_entries,
            max_bytes,
        }
    }

    /// Returns the entry ceiling.
    #[must_use]
    pub const fn max_entries(self) -> usize {
        self.max_entries
    }

    /// Returns the changed-byte ceiling.
    #[must_use]
    pub const fn max_bytes(self) -> usize {
        self.max_bytes
    }
}

impl Default for HistoryLimits {
    fn default() -> Self {
        Self::new(DEFAULT_HISTORY_ENTRIES, DEFAULT_HISTORY_BYTES)
    }
}

/// Current bounded-history accounting.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct HistorySnapshot {
    undo_entries: usize,
    redo_entries: usize,
    retained_changed_bytes: usize,
    max_entries: usize,
    max_bytes: usize,
}

impl HistorySnapshot {
    /// Returns available undo entries.
    #[must_use]
    pub const fn undo_entries(self) -> usize {
        self.undo_entries
    }

    /// Returns available redo entries.
    #[must_use]
    pub const fn redo_entries(self) -> usize {
        self.redo_entries
    }

    /// Returns bytes changed by retained history entries.
    #[must_use]
    pub const fn retained_changed_bytes(self) -> usize {
        self.retained_changed_bytes
    }

    /// Returns the configured entry ceiling.
    #[must_use]
    pub const fn max_entries(self) -> usize {
        self.max_entries
    }

    /// Returns the configured changed-byte ceiling.
    #[must_use]
    pub const fn max_bytes(self) -> usize {
        self.max_bytes
    }
}

/// A structured failure from checked text operations.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TextError {
    /// The transaction was built against an obsolete revision.
    StaleRevision {
        /// Required current revision.
        expected: BufferRevision,
        /// Transaction revision.
        actual: BufferRevision,
    },
    /// A byte offset exceeded the snapshot length.
    ByteOutOfBounds {
        /// Rejected offset.
        offset: usize,
        /// Current byte length.
        len: usize,
    },
    /// A byte offset did not identify a UTF-8 character boundary.
    InvalidByteBoundary {
        /// Rejected offset.
        offset: usize,
    },
    /// A replacement range was reversed or overlapped another edit.
    InvalidEditRange {
        /// Rejected start.
        start: usize,
        /// Rejected end.
        end: usize,
    },
    /// A line index did not exist.
    LineOutOfBounds {
        /// Rejected line.
        line: usize,
        /// Available line count.
        line_count: usize,
    },
    /// A byte column exceeded its line or split a character.
    InvalidLineColumn(LineColumn),
    /// A byte offset was not a grapheme boundary.
    InvalidGraphemeBoundary {
        /// Rejected byte offset.
        offset: usize,
    },
    /// A UTF-16 coordinate exceeded the text or split a surrogate pair.
    InvalidUtf16Boundary {
        /// Rejected UTF-16 code-unit offset.
        offset: usize,
    },
    /// A selection set was empty.
    EmptySelectionSet,
    /// The monotonic revision space was exhausted.
    RevisionExhausted,
}

impl fmt::Display for TextError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for TextError {}

/// One immutable, cheaply cloned text snapshot.
#[derive(Clone)]
pub struct BufferSnapshot {
    rope: Rope,
    revision: BufferRevision,
}

impl fmt::Debug for BufferSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BufferSnapshot")
            .field("revision", &self.revision)
            .field("len_bytes", &self.len_bytes())
            .finish_non_exhaustive()
    }
}

impl BufferSnapshot {
    /// Returns the captured revision.
    #[must_use]
    pub const fn revision(&self) -> BufferRevision {
        self.revision
    }

    /// Returns the UTF-8 byte length.
    #[must_use]
    pub fn len_bytes(&self) -> usize {
        self.rope.len_bytes()
    }

    /// Returns whether the text is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rope.len_bytes() == 0
    }

    /// Materializes the complete snapshot.
    #[must_use]
    pub fn text(&self) -> String {
        self.rope.to_string()
    }

    /// Materializes one validated byte range.
    ///
    /// # Errors
    ///
    /// Returns a structured bounds or UTF-8 boundary error.
    pub fn slice(&self, range: Range<usize>) -> Result<String, TextError> {
        self.validate_range(&range)?;
        self.rope
            .get_byte_slice(range.clone())
            .map(|slice| slice.to_string())
            .ok_or(TextError::InvalidEditRange {
                start: range.start,
                end: range.end,
            })
    }

    /// Returns whether an offset is an in-bounds UTF-8 character boundary.
    #[must_use]
    pub fn is_char_boundary(&self, offset: ByteOffset) -> bool {
        if offset.0 > self.len_bytes() {
            return false;
        }
        self.rope.try_byte_to_char(offset.0).is_ok_and(|character| {
            self.rope
                .try_char_to_byte(character)
                .is_ok_and(|round_trip| round_trip == offset.0)
        })
    }

    /// Converts a canonical byte offset to a line and byte column.
    ///
    /// # Errors
    ///
    /// Rejects out-of-bounds, character-splitting, and CRLF-splitting offsets.
    pub fn line_column_of_byte(&self, offset: ByteOffset) -> Result<LineColumn, TextError> {
        self.validate_offset(offset.0)?;
        let text = self.text();
        for (line, bounds) in LineBounds::all(&text).enumerate() {
            if offset.0 <= bounds.content_end {
                return Ok(LineColumn::new(line, offset.0 - bounds.start));
            }
            if offset.0 < bounds.next_start {
                return Err(TextError::InvalidByteBoundary { offset: offset.0 });
            }
        }
        Err(TextError::ByteOutOfBounds {
            offset: offset.0,
            len: text.len(),
        })
    }

    /// Converts a line and byte column to a canonical byte offset.
    ///
    /// # Errors
    ///
    /// Rejects missing lines, columns past line content, and character splits.
    pub fn byte_of_line_column(&self, point: LineColumn) -> Result<ByteOffset, TextError> {
        let text = self.text();
        let line_count = LineBounds::all(&text).count();
        let Some(bounds) = LineBounds::all(&text).nth(point.line) else {
            return Err(TextError::LineOutOfBounds {
                line: point.line,
                line_count,
            });
        };
        let content_len = bounds.content_end - bounds.start;
        let Some(offset) = bounds.start.checked_add(point.byte_column) else {
            return Err(TextError::InvalidLineColumn(point));
        };
        if point.byte_column > content_len || !text.is_char_boundary(offset) {
            return Err(TextError::InvalidLineColumn(point));
        }
        Ok(ByteOffset(offset))
    }

    /// Converts a byte offset to a global `AppKit` UTF-16 code-unit offset.
    ///
    /// # Errors
    ///
    /// Rejects invalid byte offsets.
    pub fn appkit_utf16_of_byte(&self, offset: ByteOffset) -> Result<usize, TextError> {
        self.validate_offset(offset.0)?;
        Ok(self.text()[..offset.0].encode_utf16().count())
    }

    /// Converts a global `AppKit` UTF-16 code-unit offset to bytes.
    ///
    /// # Errors
    ///
    /// Rejects offsets beyond the snapshot and offsets inside surrogate pairs.
    pub fn byte_of_appkit_utf16(&self, utf16_offset: usize) -> Result<ByteOffset, TextError> {
        byte_of_utf16(&self.text(), utf16_offset).map(ByteOffset)
    }

    /// Converts a byte offset to an LSP line and UTF-16 column.
    ///
    /// # Errors
    ///
    /// Rejects invalid byte offsets and line-ending interior positions.
    pub fn lsp_position_of_byte(&self, offset: ByteOffset) -> Result<LspPosition, TextError> {
        let line_column = self.line_column_of_byte(offset)?;
        let text = self.text();
        let bounds =
            LineBounds::all(&text)
                .nth(line_column.line)
                .ok_or(TextError::LineOutOfBounds {
                    line: line_column.line,
                    line_count: LineBounds::all(&text).count(),
                })?;
        let prefix_end = bounds.start + line_column.byte_column;
        Ok(LspPosition::new(
            line_column.line,
            text[bounds.start..prefix_end].encode_utf16().count(),
        ))
    }

    /// Converts an LSP line and UTF-16 column to bytes.
    ///
    /// # Errors
    ///
    /// Rejects missing lines, columns beyond line content, and surrogate splits.
    pub fn byte_of_lsp_position(&self, point: LspPosition) -> Result<ByteOffset, TextError> {
        let text = self.text();
        let line_count = LineBounds::all(&text).count();
        let Some(bounds) = LineBounds::all(&text).nth(point.line) else {
            return Err(TextError::LineOutOfBounds {
                line: point.line,
                line_count,
            });
        };
        byte_of_utf16(&text[bounds.start..bounds.content_end], point.utf16_column)
            .map(|relative| ByteOffset(bounds.start + relative))
    }

    /// Converts a grapheme boundary to its zero-based grapheme index.
    ///
    /// # Errors
    ///
    /// Rejects invalid bytes and offsets inside a grapheme cluster.
    pub fn grapheme_index_of_byte(&self, offset: ByteOffset) -> Result<usize, TextError> {
        self.validate_offset(offset.0)?;
        let text = self.text();
        if offset.0 == text.len() {
            return Ok(text.graphemes(true).count());
        }
        text.grapheme_indices(true)
            .position(|(byte, _)| byte == offset.0)
            .ok_or(TextError::InvalidGraphemeBoundary { offset: offset.0 })
    }

    /// Converts a zero-based grapheme index to bytes.
    ///
    /// # Errors
    ///
    /// Rejects indices beyond the final grapheme boundary.
    pub fn byte_of_grapheme_index(&self, index: usize) -> Result<ByteOffset, TextError> {
        let text = self.text();
        if let Some((byte, _)) = text.grapheme_indices(true).nth(index) {
            return Ok(ByteOffset(byte));
        }
        if index == text.graphemes(true).count() {
            return Ok(ByteOffset(text.len()));
        }
        Err(TextError::ByteOutOfBounds {
            offset: index,
            len: text.graphemes(true).count(),
        })
    }

    fn validate_offset(&self, offset: usize) -> Result<(), TextError> {
        if offset > self.len_bytes() {
            return Err(TextError::ByteOutOfBounds {
                offset,
                len: self.len_bytes(),
            });
        }
        if !self.is_char_boundary(ByteOffset(offset)) {
            return Err(TextError::InvalidByteBoundary { offset });
        }
        Ok(())
    }

    fn validate_range(&self, range: &Range<usize>) -> Result<(), TextError> {
        if range.start > range.end {
            return Err(TextError::InvalidEditRange {
                start: range.start,
                end: range.end,
            });
        }
        self.validate_offset(range.start)?;
        self.validate_offset(range.end)
    }

    fn validate_selections(&self, selections: &SelectionSet) -> Result<(), TextError> {
        for selection in selections.as_slice() {
            self.validate_offset(selection.anchor.0)?;
            self.validate_offset(selection.head.0)?;
        }
        Ok(())
    }

    fn write_to(&self, writer: &mut File) -> io::Result<()> {
        for chunk in self.rope.chunks() {
            writer.write_all(chunk.as_bytes())?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Edit {
    range: Range<usize>,
    replacement: String,
}

/// An atomic transaction built against one exact revision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Transaction {
    base_revision: BufferRevision,
    edits: Vec<Edit>,
    selections: Option<SelectionSet>,
}

impl Transaction {
    /// Starts a transaction for an exact snapshot revision.
    #[must_use]
    pub const fn new(base_revision: BufferRevision) -> Self {
        Self {
            base_revision,
            edits: Vec::new(),
            selections: None,
        }
    }

    /// Adds a replacement in base-snapshot byte coordinates.
    ///
    /// # Errors
    ///
    /// Rejects a reversed range immediately. Bounds and overlap are checked
    /// atomically when the transaction is applied.
    pub fn replace(
        &mut self,
        range: Range<usize>,
        replacement: impl Into<String>,
    ) -> Result<(), TextError> {
        if range.start > range.end {
            return Err(TextError::InvalidEditRange {
                start: range.start,
                end: range.end,
            });
        }
        self.edits.push(Edit {
            range,
            replacement: replacement.into(),
        });
        Ok(())
    }

    /// Replaces the post-edit selections instead of transforming current ones.
    pub fn set_selections(&mut self, selections: SelectionSet) {
        self.selections = Some(selections);
    }

    /// Returns the revision this transaction requires.
    #[must_use]
    pub const fn base_revision(&self) -> BufferRevision {
        self.base_revision
    }
}

/// Evidence returned after an accepted text transaction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChangeSet {
    before: BufferRevision,
    after: BufferRevision,
    replacements: usize,
    removed_bytes: usize,
    inserted_bytes: usize,
}

impl ChangeSet {
    /// Returns the prior revision.
    #[must_use]
    pub const fn before(self) -> BufferRevision {
        self.before
    }

    /// Returns the accepted revision.
    #[must_use]
    pub const fn after(self) -> BufferRevision {
        self.after
    }

    /// Returns the number of replacements.
    #[must_use]
    pub const fn replacements(self) -> usize {
        self.replacements
    }

    /// Returns removed UTF-8 bytes.
    #[must_use]
    pub const fn removed_bytes(self) -> usize {
        self.removed_bytes
    }

    /// Returns inserted UTF-8 bytes.
    #[must_use]
    pub const fn inserted_bytes(self) -> usize {
        self.inserted_bytes
    }
}

#[derive(Clone)]
struct HistoryState {
    rope: Rope,
    selections: SelectionSet,
}

#[derive(Clone)]
struct HistoryEntry {
    before: HistoryState,
    after: HistoryState,
    changed_bytes: usize,
}

/// A local copy-on-write buffer with bounded deterministic history.
pub struct Buffer {
    rope: Rope,
    revision: BufferRevision,
    selections: SelectionSet,
    undo: VecDeque<HistoryEntry>,
    redo: VecDeque<HistoryEntry>,
    history_bytes: usize,
    history_limits: HistoryLimits,
}

impl fmt::Debug for Buffer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Buffer")
            .field("revision", &self.revision)
            .field("len_bytes", &self.rope.len_bytes())
            .field("selections", &self.selections)
            .field("history", &self.history_snapshot())
            .finish_non_exhaustive()
    }
}

impl Buffer {
    /// Creates a buffer with default bounded history.
    #[must_use]
    pub fn new(text: &str) -> Self {
        Self::with_history_limits(text, HistoryLimits::default())
    }

    /// Creates a buffer with explicit bounded history.
    #[must_use]
    pub fn with_history_limits(text: &str, history_limits: HistoryLimits) -> Self {
        Self {
            rope: Rope::from_str(text),
            revision: BufferRevision::INITIAL,
            selections: SelectionSet::caret(ByteOffset::new(0)),
            undo: VecDeque::new(),
            redo: VecDeque::new(),
            history_bytes: 0,
            history_limits,
        }
    }

    /// Returns the current revision.
    #[must_use]
    pub const fn revision(&self) -> BufferRevision {
        self.revision
    }

    /// Returns the current selections.
    #[must_use]
    pub const fn selections(&self) -> &SelectionSet {
        &self.selections
    }

    /// Returns an immutable O(1) copy-on-write snapshot.
    #[must_use]
    pub fn snapshot(&self) -> BufferSnapshot {
        BufferSnapshot {
            rope: self.rope.clone(),
            revision: self.revision,
        }
    }

    /// Returns bounded-history accounting.
    #[must_use]
    pub fn history_snapshot(&self) -> HistorySnapshot {
        HistorySnapshot {
            undo_entries: self.undo.len(),
            redo_entries: self.redo.len(),
            retained_changed_bytes: self.history_bytes,
            max_entries: self.history_limits.max_entries,
            max_bytes: self.history_limits.max_bytes,
        }
    }

    /// Applies all replacements and selection changes atomically.
    ///
    /// # Errors
    ///
    /// Rejects stale revisions, invalid or overlapping ranges, invalid
    /// selections, and revision exhaustion without changing buffer state.
    pub fn apply(&mut self, mut transaction: Transaction) -> Result<ChangeSet, TextError> {
        if transaction.base_revision != self.revision {
            return Err(TextError::StaleRevision {
                expected: self.revision,
                actual: transaction.base_revision,
            });
        }
        let next_revision = self.revision.next()?;
        transaction.edits.sort_unstable_by(|left, right| {
            left.range
                .start
                .cmp(&right.range.start)
                .then(left.range.end.cmp(&right.range.end))
        });
        let snapshot = self.snapshot();
        let mut prior: Option<&Edit> = None;
        let mut removed_bytes = 0_usize;
        let mut inserted_bytes = 0_usize;
        for edit in &transaction.edits {
            snapshot.validate_range(&edit.range)?;
            if let Some(previous) = prior
                && (edit.range.start < previous.range.end
                    || edit.range.start == previous.range.start)
            {
                return Err(TextError::InvalidEditRange {
                    start: edit.range.start,
                    end: edit.range.end,
                });
            }
            removed_bytes = removed_bytes.saturating_add(edit.range.len());
            inserted_bytes = inserted_bytes.saturating_add(edit.replacement.len());
            prior = Some(edit);
        }

        let before = HistoryState {
            rope: self.rope.clone(),
            selections: self.selections.clone(),
        };
        let mut next_rope = self.rope.clone();
        for edit in transaction.edits.iter().rev() {
            let start = next_rope.try_byte_to_char(edit.range.start).map_err(|_| {
                TextError::InvalidByteBoundary {
                    offset: edit.range.start,
                }
            })?;
            let end = next_rope.try_byte_to_char(edit.range.end).map_err(|_| {
                TextError::InvalidByteBoundary {
                    offset: edit.range.end,
                }
            })?;
            if start != end {
                next_rope.remove(start..end);
            }
            if !edit.replacement.is_empty() {
                next_rope.insert(start, &edit.replacement);
            }
        }
        let next_snapshot = BufferSnapshot {
            rope: next_rope.clone(),
            revision: next_revision,
        };
        let next_selections = transaction
            .selections
            .unwrap_or_else(|| self.selections.transformed(&transaction.edits));
        next_snapshot.validate_selections(&next_selections)?;

        let after = HistoryState {
            rope: next_rope.clone(),
            selections: next_selections.clone(),
        };
        self.rope = next_rope;
        self.selections = next_selections;
        self.revision = next_revision;
        self.clear_redo();
        self.retain_history(HistoryEntry {
            before,
            after,
            changed_bytes: removed_bytes.saturating_add(inserted_bytes),
        });
        Ok(ChangeSet {
            before: transaction.base_revision,
            after: next_revision,
            replacements: transaction.edits.len(),
            removed_bytes,
            inserted_bytes,
        })
    }

    /// Restores the state before the newest retained transaction while
    /// advancing the monotonic revision.
    ///
    /// # Errors
    ///
    /// Returns revision exhaustion without changing state.
    pub fn undo(&mut self) -> Result<bool, TextError> {
        let Some(entry) = self.undo.pop_back() else {
            return Ok(false);
        };
        let next_revision = match self.revision.next() {
            Ok(revision) => revision,
            Err(error) => {
                self.undo.push_back(entry);
                return Err(error);
            }
        };
        self.rope = entry.before.rope.clone();
        self.selections = entry.before.selections.clone();
        self.revision = next_revision;
        self.redo.push_back(entry);
        Ok(true)
    }

    /// Reapplies the newest retained undo while advancing the monotonic
    /// revision.
    ///
    /// # Errors
    ///
    /// Returns revision exhaustion without changing state.
    pub fn redo(&mut self) -> Result<bool, TextError> {
        let Some(entry) = self.redo.pop_back() else {
            return Ok(false);
        };
        let next_revision = match self.revision.next() {
            Ok(revision) => revision,
            Err(error) => {
                self.redo.push_back(entry);
                return Err(error);
            }
        };
        self.rope = entry.after.rope.clone();
        self.selections = entry.after.selections.clone();
        self.revision = next_revision;
        self.undo.push_back(entry);
        Ok(true)
    }

    fn retain_history(&mut self, entry: HistoryEntry) {
        if self.history_limits.max_entries == 0
            || self.history_limits.max_bytes == 0
            || entry.changed_bytes > self.history_limits.max_bytes
        {
            return;
        }
        self.history_bytes = self.history_bytes.saturating_add(entry.changed_bytes);
        self.undo.push_back(entry);
        while self.undo.len() > self.history_limits.max_entries
            || self.history_bytes > self.history_limits.max_bytes
        {
            if let Some(evicted) = self.undo.pop_front() {
                self.history_bytes = self.history_bytes.saturating_sub(evicted.changed_bytes);
            } else {
                break;
            }
        }
    }

    fn clear_redo(&mut self) {
        while let Some(entry) = self.redo.pop_front() {
            self.history_bytes = self.history_bytes.saturating_sub(entry.changed_bytes);
        }
    }
}

/// The observed state of the file since the last open or successful save.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExternalChange {
    /// Disk bytes still match the editor's accepted fingerprint.
    Unchanged,
    /// Disk bytes differ from the accepted fingerprint.
    Modified,
    /// The accepted file no longer exists.
    Deleted,
}

/// A structured one-file editor I/O failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FileError {
    /// An operation failed with an operating-system error kind.
    Io {
        /// Stable operation identity.
        operation: &'static str,
        /// Operating-system error classification.
        kind: io::ErrorKind,
    },
    /// The file was not valid UTF-8.
    InvalidUtf8,
    /// Disk changed after the editor accepted its prior fingerprint.
    Conflict(ExternalChange),
    /// Atomic replacement is not implemented on the current shipping target.
    UnsupportedAtomicReplace,
}

impl fmt::Display for FileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for FileError {}

/// Successful atomic-save evidence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SaveReport {
    revision: BufferRevision,
    bytes_written: usize,
}

impl SaveReport {
    /// Returns the persisted revision.
    #[must_use]
    pub const fn revision(self) -> BufferRevision {
        self.revision
    }

    /// Returns the exact persisted UTF-8 byte count.
    #[must_use]
    pub const fn bytes_written(self) -> usize {
        self.bytes_written
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FileFingerprint {
    len: usize,
    hash: u64,
}

/// One local file, its checked buffer, and conflict-aware save identity.
pub struct Editor {
    path: PathBuf,
    buffer: Buffer,
    accepted_fingerprint: FileFingerprint,
    saved_revision: BufferRevision,
}

impl fmt::Debug for Editor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Editor")
            .field("path", &self.path)
            .field("buffer", &self.buffer)
            .field("saved_revision", &self.saved_revision)
            .finish_non_exhaustive()
    }
}

impl Editor {
    /// Opens one existing UTF-8 file.
    ///
    /// # Errors
    ///
    /// Reports read errors and invalid UTF-8 without creating editor state.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, FileError> {
        let path = path.as_ref().to_path_buf();
        let bytes = fs::read(&path).map_err(|error| file_io("read", &error))?;
        let accepted_fingerprint = fingerprint(&bytes);
        let text = String::from_utf8(bytes).map_err(|_| FileError::InvalidUtf8)?;
        Ok(Self {
            path,
            buffer: Buffer::new(&text),
            accepted_fingerprint,
            saved_revision: BufferRevision::INITIAL,
        })
    }

    /// Returns the file path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the text buffer.
    #[must_use]
    pub const fn buffer(&self) -> &Buffer {
        &self.buffer
    }

    /// Returns mutable access to the text buffer.
    pub const fn buffer_mut(&mut self) -> &mut Buffer {
        &mut self.buffer
    }

    /// Returns whether the local revision differs from the saved revision.
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.buffer.revision != self.saved_revision
    }

    /// Compares current disk bytes with the last accepted fingerprint.
    ///
    /// # Errors
    ///
    /// Reports disk reads other than not-found structurally.
    pub fn external_change(&self) -> Result<ExternalChange, FileError> {
        match fs::read(&self.path) {
            Ok(bytes) if fingerprint(&bytes) == self.accepted_fingerprint => {
                Ok(ExternalChange::Unchanged)
            }
            Ok(_) => Ok(ExternalChange::Modified),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(ExternalChange::Deleted),
            Err(error) => Err(file_io("read", &error)),
        }
    }

    /// Atomically replaces the file only while its disk fingerprint remains
    /// unchanged.
    ///
    /// # Errors
    ///
    /// Reports external conflicts, temporary-file failures, write failures,
    /// synchronization failures, and replacement failures. A pre-replacement
    /// failure leaves the accepted file bytes unchanged.
    pub fn save(&mut self) -> Result<SaveReport, FileError> {
        let external = self.external_change()?;
        if external != ExternalChange::Unchanged {
            return Err(FileError::Conflict(external));
        }
        let snapshot = self.buffer.snapshot();
        let accepted = self.accepted_fingerprint;
        atomic_replace(&self.path, accepted, |file| snapshot.write_to(file))?;
        let bytes = fs::read(&self.path).map_err(|error| file_io("verify", &error))?;
        self.accepted_fingerprint = fingerprint(&bytes);
        self.saved_revision = snapshot.revision;
        Ok(SaveReport {
            revision: snapshot.revision,
            bytes_written: snapshot.len_bytes(),
        })
    }
}

fn transform_offset(offset: usize, edits: &[Edit]) -> usize {
    let mut transformed = offset;
    let mut prior_delta: i128 = 0;
    for edit in edits {
        let start = i128::try_from(edit.range.start).unwrap_or(i128::MAX);
        let end = i128::try_from(edit.range.end).unwrap_or(i128::MAX);
        let source = i128::try_from(offset).unwrap_or(i128::MAX);
        let inserted = i128::try_from(edit.replacement.len()).unwrap_or(i128::MAX);
        let removed = end.saturating_sub(start);
        if source < start {
            break;
        }
        if source == start && start != end {
            return usize::try_from(start.saturating_add(prior_delta)).unwrap_or(usize::MAX);
        }
        if source <= end {
            let result = start.saturating_add(prior_delta).saturating_add(inserted);
            return usize::try_from(result).unwrap_or(usize::MAX);
        }
        prior_delta = prior_delta.saturating_add(inserted.saturating_sub(removed));
        transformed = usize::try_from(source.saturating_add(prior_delta)).unwrap_or(usize::MAX);
    }
    transformed
}

fn byte_of_utf16(text: &str, target: usize) -> Result<usize, TextError> {
    let mut code_units = 0_usize;
    for (byte, character) in text.char_indices() {
        if code_units == target {
            return Ok(byte);
        }
        let next = code_units.saturating_add(character.len_utf16());
        if target < next {
            return Err(TextError::InvalidUtf16Boundary { offset: target });
        }
        code_units = next;
    }
    if code_units == target {
        Ok(text.len())
    } else {
        Err(TextError::InvalidUtf16Boundary { offset: target })
    }
}

#[derive(Clone, Copy)]
struct LineBounds {
    start: usize,
    content_end: usize,
    next_start: usize,
}

impl LineBounds {
    fn all(text: &str) -> LineBoundsIter<'_> {
        LineBoundsIter {
            text,
            cursor: 0,
            yielded_terminal: false,
        }
    }
}

struct LineBoundsIter<'a> {
    text: &'a str,
    cursor: usize,
    yielded_terminal: bool,
}

impl Iterator for LineBoundsIter<'_> {
    type Item = LineBounds;

    fn next(&mut self) -> Option<Self::Item> {
        if self.cursor > self.text.len() || self.yielded_terminal {
            return None;
        }
        let start = self.cursor;
        let rest = &self.text[start..];
        for (relative, character) in rest.char_indices() {
            let terminator = match character {
                '\r' => {
                    let after = start + relative + 1;
                    if self.text.as_bytes().get(after) == Some(&b'\n') {
                        2
                    } else {
                        1
                    }
                }
                '\n' => 1,
                '\u{0085}' | '\u{2028}' | '\u{2029}' => character.len_utf8(),
                _ => continue,
            };
            let content_end = start + relative;
            self.cursor = content_end + terminator;
            return Some(LineBounds {
                start,
                content_end,
                next_start: self.cursor,
            });
        }
        self.yielded_terminal = true;
        Some(LineBounds {
            start,
            content_end: self.text.len(),
            next_start: self.text.len(),
        })
    }
}

fn fingerprint(bytes: &[u8]) -> FileFingerprint {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    FileFingerprint {
        len: bytes.len(),
        hash,
    }
}

fn file_io(operation: &'static str, error: &io::Error) -> FileError {
    FileError::Io {
        operation,
        kind: error.kind(),
    }
}

#[cfg(target_family = "windows")]
fn atomic_replace<F>(_path: &Path, _accepted: FileFingerprint, _write: F) -> Result<(), FileError>
where
    F: FnOnce(&mut File) -> io::Result<()>,
{
    Err(FileError::UnsupportedAtomicReplace)
}

#[cfg(not(target_family = "windows"))]
fn atomic_replace<F>(path: &Path, accepted: FileFingerprint, write: F) -> Result<(), FileError>
where
    F: FnOnce(&mut File) -> io::Result<()>,
{
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path.file_name().ok_or(FileError::Io {
        operation: "temporary-name",
        kind: io::ErrorKind::InvalidInput,
    })?;
    let mut created: Option<(PathBuf, File)> = None;
    for _ in 0..TEMPORARY_FILE_ATTEMPTS {
        let sequence = TEMPORARY_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let mut temporary_name = OsString::from(".");
        temporary_name.push(file_name);
        temporary_name.push(format!(".alpine-save-{}-{sequence}", std::process::id()));
        let temporary_path = parent.join(temporary_name);
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary_path)
        {
            Ok(file) => {
                created = Some((temporary_path, file));
                break;
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(file_io("create-temporary", &error)),
        }
    }
    let (temporary_path, mut temporary_file) = created.ok_or(FileError::Io {
        operation: "create-temporary",
        kind: io::ErrorKind::AlreadyExists,
    })?;

    let operation = (|| {
        let permissions = fs::metadata(path)
            .map_err(|error| file_io("metadata", &error))?
            .permissions();
        fs::set_permissions(&temporary_path, permissions)
            .map_err(|error| file_io("permissions", &error))?;
        write(&mut temporary_file).map_err(|error| file_io("write", &error))?;
        temporary_file
            .flush()
            .map_err(|error| file_io("flush", &error))?;
        temporary_file
            .sync_all()
            .map_err(|error| file_io("sync", &error))?;
        drop(temporary_file);
        let current = fs::read(path).map_err(|error| file_io("conflict-check", &error))?;
        if fingerprint(&current) != accepted {
            return Err(FileError::Conflict(ExternalChange::Modified));
        }
        fs::rename(&temporary_path, path).map_err(|error| file_io("replace", &error))?;
        Ok(())
    })();

    if operation.is_err() {
        let _ = fs::remove_file(&temporary_path);
    }
    operation
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_PATH_SEQUENCE: AtomicU64 = AtomicU64::new(1);

    fn transaction(buffer: &Buffer, range: Range<usize>, replacement: &str) -> Transaction {
        let mut transaction = Transaction::new(buffer.revision());
        assert!(transaction.replace(range, replacement).is_ok());
        transaction
    }

    fn sample_index(seed: u64, len: usize) -> usize {
        let bytes = seed.to_le_bytes();
        usize::from(u16::from_le_bytes([bytes[0], bytes[1]])) % len
    }

    fn test_directory() -> Result<PathBuf, FileError> {
        let sequence = TEST_PATH_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("alpine-text-{}-{sequence}", std::process::id()));
        fs::create_dir(&path).map_err(|error| file_io("test-directory", &error))?;
        Ok(path)
    }

    #[test]
    fn snapshots_remain_immutable_and_rejected_edits_are_atomic() -> Result<(), TextError> {
        let mut buffer = Buffer::new("hello 😀");
        let before = buffer.snapshot();
        let change = buffer.apply(transaction(&buffer, 6..10, "Alpine"))?;
        assert_eq!(change.before(), BufferRevision::INITIAL);
        assert_eq!(change.after().get(), 1);
        assert_eq!(before.text(), "hello 😀");
        assert_eq!(buffer.snapshot().text(), "hello Alpine");

        let state = buffer.snapshot().text();
        let revision = buffer.revision();
        let invalid = transaction(&buffer, 99..99, "x");
        assert!(matches!(
            buffer.apply(invalid),
            Err(TextError::ByteOutOfBounds { offset: 99, .. })
        ));
        assert_eq!(buffer.snapshot().text(), state);
        assert_eq!(buffer.revision(), revision);

        let obsolete_transaction = Transaction::new(BufferRevision::INITIAL);
        assert!(matches!(
            buffer.apply(obsolete_transaction),
            Err(TextError::StaleRevision { .. })
        ));
        Ok(())
    }

    #[test]
    fn transaction_overlap_and_selection_transform_are_deterministic() -> Result<(), TextError> {
        let mut buffer = Buffer::new("abcdef");
        let mut selections = Transaction::new(buffer.revision());
        selections.set_selections(SelectionSet::new(vec![Selection::new(
            ByteOffset::new(2),
            ByteOffset::new(5),
        )])?);
        buffer.apply(selections)?;

        let mut edit = Transaction::new(buffer.revision());
        edit.replace(1..3, "XYZ")?;
        edit.replace(5..6, "!")?;
        buffer.apply(edit)?;
        assert_eq!(buffer.snapshot().text(), "aXYZde!");
        assert_eq!(
            buffer.selections().as_slice(),
            &[Selection::new(ByteOffset::new(4), ByteOffset::new(6))]
        );

        let before = buffer.snapshot().text();
        let mut overlap = Transaction::new(buffer.revision());
        overlap.replace(0..2, "x")?;
        overlap.replace(1..3, "y")?;
        assert!(matches!(
            buffer.apply(overlap),
            Err(TextError::InvalidEditRange { .. })
        ));
        assert_eq!(buffer.snapshot().text(), before);
        Ok(())
    }

    #[test]
    fn undo_redo_are_bounded_and_revisions_never_move_backward() -> Result<(), TextError> {
        let mut buffer = Buffer::with_history_limits("a", HistoryLimits::new(2, 2));
        buffer.apply(transaction(&buffer, 1..1, "b"))?;
        buffer.apply(transaction(&buffer, 2..2, "c"))?;
        buffer.apply(transaction(&buffer, 3..3, "d"))?;
        assert_eq!(buffer.snapshot().text(), "abcd");
        assert_eq!(buffer.history_snapshot().undo_entries(), 2);
        let edited_revision = buffer.revision();
        assert!(buffer.undo()?);
        assert_eq!(buffer.snapshot().text(), "abc");
        assert!(buffer.revision() > edited_revision);
        assert!(buffer.undo()?);
        assert_eq!(buffer.snapshot().text(), "ab");
        assert!(!buffer.undo()?);
        assert!(buffer.redo()?);
        assert_eq!(buffer.snapshot().text(), "abc");
        assert!(buffer.history_snapshot().retained_changed_bytes() <= 2);
        Ok(())
    }

    #[test]
    fn coordinate_systems_reject_ambiguous_unicode_boundaries() -> Result<(), TextError> {
        let snapshot = Buffer::new("a😀e\u{301}\r\nnext").snapshot();
        assert_eq!(snapshot.appkit_utf16_of_byte(ByteOffset::new(5))?, 3);
        assert_eq!(snapshot.byte_of_appkit_utf16(3)?, ByteOffset::new(5));
        assert!(matches!(
            snapshot.byte_of_appkit_utf16(2),
            Err(TextError::InvalidUtf16Boundary { offset: 2 })
        ));
        assert_eq!(
            snapshot.lsp_position_of_byte(ByteOffset::new(5))?,
            LspPosition::new(0, 3)
        );
        assert_eq!(
            snapshot.byte_of_lsp_position(LspPosition::new(0, 3))?,
            ByteOffset::new(5)
        );
        assert_eq!(snapshot.grapheme_index_of_byte(ByteOffset::new(1))?, 1);
        assert!(matches!(
            snapshot.grapheme_index_of_byte(ByteOffset::new(6)),
            Err(TextError::InvalidGraphemeBoundary { offset: 6 })
        ));
        assert_eq!(
            snapshot.byte_of_line_column(LineColumn::new(1, 4))?,
            ByteOffset::new(snapshot.len_bytes())
        );
        assert!(matches!(
            snapshot.line_column_of_byte(ByteOffset::new(9)),
            Err(TextError::InvalidByteBoundary { .. })
        ));
        Ok(())
    }

    #[test]
    fn random_edits_match_an_independent_string_oracle() -> Result<(), TextError> {
        let mut seed = 0x4d59_5df4_d0f3_3173_u64;
        let mut oracle = String::from("alpha\r\nβeta\n😀");
        let mut buffer = Buffer::new(&oracle);
        let replacements = ["", "x", "😀", "\n", "e\u{301}"];
        for _ in 0..1_000 {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            let boundaries: Vec<usize> = oracle
                .char_indices()
                .map(|(byte, _)| byte)
                .chain(std::iter::once(oracle.len()))
                .collect();
            let first = boundaries[sample_index(seed, boundaries.len())];
            seed = seed.rotate_left(17).wrapping_add(0x9e37_79b9);
            let second = boundaries[sample_index(seed, boundaries.len())];
            let range = first.min(second)..first.max(second);
            let replacement = replacements[sample_index(seed.rotate_left(11), replacements.len())];
            oracle.replace_range(range.clone(), replacement);
            buffer.apply(transaction(&buffer, range, replacement))?;
            assert_eq!(buffer.snapshot().text(), oracle);
        }
        Ok(())
    }

    #[cfg(not(target_family = "windows"))]
    #[test]
    fn editor_saves_atomically_and_detects_external_change()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = test_directory()?;
        let path = directory.join("document.txt");
        fs::write(&path, "before")?;
        let mut editor = Editor::open(&path)?;
        let replace = transaction(editor.buffer(), 0..6, "after");
        editor.buffer_mut().apply(replace)?;
        let report = editor.save()?;
        assert_eq!(report.bytes_written(), 5);
        assert_eq!(fs::read_to_string(&path)?, "after");
        assert!(!editor.is_dirty());

        let append = transaction(editor.buffer(), 5..5, " local");
        editor.buffer_mut().apply(append)?;
        fs::write(&path, "external")?;
        assert_eq!(editor.external_change()?, ExternalChange::Modified);
        assert_eq!(
            editor.save(),
            Err(FileError::Conflict(ExternalChange::Modified))
        );
        assert_eq!(fs::read_to_string(&path)?, "external");
        fs::remove_dir_all(directory)?;
        Ok(())
    }

    #[cfg(not(target_family = "windows"))]
    #[test]
    fn injected_write_failure_preserves_target_and_cleans_temporary_file()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = test_directory()?;
        let path = directory.join("document.txt");
        fs::write(&path, "accepted")?;
        let accepted = fingerprint(b"accepted");
        let result = atomic_replace(&path, accepted, |_file| {
            Err(io::Error::new(io::ErrorKind::StorageFull, "injected"))
        });
        assert!(matches!(
            result,
            Err(FileError::Io {
                operation: "write",
                kind: io::ErrorKind::StorageFull,
            })
        ));
        assert_eq!(fs::read_to_string(&path)?, "accepted");
        assert_eq!(fs::read_dir(&directory)?.count(), 1);
        fs::remove_dir_all(directory)?;
        Ok(())
    }

    #[test]
    fn large_snapshot_stays_independent_after_small_edit() -> Result<(), TextError> {
        let text = "line of text\n".repeat(100_000);
        let mut buffer = Buffer::new(&text);
        let snapshot = buffer.snapshot();
        buffer.apply(transaction(&buffer, 0..4, "row"))?;
        assert_eq!(snapshot.len_bytes(), text.len());
        assert_eq!(snapshot.slice(0..4)?, "line");
        assert_eq!(buffer.snapshot().slice(0..3)?, "row");
        assert!(buffer.history_snapshot().retained_changed_bytes() <= 7);
        Ok(())
    }
}
