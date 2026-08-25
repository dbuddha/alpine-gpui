//! Bounded Rust document and workspace symbol admission and picker state.

use std::{error::Error, fmt, mem::size_of, ops::Range};

use serde_json::{Value, value::RawValue};

use crate::{
    lsp_language::parse_range,
    rust_navigation::{NavigationError, SourceLocation},
};

const MAX_SYMBOL_WIRE_BYTES: usize = 1_048_576;
pub(crate) const MAX_SYMBOL_ITEMS: usize = 512;
pub(crate) const MAX_SYMBOL_DEPTH: usize = 32;
pub(crate) const MAX_SYMBOL_LABEL_BYTES: usize = 1_024;
pub(crate) const MAX_SYMBOL_QUERY_BYTES: usize = 256;
pub(crate) const MAX_SYMBOL_RETAINED_BYTES: usize = 512 * 1_024;
pub(crate) const MAX_VISIBLE_SYMBOL_ROWS: usize = 12;
const MAX_SYMBOL_BATCH_RETAINED_BYTES: usize = MAX_SYMBOL_RETAINED_BYTES
    - MAX_SYMBOL_ITEMS * size_of::<SymbolMatch>()
    - 2 * MAX_SYMBOL_QUERY_BYTES;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SymbolRequestKind {
    Document,
    Workspace,
}

impl SymbolRequestKind {
    pub(crate) const fn method(self) -> &'static str {
        match self {
            Self::Document => "textDocument/documentSymbol",
            Self::Workspace => "workspace/symbol",
        }
    }

    pub(crate) fn from_method(method: &str) -> Option<Self> {
        match method {
            "textDocument/documentSymbol" => Some(Self::Document),
            "workspace/symbol" => Some(Self::Workspace),
            _ => None,
        }
    }

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Document => "Rust document symbols",
            Self::Workspace => "Rust workspace symbols",
        }
    }

    pub(crate) const fn empty_status(self) -> &'static str {
        match self {
            Self::Document => "No Rust document symbols.",
            Self::Workspace => "No Rust workspace symbols.",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SymbolError {
    WireTooLarge,
    Malformed,
    InvalidKind,
    InvalidRange,
    HierarchyTooDeep,
    LabelTooLong,
    QueryTooLong,
    InvalidComposition,
    RevisionExhausted,
    RetentionExceeded,
    AllocationFailed,
    Navigation(NavigationError),
}

impl fmt::Display for SymbolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Rust symbols rejected input: {self:?}")
    }
}

impl Error for SymbolError {}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SymbolItem {
    label: Box<str>,
    location: SourceLocation,
    depth: u8,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SymbolBatch {
    items: Box<[SymbolItem]>,
    retained_bytes: usize,
    omitted: usize,
}

impl SymbolBatch {
    pub(crate) fn admit(
        kind: SymbolRequestKind,
        result: &RawValue,
        document_uri: &str,
    ) -> Result<Self, SymbolError> {
        if result.get().len() > MAX_SYMBOL_WIRE_BYTES {
            return Err(SymbolError::WireTooLarge);
        }
        let value: Value =
            serde_json::from_str(result.get()).map_err(|_| SymbolError::Malformed)?;
        if value.is_null() {
            return Ok(Self::empty());
        }
        let values = value.as_array().ok_or(SymbolError::Malformed)?;
        let mut collector = SymbolCollector::new()?;
        match kind {
            SymbolRequestKind::Document => {
                for value in values {
                    collector.visit_document(value, document_uri, 0)?;
                }
            }
            SymbolRequestKind::Workspace => {
                for value in values {
                    collector.visit_workspace(value)?;
                }
            }
        }
        Ok(collector.finish())
    }

    fn empty() -> Self {
        Self {
            items: Box::new([]),
            retained_bytes: 0,
            omitted: 0,
        }
    }

    pub(crate) const fn retained_bytes(&self) -> usize {
        self.retained_bytes
    }

    pub(crate) const fn omitted(&self) -> usize {
        self.omitted
    }

    pub(crate) fn len(&self) -> usize {
        self.items.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    #[cfg(test)]
    pub(crate) fn oversized_for_test() -> Self {
        Self {
            items: Box::new([]),
            retained_bytes: MAX_SYMBOL_RETAINED_BYTES.saturating_add(1),
            omitted: 0,
        }
    }
}

struct SymbolCollector {
    items: Vec<SymbolItem>,
    retained_bytes: usize,
    omitted: usize,
}

impl SymbolCollector {
    fn new() -> Result<Self, SymbolError> {
        let mut items = Vec::new();
        items
            .try_reserve(32)
            .map_err(|_| SymbolError::AllocationFailed)?;
        Ok(Self {
            items,
            retained_bytes: 0,
            omitted: 0,
        })
    }

    fn visit_document(
        &mut self,
        value: &Value,
        document_uri: &str,
        depth: usize,
    ) -> Result<(), SymbolError> {
        if depth > MAX_SYMBOL_DEPTH {
            return Err(SymbolError::HierarchyTooDeep);
        }
        let object = value.as_object().ok_or(SymbolError::Malformed)?;
        let name = required_label(object.get("name"))?;
        let detail = optional_label(object.get("detail"))?;
        validate_symbol_kind(object.get("kind"))?;
        let range = parse_range(
            object
                .get("selectionRange")
                .or_else(|| object.get("range"))
                .ok_or(SymbolError::Malformed)?,
        )
        .map_err(|_| SymbolError::InvalidRange)?;
        let label = display_label(depth, name, detail)?;
        let location = SourceLocation::new(document_uri, range).map_err(SymbolError::Navigation)?;
        self.push(label, location, depth)?;
        if let Some(children) = object.get("children") {
            let children = children.as_array().ok_or(SymbolError::Malformed)?;
            let child_depth = depth.checked_add(1).ok_or(SymbolError::HierarchyTooDeep)?;
            for child in children {
                self.visit_document(child, document_uri, child_depth)?;
            }
        }
        Ok(())
    }

