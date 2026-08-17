//! Lazy, bounded, revision-gated local project search.

use std::{
    error::Error,
    ffi::OsStr,
    fmt::{self, Write as _},
    fs::{self, File},
    io::Read,
    mem::size_of,
    path::{Component, Path, PathBuf},
    str,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::SystemTime,
};

use alpine_text::BufferSnapshot;
use ignore::WalkBuilder;

pub(crate) const MAX_QUERY_BYTES: usize = 4 * 1_024;
pub(crate) const MAX_SCANNED_ENTRIES: usize = 250_000;
pub(crate) const MAX_FILES: usize = 100_000;
pub(crate) const MAX_PATH_BYTES: usize = 4 * 1_024;
pub(crate) const MAX_INVENTORY_BYTES: usize = 16 * 1_024 * 1_024;
pub(crate) const MAX_DEPTH: usize = 256;
pub(crate) const MAX_FILE_BYTES: usize = 16 * 1_024 * 1_024;
pub(crate) const MAX_TOTAL_READ_BYTES: usize = 512 * 1_024 * 1_024;
pub(crate) const MAX_RESULTS: usize = 16_384;
pub(crate) const MAX_RESULT_BYTES: usize = 4 * 1_024 * 1_024;
pub(crate) const MAX_BATCH_MATCHES: usize = 256;
pub(crate) const MAX_BATCH_BYTES: usize = 256 * 1_024;
pub(crate) const MAX_EXCERPT_BYTES: usize = 512;
pub(crate) const MAX_VISIBLE_RESULTS: usize = 256;
const MAX_FILES_PER_BATCH: usize = 64;
const MAX_READ_BYTES_PER_BATCH: usize = MAX_FILE_BYTES;
const MAX_DIAGNOSTIC_BYTES: usize = 4 * 1_024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ProjectSearchLimits {
    scanned: usize,
    files: usize,
    path_bytes: usize,
    inventory_bytes: usize,
    depth: usize,
    file_bytes: usize,
    total_read_bytes: usize,
    results: usize,
    result_bytes: usize,
    batch_matches: usize,
    batch_bytes: usize,
    excerpt_bytes: usize,
    files_per_batch: usize,
    read_bytes_per_batch: usize,
}

impl ProjectSearchLimits {
    #[cfg(test)]
    #[allow(
        clippy::too_many_arguments,
        reason = "tests lock every independent hard limit"
    )]
    pub(crate) const fn new(
        scanned: usize,
        files: usize,
        path_bytes: usize,
        inventory_bytes: usize,
        depth: usize,
        file_bytes: usize,
        total_read_bytes: usize,
        results: usize,
        result_bytes: usize,
        batch_matches: usize,
        batch_bytes: usize,
        excerpt_bytes: usize,
        files_per_batch: usize,
        read_bytes_per_batch: usize,
    ) -> Self {
        Self {
            scanned,
            files,
            path_bytes,
            inventory_bytes,
            depth,
            file_bytes,
            total_read_bytes,
            results,
            result_bytes,
            batch_matches,
            batch_bytes,
            excerpt_bytes,
            files_per_batch,
            read_bytes_per_batch,
        }
    }

    const fn is_valid(self) -> bool {
        self.scanned > 0
            && self.files > 0
            && self.path_bytes > 0
            && self.inventory_bytes > 0
            && self.depth > 0
            && self.file_bytes > 0
            && self.total_read_bytes >= self.file_bytes
            && self.results > 0
            && self.result_bytes >= size_of::<ProjectMatch>()
            && self.batch_matches > 0
            && self.batch_matches <= self.results
            && self.batch_bytes >= size_of::<ProjectMatch>()
            && self.batch_bytes <= self.result_bytes
            && self.excerpt_bytes > 0
            && self.excerpt_bytes <= self.batch_bytes
            && self.files_per_batch > 0
            && self.read_bytes_per_batch >= self.file_bytes
    }
}

impl Default for ProjectSearchLimits {
    fn default() -> Self {
        Self {
            scanned: MAX_SCANNED_ENTRIES,
            files: MAX_FILES,
            path_bytes: MAX_PATH_BYTES,
            inventory_bytes: MAX_INVENTORY_BYTES,
            depth: MAX_DEPTH,
            file_bytes: MAX_FILE_BYTES,
            total_read_bytes: MAX_TOTAL_READ_BYTES,
            results: MAX_RESULTS,
            result_bytes: MAX_RESULT_BYTES,
            batch_matches: MAX_BATCH_MATCHES,
            batch_bytes: MAX_BATCH_BYTES,
            excerpt_bytes: MAX_EXCERPT_BYTES,
            files_per_batch: MAX_FILES_PER_BATCH,
            read_bytes_per_batch: MAX_READ_BYTES_PER_BATCH,
        }
    }
}

#[derive(Debug)]
pub(crate) enum ProjectSearchError {
    NoWorkspace,
    InvalidLimits,
    GenerationExhausted,
    QueryTooLong { actual: usize, limit: usize },
    AllocationFailed,
    InvalidComposition,
    MissingSelection,
    StaleMatch,
}

impl fmt::Display for ProjectSearchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoWorkspace => formatter.write_str("project search requires one local workspace"),
            Self::InvalidLimits => formatter.write_str("project-search limits are invalid"),
            Self::GenerationExhausted => {
                formatter.write_str("project-search generation is exhausted")
            }
            Self::QueryTooLong { actual, limit } => {
                write!(
                    formatter,
                    "project-search query is {actual} bytes; limit is {limit}"
                )
            }
            Self::AllocationFailed => formatter.write_str("project-search allocation failed"),
            Self::InvalidComposition => {
                formatter.write_str("project-search composition range is invalid")
            }
            Self::MissingSelection => formatter.write_str("project search has no selected match"),
            Self::StaleMatch => formatter.write_str("project-search match is no longer current"),
        }
    }
}

