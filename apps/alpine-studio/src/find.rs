//! Bounded literal search state for Alpine Studio.

use std::{error::Error, fmt, fmt::Write as _, mem::size_of, ops::Range};

use alpine_text::{BufferSnapshot, TextError};

pub(crate) const MAX_QUERY_BYTES: usize = 4 * 1_024;
pub(crate) const MAX_SOURCE_BYTES: usize = 16 * 1_024 * 1_024;
pub(crate) const MAX_MATCHES: usize = 16_384;
pub(crate) const MAX_MATCH_METADATA_BYTES: usize = 256 * 1_024;
pub(crate) const MAX_VISIBLE_MATCHES: usize = 2_048;
pub(crate) const MAX_REPLACEMENT_TRANSACTION_BYTES: usize = 16 * 1_024 * 1_024;
const MAX_DISPLAY_BYTES: usize = 256;
const UTF8_BOUNDARY_BACKTRACK: usize = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FindLimits {
    query_bytes: usize,
    source_bytes: usize,
    matches: usize,
    match_metadata_bytes: usize,
}

impl FindLimits {
    const fn shipping() -> Self {
        Self {
            query_bytes: MAX_QUERY_BYTES,
            source_bytes: MAX_SOURCE_BYTES,
            matches: MAX_MATCHES,
            match_metadata_bytes: MAX_MATCH_METADATA_BYTES,
        }
    }

    fn match_capacity(self) -> usize {
        let metadata_capacity = self.match_metadata_bytes / size_of::<Range<usize>>();
        self.matches.min(metadata_capacity)
    }

    fn is_valid(self) -> bool {
        self.query_bytes > 0 && self.source_bytes > 0 && self.match_capacity() > 0
    }
}

impl Default for FindLimits {
    fn default() -> Self {
        Self::shipping()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FindIdentity {
    document: u64,
    buffer_revision: u64,
    generation: u64,
}

impl FindIdentity {
    const fn new(document: u64, buffer_revision: u64, generation: u64) -> Self {
        Self {
            document,
            buffer_revision,
            generation,
        }
    }
}

#[derive(Debug)]
pub(crate) enum FindError {
    InvalidLimits,
    QueryTooLong { actual: usize, limit: usize },
    ReplacementTooLong { actual: usize, limit: usize },
    IncompleteResult,
    ReplacementBudgetExceeded { actual: usize, limit: usize },
    InvalidSourceLength { retained: usize, total: usize },
    GenerationExhausted,
    OffsetOverflow,
    AllocationFailed,
    WorkerUnavailable,
    Text(TextError),
}

impl fmt::Display for FindError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLimits => formatter.write_str("find limits must be non-zero and coherent"),
            Self::QueryTooLong { actual, limit } => {
                write!(formatter, "find query is {actual} bytes; limit is {limit}")
            }
            Self::ReplacementTooLong { actual, limit } => {
                write!(formatter, "replacement is {actual} bytes; limit is {limit}")
            }
            Self::IncompleteResult => {
                formatter.write_str("replace all requires one complete current find result")
            }
            Self::ReplacementBudgetExceeded { actual, limit } => write!(
                formatter,
                "replacement transaction is {actual} bytes; limit is {limit}"
            ),
            Self::InvalidSourceLength { retained, total } => write!(
                formatter,
                "find source retained {retained} bytes but total length is {total}"
            ),
            Self::GenerationExhausted => formatter.write_str("find query generation is exhausted"),
            Self::OffsetOverflow => formatter.write_str("find match offset overflowed"),
            Self::AllocationFailed => formatter.write_str("find allocation failed"),
            Self::WorkerUnavailable => formatter.write_str("find worker admission failed"),
            Self::Text(error) => write!(formatter, "find snapshot failed: {error}"),
        }
    }
}

impl Error for FindError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Text(error) => Some(error),
            Self::InvalidLimits
            | Self::QueryTooLong { .. }
            | Self::ReplacementTooLong { .. }
            | Self::IncompleteResult
            | Self::ReplacementBudgetExceeded { .. }
            | Self::InvalidSourceLength { .. }
            | Self::GenerationExhausted
            | Self::OffsetOverflow
            | Self::AllocationFailed
            | Self::WorkerUnavailable => None,
        }
    }
}