    fn visit_workspace(&mut self, value: &Value) -> Result<(), SymbolError> {
        let object = value.as_object().ok_or(SymbolError::Malformed)?;
        let name = required_label(object.get("name"))?;
        let container = optional_label(object.get("containerName"))?;
        validate_symbol_kind(object.get("kind"))?;
        let location = object
            .get("location")
            .and_then(Value::as_object)
            .ok_or(SymbolError::Malformed)?;
        let uri = location
            .get("uri")
            .and_then(Value::as_str)
            .ok_or(SymbolError::Malformed)?;
        let range = parse_range(location.get("range").ok_or(SymbolError::Malformed)?)
            .map_err(|_| SymbolError::InvalidRange)?;
        let label = display_label(0, name, container)?;
        let location = SourceLocation::new(uri, range).map_err(SymbolError::Navigation)?;
        self.push(label, location, 0)
    }

    fn push(
        &mut self,
        label: Box<str>,
        location: SourceLocation,
        depth: usize,
    ) -> Result<(), SymbolError> {
        let retained = size_of::<SymbolItem>()
            .checked_add(label.len())
            .and_then(|bytes| bytes.checked_add(location.uri().len()))
            .ok_or(SymbolError::RetentionExceeded)?;
        let next = self
            .retained_bytes
            .checked_add(retained)
            .ok_or(SymbolError::RetentionExceeded)?;
        if self.items.len() == MAX_SYMBOL_ITEMS || next > MAX_SYMBOL_BATCH_RETAINED_BYTES {
            self.omitted = self.omitted.saturating_add(1);
            return Ok(());
        }
        let depth = u8::try_from(depth).map_err(|_| SymbolError::HierarchyTooDeep)?;
        self.retained_bytes = next;
        self.items.push(SymbolItem {
            label,
            location,
            depth,
        });
        Ok(())
    }