impl Error for ProjectSearchError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct InventoryIdentity {
    workspace: u64,
    generation: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SearchIdentity {
    workspace: u64,
    inventory: u64,
    query: u64,
    request: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RequestIdentity {
    Inventory(InventoryIdentity),
    Search(SearchIdentity),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct InventoryReport {
    pub(crate) scanned: usize,
    pub(crate) files: usize,
    pub(crate) path_bytes: usize,
    pub(crate) omitted: usize,
    pub(crate) errors: usize,
    pub(crate) truncated: bool,
}

#[derive(Debug)]
pub(crate) struct SearchInventory {
    generation: u64,
    paths: Box<[Arc<str>]>,
    report: InventoryReport,
    first_error: Option<Arc<str>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ProjectMatch {
    relative: Box<str>,
    excerpt: Box<str>,
    start: usize,
    end: usize,
    line: u32,
    column: u32,
}

impl ProjectMatch {
    fn retained_bytes(&self) -> usize {
        size_of::<Self>()
            .saturating_add(self.relative.len())
            .saturating_add(self.excerpt.len())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SelectedProjectMatch {
    pub(crate) relative: Arc<str>,
    pub(crate) query: Arc<str>,
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) line: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProjectSearchRow {
    pub(crate) label: Box<str>,
    pub(crate) selected: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ScanCounters {
    files: usize,
    read_bytes: usize,
    unreadable: usize,
    invalid_utf8: usize,
    binary: usize,
    oversized: usize,
    replaced: usize,
}

impl ScanCounters {
    fn add(&mut self, other: Self) {
        self.files = self.files.saturating_add(other.files);
        self.read_bytes = self.read_bytes.saturating_add(other.read_bytes);
        self.unreadable = self.unreadable.saturating_add(other.unreadable);
        self.invalid_utf8 = self.invalid_utf8.saturating_add(other.invalid_utf8);
        self.binary = self.binary.saturating_add(other.binary);
        self.oversized = self.oversized.saturating_add(other.oversized);
        self.replaced = self.replaced.saturating_add(other.replaced);
    }
}

#[derive(Debug)]
struct FileContinuation {
    file_index: usize,
    relative: Arc<str>,
    bytes: Box<[u8]>,
    cursor: usize,
    line: u32,
    line_start: usize,
}

#[derive(Debug, Default)]
pub(crate) struct ScanCursor {
    next_file: usize,
    file: Option<FileContinuation>,
}

#[derive(Debug)]
pub(crate) struct SearchBatch {
    identity: SearchIdentity,
    matches: Box<[ProjectMatch]>,
    bytes: usize,
    cursor: Option<ScanCursor>,
    counters: ScanCounters,
    terminal: bool,
    truncated: bool,
    cancelled: bool,
}

#[derive(Debug)]
pub(crate) enum ProjectSearchRequest {
    Inventory {
        identity: InventoryIdentity,
        root: PathBuf,
        limits: ProjectSearchLimits,
    },
    Search {
        identity: SearchIdentity,
        root: PathBuf,
        inventory: Arc<SearchInventory>,
        query: Arc<str>,
        cursor: ScanCursor,
        counters: ScanCounters,
        retained_results: usize,
        retained_bytes: usize,
        limits: ProjectSearchLimits,
        cancellation: Arc<AtomicU64>,
    },
}

impl ProjectSearchRequest {
    pub(crate) const fn identity(&self) -> RequestIdentity {
        match self {
            Self::Inventory { identity, .. } => RequestIdentity::Inventory(*identity),
            Self::Search { identity, .. } => RequestIdentity::Search(*identity),
        }
    }

    pub(crate) fn execute(self) -> ProjectSearchWorkerOutput {
        match self {
            Self::Inventory {
                identity,
                root,
                limits,
            } => ProjectSearchWorkerOutput::Inventory {
                identity,
                result: build_inventory(identity, &root, limits),
            },
            Self::Search {
                identity,
                root,
                inventory,
                query,
                cursor,
                counters,
                retained_results,
                retained_bytes,
                limits,
                cancellation,
            } => ProjectSearchWorkerOutput::Batch {
                identity,
                result: scan_batch(
                    identity,
                    &root,
                    &inventory,
                    &query,
                    cursor,
                    counters,
                    retained_results,
                    retained_bytes,
                    limits,
                    &cancellation,
                ),
            },
        }
    }
}

#[derive(Debug)]
pub(crate) enum ProjectSearchWorkerOutput {
    Inventory {
        identity: InventoryIdentity,
        result: Result<Arc<SearchInventory>, ProjectSearchError>,
    },
    Batch {
        identity: SearchIdentity,
        result: Result<SearchBatch, ProjectSearchError>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProjectSearchAdmission {
    Inventory,
    Batch,
    Complete,
    Failed,
    Stale,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[allow(
    dead_code,
    reason = "the local diagnostics overlay will consume this snapshot"
)]
pub(crate) struct ProjectSearchReport {
    pub(crate) query_bytes: usize,
    pub(crate) composition_bytes: usize,
    pub(crate) inventory_files: usize,
    pub(crate) inventory_bytes: usize,
    pub(crate) scanned_entries: usize,
    pub(crate) searched_files: usize,
    pub(crate) read_bytes: usize,
    pub(crate) retained_matches: usize,
    pub(crate) result_bytes: usize,
    pub(crate) retained_bytes: usize,
    pub(crate) peak_retained_bytes: usize,
    pub(crate) visible_rows: usize,
    pub(crate) batches: usize,
    pub(crate) unreadable: usize,
    pub(crate) invalid_utf8: usize,
    pub(crate) binary: usize,
    pub(crate) oversized: usize,
    pub(crate) replaced: usize,
    pub(crate) cancellations: u64,
    pub(crate) stale_rejections: u64,
    pub(crate) truncated: bool,
    pub(crate) terminal: bool,
}

#[derive(Debug)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent admission and terminal flags preserve fail-closed worker ownership"
)]
pub(crate) struct ProjectSearchState {
    open: bool,
    workspace: Option<u64>,
    inventory_generation: u64,
    query_generation: u64,
    request_generation: u64,
    inventory: Option<Arc<SearchInventory>>,
    query: String,
    composition: Option<Box<str>>,
    results: Vec<ProjectMatch>,
    result_bytes: usize,
    selected: usize,
    first_visible: usize,
    needs_inventory: bool,
    needs_search: bool,
    pending_inventory: Option<InventoryIdentity>,
    pending_search: Option<SearchIdentity>,
    cursor: Option<ScanCursor>,
    counters: ScanCounters,
    batches: usize,
    truncated: bool,
    terminal: bool,
    error: Option<Arc<str>>,
    cancellations: u64,
    stale_rejections: u64,
    peak_retained_bytes: usize,
    limits: ProjectSearchLimits,
    cancellation: Arc<AtomicU64>,
}

impl Default for ProjectSearchState {
    fn default() -> Self {
        Self::with_limits(ProjectSearchLimits::default())
    }
}

impl ProjectSearchState {
    fn with_limits(limits: ProjectSearchLimits) -> Self {
        Self {
            open: false,
            workspace: None,
            inventory_generation: 0,
            query_generation: 0,
            request_generation: 0,
            inventory: None,
            query: String::new(),
            composition: None,
            results: Vec::new(),
            result_bytes: 0,
            selected: 0,
            first_visible: 0,
            needs_inventory: false,
            needs_search: false,
            pending_inventory: None,
            pending_search: None,
            cursor: None,
            counters: ScanCounters::default(),
            batches: 0,
            truncated: false,
            terminal: false,
            error: None,
            cancellations: 0,
            stale_rejections: 0,
            peak_retained_bytes: 0,
            limits,
            cancellation: Arc::new(AtomicU64::new(0)),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_test_limits(limits: ProjectSearchLimits) -> Self {
        Self::with_limits(limits)
    }

    pub(crate) const fn is_open(&self) -> bool {
        self.open
    }

    pub(crate) fn open(&mut self, workspace: u64) -> Result<bool, ProjectSearchError> {
        if !self.limits.is_valid() {
            return Err(ProjectSearchError::InvalidLimits);
        }
        if self.open && self.workspace == Some(workspace) {
            return Ok(false);
        }
        self.release_dynamic();
        self.open = true;
        self.workspace = Some(workspace);
        self.observe_peak();
        Ok(true)
    }

    pub(crate) fn close(&mut self) -> bool {
        if !self.open {
            return false;
        }
        if self.pending_inventory.is_some() || self.pending_search.is_some() || self.needs_search {
            self.cancellations = self.cancellations.saturating_add(1);
        }
        self.cancellation.store(u64::MAX, Ordering::Release);
        self.open = false;
        self.workspace = None;
        self.release_dynamic();
        true
    }

    pub(crate) fn begin_composition(&mut self) -> bool {
        if !self.open || self.composition.is_some() {
            false
        } else {
            self.composition = Some(Box::default());
            self.observe_peak();
            true
        }
    }

    pub(crate) fn update_composition(
        &mut self,
        text: &str,
        selected_start_utf16: u32,
        selected_length_utf16: u32,
    ) -> Result<bool, ProjectSearchError> {
        let selected_end = selected_start_utf16
            .checked_add(selected_length_utf16)
            .ok_or(ProjectSearchError::InvalidComposition)?;
        let units = u32::try_from(text.encode_utf16().count())
            .map_err(|_| ProjectSearchError::InvalidComposition)?;
        if selected_end > units {
            return Err(ProjectSearchError::InvalidComposition);
        }
        Self::check_query_length(self.query.len().saturating_add(text.len()))?;
        let changed = self.composition.as_deref() != Some(text);
        if changed {
            let mut replacement = String::new();
            replacement
                .try_reserve_exact(text.len())
                .map_err(|_| ProjectSearchError::AllocationFailed)?;
            replacement.push_str(text);
            self.composition = Some(replacement.into_boxed_str());
            self.observe_peak();
        }
        Ok(changed)
    }

    pub(crate) fn cancel_composition(&mut self) -> bool {
        self.composition.take().is_some()
    }

    pub(crate) fn commit_text(&mut self, text: &str) -> Result<bool, ProjectSearchError> {
        if !self.open || text.is_empty() {
            self.composition = None;
            return Ok(false);
        }
        let length =
            self.query
                .len()
                .checked_add(text.len())
                .ok_or(ProjectSearchError::QueryTooLong {
                    actual: usize::MAX,
                    limit: MAX_QUERY_BYTES,
                })?;
        Self::check_query_length(length)?;
        let next_generation = self.next_query_generation()?;
        let mut query = String::new();
        query
            .try_reserve_exact(length)
            .map_err(|_| ProjectSearchError::AllocationFailed)?;
        query.push_str(&self.query);
        query.push_str(text);
        self.composition = None;
        self.replace_query(query, next_generation)?;
        Ok(true)
    }

    pub(crate) fn delete_backward(&mut self) -> Result<bool, ProjectSearchError> {
        self.composition = None;
        if self.query.is_empty() {
            return Ok(false);
        }
        let next_generation = self.next_query_generation()?;
        let mut query = String::new();
        query
            .try_reserve_exact(self.query.len())
            .map_err(|_| ProjectSearchError::AllocationFailed)?;
        query.push_str(&self.query);
        let _ = query.pop();
        self.replace_query(query, next_generation)?;
        Ok(true)
    }

    pub(crate) fn take_request(
        &mut self,
        root: &Path,
    ) -> Result<Option<ProjectSearchRequest>, ProjectSearchError> {
        if !self.open || self.query.is_empty() {
            return Ok(None);
        }
        let workspace = self.workspace.ok_or(ProjectSearchError::NoWorkspace)?;
        if self.needs_inventory {
            self.needs_inventory = false;
            let identity = InventoryIdentity {
                workspace,
                generation: self.inventory_generation,
            };
            self.pending_inventory = Some(identity);
            return Ok(Some(ProjectSearchRequest::Inventory {
                identity,
                root: root.to_path_buf(),
                limits: self.limits,
            }));
        }
        if self.needs_search {
            let inventory = Arc::clone(
                self.inventory
                    .as_ref()
                    .ok_or(ProjectSearchError::InvalidLimits)?,
            );
            self.request_generation = self
                .request_generation
                .checked_add(1)
                .ok_or(ProjectSearchError::GenerationExhausted)?;
            let identity = SearchIdentity {
                workspace,
                inventory: inventory.generation,
                query: self.query_generation,
                request: self.request_generation,
            };
            let cursor = self.cursor.take().unwrap_or_default();
            self.needs_search = false;
            self.pending_search = Some(identity);
            return Ok(Some(ProjectSearchRequest::Search {
                identity,
                root: root.to_path_buf(),
                inventory,
                query: Arc::from(self.query.as_str()),
                cursor,
                counters: self.counters,
                retained_results: self.results.len(),
                retained_bytes: self.result_bytes,
                limits: self.limits,
                cancellation: Arc::clone(&self.cancellation),
            }));
        }
        Ok(None)
    }

    pub(crate) fn reject_submission(&mut self, identity: RequestIdentity) -> bool {
        let current = match identity {
            RequestIdentity::Inventory(identity) if self.pending_inventory == Some(identity) => {
                self.pending_inventory = None;
                self.needs_inventory = self.open && !self.query.is_empty();
                true
            }
            RequestIdentity::Search(identity) if self.pending_search == Some(identity) => {
                self.pending_search = None;
                self.needs_search = false;
                self.cursor = None;
                self.terminal = true;
                true
            }
            RequestIdentity::Inventory(_) | RequestIdentity::Search(_) => false,
        };
        if current {
            self.record_error(&ProjectSearchError::AllocationFailed);
        }
        current
    }

    pub(crate) fn admit(&mut self, output: ProjectSearchWorkerOutput) -> ProjectSearchAdmission {
        match output {
            ProjectSearchWorkerOutput::Inventory { identity, result } => {
                if !self.open
                    || self.workspace != Some(identity.workspace)
                    || self.inventory_generation != identity.generation
                    || self.pending_inventory != Some(identity)
                {
                    self.stale_rejections = self.stale_rejections.saturating_add(1);
                    return ProjectSearchAdmission::Stale;
                }
                self.pending_inventory = None;
                match result {
                    Ok(inventory) => {
                        self.inventory = Some(inventory);
                        self.error = None;
                        self.needs_search = !self.query.is_empty();
                        self.cursor = None;
                        self.observe_peak();
                        ProjectSearchAdmission::Inventory
                    }
                    Err(error) => {
                        self.record_error(&error);
                        self.terminal = true;
                        ProjectSearchAdmission::Failed
                    }
                }
            }
            ProjectSearchWorkerOutput::Batch { identity, result } => {
                let inventory = self.inventory.as_ref().map(|value| value.generation);
                if !self.open
                    || self.workspace != Some(identity.workspace)
                    || inventory != Some(identity.inventory)
                    || self.query_generation != identity.query
                    || self.pending_search != Some(identity)
                {
                    self.stale_rejections = self.stale_rejections.saturating_add(1);
                    return ProjectSearchAdmission::Stale;
                }
                self.pending_search = None;
                match result {
                    Ok(batch) if batch.identity == identity && !batch.cancelled => {
                        if self.append_batch(batch.matches, batch.bytes).is_err() {
                            self.record_error(&ProjectSearchError::AllocationFailed);
                            self.terminal = true;
                            return ProjectSearchAdmission::Failed;
                        }
                        self.counters = batch.counters;
                        self.batches = self.batches.saturating_add(1);
                        self.truncated |= batch.truncated;
                        self.terminal = batch.terminal;
                        self.cursor = batch.cursor;
                        self.needs_search = !self.terminal && self.cursor.is_some();
                        self.error = None;
                        self.observe_peak();
                        if self.terminal {
                            ProjectSearchAdmission::Complete
                        } else {
                            ProjectSearchAdmission::Batch
                        }
                    }
                    Ok(_) => {
                        self.stale_rejections = self.stale_rejections.saturating_add(1);
                        ProjectSearchAdmission::Stale
                    }
                    Err(error) => {
                        self.record_error(&error);
                        self.terminal = true;
                        ProjectSearchAdmission::Failed
                    }
                }
            }
        }
    }

    pub(crate) fn navigate(&mut self, forward: bool, visible_rows: usize) -> bool {
        if self.results.is_empty() {
            return false;
        }
        let previous = self.selected;
        self.selected = if forward {
            (self.selected + 1) % self.results.len()
        } else if self.selected == 0 {
            self.results.len() - 1
        } else {
            self.selected - 1
        };
        let visible_rows = visible_rows.clamp(1, MAX_VISIBLE_RESULTS);
        if self.selected >= self.first_visible.saturating_add(visible_rows) {
            self.first_visible = self.selected.saturating_add(1).saturating_sub(visible_rows);
        }
        self.first_visible = self.first_visible.min(self.selected);
        self.selected != previous
    }

    pub(crate) fn visible_results(
        &self,
        visible_rows: usize,
        overscan: usize,
    ) -> Result<Vec<ProjectSearchRow>, ProjectSearchError> {
        let start = self.first_visible.saturating_sub(overscan);
        let end = self
            .first_visible
            .saturating_add(visible_rows.min(MAX_VISIBLE_RESULTS))
            .saturating_add(overscan)
            .min(self.results.len())
            .min(start.saturating_add(MAX_VISIBLE_RESULTS));
        let mut rows = Vec::new();
        rows.try_reserve_exact(end.saturating_sub(start))
            .map_err(|_| ProjectSearchError::AllocationFailed)?;
        for (offset, found) in self.results[start..end].iter().enumerate() {
            let mut label = String::new();
            label
                .try_reserve_exact(
                    found
                        .relative
                        .len()
                        .saturating_add(found.excerpt.len())
                        .saturating_add(48),
                )
                .map_err(|_| ProjectSearchError::AllocationFailed)?;
            write!(
                label,
                "{}:{}:{}  {}",
                found.relative, found.line, found.column, found.excerpt
            )
            .map_err(|_| ProjectSearchError::AllocationFailed)?;
            rows.push(ProjectSearchRow {
                label: label.into_boxed_str(),
                selected: start.saturating_add(offset) == self.selected,
            });
        }
        Ok(rows)
    }

    pub(crate) fn selected_match(&self) -> Result<SelectedProjectMatch, ProjectSearchError> {
        let found = self
            .results
            .get(self.selected)
            .ok_or(ProjectSearchError::MissingSelection)?;
        Ok(SelectedProjectMatch {
            relative: Arc::from(found.relative.as_ref()),
            query: Arc::from(self.query.as_str()),
            start: found.start,
            end: found.end,
            line: found.line,
        })
    }

    pub(crate) fn display_text(&self) -> Result<String, ProjectSearchError> {
        let composition = self.composition.as_deref().unwrap_or_default();
        let inventory = self
            .inventory
            .as_ref()
            .map_or(InventoryReport::default(), |value| value.report);
        let first_error = self.error.as_deref().or_else(|| {
            self.inventory
                .as_ref()
                .and_then(|value| value.first_error.as_deref())
        });
        let mut display = String::new();
        display
            .try_reserve_exact(MAX_DIAGNOSTIC_BYTES)
            .map_err(|_| ProjectSearchError::AllocationFailed)?;
        write!(
            display,
            "Project Search: {}{} | {} matches, {}/{} files, {} B, {} batches{}{}",
            self.query,
            composition,
            self.results.len(),
            self.counters.files,
            inventory.files,
            self.counters.read_bytes,
            self.batches,
            if self.truncated { ", truncated" } else { "" },
            first_error.map_or_else(String::new, |message| format!(" | {message}")),
        )
        .map_err(|_| ProjectSearchError::AllocationFailed)?;
        if display.len() > MAX_DIAGNOSTIC_BYTES {
            display.truncate(MAX_DIAGNOSTIC_BYTES);
            while !display.is_char_boundary(display.len()) {
                let _ = display.pop();
            }
        }
        Ok(display)
    }

    #[allow(
        dead_code,
        reason = "the local diagnostics overlay will consume this snapshot"
    )]
    pub(crate) fn report(&self) -> ProjectSearchReport {
        let inventory = self
            .inventory
            .as_ref()
            .map_or(InventoryReport::default(), |value| value.report);
        ProjectSearchReport {
            query_bytes: self.query.len(),
            composition_bytes: self.composition.as_deref().map_or(0, str::len),
            inventory_files: inventory.files,
            inventory_bytes: inventory.path_bytes,
            scanned_entries: inventory.scanned,
            searched_files: self.counters.files,
            read_bytes: self.counters.read_bytes,
            retained_matches: self.results.len(),
            result_bytes: self.result_bytes,
            retained_bytes: self.retained_bytes(),
            peak_retained_bytes: self.peak_retained_bytes,
            visible_rows: self.results.len().min(MAX_VISIBLE_RESULTS),
            batches: self.batches,
            unreadable: self.counters.unreadable,
            invalid_utf8: self.counters.invalid_utf8,
            binary: self.counters.binary,
            oversized: self.counters.oversized,
            replaced: self.counters.replaced,
            cancellations: self.cancellations,
            stale_rejections: self.stale_rejections,
            truncated: self.truncated,
            terminal: self.terminal,
        }
    }

    #[cfg(test)]
    pub(crate) fn query(&self) -> &str {
        &self.query
    }

    #[cfg(test)]
    pub(crate) fn exhaust_generations_for_test(&mut self) {
        self.query_generation = u64::MAX;
    }

    fn next_query_generation(&self) -> Result<u64, ProjectSearchError> {
        self.query_generation
            .checked_add(1)
            .ok_or(ProjectSearchError::GenerationExhausted)
    }

    fn replace_query(&mut self, query: String, generation: u64) -> Result<(), ProjectSearchError> {
        let next_inventory_generation =
            if self.inventory.is_none() && !query.is_empty() && self.pending_inventory.is_none() {
                Some(
                    self.inventory_generation
                        .checked_add(1)
                        .ok_or(ProjectSearchError::GenerationExhausted)?,
                )
            } else {
                None
            };
        self.query_generation = generation;
        self.cancellation.store(generation, Ordering::Release);
        self.query = query;
        self.pending_search = None;
        self.cursor = None;
        self.results = Vec::new();
        self.result_bytes = 0;
        self.selected = 0;
        self.first_visible = 0;
        self.counters = ScanCounters::default();
        self.batches = 0;
        self.truncated = false;
        self.terminal = self.query.is_empty();
        self.error = None;
        self.needs_search = self.inventory.is_some() && !self.query.is_empty();
        if let Some(next_inventory_generation) = next_inventory_generation {
            self.inventory_generation = next_inventory_generation;
            self.needs_inventory = true;
        }
        self.observe_peak();
        Ok(())
    }

    fn append_batch(
        &mut self,
        matches: Box<[ProjectMatch]>,
        logical_bytes: usize,
    ) -> Result<(), ProjectSearchError> {
        let next_count = self
            .results
            .len()
            .checked_add(matches.len())
            .ok_or(ProjectSearchError::AllocationFailed)?;
        let next_bytes = self
            .result_bytes
            .checked_add(logical_bytes)
            .ok_or(ProjectSearchError::AllocationFailed)?;
        if next_count > self.limits.results || next_bytes > self.limits.result_bytes {
            return Err(ProjectSearchError::AllocationFailed);
        }
        self.results
            .try_reserve_exact(matches.len())
            .map_err(|_| ProjectSearchError::AllocationFailed)?;
        self.results.extend(matches);
        self.result_bytes = next_bytes;
        Ok(())
    }

    fn check_query_length(actual: usize) -> Result<(), ProjectSearchError> {
        if actual > MAX_QUERY_BYTES {
            Err(ProjectSearchError::QueryTooLong {
                actual,
                limit: MAX_QUERY_BYTES,
            })
        } else {
            Ok(())
        }
    }

    fn record_error(&mut self, error: &ProjectSearchError) {
        self.error = Some(Arc::from(error.to_string()));
    }

    fn retained_bytes(&self) -> usize {
        let inventory = self.inventory.as_ref().map_or(0, |value| {
            value
                .paths
                .len()
                .saturating_mul(size_of::<Arc<str>>())
                .saturating_add(value.report.path_bytes)
        });
        self.query
            .capacity()
            .saturating_add(self.composition.as_deref().map_or(0, str::len))
            .saturating_add(inventory)
            .saturating_add(self.result_bytes)
            .saturating_add(self.error.as_deref().map_or(0, str::len))
    }

    fn observe_peak(&mut self) {
        self.peak_retained_bytes = self.peak_retained_bytes.max(self.retained_bytes());
    }

    fn release_dynamic(&mut self) {
        self.cancellation.store(u64::MAX, Ordering::Release);
        self.inventory = None;
        self.query = String::new();
        self.composition = None;
        self.results = Vec::new();
        self.result_bytes = 0;
        self.selected = 0;
        self.first_visible = 0;
        self.needs_inventory = false;
        self.needs_search = false;
        self.pending_inventory = None;
        self.pending_search = None;
        self.cursor = None;
        self.counters = ScanCounters::default();
        self.batches = 0;
        self.truncated = false;
        self.terminal = false;
        self.error = None;
    }
}

pub(crate) fn verify_snapshot_match(
    snapshot: &BufferSnapshot,
    selected: &SelectedProjectMatch,
) -> Result<(), ProjectSearchError> {
    snapshot
        .slice(selected.start..selected.end)
        .ok()
        .filter(|text| text.as_bytes() == selected.query.as_bytes())
        .map(|_| ())
        .ok_or(ProjectSearchError::StaleMatch)
}

fn build_inventory(
    identity: InventoryIdentity,
    root: &Path,
    limits: ProjectSearchLimits,
) -> Result<Arc<SearchInventory>, ProjectSearchError> {
    if !limits.is_valid() {
        return Err(ProjectSearchError::InvalidLimits);
    }
    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(false)
        .parents(false)
        .ignore(true)
        .git_ignore(true)
        .require_git(false)
        .git_global(false)
        .git_exclude(true)
        .follow_links(false)
        .same_file_system(true)
        .max_depth(Some(limits.depth))
        .filter_entry(|entry| entry.file_name() != OsStr::new(".git"));
    let mut paths: Vec<Arc<str>> = Vec::new();
    let mut report = InventoryReport::default();
    let mut first_error = None;
    for item in builder.build() {
        if report.scanned == limits.scanned {
            report.truncated = true;
            report.omitted = report.omitted.saturating_add(1);
            break;
        }
        let entry = match item {
            Ok(entry) if entry.depth() == 0 => continue,
            Ok(entry) => entry,
            Err(error) => {
                report.scanned = report.scanned.saturating_add(1);
                report.errors = report.errors.saturating_add(1);
                report.omitted = report.omitted.saturating_add(1);
                if first_error.is_none() {
                    first_error = Some(Arc::from(error.to_string()));
                }
                continue;
            }
        };
        report.scanned = report.scanned.saturating_add(1);
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        let Some(relative) = portable_relative_path(root, entry.path())? else {
            report.omitted = report.omitted.saturating_add(1);
            continue;
        };
        let next_bytes = report.path_bytes.checked_add(relative.len());
        if relative.is_empty()
            || relative.len() > limits.path_bytes
            || paths.len() == limits.files
            || next_bytes.is_none_or(|bytes| bytes > limits.inventory_bytes)
        {
            report.omitted = report.omitted.saturating_add(1);
            report.truncated = true;
            continue;
        }
        paths
            .try_reserve(1)
            .map_err(|_| ProjectSearchError::AllocationFailed)?;
        paths.push(Arc::from(relative));
        report.path_bytes = next_bytes.ok_or(ProjectSearchError::AllocationFailed)?;
    }
    paths.sort_unstable_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
    let before = paths.len();
    paths.dedup();
    report.omitted = report
        .omitted
        .saturating_add(before.saturating_sub(paths.len()));
    report.files = paths.len();
    report.path_bytes = paths.iter().map(|path| path.len()).sum();
    paths.shrink_to_fit();
    Ok(Arc::new(SearchInventory {
        generation: identity.generation,
        paths: paths.into_boxed_slice(),
        report,
        first_error,
    }))
}

fn portable_relative_path(root: &Path, path: &Path) -> Result<Option<String>, ProjectSearchError> {
    let Ok(relative) = path.strip_prefix(root) else {
        return Ok(None);
    };
    let mut rendered = String::new();
    rendered
        .try_reserve_exact(relative.as_os_str().as_encoded_bytes().len())
        .map_err(|_| ProjectSearchError::AllocationFailed)?;
    for component in relative.components() {
        let Component::Normal(component) = component else {
            return Ok(None);
        };
        let Some(component) = component.to_str() else {
            return Ok(None);
        };
        if !rendered.is_empty() {
            rendered.push('/');
        }
        rendered.push_str(component);
    }
    Ok((!rendered.is_empty()).then_some(rendered))
}

#[allow(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    reason = "worker ownership, continuation, and every independent budget remain explicit"
)]
fn scan_batch(
    identity: SearchIdentity,
    root: &Path,
    inventory: &SearchInventory,
    query: &str,
    mut cursor: ScanCursor,
    cumulative: ScanCounters,
    retained_results: usize,
    retained_bytes: usize,
    limits: ProjectSearchLimits,
    cancellation: &AtomicU64,
) -> Result<SearchBatch, ProjectSearchError> {
    if !limits.is_valid() || query.is_empty() || query.len() > MAX_QUERY_BYTES {
        return Err(ProjectSearchError::InvalidLimits);
    }
    let mut matches = Vec::new();
    matches
        .try_reserve_exact(limits.batch_matches)
        .map_err(|_| ProjectSearchError::AllocationFailed)?;
    let mut batch_bytes = 0_usize;
    let mut delta = ScanCounters::default();
    let mut truncated = false;
    let mut terminal = false;
    let mut cancelled = false;
    loop {
        if cancellation.load(Ordering::Acquire) != identity.query {
            cancelled = true;
            break;
        }
        if retained_results.saturating_add(matches.len()) == limits.results
            || retained_bytes.saturating_add(batch_bytes) == limits.result_bytes
        {
            truncated = true;
            terminal = true;
            cursor.file = None;
            break;
        }
        let mut file = if let Some(file) = cursor.file.take() {
            file
        } else {
            if cursor.next_file >= inventory.paths.len() {
                terminal = true;
                break;
            }
            if delta.files == limits.files_per_batch
                || delta.read_bytes == limits.read_bytes_per_batch
            {
                break;
            }
            let relative = Arc::clone(&inventory.paths[cursor.next_file]);
            let file_index = cursor.next_file;
            let candidate = root.join(relative.as_ref());
            let remaining_total = limits
                .total_read_bytes
                .saturating_sub(cumulative.read_bytes.saturating_add(delta.read_bytes));
            let remaining_batch = limits.read_bytes_per_batch.saturating_sub(delta.read_bytes);
            match read_search_file(
                &candidate,
                limits.file_bytes,
                remaining_total,
                remaining_batch,
            )? {
                FileRead::Ready { bytes, read_bytes } => {
                    delta.files = delta.files.saturating_add(1);
                    delta.read_bytes = delta.read_bytes.saturating_add(read_bytes);
                    cursor.next_file = cursor.next_file.saturating_add(1);
                    FileContinuation {
                        file_index,
                        relative,
                        bytes,
                        cursor: 0,
                        line: 1,
                        line_start: 0,
                    }
                }
                FileRead::Yield => break,
                FileRead::TotalLimit => {
                    truncated = true;
                    terminal = true;
                    break;
                }
                FileRead::Skipped { reason, read_bytes } => {
                    delta.files = delta.files.saturating_add(1);
                    delta.read_bytes = delta.read_bytes.saturating_add(read_bytes);
                    reason.observe(&mut delta);
                    cursor.next_file = cursor.next_file.saturating_add(1);
                    continue;
                }
            }
        };
        let completed = scan_file_matches(
            &mut file,
            query.as_bytes(),
            &mut matches,
            &mut batch_bytes,
            retained_results,
            retained_bytes,
            limits,
        )?;
        if !completed {
            cursor.file = Some(file);
            break;
        }
        if file.file_index.saturating_add(1) > cursor.next_file {
            cursor.next_file = file.file_index.saturating_add(1);
        }
    }
    let mut counters = cumulative;
    counters.add(delta);
    let continuation = (!terminal && !cancelled).then_some(cursor);
    Ok(SearchBatch {
        identity,
        matches: matches.into_boxed_slice(),
        bytes: batch_bytes,
        cursor: continuation,
        counters,
        terminal,
        truncated,
        cancelled,
    })
}

enum FileRead {
    Ready {
        bytes: Box<[u8]>,
        read_bytes: usize,
    },
    Yield,
    TotalLimit,
    Skipped {
        reason: SkipReason,
        read_bytes: usize,
    },
}

#[derive(Clone, Copy)]
enum SkipReason {
    Unreadable,
    InvalidUtf8,
    Binary,
    Oversized,
    Replaced,
}

impl SkipReason {
    fn observe(self, counters: &mut ScanCounters) {
        match self {
            Self::Unreadable => counters.unreadable = counters.unreadable.saturating_add(1),
            Self::InvalidUtf8 => counters.invalid_utf8 = counters.invalid_utf8.saturating_add(1),
            Self::Binary => counters.binary = counters.binary.saturating_add(1),
            Self::Oversized => counters.oversized = counters.oversized.saturating_add(1),
            Self::Replaced => counters.replaced = counters.replaced.saturating_add(1),
        }
    }
}

fn read_search_file(
    path: &Path,
    file_limit: usize,
    remaining_total: usize,
    remaining_batch: usize,
) -> Result<FileRead, ProjectSearchError> {
    let before = match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => metadata,
        Ok(_) => {
            return Ok(FileRead::Skipped {
                reason: SkipReason::Replaced,
                read_bytes: 0,
            });
        }
        Err(_) => {
            return Ok(FileRead::Skipped {
                reason: SkipReason::Unreadable,
                read_bytes: 0,
            });
        }
    };
    let Ok(length) = usize::try_from(before.len()) else {
        return Ok(FileRead::Skipped {
            reason: SkipReason::Oversized,
            read_bytes: 0,
        });
    };
    if length > file_limit {
        return Ok(FileRead::Skipped {
            reason: SkipReason::Oversized,
            read_bytes: 0,
        });
    }
    if length > remaining_total {
        return Ok(FileRead::TotalLimit);
    }
    if length > remaining_batch {
        return Ok(FileRead::Yield);
    }
    let fingerprint = FileFingerprint::new(&before);
    let Ok(mut file) = File::open(path) else {
        return Ok(FileRead::Skipped {
            reason: SkipReason::Unreadable,
            read_bytes: 0,
        });
    };
    let reserve = length.saturating_add(1).min(file_limit.saturating_add(1));
    let mut bytes = Vec::new();
    if bytes.try_reserve_exact(reserve).is_err() {
        return Err(ProjectSearchError::AllocationFailed);
    }
    let take = u64::try_from(length).unwrap_or(u64::MAX);
    if file.by_ref().take(take).read_to_end(&mut bytes).is_err() {
        return Ok(FileRead::Skipped {
            reason: SkipReason::Unreadable,
            read_bytes: bytes.len(),
        });
    }
    if bytes.len() > file_limit {
        return Ok(FileRead::Skipped {
            reason: SkipReason::Oversized,
            read_bytes: bytes.len(),
        });
    }
    let after = match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => metadata,
        _ => {
            return Ok(FileRead::Skipped {
                reason: SkipReason::Replaced,
                read_bytes: bytes.len(),
            });
        }
    };
    if fingerprint != FileFingerprint::new(&after) {
        return Ok(FileRead::Skipped {
            reason: SkipReason::Replaced,
            read_bytes: bytes.len(),
        });
    }
    if bytes.contains(&0) {
        return Ok(FileRead::Skipped {
            reason: SkipReason::Binary,
            read_bytes: bytes.len(),
        });
    }
    if str::from_utf8(&bytes).is_err() {
        return Ok(FileRead::Skipped {
            reason: SkipReason::InvalidUtf8,
            read_bytes: bytes.len(),
        });
    }
    let read_bytes = bytes.len();
    Ok(FileRead::Ready {
        bytes: bytes.into_boxed_slice(),
        read_bytes,
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FileFingerprint {
    len: u64,
    modified: Option<SystemTime>,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(unix)]
    modified_seconds: i64,
    #[cfg(unix)]
    modified_nanos: i64,
    #[cfg(unix)]
    changed_seconds: i64,
    #[cfg(unix)]
    changed_nanos: i64,
}

impl FileFingerprint {
    fn new(metadata: &fs::Metadata) -> Self {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt as _;
        Self {
            len: metadata.len(),
            modified: metadata.modified().ok(),
            #[cfg(unix)]
            device: metadata.dev(),
            #[cfg(unix)]
            inode: metadata.ino(),
            #[cfg(unix)]
            modified_seconds: metadata.mtime(),
            #[cfg(unix)]
            modified_nanos: metadata.mtime_nsec(),
            #[cfg(unix)]
            changed_seconds: metadata.ctime(),
            #[cfg(unix)]
            changed_nanos: metadata.ctime_nsec(),
        }
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "all result and batch budgets remain explicit"
)]
fn scan_file_matches(
    file: &mut FileContinuation,
    query: &[u8],
    matches: &mut Vec<ProjectMatch>,
    batch_bytes: &mut usize,
    retained_results: usize,
    retained_bytes: usize,
    limits: ProjectSearchLimits,
) -> Result<bool, ProjectSearchError> {
    while let Some(start) = find_literal(&file.bytes, query, file.cursor) {
        advance_position(
            &file.bytes[file.cursor..start],
            &mut file.line,
            &mut file.line_start,
            file.cursor,
        );
        let end = start
            .checked_add(query.len())
            .ok_or(ProjectSearchError::AllocationFailed)?;
        let excerpt = excerpt_for(&file.bytes, start, limits.excerpt_bytes)?;
        let line = file.line;
        let column = u32::try_from(start.saturating_sub(file.line_start).saturating_add(1))
            .map_err(|_| ProjectSearchError::AllocationFailed)?;
        let found = ProjectMatch {
            relative: Box::from(file.relative.as_ref()),
            excerpt,
            start,
            end,
            line,
            column,
        };
        let found_bytes = found.retained_bytes();
        if matches.len() == limits.batch_matches
            || batch_bytes.saturating_add(found_bytes) > limits.batch_bytes
            || retained_results.saturating_add(matches.len()) == limits.results
            || retained_bytes
                .saturating_add(*batch_bytes)
                .saturating_add(found_bytes)
                > limits.result_bytes
        {
            return Ok(false);
        }
        matches
            .try_reserve(1)
            .map_err(|_| ProjectSearchError::AllocationFailed)?;
        matches.push(found);
        *batch_bytes = batch_bytes.saturating_add(found_bytes);
        advance_position(
            &file.bytes[start..end],
            &mut file.line,
            &mut file.line_start,
            start,
        );
        file.cursor = end;
    }
    Ok(true)
}

fn find_literal(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || from > haystack.len().saturating_sub(needle.len()) {
        return None;
    }
    let mut skip = [needle.len(); 256];
    for (index, byte) in needle
        .iter()
        .copied()
        .enumerate()
        .take(needle.len().saturating_sub(1))
    {
        skip[usize::from(byte)] = needle.len().saturating_sub(index).saturating_sub(1);
    }
    let mut candidate = from;
    while candidate <= haystack.len().saturating_sub(needle.len()) {
        let end = candidate.saturating_add(needle.len());
        if haystack.get(candidate..end) == Some(needle) {
            return Some(candidate);
        }
        let last = haystack[end - 1];
        candidate = candidate.saturating_add(skip[usize::from(last)].max(1));
    }
    None
}

fn advance_position(bytes: &[u8], line: &mut u32, line_start: &mut usize, base: usize) {
    for (offset, byte) in bytes.iter().copied().enumerate() {
        if byte == b'\n' {
            *line = line.saturating_add(1);
            *line_start = base.saturating_add(offset).saturating_add(1);
        }
    }
}

fn excerpt_for(
    bytes: &[u8],
    match_start: usize,
    limit: usize,
) -> Result<Box<str>, ProjectSearchError> {
    let line_start = bytes[..match_start]
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |index| index.saturating_add(1));
    let line_end = bytes[match_start..]
        .iter()
        .position(|byte| *byte == b'\n')
        .map_or(bytes.len(), |offset| match_start.saturating_add(offset));
    let text = str::from_utf8(bytes).map_err(|_| ProjectSearchError::AllocationFailed)?;
    let mut start = line_start;
    let mut end = line_end;
    if end.saturating_sub(start) > limit {
        let half = limit / 2;
        start = match_start.saturating_sub(half).max(line_start);
        end = start.saturating_add(limit).min(line_end);
        start = end.saturating_sub(limit).max(line_start);
        while start < end && !text.is_char_boundary(start) {
            start = start.saturating_add(1);
        }
        while end > start && !text.is_char_boundary(end) {
            end = end.saturating_sub(1);
        }
    }
    let source = text
        .get(start..end)
        .ok_or(ProjectSearchError::AllocationFailed)?;
    let mut excerpt = String::new();
    excerpt
        .try_reserve_exact(source.len())
        .map_err(|_| ProjectSearchError::AllocationFailed)?;
    for character in source.trim_end_matches('\r').chars() {
        if character == '\t' || !character.is_control() {
            excerpt.push(character);
        } else {
            excerpt.push(' ');
        }
    }
    Ok(excerpt.into_boxed_str())
}

#[cfg(test)]
mod tests {
    use std::{
        fs, io,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    use super::*;

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(1);

    struct TempProject(PathBuf);

    impl TempProject {
        fn new() -> io::Result<Self> {
            let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir()
                .join(format!("alpine-project-search-{}-{id}", std::process::id()));
            fs::create_dir(&root)?;
            Ok(Self(root))
        }

        fn write(&self, relative: &str, bytes: &[u8]) -> io::Result<()> {
            let path = self.0.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(path, bytes)
        }
    }

    impl Drop for TempProject {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn tiny_limits() -> ProjectSearchLimits {
        ProjectSearchLimits::new(
            64, 32, 128, 2_048, 8, 1_024, 8_192, 16, 4_096, 2, 1_024, 64, 2, 1_024,
        )
    }

    fn drain(
        state: &mut ProjectSearchState,
        root: &Path,
    ) -> Result<Vec<ProjectSearchAdmission>, ProjectSearchError> {
        let mut admissions = Vec::new();
        for _ in 0..128 {
            let Some(request) = state.take_request(root)? else {
                break;
            };
            let admission = state.admit(request.execute());
            admissions.push(admission);
        }
        Ok(admissions)
    }

    #[test]
    fn search_is_lazy_streaming_bounded_and_releases_every_dynamic_byte()
    -> Result<(), Box<dyn Error>> {
        let project = TempProject::new()?;
        project.write("a.rs", b"needle one\nneedle two\nneedle three\n")?;
        project.write("b.rs", b"nothing\nneedle four\n")?;
        project.write("ignored.rs", b"needle ignored\n")?;
        project.write(".gitignore", b"ignored.rs\n")?;
        project.write("binary.bin", b"needle\0binary")?;
        project.write("invalid.txt", &[0xff, b'n', b'e'])?;

        let mut state = ProjectSearchState::with_test_limits(tiny_limits());
        assert!(state.open(7)?);
        assert!(state.take_request(&project.0)?.is_none());
        assert!(state.commit_text("needle")?);
        let admissions = drain(&mut state, &project.0)?;
        assert_eq!(admissions.first(), Some(&ProjectSearchAdmission::Inventory));
        assert!(admissions.contains(&ProjectSearchAdmission::Batch));
        assert_eq!(admissions.last(), Some(&ProjectSearchAdmission::Complete));
        let report = state.report();
        assert_eq!(report.retained_matches, 4);
        assert_eq!(report.batches, 3);
        assert_eq!(report.binary, 1);
        assert_eq!(report.invalid_utf8, 1);
        assert!(report.terminal);
        assert!(!report.truncated);
        assert!(report.result_bytes <= tiny_limits().result_bytes);
        assert!(report.inventory_bytes <= tiny_limits().inventory_bytes);
        let rows = state.visible_results(1, 1)?;
        assert_eq!(rows.len(), 2);
        assert!(rows[0].selected);
        assert!(rows[0].label.contains("a.rs:1:1"));
        assert!(state.close());
        let released = state.report();
        assert_eq!(released.query_bytes, 0);
        assert_eq!(released.inventory_files, 0);
        assert_eq!(released.retained_matches, 0);
        assert_eq!(released.retained_bytes, 0);
        assert!(released.peak_retained_bytes > 0);
        Ok(())
    }

    #[test]
    fn query_caps_composition_and_generation_failure_are_atomic() -> Result<(), Box<dyn Error>> {
        let mut state = ProjectSearchState::default();
        assert!(state.open(1)?);
        assert!(state.begin_composition());
        assert!(!state.begin_composition());
        assert!(matches!(
            state.update_composition("x", 2, 0),
            Err(ProjectSearchError::InvalidComposition)
        ));
        assert!(state.update_composition("x", 1, 0)?);
        assert!(state.cancel_composition());
        let exact = "q".repeat(MAX_QUERY_BYTES);
        assert!(state.commit_text(&exact)?);
        assert!(matches!(
            state.commit_text("x"),
            Err(ProjectSearchError::QueryTooLong { actual, limit })
                if actual == MAX_QUERY_BYTES + 1 && limit == MAX_QUERY_BYTES
        ));
        assert_eq!(state.query(), exact);
        let before = state.query().to_owned();
        state.exhaust_generations_for_test();
        assert!(matches!(
            state.delete_backward(),
            Err(ProjectSearchError::GenerationExhausted)
        ));
        assert_eq!(state.query(), before);
        Ok(())
    }

    #[test]
    fn stale_and_cancelled_outputs_never_publish() -> Result<(), Box<dyn Error>> {
        let project = TempProject::new()?;
        project.write("a.txt", b"alpha beta alpha")?;
        let mut state = ProjectSearchState::with_test_limits(tiny_limits());
        state.open(1)?;
        state.commit_text("alpha")?;
        let inventory = state.take_request(&project.0)?.ok_or("inventory")?;
        assert_eq!(
            state.admit(inventory.execute()),
            ProjectSearchAdmission::Inventory
        );
        let stale_request = state.take_request(&project.0)?.ok_or("search")?;
        state.commit_text("x")?;
        assert_eq!(
            state.admit(stale_request.execute()),
            ProjectSearchAdmission::Stale
        );
        assert_eq!(state.report().retained_matches, 0);
        assert!(state.close());
        assert!(state.report().stale_rejections > 0);
        Ok(())
    }

    #[test]
    fn failed_continuation_submission_terminates_without_duplicate_restart()
    -> Result<(), Box<dyn Error>> {
        let project = TempProject::new()?;
        project.write("a.txt", b"alpha alpha alpha")?;
        let mut state = ProjectSearchState::with_test_limits(tiny_limits());
        state.open(1)?;
        state.commit_text("alpha")?;
        let inventory = state.take_request(&project.0)?.ok_or("inventory")?;
        assert_eq!(
            state.admit(inventory.execute()),
            ProjectSearchAdmission::Inventory
        );
        let request = state.take_request(&project.0)?.ok_or("search")?;
        let identity = request.identity();
        assert!(state.reject_submission(identity));
        assert!(state.report().terminal);
        assert_eq!(state.report().retained_matches, 0);
        assert!(state.take_request(&project.0)?.is_none());
        assert!(state.display_text()?.contains("allocation failed"));
        drop(request);
        Ok(())
    }

    #[test]
    fn randomized_query_cancel_and_publication_sequences_preserve_bounds()
    -> Result<(), Box<dyn Error>> {
        let project = TempProject::new()?;
        project.write("a.txt", b"alpha beta alpha beta\n")?;
        project.write("b.txt", b"beta alpha beta alpha\n")?;
        let limits = tiny_limits();
        let mut state = ProjectSearchState::with_test_limits(limits);
        state.open(1)?;
        let mut random = 0x9e37_79b9_u64;
        for _ in 0..4_096 {
            random = random
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            match random % 8 {
                0 if state.query().len() < 12 => {
                    let _ = state.commit_text(if random & 8 == 0 { "a" } else { "b" })?;
                }
                1 => {
                    let _ = state.delete_backward()?;
                }
                2 => {
                    if let Some(request) = state.take_request(&project.0)? {
                        let _ = state.admit(request.execute());
                    }
                }
                3 => {
                    let _ = state.navigate(random & 16 == 0, 2);
                }
                4 => {
                    let _ = state.close();
                    let _ = state.open(1)?;
                }
                5 => {
                    let _ = state.begin_composition();
                    let _ = state.update_composition("x", 1, 0)?;
                    let _ = state.cancel_composition();
                }
                6 => {
                    let _ = state.visible_results(2, 1)?;
                }
                _ => {
                    let _ = state.display_text()?;
                }
            }
            let report = state.report();
            assert!(report.query_bytes <= MAX_QUERY_BYTES);
            assert!(report.inventory_files <= limits.files);
            assert!(report.inventory_bytes <= limits.inventory_bytes);
            assert!(report.retained_matches <= limits.results);
            assert!(report.result_bytes <= limits.result_bytes);
            assert!(report.visible_rows <= MAX_VISIBLE_RESULTS);
        }
        let _ = state.close();
        assert_eq!(state.report().retained_bytes, 0);
        Ok(())
    }

    #[test]
    fn byte_matching_excerpt_and_snapshot_verification_are_exact() -> Result<(), Box<dyn Error>> {
        assert_eq!(find_literal(b"ababa", b"aba", 0), Some(0));
        assert_eq!(find_literal(b"ababa", b"aba", 3), None);
        assert_eq!(find_literal(b"abcdef", b"def", 0), Some(3));
        assert_eq!(find_literal(b"abcdef", b"", 0), None);
        let text = format!("{}needle{}", "a".repeat(400), "b".repeat(400));
        let excerpt = excerpt_for(text.as_bytes(), 400, MAX_EXCERPT_BYTES)?;
        assert!(excerpt.len() <= MAX_EXCERPT_BYTES);
        assert!(excerpt.contains("needle"));
        let selected = SelectedProjectMatch {
            relative: Arc::from("a.txt"),
            query: Arc::from("needle"),
            start: 400,
            end: 406,
            line: 1,
        };
        let snapshot = alpine_text::Buffer::new(&text).snapshot();
        verify_snapshot_match(&snapshot, &selected)?;
        let stale = alpine_text::Buffer::new("different").snapshot();
        assert!(matches!(
            verify_snapshot_match(&stale, &selected),
            Err(ProjectSearchError::StaleMatch)
        ));
        Ok(())
    }

    #[test]
    fn invalid_limits_and_every_error_are_structured() {
        let mut invalid = ProjectSearchState::with_test_limits(ProjectSearchLimits::new(
            0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
        ));
        assert!(matches!(
            invalid.open(1),
            Err(ProjectSearchError::InvalidLimits)
        ));
        let errors = [
            ProjectSearchError::NoWorkspace,
            ProjectSearchError::InvalidLimits,
            ProjectSearchError::GenerationExhausted,
            ProjectSearchError::QueryTooLong {
                actual: 2,
                limit: 1,
            },
            ProjectSearchError::AllocationFailed,
            ProjectSearchError::InvalidComposition,
            ProjectSearchError::MissingSelection,
            ProjectSearchError::StaleMatch,
        ];
        for error in errors {
            assert!(!error.to_string().is_empty());
            assert!(error.source().is_none());
        }
    }
}