impl From<TextError> for FindError {
    fn from(error: TextError) -> Self {
        Self::Text(error)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FindField {
    Query,
    Replacement,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FindNavigation {
    index: usize,
    wrapped: bool,
}

impl FindNavigation {
    pub(crate) const fn index(self) -> usize {
        self.index
    }

    pub(crate) const fn wrapped(self) -> bool {
        self.wrapped
    }
}

#[derive(Debug)]
pub(crate) struct FindResult {
    identity: FindIdentity,
    matches: Box<[Range<usize>]>,
    source_bytes: usize,
    total_source_bytes: usize,
    matches_truncated: bool,
}

impl FindResult {
    pub(crate) const fn identity(&self) -> FindIdentity {
        self.identity
    }

    pub(crate) const fn len(&self) -> usize {
        self.matches.len()
    }

    pub(crate) const fn is_empty(&self) -> bool {
        self.matches.is_empty()
    }

    #[cfg(test)]
    pub(crate) const fn source_bytes(&self) -> usize {
        self.source_bytes
    }

    #[cfg(test)]
    pub(crate) const fn total_source_bytes(&self) -> usize {
        self.total_source_bytes
    }

    pub(crate) const fn source_truncated(&self) -> bool {
        self.source_bytes < self.total_source_bytes
    }

    pub(crate) const fn matches_truncated(&self) -> bool {
        self.matches_truncated
    }

    #[cfg(test)]
    pub(crate) const fn retained_metadata_bytes(&self) -> usize {
        self.matches.len() * size_of::<Range<usize>>()
    }

    pub(crate) fn range(&self, index: usize) -> Option<Range<usize>> {
        self.matches.get(index).cloned()
    }

    fn visible(&self, bytes: Range<usize>) -> &[Range<usize>] {
        let start = self
            .matches
            .partition_point(|range| range.end <= bytes.start);
        let end = self
            .matches
            .partition_point(|range| range.start < bytes.end);
        let bounded_end = end.min(start.saturating_add(MAX_VISIBLE_MATCHES));
        &self.matches[start.min(bounded_end)..bounded_end]
    }
}

pub(crate) struct FindRequest {
    identity: FindIdentity,
    snapshot: BufferSnapshot,
    query: Box<str>,
    limits: FindLimits,
}

impl FindRequest {
    pub(crate) fn new(
        identity: FindIdentity,
        snapshot: BufferSnapshot,
        query: &str,
    ) -> Result<Self, FindError> {
        Self::with_limits(identity, snapshot, query, FindLimits::default())
    }

    fn with_limits(
        identity: FindIdentity,
        snapshot: BufferSnapshot,
        query: &str,
        limits: FindLimits,
    ) -> Result<Self, FindError> {
        if !limits.is_valid() {
            return Err(FindError::InvalidLimits);
        }
        if query.len() > limits.query_bytes {
            return Err(FindError::QueryTooLong {
                actual: query.len(),
                limit: limits.query_bytes,
            });
        }
        let mut owned = String::new();
        owned
            .try_reserve_exact(query.len())
            .map_err(|_| FindError::AllocationFailed)?;
        owned.push_str(query);
        Ok(Self {
            identity,
            snapshot,
            query: owned.into_boxed_str(),
            limits,
        })
    }

    pub(crate) const fn identity(&self) -> FindIdentity {
        self.identity
    }

    pub(crate) fn execute(self) -> FindWorkerOutput {
        let identity = self.identity;
        let result = self.execute_inner();
        FindWorkerOutput { identity, result }
    }

    fn execute_inner(self) -> Result<FindResult, FindError> {
        let total_source_bytes = self.snapshot.len_bytes();
        let retained_limit = total_source_bytes.min(self.limits.source_bytes);
        let mut retained_end = retained_limit;
        for _ in 0..UTF8_BOUNDARY_BACKTRACK {
            match self.snapshot.slice(0..retained_end) {
                Ok(source) => {
                    return search_text(
                        self.identity,
                        &source,
                        total_source_bytes,
                        &self.query,
                        self.limits,
                    );
                }
                Err(TextError::InvalidByteBoundary { .. }) => {
                    retained_end = retained_end.saturating_sub(1);
                }
                Err(error) => return Err(error.into()),
            }
        }
        let source = self.snapshot.slice(0..retained_end)?;
        search_text(
            self.identity,
            &source,
            total_source_bytes,
            &self.query,
            self.limits,
        )
    }
}

#[derive(Debug)]
pub(crate) struct FindWorkerOutput {
    identity: FindIdentity,
    result: Result<FindResult, FindError>,
}

impl FindWorkerOutput {
    #[cfg(test)]
    pub(crate) const fn failure_for_test(identity: FindIdentity, error: FindError) -> Self {
        Self {
            identity,
            result: Err(error),
        }
    }
}

fn search_text(
    identity: FindIdentity,
    source: &str,
    total_source_bytes: usize,
    query: &str,
    limits: FindLimits,
) -> Result<FindResult, FindError> {
    if source.len() > total_source_bytes {
        return Err(FindError::InvalidSourceLength {
            retained: source.len(),
            total: total_source_bytes,
        });
    }
    if query.len() > limits.query_bytes {
        return Err(FindError::QueryTooLong {
            actual: query.len(),
            limit: limits.query_bytes,
        });
    }
    let match_capacity = limits.match_capacity();
    let reserve = if query.is_empty() {
        0
    } else {
        source
            .len()
            .checked_div(query.len())
            .unwrap_or(0)
            .min(match_capacity)
    };
    let mut matches = Vec::new();
    matches
        .try_reserve_exact(reserve)
        .map_err(|_| FindError::AllocationFailed)?;
    let mut matches_truncated = false;
    if !query.is_empty() {
        for (start, _) in source.match_indices(query) {
            if matches.len() == match_capacity {
                matches_truncated = true;
                break;
            }
            let end = start
                .checked_add(query.len())
                .ok_or(FindError::OffsetOverflow)?;
            matches.push(start..end);
        }
    }
    Ok(FindResult {
        identity,
        matches: matches.into_boxed_slice(),
        source_bytes: source.len(),
        total_source_bytes,
        matches_truncated,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FindAdmission {
    Accepted,
    Stale,
    Failed,
}

pub(crate) struct FindState {
    open: bool,
    replace_visible: bool,
    field: FindField,
    query: String,
    replacement: String,
    composition: Option<Box<str>>,
    generation: u64,
    pending: Option<FindIdentity>,
    result: Option<FindResult>,
    active: Option<usize>,
    status: Option<Box<str>>,
    failures: u64,
    stale_results: u64,
}

impl Default for FindState {
    fn default() -> Self {
        Self {
            open: false,
            replace_visible: false,
            field: FindField::Query,
            query: String::new(),
            replacement: String::new(),
            composition: None,
            generation: 0,
            pending: None,
            result: None,
            active: None,
            status: None,
            failures: 0,
            stale_results: 0,
        }
    }
}

impl FindState {
    pub(crate) const fn is_open(&self) -> bool {
        self.open
    }

    #[cfg(test)]
    pub(crate) const fn field(&self) -> FindField {
        self.field
    }

    pub(crate) fn query(&self) -> &str {
        &self.query
    }

    pub(crate) fn replacement(&self) -> &str {
        &self.replacement
    }

    #[cfg(test)]
    pub(crate) const fn generation(&self) -> u64 {
        self.generation
    }

    #[cfg(test)]
    pub(crate) const fn failures(&self) -> u64 {
        self.failures
    }

    #[cfg(test)]
    pub(crate) const fn stale_results(&self) -> u64 {
        self.stale_results
    }

    #[cfg(test)]
    pub(crate) fn exhaust_generation_for_test(&mut self) {
        self.generation = u64::MAX;
    }

    #[cfg(test)]
    pub(crate) fn replace_ranges_for_test(&mut self, ranges: Vec<Range<usize>>) {
        if let Some(result) = self.result.as_mut() {
            result.matches = ranges.into_boxed_slice();
        }
    }

    #[cfg(test)]
    pub(crate) fn oversize_query_for_test(&mut self) {
        self.open = true;
        self.field = FindField::Query;
        self.query = "x".repeat(MAX_QUERY_BYTES + 1);
    }

    pub(crate) fn open(&mut self, replace_visible: bool) -> bool {
        let changed = !self.open || (replace_visible && !self.replace_visible);
        self.open = true;
        self.replace_visible |= replace_visible;
        self.field = if replace_visible {
            FindField::Replacement
        } else {
            FindField::Query
        };
        self.composition = None;
        changed
    }

    pub(crate) fn close(&mut self) -> bool {
        let changed = self.open;
        self.open = false;
        self.composition = None;
        self.pending = None;
        self.result = None;
        self.active = None;
        self.status = None;
        changed
    }

    pub(crate) fn toggle_field(&mut self) -> bool {
        if !self.replace_visible {
            return false;
        }
        self.field = match self.field {
            FindField::Query => FindField::Replacement,
            FindField::Replacement => FindField::Query,
        };
        self.composition = None;
        true
    }

    pub(crate) fn begin_composition(&mut self) -> bool {
        let changed = self.composition.as_deref() != Some("");
        self.composition = Some(Box::from(""));
        changed
    }

    pub(crate) fn update_composition(&mut self, text: &str) -> Result<bool, FindError> {
        let limit = MAX_QUERY_BYTES;
        if text.len() > limit {
            return Err(match self.field {
                FindField::Query => FindError::QueryTooLong {
                    actual: text.len(),
                    limit,
                },
                FindField::Replacement => FindError::ReplacementTooLong {
                    actual: text.len(),
                    limit,
                },
            });
        }
        let changed = self.composition.as_deref() != Some(text);
        let mut owned = String::new();
        owned
            .try_reserve_exact(text.len())
            .map_err(|_| FindError::AllocationFailed)?;
        owned.push_str(text);
        self.composition = Some(owned.into_boxed_str());
        Ok(changed)
    }

    pub(crate) fn cancel_composition(&mut self) -> bool {
        self.composition.take().is_some()
    }

    pub(crate) fn commit_text(&mut self, text: &str) -> Result<bool, FindError> {
        self.composition = None;
        let query_changed = self.field == FindField::Query;
        let next_generation = if query_changed {
            Some(
                self.generation
                    .checked_add(1)
                    .ok_or(FindError::GenerationExhausted)?,
            )
        } else {
            None
        };
        let target = match self.field {
            FindField::Query => &mut self.query,
            FindField::Replacement => &mut self.replacement,
        };
        let next = target
            .len()
            .checked_add(text.len())
            .ok_or(FindError::OffsetOverflow)?;
        if next > MAX_QUERY_BYTES {
            return Err(match self.field {
                FindField::Query => FindError::QueryTooLong {
                    actual: next,
                    limit: MAX_QUERY_BYTES,
                },
                FindField::Replacement => FindError::ReplacementTooLong {
                    actual: next,
                    limit: MAX_QUERY_BYTES,
                },
            });
        }
        target
            .try_reserve_exact(text.len())
            .map_err(|_| FindError::AllocationFailed)?;
        target.push_str(text);
        if let Some(generation) = next_generation {
            self.query_changed(generation);
        }
        Ok(query_changed)
    }

    pub(crate) fn delete_backward(&mut self) -> Result<bool, FindError> {
        self.composition = None;
        let query_changed = self.field == FindField::Query;
        let next_generation = if query_changed && !self.query.is_empty() {
            Some(
                self.generation
                    .checked_add(1)
                    .ok_or(FindError::GenerationExhausted)?,
            )
        } else {
            None
        };
        let target = match self.field {
            FindField::Query => &mut self.query,
            FindField::Replacement => &mut self.replacement,
        };
        if target.pop().is_none() {
            return Ok(false);
        }
        if let Some(generation) = next_generation {
            self.query_changed(generation);
        }
        Ok(query_changed)
    }

    fn query_changed(&mut self, generation: u64) {
        self.generation = generation;
        self.pending = None;
        self.result = None;
        self.active = None;
        self.status = None;
    }

    pub(crate) fn document_changed(&mut self) -> Result<bool, FindError> {
        self.pending = None;
        self.result = None;
        self.active = None;
        if !self.open || self.query.is_empty() {
            return Ok(false);
        }
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or(FindError::GenerationExhausted)?;
        Ok(true)
    }

    pub(crate) fn request(
        &mut self,
        document: u64,
        snapshot: BufferSnapshot,
    ) -> Result<Option<FindRequest>, FindError> {
        if !self.open || self.query.is_empty() {
            self.pending = None;
            self.result = None;
            self.active = None;
            return Ok(None);
        }
        let identity = FindIdentity::new(document, snapshot.revision().get(), self.generation);
        let request = FindRequest::new(identity, snapshot, &self.query)?;
        self.pending = Some(identity);
        self.status = None;
        Ok(Some(request))
    }

    pub(crate) fn reject_submission(&mut self, identity: FindIdentity) -> bool {
        if self.pending != Some(identity) {
            return false;
        }
        self.pending = None;
        self.record_error(&FindError::WorkerUnavailable);
        true
    }

    pub(crate) fn admit(
        &mut self,
        output: FindWorkerOutput,
        document: u64,
        buffer_revision: u64,
    ) -> FindAdmission {
        let identity = output.identity;
        if self.pending != Some(identity)
            || identity.document != document
            || identity.buffer_revision != buffer_revision
            || identity.generation != self.generation
        {
            self.stale_results = self.stale_results.saturating_add(1);
            return FindAdmission::Stale;
        }
        self.pending = None;
        match output.result {
            Ok(result) => {
                if result.identity != identity {
                    self.stale_results = self.stale_results.saturating_add(1);
                    return FindAdmission::Stale;
                }
                self.active = (!result.is_empty()).then_some(0);
                self.result = Some(result);
                self.status = None;
                FindAdmission::Accepted
            }
            Err(error) => {
                self.result = None;
                self.active = None;
                self.record_error(&error);
                FindAdmission::Failed
            }
        }
    }

    pub(crate) fn record_error(&mut self, error: &FindError) {
        self.failures = self.failures.saturating_add(1);
        self.status = Some(error.to_string().into_boxed_str());
    }

    pub(crate) fn navigate(&mut self, forward: bool) -> Option<FindNavigation> {
        let result = self.result.as_ref()?;
        if result.is_empty() {
            self.active = None;
            return None;
        }
        let (index, wrapped) = match (self.active, forward) {
            (None, true) => (0, false),
            (None, false) => (result.len() - 1, false),
            (Some(active), true) if active + 1 < result.len() => (active + 1, false),
            (Some(_), true) => (0, true),
            (Some(active), false) if active > 0 => (active - 1, false),
            (Some(_), false) => (result.len() - 1, true),
        };
        self.active = Some(index);
        Some(FindNavigation { index, wrapped })
    }

    pub(crate) fn active_range(&self, document: u64, buffer_revision: u64) -> Option<Range<usize>> {
        let result = self.result.as_ref()?;
        let identity = result.identity();
        if identity.document != document
            || identity.buffer_revision != buffer_revision
            || identity.generation != self.generation
        {
            return None;
        }
        result.range(self.active?)
    }

    pub(crate) fn all_ranges(
        &self,
        document: u64,
        buffer_revision: u64,
    ) -> Option<&[Range<usize>]> {
        let result = self.result.as_ref()?;
        let identity = result.identity();
        if identity.document != document
            || identity.buffer_revision != buffer_revision
            || identity.generation != self.generation
        {
            return None;
        }
        if result.source_truncated() || result.matches_truncated() {
            return None;
        }
        Some(&result.matches)
    }

    pub(crate) fn visible_ranges(
        &self,
        document: u64,
        buffer_revision: u64,
        bytes: Range<usize>,
    ) -> &[Range<usize>] {
        let Some(result) = &self.result else {
            return &[];
        };
        let identity = result.identity();
        if identity.document != document
            || identity.buffer_revision != buffer_revision
            || identity.generation != self.generation
        {
            return &[];
        }
        result.visible(bytes)
    }

    pub(crate) fn result(&self) -> Option<&FindResult> {
        self.result.as_ref()
    }

    pub(crate) fn display_text(&self) -> Result<String, FindError> {
        let mut display = String::new();
        display
            .try_reserve_exact(MAX_DISPLAY_BYTES.saturating_add(96))
            .map_err(|_| FindError::AllocationFailed)?;
        let field = match self.field {
            FindField::Query => "Find",
            FindField::Replacement => "Replace",
        };
        write!(&mut display, "{field}: ").map_err(|_| FindError::AllocationFailed)?;
        let value = match self.field {
            FindField::Query => &self.query,
            FindField::Replacement => &self.replacement,
        };
        let start = suffix_boundary(value, MAX_DISPLAY_BYTES);
        if start > 0 {
            display.push_str("...");
        }
        display.push_str(&value[start..]);
        if let Some(composition) = &self.composition {
            display.push_str(composition);
        }
        if let Some(result) = &self.result {
            let active = self.active.map_or(0, |index| index.saturating_add(1));
            write!(&mut display, "  {active}/{}", result.len())
                .map_err(|_| FindError::AllocationFailed)?;
            if result.source_truncated() || result.matches_truncated() {
                display.push_str("  truncated");
            }
        }
        if let Some(status) = &self.status {
            display.push_str("  ");
            display.push_str(status);
        }
        Ok(display)
    }
}

fn suffix_boundary(value: &str, bytes: usize) -> usize {
    let candidate = value.len().saturating_sub(bytes);
    value
        .char_indices()
        .map(|(index, _)| index)
        .find(|index| *index >= candidate)
        .unwrap_or(value.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use alpine_text::Buffer;

    fn identity(generation: u64) -> FindIdentity {
        FindIdentity::new(7, 11, generation)
    }

    #[test]
    fn literal_unicode_matches_are_ordered_non_overlapping_and_bounded()
    -> Result<(), Box<dyn std::error::Error>> {
        let limits = FindLimits {
            query_bytes: 8,
            source_bytes: 64,
            matches: 2,
            match_metadata_bytes: 2 * size_of::<Range<usize>>(),
        };
        let result = search_text(identity(1), "ééé", 8, "é", limits)?;
        assert_eq!(&*result.matches, &[0..2, 2..4]);
        assert!(result.matches_truncated());
        assert!(result.source_truncated());
        assert_eq!(
            result.retained_metadata_bytes(),
            2 * size_of::<Range<usize>>()
        );
        Ok(())
    }

    #[test]
    fn empty_query_and_invalid_limits_fail_without_hidden_matches()
    -> Result<(), Box<dyn std::error::Error>> {
        let result = search_text(identity(1), "alpha", 5, "", FindLimits::default())?;
        assert!(result.is_empty());
        assert!(!result.matches_truncated());
        assert!(matches!(
            FindRequest::with_limits(
                identity(1),
                Buffer::new("x").snapshot(),
                "x",
                FindLimits {
                    query_bytes: 0,
                    ..FindLimits::default()
                }
            ),
            Err(FindError::InvalidLimits)
        ));
        assert!(matches!(
            search_text(identity(1), "long", 3, "x", FindLimits::default()),
            Err(FindError::InvalidSourceLength {
                retained: 4,
                total: 3
            })
        ));
        Ok(())
    }

    #[test]
    fn request_materialization_backtracks_to_a_utf8_boundary()
    -> Result<(), Box<dyn std::error::Error>> {
        let buffer = Buffer::new("a🦀a");
        let limits = FindLimits {
            query_bytes: 8,
            source_bytes: 4,
            matches: 8,
            match_metadata_bytes: 8 * size_of::<Range<usize>>(),
        };
        let output =
            FindRequest::with_limits(identity(1), buffer.snapshot(), "a", limits)?.execute();
        let result = output.result?;
        assert_eq!(result.source_bytes(), 1);
        assert_eq!(result.total_source_bytes(), 6);
        assert_eq!(result.range(0), Some(0..1));
        Ok(())
    }

    #[test]
    fn stale_result_never_replaces_current_generation() -> Result<(), Box<dyn std::error::Error>> {
        let buffer = Buffer::new("alpha alpha");
        let mut state = FindState::default();
        assert!(state.open(false));
        assert!(state.commit_text("alpha")?);
        let request = state.request(7, buffer.snapshot())?.ok_or("request")?;
        let stale_output = request.execute();
        assert!(state.commit_text("x")?);
        assert_eq!(
            state.admit(stale_output, 7, buffer.revision().get()),
            FindAdmission::Stale
        );
        assert!(state.result().is_none());
        assert_eq!(state.stale_results(), 1);
        Ok(())
    }

    #[test]
    fn navigation_wraps_and_visible_projection_is_capped() -> Result<(), Box<dyn std::error::Error>>
    {
        let source = "x".repeat(MAX_VISIBLE_MATCHES + 2);
        let oversized_query = "x".repeat(MAX_QUERY_BYTES + 1);
        let mut accepted = false;
        for query in [oversized_query.as_str(), "x"] {
            if let Ok(result) = search_text(
                identity(1),
                &source,
                source.len(),
                query,
                FindLimits::default(),
            ) {
                accepted = true;
                let output = FindWorkerOutput {
                    identity: identity(1),
                    result: Ok(result),
                };
                let mut state = FindState::default();
                state.open(false);
                state.commit_text("x")?;
                state.pending = Some(identity(1));
                assert_eq!(state.admit(output, 7, 11), FindAdmission::Accepted);
                assert_eq!(
                    state.visible_ranges(7, 11, 0..source.len()).len(),
                    MAX_VISIBLE_MATCHES
                );
                let first = state.navigate(true).ok_or("next")?;
                assert_eq!(first.index(), 1);
                assert!(!first.wrapped());
                state.active = Some(state.result().ok_or("result")?.len() - 1);
                let wrapped = state.navigate(true).ok_or("wrapped")?;
                assert_eq!(wrapped.index(), 0);
                assert!(wrapped.wrapped());
            }
        }
        assert!(accepted);
        Ok(())
    }

    #[test]
    fn query_replacement_composition_and_errors_preserve_caps()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut state = FindState::default();
        state.open(true);
        assert_eq!(state.field(), FindField::Replacement);
        assert!(state.begin_composition());
        assert!(state.update_composition("漢")?);
        assert!(state.cancel_composition());
        assert!(!state.commit_text("value")?);
        assert_eq!(state.replacement(), "value");
        assert!(state.toggle_field());
        assert!(state.commit_text("needle")?);
        assert_eq!(state.query(), "needle");
        assert!(state.delete_backward()?);
        assert_eq!(state.query(), "needl");
        assert!(matches!(
            state.commit_text(&"x".repeat(MAX_QUERY_BYTES)),
            Err(FindError::QueryTooLong { .. })
        ));
        for error in [
            FindError::InvalidLimits,
            FindError::IncompleteResult,
            FindError::GenerationExhausted,
            FindError::OffsetOverflow,
            FindError::AllocationFailed,
            FindError::WorkerUnavailable,
        ] {
            assert!(!error.to_string().is_empty());
            assert!(error.source().is_none());
        }
        Ok(())
    }

    #[test]
    fn document_change_and_submission_failure_revoke_exact_pending_identity()
    -> Result<(), Box<dyn std::error::Error>> {
        let buffer = Buffer::new("alpha");
        let mut state = FindState::default();
        state.open(false);
        state.commit_text("alpha")?;
        let request = state.request(7, buffer.snapshot())?.ok_or("request")?;
        assert!(!state.reject_submission(FindIdentity::new(9, 9, 9)));
        assert!(state.reject_submission(request.identity()));
        assert_eq!(state.failures(), 1);
        assert!(state.document_changed()?);
        assert_eq!(state.generation(), 2);
        assert!(state.close());
        assert!(!state.document_changed()?);
        Ok(())
    }
}

#[cfg(test)]
#[path = "find_coverage_tests.rs"]
mod coverage_tests;