    fn finish(self) -> SymbolBatch {
        SymbolBatch {
            items: self.items.into_boxed_slice(),
            retained_bytes: self.retained_bytes,
            omitted: self.omitted,
        }
    }
}

fn required_label(value: Option<&Value>) -> Result<&str, SymbolError> {
    let label = value
        .and_then(Value::as_str)
        .ok_or(SymbolError::Malformed)?;
    validate_label(label)?;
    Ok(label)
}

fn optional_label(value: Option<&Value>) -> Result<Option<&str>, SymbolError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let label = value.as_str().ok_or(SymbolError::Malformed)?;
    if label.is_empty() {
        return Ok(None);
    }
    validate_label(label)?;
    Ok(Some(label))
}

fn validate_label(label: &str) -> Result<(), SymbolError> {
    if label.is_empty()
        || label.len() > MAX_SYMBOL_LABEL_BYTES
        || label.chars().any(char::is_control)
    {
        return Err(SymbolError::LabelTooLong);
    }
    Ok(())
}

fn validate_symbol_kind(value: Option<&Value>) -> Result<(), SymbolError> {
    if value
        .and_then(Value::as_u64)
        .is_none_or(|kind| !(1..=26).contains(&kind))
    {
        return Err(SymbolError::InvalidKind);
    }
    Ok(())
}

fn display_label(depth: usize, name: &str, detail: Option<&str>) -> Result<Box<str>, SymbolError> {
    let indent = depth.checked_mul(2).ok_or(SymbolError::LabelTooLong)?;
    let detail_bytes = detail.map_or(0, |value| value.len().saturating_add(2));
    let bytes = indent
        .checked_add(name.len())
        .and_then(|value| value.checked_add(detail_bytes))
        .ok_or(SymbolError::LabelTooLong)?;
    if bytes > MAX_SYMBOL_LABEL_BYTES {
        return Err(SymbolError::LabelTooLong);
    }
    let mut label = String::new();
    label
        .try_reserve_exact(bytes)
        .map_err(|_| SymbolError::AllocationFailed)?;
    for _ in 0..indent {
        label.push(' ');
    }
    label.push_str(name);
    if let Some(detail) = detail {
        label.push_str("  ");
        label.push_str(detail);
    }
    Ok(label.into_boxed_str())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SymbolMatch {
    item: u16,
    rank: u8,
    gaps: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SymbolRow<'a> {
    pub(crate) label: &'a str,
    pub(crate) selected: bool,
    pub(crate) depth: u8,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct SymbolPickerReport {
    pub(crate) items: usize,
    pub(crate) matches: usize,
    pub(crate) retained_bytes: usize,
    pub(crate) peak_retained_bytes: usize,
    pub(crate) omitted: usize,
    pub(crate) query_bytes: usize,
    pub(crate) composition_bytes: usize,
    pub(crate) query_revision: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SymbolPicker {
    kind: SymbolRequestKind,
    query: String,
    composition: Option<String>,
    query_revision: u64,
    batch: SymbolBatch,
    matches: Vec<SymbolMatch>,
    selected: usize,
    first_visible: usize,
    peak_retained_bytes: usize,
}

impl SymbolPicker {
    pub(crate) fn new(kind: SymbolRequestKind) -> Self {
        Self {
            kind,
            query: String::new(),
            composition: None,
            query_revision: 1,
            batch: SymbolBatch::empty(),
            matches: Vec::new(),
            selected: 0,
            first_visible: 0,
            peak_retained_bytes: 0,
        }
    }

    pub(crate) const fn kind(&self) -> SymbolRequestKind {
        self.kind
    }

    pub(crate) fn query(&self) -> &str {
        &self.query
    }

    pub(crate) const fn query_revision(&self) -> u64 {
        self.query_revision
    }

    pub(crate) fn admit(&mut self, batch: SymbolBatch) -> Result<bool, SymbolError> {
        let matches = rank_matches(&batch, &self.query)?;
        let retained_bytes = batch
            .retained_bytes()
            .saturating_add(self.query.capacity())
            .saturating_add(self.composition.as_ref().map_or(0, String::capacity))
            .saturating_add(matches.capacity().saturating_mul(size_of::<SymbolMatch>()));
        if retained_bytes > MAX_SYMBOL_RETAINED_BYTES {
            return Err(SymbolError::RetentionExceeded);
        }
        self.batch = batch;
        self.matches = matches;
        self.selected = 0;
        self.first_visible = 0;
        self.update_peak();
        Ok(true)
    }

    pub(crate) fn clear_results(&mut self) -> bool {
        let changed = !self.batch.is_empty() || !self.matches.is_empty();
        self.batch = SymbolBatch::empty();
        self.matches.clear();
        self.selected = 0;
        self.first_visible = 0;
        changed
    }

    pub(crate) fn commit_text(&mut self, text: &str) -> Result<bool, SymbolError> {
        let next = self
            .query
            .len()
            .checked_add(text.len())
            .ok_or(SymbolError::QueryTooLong)?;
        if next > MAX_SYMBOL_QUERY_BYTES || text.chars().any(char::is_control) {
            return Err(SymbolError::QueryTooLong);
        }
        if text.is_empty() {
            return Ok(false);
        }
        let mut query = bounded_string(next)?;
        query.push_str(&self.query);
        query.push_str(text);
        self.replace_query(query)?;
        Ok(true)
    }

    pub(crate) fn delete_backward(&mut self) -> Result<bool, SymbolError> {
        if self.query.is_empty() {
            return Ok(false);
        }
        let mut query = bounded_string(self.query.len())?;
        query.push_str(&self.query);
        let removed = query.pop();
        debug_assert!(removed.is_some());
        self.replace_query(query)?;
        Ok(true)
    }

    pub(crate) fn begin_composition(&mut self) -> bool {
        if self.composition.is_some() {
            return false;
        }
        self.composition = Some(String::new());
        true
    }

    pub(crate) fn update_composition(
        &mut self,
        text: &str,
        selected_start_utf16: u32,
        selected_length_utf16: u32,
    ) -> Result<bool, SymbolError> {
        if self.composition.is_none() {
            return Err(SymbolError::InvalidComposition);
        }
        let selected_end = selected_start_utf16
            .checked_add(selected_length_utf16)
            .ok_or(SymbolError::InvalidComposition)?;
        let units = u32::try_from(text.encode_utf16().count())
            .map_err(|_| SymbolError::InvalidComposition)?;
        if selected_end > units
            || self.query.len().saturating_add(text.len()) > MAX_SYMBOL_QUERY_BYTES
            || text.chars().any(char::is_control)
        {
            return Err(SymbolError::InvalidComposition);
        }
        let changed = self.composition.as_deref() != Some(text);
        if !changed {
            return Ok(false);
        }
        let mut composition = bounded_string(text.len())?;
        composition.push_str(text);
        let previous = self.composition.replace(composition);
        if self.retained_bytes() > MAX_SYMBOL_RETAINED_BYTES {
            self.composition = previous;
            return Err(SymbolError::RetentionExceeded);
        }
        self.update_peak();
        Ok(true)
    }

    pub(crate) fn cancel_composition(&mut self) -> bool {
        self.composition.take().is_some()
    }

    pub(crate) fn display_text(&self) -> Result<String, SymbolError> {
        let composition = self.composition.as_deref().unwrap_or_default();
        let bytes = self.query.len().saturating_add(composition.len());
        if bytes > MAX_SYMBOL_QUERY_BYTES {
            return Err(SymbolError::QueryTooLong);
        }
        let mut value = bounded_string(bytes)?;
        value.push_str(&self.query);
        value.push_str(composition);
        Ok(value)
    }

    pub(crate) fn navigate(&mut self, delta: isize) -> bool {
        if self.matches.is_empty() {
            return false;
        }
        let previous = self.selected;
        self.selected = bounded_index(self.selected, self.matches.len(), delta);
        if selection_precedes_visible(self.selected, self.first_visible) {
            self.first_visible = self.selected;
        } else if self.selected >= self.first_visible.saturating_add(MAX_VISIBLE_SYMBOL_ROWS) {
            self.first_visible = self
                .selected
                .saturating_add(1)
                .saturating_sub(MAX_VISIBLE_SYMBOL_ROWS);
        }
        previous != self.selected
    }

    pub(crate) fn visible_range(&self) -> Range<usize> {
        let start = self.first_visible.min(self.matches.len());
        let end = start
            .saturating_add(MAX_VISIBLE_SYMBOL_ROWS)
            .min(self.matches.len());
        start..end
    }

    pub(crate) fn row(&self, match_index: usize) -> Option<SymbolRow<'_>> {
        let matched = self.matches.get(match_index)?;
        let item = self.batch.items.get(usize::from(matched.item))?;
        Some(SymbolRow {
            label: &item.label,
            selected: match_index == self.selected,
            depth: item.depth,
        })
    }

    pub(crate) fn selected_location(&self) -> Option<SourceLocation> {
        let matched = self.matches.get(self.selected)?;
        self.batch
            .items
            .get(usize::from(matched.item))
            .map(|item| item.location.clone())
    }

    pub(crate) fn accessibility_label(&self) -> std::sync::Arc<str> {
        let selected = self
            .matches
            .get(self.selected)
            .and_then(|matched| self.batch.items.get(usize::from(matched.item)))
            .map_or("none", |item| item.label.as_ref());
        std::sync::Arc::from(format!(
            "{}: {} result(s), selected {selected}",
            self.kind.label(),
            self.matches.len()
        ))
    }

    pub(crate) fn report(&self) -> SymbolPickerReport {
        SymbolPickerReport {
            items: self.batch.len(),
            matches: self.matches.len(),
            retained_bytes: self.retained_bytes(),
            peak_retained_bytes: self.peak_retained_bytes,
            omitted: self.batch.omitted(),
            query_bytes: self.query.len(),
            composition_bytes: self.composition.as_deref().map_or(0, str::len),
            query_revision: self.query_revision,
        }
    }

    fn replace_query(&mut self, query: String) -> Result<(), SymbolError> {
        let revision = self
            .query_revision
            .checked_add(1)
            .ok_or(SymbolError::RevisionExhausted)?;
        let matches = rank_matches(&self.batch, &query)?;
        let previous_query = std::mem::replace(&mut self.query, query);
        let previous_composition = self.composition.take();
        let previous_matches = std::mem::replace(&mut self.matches, matches);
        if self.retained_bytes() > MAX_SYMBOL_RETAINED_BYTES {
            self.query = previous_query;
            self.composition = previous_composition;
            self.matches = previous_matches;
            return Err(SymbolError::RetentionExceeded);
        }
        self.query_revision = revision;
        self.selected = 0;
        self.first_visible = 0;
        self.update_peak();
        Ok(())
    }

    fn retained_bytes(&self) -> usize {
        self.batch
            .retained_bytes()
            .saturating_add(self.query.capacity())
            .saturating_add(self.composition.as_ref().map_or(0, String::capacity))
            .saturating_add(
                self.matches
                    .capacity()
                    .saturating_mul(size_of::<SymbolMatch>()),
            )
    }

    fn update_peak(&mut self) {
        self.peak_retained_bytes = self.peak_retained_bytes.max(self.retained_bytes());
    }
}

const fn selection_precedes_visible(selected: usize, first_visible: usize) -> bool {
    selected < first_visible
}

fn bounded_string(capacity: usize) -> Result<String, SymbolError> {
    let mut value = String::new();
    value
        .try_reserve_exact(capacity)
        .map_err(|_| SymbolError::AllocationFailed)?;
    Ok(value)
}

fn rank_matches(batch: &SymbolBatch, query: &str) -> Result<Vec<SymbolMatch>, SymbolError> {
    let mut matches = Vec::new();
    matches
        .try_reserve_exact(batch.items.len())
        .map_err(|_| SymbolError::AllocationFailed)?;
    let query = query.trim();
    for (index, item) in batch.items.iter().enumerate() {
        let Some((rank, gaps)) = match_score(&item.label, query) else {
            continue;
        };
        matches.push(SymbolMatch {
            item: u16::try_from(index).map_err(|_| SymbolError::RetentionExceeded)?,
            rank,
            gaps,
        });
    }
    matches.sort_unstable_by_key(|matched| (matched.rank, matched.gaps, matched.item));
    Ok(matches)
}

fn bounded_index(current: usize, count: usize, delta: isize) -> usize {
    if count == 0 {
        0
    } else {
        current.saturating_add_signed(delta).min(count - 1)
    }
}

fn match_score(label: &str, query: &str) -> Option<(u8, u16)> {
    if query.is_empty() {
        return Some((0, 0));
    }
    if ascii_equal(label.trim_start(), query) {
        return Some((0, 0));
    }
    if ascii_prefix(label.trim_start(), query) {
        return Some((1, 0));
    }
    if let Some(position) = ascii_find(label, query) {
        return Some((2, u16::try_from(position).unwrap_or(u16::MAX)));
    }
    ascii_subsequence_gaps(label, query).map(|gaps| (3, gaps))
}

fn ascii_equal(left: &str, right: &str) -> bool {
    left.len() == right.len()
        && left
            .bytes()
            .zip(right.bytes())
            .all(|(left, right)| left.eq_ignore_ascii_case(&right))
}

fn ascii_prefix(value: &str, prefix: &str) -> bool {
    value.len() >= prefix.len()
        && value
            .bytes()
            .take(prefix.len())
            .zip(prefix.bytes())
            .all(|(left, right)| left.eq_ignore_ascii_case(&right))
}

fn ascii_find(value: &str, needle: &str) -> Option<usize> {
    if needle.len() > value.len() {
        return None;
    }
    (0..=value.len().saturating_sub(needle.len())).find(|start| {
        value.is_char_boundary(*start)
            && value.is_char_boundary(start.saturating_add(needle.len()))
            && ascii_equal(&value[*start..start + needle.len()], needle)
    })
}

fn ascii_subsequence_gaps(value: &str, query: &str) -> Option<u16> {
    let mut query = query.chars();
    let mut target = query.next()?;
    let mut seen = false;
    let mut gaps = 0_u16;
    for character in value.chars() {
        if symbol_character_equal(character, target) {
            seen = true;
            if let Some(next) = query.next() {
                target = next;
            } else {
                return Some(gaps);
            }
        } else if seen {
            gaps = gaps.saturating_add(1);
        }
    }
    None
}

fn symbol_character_equal(left: char, right: char) -> bool {
    left == right || (left.is_ascii() && right.is_ascii() && left.eq_ignore_ascii_case(&right))
}

pub(crate) fn workspace_symbol_params(query: &str) -> Result<Box<RawValue>, SymbolError> {
    if query.len() > MAX_SYMBOL_QUERY_BYTES || query.chars().any(char::is_control) {
        return Err(SymbolError::QueryTooLong);
    }
    RawValue::from_string(serde_json::json!({ "query": query }).to_string())
        .map_err(|_| SymbolError::Malformed)
}

#[cfg(kani)]
mod proofs {
    use super::*;

    #[cfg_attr(test, mutants::skip)] // The dedicated Kani gate executes this proof and its faulty controls.
    #[kani::proof]
    fn symbol_selection_window_remains_bounded() {
        let count: usize = kani::any();
        let current: usize = kani::any();
        let delta: isize = kani::any();
        kani::assume(count <= MAX_SYMBOL_ITEMS);
        kani::assume(count == 0 || current < count);
        let selected = bounded_index(current, count, delta);
        kani::cover!(count == 0, "empty picker");
        kani::cover!(count > 0 && selected == 0, "first result selected");
        kani::cover!(count > 1 && selected == count - 1, "last result selected");
        assert!(count == 0 || selected < count);
    }
}

#[cfg(test)]
mod tests {
    use serde_json::value::RawValue;

    use super::*;

    fn raw(value: &str) -> Box<RawValue> {
        RawValue::from_string(value.to_owned()).unwrap_or_else(|_| unreachable!())
    }

    const URI: &str = "file:///tmp/alpine/main.rs";

    #[test]
    fn hierarchical_document_symbols_flatten_in_source_order_with_depth() {
        let value = raw(
            r#"[{"name":"outer","detail":"fn()","kind":12,"range":{"start":{"line":0,"character":0},"end":{"line":4,"character":0}},"selectionRange":{"start":{"line":0,"character":3},"end":{"line":0,"character":8}},"children":[{"name":"inner","kind":13,"range":{"start":{"line":1,"character":0},"end":{"line":2,"character":0}},"selectionRange":{"start":{"line":1,"character":4},"end":{"line":1,"character":9}}}]}]"#,
        );
        let batch = SymbolBatch::admit(SymbolRequestKind::Document, &value, URI)
            .unwrap_or_else(|_| unreachable!());
        assert_eq!(batch.len(), 2);
        let mut picker = SymbolPicker::new(SymbolRequestKind::Document);
        picker.admit(batch).unwrap_or_else(|_| unreachable!());
        assert_eq!(
            picker.row(0).map(|row| (row.label, row.depth)),
            Some(("outer  fn()", 0))
        );
        assert_eq!(
            picker.row(1).map(|row| (row.label, row.depth)),
            Some(("  inner", 1))
        );
        assert!(picker.report().retained_bytes <= MAX_SYMBOL_RETAINED_BYTES);
    }

    #[test]
    fn workspace_symbols_require_resolved_local_locations() {
        let valid = raw(
            r#"[{"name":"main","kind":12,"location":{"uri":"file:///tmp/alpine/main.rs","range":{"start":{"line":0,"character":0},"end":{"line":0,"character":4}}},"containerName":"crate"}]"#,
        );
        let batch = SymbolBatch::admit(SymbolRequestKind::Workspace, &valid, URI)
            .unwrap_or_else(|_| unreachable!());
        assert_eq!(batch.len(), 1);
        let unresolved = raw(
            r#"[{"name":"main","kind":12,"location":{"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":4}}}}]"#,
        );
        assert_eq!(
            SymbolBatch::admit(SymbolRequestKind::Workspace, &unresolved, URI),
            Err(SymbolError::Malformed)
        );
    }

    #[test]
    fn picker_ranking_query_composition_and_visible_rows_are_bounded() {
        let mut values = Vec::new();
        for index in 0..20 {
            values.push(format!(
                r#"{{"name":"symbol_{index:02}","kind":12,"location":{{"uri":"{URI}","range":{{"start":{{"line":{index},"character":0}},"end":{{"line":{index},"character":1}}}}}}}}"#
            ));
        }
        let batch = SymbolBatch::admit(
            SymbolRequestKind::Workspace,
            &raw(&format!("[{}]", values.join(","))),
            URI,
        )
        .unwrap_or_else(|_| unreachable!());
        let mut picker = SymbolPicker::new(SymbolRequestKind::Workspace);
        picker.admit(batch).unwrap_or_else(|_| unreachable!());
        assert_eq!(picker.visible_range().len(), MAX_VISIBLE_SYMBOL_ROWS);
        assert!(picker.commit_text("symbol_19").unwrap_or(false));
        assert_eq!(picker.row(0).map(|row| row.label), Some("symbol_19"));
        assert!(picker.begin_composition());
        assert!(picker.update_composition("x", 1, 0).unwrap_or(false));
        assert_eq!(picker.display_text().as_deref(), Ok("symbol_19x"));
        assert!(picker.cancel_composition());
        assert!(picker.delete_backward().unwrap_or(false));
        assert!(picker.report().query_bytes <= MAX_SYMBOL_QUERY_BYTES);
    }

    #[test]
    fn malformed_depth_label_kind_range_and_wire_fail_closed() {
        assert_eq!(
            SymbolBatch::admit(SymbolRequestKind::Document, &raw("{}"), URI),
            Err(SymbolError::Malformed)
        );
        let invalid_kind = raw(
            r#"[{"name":"x","kind":0,"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":1}},"selectionRange":{"start":{"line":0,"character":0},"end":{"line":0,"character":1}}}]"#,
        );
        assert_eq!(
            SymbolBatch::admit(SymbolRequestKind::Document, &invalid_kind, URI),
            Err(SymbolError::InvalidKind)
        );
        let oversized = raw(&format!("\"{}\"", "x".repeat(MAX_SYMBOL_WIRE_BYTES)));
        assert_eq!(
            SymbolBatch::admit(SymbolRequestKind::Document, &oversized, URI),
            Err(SymbolError::WireTooLarge)
        );
        let exact_wire = raw(&format!(
            "[{}]",
            " ".repeat(MAX_SYMBOL_WIRE_BYTES.saturating_sub(2))
        ));
        assert!(
            SymbolBatch::admit(SymbolRequestKind::Document, &exact_wire, URI)
                .is_ok_and(|batch| batch.is_empty())
        );
        let long_label = raw(&format!(
            r#"[{{"name":"{}","kind":12,"range":{{"start":{{"line":0,"character":0}},"end":{{"line":0,"character":1}}}},"selectionRange":{{"start":{{"line":0,"character":0}},"end":{{"line":0,"character":1}}}}}}]"#,
            "x".repeat(MAX_SYMBOL_LABEL_BYTES + 1)
        ));
        assert_eq!(
            SymbolBatch::admit(SymbolRequestKind::Document, &long_label, URI),
            Err(SymbolError::LabelTooLong)
        );
        let invalid_range = raw(
            r#"[{"name":"x","kind":12,"range":{"start":{"line":1,"character":0},"end":{"line":0,"character":1}},"selectionRange":{"start":{"line":1,"character":0},"end":{"line":0,"character":1}}}]"#,
        );
        assert_eq!(
            SymbolBatch::admit(SymbolRequestKind::Document, &invalid_range, URI),
            Err(SymbolError::InvalidRange)
        );
        let value: Value = serde_json::from_str(
            r#"{"name":"deep","kind":12,"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":1}},"selectionRange":{"start":{"line":0,"character":0},"end":{"line":0,"character":1}}}"#,
        )
        .unwrap_or_else(|_| unreachable!());
        let mut collector = SymbolCollector::new().unwrap_or_else(|_| unreachable!());
        assert_eq!(
            collector.visit_document(&value, URI, MAX_SYMBOL_DEPTH),
            Ok(())
        );
        assert_eq!(collector.items.len(), 1);
        assert_eq!(
            collector.visit_document(&value, URI, MAX_SYMBOL_DEPTH + 1),
            Err(SymbolError::HierarchyTooDeep)
        );
    }

    #[test]
    fn item_truncation_and_unicode_subsequence_matching_are_exact() {
        let mut values = Vec::new();
        for index in 0..MAX_SYMBOL_ITEMS + 3 {
            values.push(format!(
                r#"{{"name":"symbol_{index}","kind":12,"location":{{"uri":"{URI}","range":{{"start":{{"line":0,"character":0}},"end":{{"line":0,"character":1}}}}}}}}"#
            ));
        }
        let batch = SymbolBatch::admit(
            SymbolRequestKind::Workspace,
            &raw(&format!("[{}]", values.join(","))),
            URI,
        )
        .unwrap_or_else(|_| unreachable!());
        assert!(!batch.is_empty());
        assert_eq!(batch.len(), MAX_SYMBOL_ITEMS);
        assert_eq!(batch.omitted(), 3);
        assert!(batch.retained_bytes() <= MAX_SYMBOL_BATCH_RETAINED_BYTES);
        assert_eq!(ascii_subsequence_gaps("fóoTarget", "óT"), Some(1));
        assert_eq!(ascii_subsequence_gaps("fóoTarget", "öT"), None);
    }

    #[test]
    fn request_methods_and_workspace_params_are_exact() {
        assert_eq!(
            SymbolRequestKind::from_method("textDocument/documentSymbol"),
            Some(SymbolRequestKind::Document)
        );
        assert_eq!(
            SymbolRequestKind::from_method("workspace/symbol"),
            Some(SymbolRequestKind::Workspace)
        );
        assert_eq!(
            SymbolRequestKind::from_method("workspace/executeCommand"),
            None
        );
        assert_eq!(
            workspace_symbol_params("main").map(|value| value.get().to_owned()),
            Ok(r#"{"query":"main"}"#.to_owned())
        );
    }

    #[test]
    fn defensive_labels_errors_and_wire_edges_are_explicit() {
        assert_eq!(MAX_SYMBOL_ITEMS, 512);
        assert_eq!(MAX_SYMBOL_QUERY_BYTES, 256);
        assert_eq!(MAX_SYMBOL_RETAINED_BYTES, 512 * 1_024);
        assert_eq!(
            MAX_SYMBOL_BATCH_RETAINED_BYTES
                + MAX_SYMBOL_ITEMS * size_of::<SymbolMatch>()
                + 2 * MAX_SYMBOL_QUERY_BYTES,
            MAX_SYMBOL_RETAINED_BYTES
        );
        assert_eq!(SymbolRequestKind::Document.label(), "Rust document symbols");
        assert_eq!(
            SymbolRequestKind::Workspace.label(),
            "Rust workspace symbols"
        );
        assert_eq!(
            SymbolRequestKind::Document.empty_status(),
            "No Rust document symbols."
        );
        assert_eq!(
            SymbolRequestKind::Workspace.empty_status(),
            "No Rust workspace symbols."
        );
        assert_eq!(
            SymbolBatch::admit(SymbolRequestKind::Document, &raw("null"), URI),
            Ok(SymbolBatch::empty())
        );
        assert_eq!(optional_label(None), Ok(None));
        assert_eq!(
            optional_label(Some(&Value::String(String::new()))),
            Ok(None)
        );
        assert_eq!(validate_label(""), Err(SymbolError::LabelTooLong));
        assert_eq!(validate_label(&"x".repeat(MAX_SYMBOL_LABEL_BYTES)), Ok(()));
        assert_eq!(
            validate_label(&"x".repeat(MAX_SYMBOL_LABEL_BYTES + 1)),
            Err(SymbolError::LabelTooLong)
        );
        assert_eq!(validate_label("x\n"), Err(SymbolError::LabelTooLong));
        assert_eq!(
            display_label(0, &"x".repeat(MAX_SYMBOL_LABEL_BYTES), None).map(|label| label.len()),
            Ok(MAX_SYMBOL_LABEL_BYTES)
        );
        assert_eq!(
            display_label(1, &"x".repeat(MAX_SYMBOL_LABEL_BYTES), None),
            Err(SymbolError::LabelTooLong)
        );

        let one = raw(
            r#"[{"name":"x","kind":12,"location":{"uri":"file:///tmp/x.rs","range":{"start":{"line":0,"character":0},"end":{"line":0,"character":1}}}}]"#,
        );
        let batch = SymbolBatch::admit(SymbolRequestKind::Workspace, &one, URI)
            .unwrap_or_else(|_| unreachable!());
        let item = batch.items[0].clone();
        let retained = size_of::<SymbolItem>() + item.label.len() + item.location.uri().len();
        let mut exact = SymbolCollector::new().unwrap_or_else(|_| unreachable!());
        exact.retained_bytes = MAX_SYMBOL_BATCH_RETAINED_BYTES - retained;
        exact
            .push(item.label.clone(), item.location.clone(), 0)
            .unwrap_or_else(|_| unreachable!());
        assert_eq!(exact.items.len(), 1);
        assert_eq!(exact.omitted, 0);
        assert_eq!(exact.retained_bytes, MAX_SYMBOL_BATCH_RETAINED_BYTES);

        let mut over = SymbolCollector::new().unwrap_or_else(|_| unreachable!());
        over.retained_bytes = MAX_SYMBOL_BATCH_RETAINED_BYTES - retained + 1;
        over.push(item.label, item.location, 0)
            .unwrap_or_else(|_| unreachable!());
        assert!(over.items.is_empty());
        assert_eq!(over.omitted, 1);
        for error in [
            SymbolError::WireTooLarge,
            SymbolError::Malformed,
            SymbolError::InvalidKind,
            SymbolError::InvalidRange,
            SymbolError::HierarchyTooDeep,
            SymbolError::LabelTooLong,
            SymbolError::QueryTooLong,
            SymbolError::InvalidComposition,
            SymbolError::RevisionExhausted,
            SymbolError::RetentionExceeded,
            SymbolError::AllocationFailed,
            SymbolError::Navigation(NavigationError::InvalidUtf8),
        ] {
            assert!(
                error
                    .to_string()
                    .starts_with("Rust symbols rejected input:")
            );
        }
    }

    #[test]
    // One stateful sequence proves atomic rollback across every picker resource boundary.
    #[allow(clippy::too_many_lines)]
    fn picker_defensive_state_transitions_are_atomic() {
        let mut picker = SymbolPicker::new(SymbolRequestKind::Workspace);
        assert_eq!(
            picker.update_composition("x", 1, 0),
            Err(SymbolError::InvalidComposition)
        );
        assert!(!picker.clear_results());
        assert_eq!(
            picker.admit(SymbolBatch::oversized_for_test()),
            Err(SymbolError::RetentionExceeded)
        );
        let mut exact_admission = SymbolPicker::new(SymbolRequestKind::Workspace);
        assert_eq!(
            exact_admission.admit(SymbolBatch {
                items: Box::new([]),
                retained_bytes: MAX_SYMBOL_RETAINED_BYTES,
                omitted: 0,
            }),
            Ok(true)
        );
        assert_eq!(
            exact_admission.report().retained_bytes,
            MAX_SYMBOL_RETAINED_BYTES
        );

        let one = raw(
            r#"[{"name":"x","kind":12,"location":{"uri":"file:///tmp/x.rs","range":{"start":{"line":0,"character":0},"end":{"line":0,"character":1}}}}]"#,
        );
        let mut batch_only = SymbolPicker::new(SymbolRequestKind::Workspace);
        batch_only
            .admit(
                SymbolBatch::admit(SymbolRequestKind::Workspace, &one, URI)
                    .unwrap_or_else(|_| unreachable!()),
            )
            .unwrap_or_else(|_| unreachable!());
        batch_only.matches.clear();
        assert!(batch_only.clear_results());
        let mut matches_only = SymbolPicker::new(SymbolRequestKind::Workspace);
        matches_only.matches.push(SymbolMatch {
            item: 0,
            rank: 0,
            gaps: 0,
        });
        assert!(matches_only.clear_results());

        assert_eq!(picker.commit_text(""), Ok(false));
        let exact_query = "q".repeat(MAX_SYMBOL_QUERY_BYTES);
        assert_eq!(picker.commit_text(&exact_query), Ok(true));
        assert_eq!(picker.query_revision(), 2);
        assert_eq!(picker.display_text(), Ok(exact_query));
        picker.query.clear();
        picker.query.shrink_to_fit();
        assert_eq!(picker.commit_text("\n"), Err(SymbolError::QueryTooLong));
        assert_eq!(
            picker.commit_text(&"x".repeat(MAX_SYMBOL_QUERY_BYTES + 1)),
            Err(SymbolError::QueryTooLong)
        );
        assert_eq!(picker.delete_backward(), Ok(false));
        assert!(!picker.navigate(1));
        assert!(picker.begin_composition());
        assert!(!picker.begin_composition());
        assert_eq!(
            picker.update_composition("x", u32::MAX, 1),
            Err(SymbolError::InvalidComposition)
        );
        assert_eq!(
            picker.update_composition("x", 2, 0),
            Err(SymbolError::InvalidComposition)
        );
        assert_eq!(
            picker.update_composition("\n", 0, 0),
            Err(SymbolError::InvalidComposition)
        );
        assert_eq!(picker.update_composition("x", 1, 0), Ok(true));
        assert_eq!(picker.update_composition("x", 1, 0), Ok(false));
        picker.query = "x".repeat(MAX_SYMBOL_QUERY_BYTES);
        assert_eq!(
            picker.update_composition("y", 1, 0),
            Err(SymbolError::InvalidComposition)
        );
        assert_eq!(picker.display_text(), Err(SymbolError::QueryTooLong));
        picker.query.clear();
        assert!(picker.cancel_composition());
        assert!(!picker.cancel_composition());

        let mut exact_composed_query = SymbolPicker::new(SymbolRequestKind::Workspace);
        assert_eq!(
            exact_composed_query.commit_text(&"q".repeat(MAX_SYMBOL_QUERY_BYTES - 1)),
            Ok(true)
        );
        assert!(exact_composed_query.begin_composition());
        assert_eq!(exact_composed_query.update_composition("x", 1, 0), Ok(true));
        assert_eq!(
            exact_composed_query.display_text().map(|text| text.len()),
            Ok(MAX_SYMBOL_QUERY_BYTES)
        );

        let one_byte_capacity = bounded_string(1)
            .unwrap_or_else(|_| unreachable!())
            .capacity();
        let mut exact_retention = SymbolPicker::new(SymbolRequestKind::Workspace);
        assert!(exact_retention.begin_composition());
        let fixed_bytes = exact_retention.retained_bytes();
        exact_retention.batch.retained_bytes = MAX_SYMBOL_RETAINED_BYTES
            .saturating_sub(fixed_bytes)
            .saturating_sub(one_byte_capacity);
        assert_eq!(exact_retention.update_composition("x", 1, 0), Ok(true));
        assert_eq!(exact_retention.retained_bytes(), MAX_SYMBOL_RETAINED_BYTES);

        let two_byte_capacity = bounded_string(2)
            .unwrap_or_else(|_| unreachable!())
            .capacity();
        let mut over_retention = SymbolPicker::new(SymbolRequestKind::Workspace);
        assert!(over_retention.begin_composition());
        let fixed_bytes = over_retention.retained_bytes();
        over_retention.batch.retained_bytes = MAX_SYMBOL_RETAINED_BYTES
            .saturating_sub(fixed_bytes)
            .saturating_sub(two_byte_capacity)
            .saturating_add(1);
        assert_eq!(
            over_retention.update_composition("xy", 2, 0),
            Err(SymbolError::RetentionExceeded)
        );
        assert_eq!(over_retention.composition.as_deref(), Some(""));

        picker.query_revision = u64::MAX;
        assert_eq!(picker.commit_text("x"), Err(SymbolError::RevisionExhausted));
        picker.query_revision = 1;
        picker.batch = SymbolBatch::oversized_for_test();
        assert_eq!(picker.commit_text("x"), Err(SymbolError::RetentionExceeded));
        assert!(picker.query().is_empty());
        assert!(picker.begin_composition());
        assert_eq!(
            picker.update_composition("x", 1, 0),
            Err(SymbolError::RetentionExceeded)
        );
        assert_eq!(picker.composition.as_deref(), Some(""));
        assert!(picker.cancel_composition());
        picker.batch = SymbolBatch {
            items: Box::new([]),
            retained_bytes: MAX_SYMBOL_RETAINED_BYTES,
            omitted: 0,
        };
        picker.matches.clear();
        picker.matches.shrink_to_fit();
        picker.query.clear();
        picker.query.shrink_to_fit();
        assert_eq!(picker.replace_query(String::new()), Ok(()));
        assert_eq!(
            picker.report().peak_retained_bytes,
            MAX_SYMBOL_RETAINED_BYTES
        );
        assert_eq!(
            bounded_string(usize::MAX),
            Err(SymbolError::AllocationFailed)
        );
    }

    #[test]
    fn picker_scrolling_matching_and_parameter_edges_are_discriminating() {
        let values = (0..20)
            .map(|index| format!(
                r#"{{"name":"item-{index:02}","kind":12,"location":{{"uri":"{URI}","range":{{"start":{{"line":{index},"character":0}},"end":{{"line":{index},"character":1}}}}}}}}"#
            ))
            .collect::<Vec<_>>()
            .join(",");
        let batch = SymbolBatch::admit(
            SymbolRequestKind::Workspace,
            &raw(&format!("[{values}]")),
            URI,
        )
        .unwrap_or_else(|_| unreachable!());
        let mut picker = SymbolPicker::new(SymbolRequestKind::Workspace);
        picker.admit(batch).unwrap_or_else(|_| unreachable!());
        assert!(picker.row(0).is_some_and(|row| row.selected));
        assert!(picker.row(1).is_some_and(|row| !row.selected));
        assert!(picker.navigate(isize::MAX));
        assert_eq!(picker.visible_range(), 8..20);
        assert!(picker.navigate(isize::MIN));
        assert_eq!(picker.visible_range(), 0..12);
        assert_eq!(bounded_index(9, 0, -1), 0);
        assert!(selection_precedes_visible(0, 1));
        assert!(!selection_precedes_visible(1, 1));
        assert_eq!(match_score("  Alpha", "alpha"), Some((0, 0)));
        assert_eq!(match_score("alphabet", "alp"), Some((1, 0)));
        assert_eq!(match_score("xxalpha", "alpha"), Some((2, 2)));
        assert_eq!(match_score("alphaBeta", "aB"), Some((2, 4)));
        assert_eq!(ascii_find("short", "longer"), None);
        assert_eq!(ascii_find("equal", "equal"), Some(0));
        assert_eq!(ascii_find("éa", "a"), Some(2));
        assert!(symbol_character_equal('A', 'a'));
        assert!(!symbol_character_equal('é', 'É'));
        assert!(matches!(
            workspace_symbol_params("\n"),
            Err(SymbolError::QueryTooLong)
        ));
        assert!(matches!(
            workspace_symbol_params(&"x".repeat(MAX_SYMBOL_QUERY_BYTES + 1)),
            Err(SymbolError::QueryTooLong)
        ));
        assert!(workspace_symbol_params(&"x".repeat(MAX_SYMBOL_QUERY_BYTES)).is_ok());
    }
}
