//! Private editing state shared by the bounded overlay fields.
//!
//! Owners retain their committed string and search/request revisions. Preparation
//! does not change either; accept only after the owner has admitted the new value.

use std::ops::Range;

use alpine_text::{Buffer, ByteOffset, Selection};

const MAX_HISTORY: usize = 32;
const MAX_VALUE_BYTES: usize = 4 * 1_024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EditError {
    InvalidSelection,
    TooLong { actual: usize, limit: usize },
    AllocationFailed,
}

impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "overlay edit rejected: {self:?}")
    }
}

impl std::error::Error for EditError {}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Saved {
    value: String,
    selection: Selection,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HistoryAction {
    Edit,
    Undo,
    Redo,
}

pub(crate) struct Prepared {
    pub(crate) value: String,
    selection: Selection,
    previous: Saved,
    action: HistoryAction,
    storage: HistoryStorage,
}

#[derive(Default)]
struct HistoryStorage {
    undo: Option<Vec<Saved>>,
    redo: Option<Vec<Saved>>,
}

/// Reserve this headroom in result budgets so admitted results remain editable.
pub(crate) const fn history_budget(value_limit: usize) -> usize {
    2 * (MAX_HISTORY + 1) * std::mem::size_of::<Saved>() + MAX_HISTORY * value_limit
}

fn reserved(history: &Vec<Saved>) -> Result<Option<Vec<Saved>>, EditError> {
    if history.len() < history.capacity() {
        return Ok(None);
    }
    let mut storage = Vec::new();
    storage
        .try_reserve_exact(MAX_HISTORY + 1)
        .map_err(|_| EditError::AllocationFailed)?;
    Ok(Some(storage))
}

fn install(history: &mut Vec<Saved>, storage: Option<Vec<Saved>>) {
    if let Some(mut storage) = storage {
        storage.append(history);
        *history = storage;
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct FieldEdit {
    selection: Option<Selection>,
    composition: Option<String>,
    composition_selection: Range<usize>,
    undo: Vec<Saved>,
    redo: Vec<Saved>,
}

impl FieldEdit {
    pub(crate) fn selection(&self, value: &str) -> Selection {
        self.selection
            .unwrap_or_else(|| Selection::caret(ByteOffset::new(value.len())))
    }

    pub(crate) fn set_selection(
        &mut self,
        value: &str,
        selection: Selection,
    ) -> Result<bool, EditError> {
        validate_selection(value, selection)?;
        let changed = self.selection(value) != selection || self.is_composing();
        self.selection = Some(selection);
        self.cancel_composition();
        Ok(changed)
    }

    pub(crate) fn reset_focus(&mut self) {
        self.selection = None;
        self.cancel_composition();
    }

    pub(crate) const fn is_composing(&self) -> bool {
        self.composition.is_some()
    }

    pub(crate) fn composition(&self) -> Option<&str> {
        self.composition.as_deref()
    }

    pub(crate) fn begin_composition(&mut self) -> bool {
        if self.is_composing() {
            return false;
        }
        self.composition = Some(String::new());
        self.composition_selection = 0..0;
        true
    }

    pub(crate) fn cancel_composition(&mut self) -> bool {
        self.composition.take().is_some()
    }

    pub(crate) fn update_composition(
        &mut self,
        value: &str,
        text: &str,
        start: u32,
        length: u32,
        limit: usize,
    ) -> Result<bool, EditError> {
        let result = (|| {
            let end = start
                .checked_add(length)
                .ok_or(EditError::InvalidSelection)?;
            let start = super::byte_at_utf16(text, start).ok_or(EditError::InvalidSelection)?;
            let end = super::byte_at_utf16(text, end).ok_or(EditError::InvalidSelection)?;
            let selection = self.selection(value);
            validate_selection(value, selection)?;
            check_length(value.len() - selection.range().len() + text.len(), limit)?;
            let next = copy(text)?;
            let changed =
                self.composition() != Some(text) || self.composition_selection != (start..end);
            self.composition = Some(next);
            self.composition_selection = start..end;
            Ok(changed)
        })();
        if result.is_err() {
            // AppKit has received the new preedit. Retaining an older projection
            // would let a subsequent callback act against text it cannot see.
            self.cancel_composition();
        }
        result
    }

    pub(crate) fn projected_value(&self, value: &str) -> Result<String, EditError> {
        let Some(mark) = self.composition() else {
            return copy(value);
        };
        splice(value, self.selection(value).range(), mark)
    }

    pub(crate) fn projected_selection(&self, value: &str) -> Selection {
        let selection = self.selection(value);
        if self.is_composing() {
            let start = selection.range().start;
            Selection::new(
                ByteOffset::new(start + self.composition_selection.start),
                ByteOffset::new(start + self.composition_selection.end),
            )
        } else {
            selection
        }
    }

    pub(crate) fn mark_range(&self, value: &str) -> Option<Range<usize>> {
        self.composition().map(|mark| {
            let start = self.selection(value).range().start;
            start..start + mark.len()
        })
    }

    pub(crate) fn source_index(&self, value: &str, projected: usize) -> usize {
        let Some(mark) = self.mark_range(value) else {
            return projected;
        };
        if projected <= mark.start {
            projected
        } else if projected < mark.end {
            mark.start
        } else {
            projected - mark.len() + self.selection(value).range().len()
        }
    }

    pub(crate) fn prepare(
        &mut self,
        value: &str,
        text: &str,
        caret: usize,
        limit: usize,
    ) -> Result<Prepared, EditError> {
        self.prepare_range(value, self.selection(value).range(), text, caret, limit)
    }

    fn prepare_range(
        &mut self,
        value: &str,
        range: Range<usize>,
        text: &str,
        caret: usize,
        limit: usize,
    ) -> Result<Prepared, EditError> {
        validate_selection(value, self.selection(value))?;
        if !text.is_char_boundary(caret) {
            return Err(EditError::InvalidSelection);
        }
        check_length(value.len() - range.len() + text.len(), limit)?;
        let next = splice(value, range.clone(), text)?;
        let storage = HistoryStorage {
            undo: if value == next {
                None
            } else {
                reserved(&self.undo)?
            },
            redo: None,
        };
        Ok(Prepared {
            value: next,
            selection: Selection::caret(ByteOffset::new(range.start + caret)),
            previous: Saved {
                value: copy(value)?,
                selection: self.selection(value),
            },
            action: HistoryAction::Edit,
            storage,
        })
    }

    pub(crate) fn prepare_delete(
        &mut self,
        value: &str,
        forward: bool,
        limit: usize,
    ) -> Result<Prepared, EditError> {
        let mut range = self.selection(value).range();
        validate_selection(value, self.selection(value))?;
        if range.is_empty() {
            let head = range.start;
            let index = if forward && head < value.len() {
                Some(head)
            } else if !forward && head > 0 {
                value[..head].char_indices().next_back().map(|(i, _)| i)
            } else {
                None
            };
            if let Some(index) = index {
                range = grapheme(value, index)?;
            }
        }
        self.prepare_range(value, range, "", 0, limit)
    }

    pub(crate) fn prepare_history(
        &mut self,
        value: &str,
        redo: bool,
    ) -> Result<Option<Prepared>, EditError> {
        let Some(saved) = (if redo { &self.redo } else { &self.undo }).last() else {
            return Ok(None);
        };
        Ok(Some(Prepared {
            value: copy(&saved.value)?,
            selection: saved.selection,
            previous: Saved {
                value: copy(value)?,
                selection: self.selection(value),
            },
            action: if redo {
                HistoryAction::Redo
            } else {
                HistoryAction::Undo
            },
            storage: HistoryStorage {
                undo: if redo { reserved(&self.undo)? } else { None },
                redo: if redo { None } else { reserved(&self.redo)? },
            },
        }))
    }

    pub(crate) fn accept(&mut self, prepared: Prepared, changed: bool) {
        self.selection = Some(prepared.selection);
        self.cancel_composition();
        install(&mut self.undo, prepared.storage.undo);
        install(&mut self.redo, prepared.storage.redo);
        match prepared.action {
            HistoryAction::Edit if changed => {
                self.redo.clear();
                self.undo.push(prepared.previous);
            }
            HistoryAction::Edit => {}
            HistoryAction::Undo => {
                self.undo.pop();
                self.redo.push(prepared.previous);
            }
            HistoryAction::Redo => {
                self.redo.pop();
                self.undo.push(prepared.previous);
            }
        }
        if self.undo.len() + self.redo.len() > MAX_HISTORY {
            self.undo.remove(0);
        }
    }

    pub(crate) fn move_caret(
        &mut self,
        value: &str,
        forward: bool,
        extend: bool,
        edge: bool,
    ) -> Result<bool, EditError> {
        let selection = self.selection(value);
        validate_selection(value, selection)?;
        let head = selection.head().get();
        let target = if edge {
            if forward { value.len() } else { 0 }
        } else if !extend && !selection.range().is_empty() {
            if forward {
                selection.range().end
            } else {
                selection.range().start
            }
        } else if forward && head < value.len() {
            grapheme(value, head)?.end
        } else if !forward && head > 0 {
            let index = value[..head]
                .char_indices()
                .next_back()
                .ok_or(EditError::InvalidSelection)?
                .0;
            grapheme(value, index)?.start
        } else {
            head
        };
        let target = ByteOffset::new(target);
        self.set_selection(
            value,
            if extend {
                Selection::new(selection.anchor(), target)
            } else {
                Selection::caret(target)
            },
        )
    }

    pub(crate) fn retained_bytes(&self) -> usize {
        self.composition.as_ref().map_or(0, String::capacity)
            + (self.undo.capacity() + self.redo.capacity()) * std::mem::size_of::<Saved>()
            + self
                .undo
                .iter()
                .chain(&self.redo)
                .map(|s| s.value.capacity())
                .sum::<usize>()
    }

    /// Exact retained editing state after acceptance, without allocating into
    /// the live owner before its admission check.
    pub(crate) fn retained_after(&self, prepared: &Prepared, changed: bool) -> usize {
        let undo_capacity = prepared
            .storage
            .undo
            .as_ref()
            .map_or(self.undo.capacity(), Vec::capacity);
        let redo_capacity = prepared
            .storage
            .redo
            .as_ref()
            .map_or(self.redo.capacity(), Vec::capacity);
        let sum = |history: &[Saved]| history.iter().map(|s| s.value.capacity()).sum::<usize>();
        let strings = match prepared.action {
            HistoryAction::Edit if changed => {
                let start = usize::from(self.undo.len() == MAX_HISTORY);
                sum(&self.undo[start..]) + prepared.previous.value.capacity()
            }
            HistoryAction::Edit => sum(&self.undo) + sum(&self.redo),
            HistoryAction::Undo => {
                sum(&self.undo[..self.undo.len() - 1])
                    + sum(&self.redo)
                    + prepared.previous.value.capacity()
            }
            HistoryAction::Redo => {
                sum(&self.redo[..self.redo.len() - 1])
                    + sum(&self.undo)
                    + prepared.previous.value.capacity()
            }
        };
        (undo_capacity + redo_capacity) * std::mem::size_of::<Saved>() + strings
    }
}

fn validate_selection(value: &str, selection: Selection) -> Result<(), EditError> {
    if value.is_char_boundary(selection.anchor().get())
        && value.is_char_boundary(selection.head().get())
    {
        Ok(())
    } else {
        Err(EditError::InvalidSelection)
    }
}

fn check_length(actual: usize, limit: usize) -> Result<(), EditError> {
    let limit = limit.min(MAX_VALUE_BYTES);
    if actual > limit {
        Err(EditError::TooLong { actual, limit })
    } else {
        Ok(())
    }
}

fn copy(value: &str) -> Result<String, EditError> {
    let mut result = String::new();
    result
        .try_reserve_exact(value.len())
        .map_err(|_| EditError::AllocationFailed)?;
    result.push_str(value);
    Ok(result)
}

fn splice(value: &str, range: Range<usize>, text: &str) -> Result<String, EditError> {
    let prefix = value
        .get(..range.start)
        .ok_or(EditError::InvalidSelection)?;
    let suffix = value.get(range.end..).ok_or(EditError::InvalidSelection)?;
    let mut result = String::new();
    result
        .try_reserve_exact(prefix.len() + text.len() + suffix.len())
        .map_err(|_| EditError::AllocationFailed)?;
    result.push_str(prefix);
    result.push_str(text);
    result.push_str(suffix);
    Ok(result)
}

fn grapheme(value: &str, index: usize) -> Result<Range<usize>, EditError> {
    Buffer::new(value)
        .snapshot()
        .grapheme_byte_range_at(ByteOffset::new(index))
        .map_err(|_| EditError::InvalidSelection)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(value: &mut String, edit: &mut FieldEdit, mut prepared: Prepared) {
        let changed = *value != prepared.value;
        let retained_after = edit.retained_after(&prepared, changed);
        *value = std::mem::take(&mut prepared.value);
        edit.accept(prepared, changed);
        assert_eq!(edit.retained_bytes(), retained_after);
    }

    #[test]
    fn preedit_replaces_selection_and_rejects_surrogate_interior() -> Result<(), EditError> {
        let value = "a😀z";
        let mut edit = FieldEdit::default();
        edit.set_selection(
            value,
            Selection::new(ByteOffset::new(1), ByteOffset::new(5)),
        )?;
        assert!(edit.begin_composition());
        edit.update_composition(value, "日本", 1, 1, 64)?;
        assert_eq!(edit.projected_value(value)?, "a日本z");
        assert_eq!(edit.projected_selection(value).range(), 4..7);
        assert_eq!(edit.source_index(value, 4), 1);
        assert_eq!(edit.source_index(value, 7), 5);
        assert_eq!(
            edit.update_composition(value, "😀", 1, 0, 64),
            Err(EditError::InvalidSelection)
        );
        assert!(!edit.is_composing());
        assert_eq!(edit.projected_value(value)?, value);
        assert_eq!(edit.selection(value).range(), 1..5);
        Ok(())
    }

    #[test]
    fn rejected_owner_admission_preserves_selection_and_history() -> Result<(), EditError> {
        let mut value = String::from("ab");
        let mut edit = FieldEdit::default();
        edit.set_selection(&value, Selection::caret(ByteOffset::new(1)))?;
        let retained_before = edit.retained_bytes();
        let rejected = edit.prepare(&value, "😀", 4, 64)?;
        assert_eq!(edit.retained_bytes(), retained_before);
        drop(rejected);
        assert_eq!(edit.retained_bytes(), retained_before);
        assert_eq!(value, "ab");
        assert_eq!(edit.selection(&value).head().get(), 1);
        assert!(edit.prepare_history(&value, false)?.is_none());
        let accepted = edit.prepare(&value, "😀", 0, 64)?;
        apply(&mut value, &mut edit, accepted);
        assert_eq!(value, "a😀b");
        assert_eq!(edit.selection(&value).head().get(), 1);
        let undo = edit
            .prepare_history(&value, false)?
            .ok_or(EditError::InvalidSelection)?;
        apply(&mut value, &mut edit, undo);
        assert_eq!(value, "ab");
        assert_eq!(edit.selection(&value).head().get(), 1);
        let redo = edit
            .prepare_history(&value, true)?
            .ok_or(EditError::InvalidSelection)?;
        apply(&mut value, &mut edit, redo);
        assert_eq!(value, "a😀b");
        Ok(())
    }

    #[test]
    fn delete_and_navigation_preserve_graphemes_and_undo_selection() -> Result<(), EditError> {
        let mut value = String::from("ae\u{301}👩\u{200d}💻z");
        let mut edit = FieldEdit::default();
        edit.move_caret(&value, false, false, false)?;
        let end = edit.selection(&value).head();
        let prepared = edit.prepare_delete(&value, false, 64)?;
        apply(&mut value, &mut edit, prepared);
        assert_eq!(value, "ae\u{301}z");
        let undo = edit
            .prepare_history(&value, false)?
            .ok_or(EditError::InvalidSelection)?;
        apply(&mut value, &mut edit, undo);
        assert_eq!(edit.selection(&value).head(), end);
        edit.set_selection(&value, Selection::caret(ByteOffset::new(1)))?;
        let prepared = edit.prepare_delete(&value, true, 64)?;
        apply(&mut value, &mut edit, prepared);
        assert_eq!(value, "a👩\u{200d}💻z");
        Ok(())
    }

    #[test]
    fn history_is_bounded_and_new_edit_discards_redo() -> Result<(), EditError> {
        let mut value = String::new();
        let mut edit = FieldEdit::default();
        for _ in 0..80 {
            let prepared = edit.prepare(&value, "x", 1, 256)?;
            apply(&mut value, &mut edit, prepared);
        }
        assert_eq!(edit.undo.len(), MAX_HISTORY);
        for _ in 0..MAX_HISTORY {
            let undo = edit
                .prepare_history(&value, false)?
                .ok_or(EditError::InvalidSelection)?;
            apply(&mut value, &mut edit, undo);
        }
        assert_eq!(value.len(), 48);
        assert!(edit.prepare_history(&value, false)?.is_none());
        let prepared = edit.prepare(&value, "y", 1, 256)?;
        apply(&mut value, &mut edit, prepared);
        assert!(edit.prepare_history(&value, true)?.is_none());
        assert!(edit.retained_bytes() < 16 * 1_024);
        Ok(())
    }

    #[test]
    fn replacement_limit_counts_surviving_source_and_failure_is_atomic() -> Result<(), EditError> {
        let mut value = String::from("abcd");
        let mut edit = FieldEdit::default();
        edit.set_selection(
            &value,
            Selection::new(ByteOffset::new(1), ByteOffset::new(3)),
        )?;
        assert!(matches!(
            edit.prepare(&value, "xyz", 3, 4),
            Err(EditError::TooLong {
                actual: 5,
                limit: 4
            })
        ));
        assert_eq!(edit.selection(&value).range(), 1..3);
        let prepared = edit.prepare(&value, "", 0, 4)?;
        apply(&mut value, &mut edit, prepared);
        assert_eq!(value, "ad");
        Ok(())
    }
}
