//! Lazy, bounded workspace inventory and quick-open state.

use std::{
    cmp::Ordering,
    collections::BinaryHeap,
    error::Error,
    ffi::OsStr,
    fmt,
    mem::size_of,
    path::{Path, PathBuf},
    sync::Arc,
};

use ignore::WalkBuilder;
use std::fmt::Write as _;

pub(crate) const MAX_QUERY_BYTES: usize = 4 * 1_024;
pub(crate) const MAX_SCANNED_ENTRIES: usize = 250_000;
pub(crate) const MAX_RETAINED_PATHS: usize = 100_000;
pub(crate) const MAX_PATH_BYTES: usize = 4 * 1_024;
pub(crate) const MAX_RETAINED_PATH_BYTES: usize = 16 * 1_024 * 1_024;
pub(crate) const MAX_DEPTH: usize = 256;
pub(crate) const MAX_RESULTS: usize = 1_024;
pub(crate) const MAX_RESULT_METADATA_BYTES: usize = 1_024 * 1_024;
pub(crate) const MAX_VISIBLE_RESULTS: usize = 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct QuickOpenLimits {
    scanned: usize,
    paths: usize,
    path_bytes: usize,
    total_path_bytes: usize,
    depth: usize,
    results: usize,
    result_bytes: usize,
}

impl QuickOpenLimits {
    #[cfg(test)]
    pub(crate) const fn new(
        scanned: usize,
        paths: usize,
        path_bytes: usize,
        total_path_bytes: usize,
        depth: usize,
        results: usize,
        result_bytes: usize,
    ) -> Self {
        Self {
            scanned,
            paths,
            path_bytes,
            total_path_bytes,
            depth,
            results,
            result_bytes,
        }
    }

    const fn is_valid(self) -> bool {
        self.scanned > 0
            && self.paths > 0
            && self.path_bytes > 0
            && self.total_path_bytes > 0
            && self.depth > 0
            && self.results > 0
            && self.result_bytes >= size_of::<RankedPath>()
    }

    const fn result_capacity(self) -> usize {
        let by_bytes = self.result_bytes / size_of::<RankedPath>();
        if self.results < by_bytes {
            self.results
        } else {
            by_bytes
        }
    }
}

impl Default for QuickOpenLimits {
    fn default() -> Self {
        Self {
            scanned: MAX_SCANNED_ENTRIES,
            paths: MAX_RETAINED_PATHS,
            path_bytes: MAX_PATH_BYTES,
            total_path_bytes: MAX_RETAINED_PATH_BYTES,
            depth: MAX_DEPTH,
            results: MAX_RESULTS,
            result_bytes: MAX_RESULT_METADATA_BYTES,
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum QuickOpenError {
    NoWorkspace,
    InvalidLimits,
    GenerationExhausted,
    QueryTooLong { actual: usize, limit: usize },
    AllocationFailed,
    MissingSelection,
}

impl fmt::Display for QuickOpenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoWorkspace => formatter.write_str("quick open requires one local workspace"),
            Self::InvalidLimits => formatter.write_str("quick-open limits must be non-zero"),
            Self::GenerationExhausted => {
                formatter.write_str("quick-open request generation is exhausted")
            }
            Self::QueryTooLong { actual, limit } => {
                write!(
                    formatter,
                    "quick-open query is {actual} bytes; limit is {limit}"
                )
            }
            Self::AllocationFailed => formatter.write_str("quick-open allocation failed"),
            Self::MissingSelection => formatter.write_str("quick open has no selected result"),
        }
    }
}

impl Error for QuickOpenError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct InventoryIdentity {
    workspace: u64,
    generation: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct QueryIdentity {
    workspace: u64,
    inventory: u64,
    query: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RequestIdentity {
    Inventory(InventoryIdentity),
    Query(QueryIdentity),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct InventoryReport {
    pub(crate) scanned: usize,
    pub(crate) paths: usize,
    pub(crate) path_bytes: usize,
    pub(crate) omitted: usize,
    pub(crate) errors: usize,
    pub(crate) truncated: bool,
}

#[derive(Debug)]
pub(crate) struct Inventory {
    generation: u64,
    paths: Box<[Arc<str>]>,
    report: InventoryReport,
    first_error: Option<Arc<str>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RankedPath {
    index: usize,
    score: u32,
}

#[derive(Debug)]
pub(crate) struct QueryResult {
    identity: QueryIdentity,
    paths: Box<[RankedPath]>,
    matched: usize,
    bytes: usize,
    truncated: bool,
}

#[derive(Debug)]
pub(crate) enum QuickOpenRequest {
    Inventory {
        identity: InventoryIdentity,
        root: PathBuf,
        limits: QuickOpenLimits,
    },
    Query {
        identity: QueryIdentity,
        inventory: Arc<Inventory>,
        query: Box<str>,
        limits: QuickOpenLimits,
    },
}

impl QuickOpenRequest {
    pub(crate) const fn identity(&self) -> RequestIdentity {
        match self {
            Self::Inventory { identity, .. } => RequestIdentity::Inventory(*identity),
            Self::Query { identity, .. } => RequestIdentity::Query(*identity),
        }
    }

    pub(crate) fn execute(self) -> QuickOpenWorkerOutput {
        match self {
            Self::Inventory {
                identity,
                root,
                limits,
            } => QuickOpenWorkerOutput::Inventory {
                identity,
                result: build_inventory(identity, &root, limits),
            },
            Self::Query {
                identity,
                inventory,
                query,
                limits,
            } => QuickOpenWorkerOutput::Query {
                identity,
                result: rank_inventory(identity, &inventory, &query, limits),
            },
        }
    }
}

#[derive(Debug)]
pub(crate) enum QuickOpenWorkerOutput {
    Inventory {
        identity: InventoryIdentity,
        result: Result<Arc<Inventory>, QuickOpenError>,
    },
    Query {
        identity: QueryIdentity,
        result: Result<QueryResult, QuickOpenError>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum QuickOpenAdmission {
    Inventory,
    Query,
    Failed,
    Stale,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct HeapEntry(RankedPath);

impl Ord for HeapEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0
            .score
            .cmp(&other.0.score)
            .then_with(|| self.0.index.cmp(&other.0.index))
    }
}

impl PartialOrd for HeapEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug)]
pub(crate) struct QuickOpenState {
    open: bool,
    workspace: Option<u64>,
    inventory_generation: u64,
    query_generation: u64,
    inventory: Option<Arc<Inventory>>,
    result: Option<QueryResult>,
    query: String,
    composition: Option<Box<str>>,
    selected: usize,
    first_visible: usize,
    needs_inventory: bool,
    needs_query: bool,
    pending_inventory: Option<InventoryIdentity>,
    pending_query: Option<QueryIdentity>,
    error: Option<Arc<str>>,
    limits: QuickOpenLimits,
}

impl Default for QuickOpenState {
    fn default() -> Self {
        Self::with_limits(QuickOpenLimits::default())
    }
}

impl QuickOpenState {
    fn with_limits(limits: QuickOpenLimits) -> Self {
        Self {
            open: false,
            workspace: None,
            inventory_generation: 0,
            query_generation: 0,
            inventory: None,
            result: None,
            query: String::new(),
            composition: None,
            selected: 0,
            first_visible: 0,
            needs_inventory: false,
            needs_query: false,
            pending_inventory: None,
            pending_query: None,
            error: None,
            limits,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_test_limits(limits: QuickOpenLimits) -> Self {
        Self::with_limits(limits)
    }

    pub(crate) const fn is_open(&self) -> bool {
        self.open
    }

    pub(crate) fn open(&mut self, workspace: u64) -> Result<bool, QuickOpenError> {
        if !self.limits.is_valid() {
            return Err(QuickOpenError::InvalidLimits);
        }
        if self.open && self.workspace == Some(workspace) {
            return Ok(false);
        }
        self.open = true;
        self.error = None;
        self.composition = None;
        self.selected = 0;
        self.first_visible = 0;
        if self.workspace != Some(workspace) {
            self.workspace = Some(workspace);
            self.inventory = None;
            self.result = None;
        }
        if self.inventory.is_some() {
            self.bump_query()?;
            self.needs_query = true;
        } else {
            self.inventory_generation = self
                .inventory_generation
                .checked_add(1)
                .ok_or(QuickOpenError::GenerationExhausted)?;
            self.pending_inventory = None;
            self.needs_inventory = true;
        }
        Ok(true)
    }

    pub(crate) fn close(&mut self) -> bool {
        if !self.open {
            return false;
        }
        self.open = false;
        self.composition = None;
        self.pending_inventory = None;
        self.pending_query = None;
        true
    }

    pub(crate) fn begin_composition(&mut self) -> bool {
        if self.composition.is_some() {
            false
        } else {
            self.composition = Some(Box::default());
            true
        }
    }

    pub(crate) fn update_composition(&mut self, text: &str) -> Result<bool, QuickOpenError> {
        Self::check_query_len(self.query.len().saturating_add(text.len()))?;
        let changed = self.composition.as_deref() != Some(text);
        self.composition = Some(text.into());
        Ok(changed)
    }

    pub(crate) fn cancel_composition(&mut self) -> bool {
        self.composition.take().is_some()
    }

    pub(crate) fn commit_text(&mut self, text: &str) -> Result<bool, QuickOpenError> {
        let length =
            self.query
                .len()
                .checked_add(text.len())
                .ok_or(QuickOpenError::QueryTooLong {
                    actual: usize::MAX,
                    limit: MAX_QUERY_BYTES,
                })?;
        Self::check_query_len(length)?;
        self.composition = None;
        if text.is_empty() {
            return Ok(false);
        }
        self.query
            .try_reserve(text.len())
            .map_err(|_| QuickOpenError::AllocationFailed)?;
        self.query.push_str(text);
        self.query_changed()?;
        Ok(true)
    }

    pub(crate) fn delete_backward(&mut self) -> Result<bool, QuickOpenError> {
        self.composition = None;
        if self.query.pop().is_none() {
            return Ok(false);
        }
        self.query_changed()?;
        Ok(true)
    }

    pub(crate) fn take_request(&mut self, root: &Path) -> Option<QuickOpenRequest> {
        if !self.open {
            return None;
        }
        let workspace = self.workspace?;
        if self.needs_inventory {
            self.needs_inventory = false;
            let identity = InventoryIdentity {
                workspace,
                generation: self.inventory_generation,
            };
            self.pending_inventory = Some(identity);
            return Some(QuickOpenRequest::Inventory {
                identity,
                root: root.to_path_buf(),
                limits: self.limits,
            });
        }
        if self.needs_query {
            let inventory = Arc::clone(self.inventory.as_ref()?);
            self.needs_query = false;
            let identity = QueryIdentity {
                workspace,
                inventory: inventory.generation,
                query: self.query_generation,
            };
            self.pending_query = Some(identity);
            return Some(QuickOpenRequest::Query {
                identity,
                inventory,
                query: self.query.clone().into_boxed_str(),
                limits: self.limits,
            });
        }
        None
    }

    pub(crate) fn reject_submission(&mut self, identity: RequestIdentity) -> bool {
        let current = match identity {
            RequestIdentity::Inventory(identity) if self.pending_inventory == Some(identity) => {
                self.pending_inventory = None;
                self.needs_inventory = self.open;
                true
            }
            RequestIdentity::Query(identity) if self.pending_query == Some(identity) => {
                self.pending_query = None;
                self.needs_query = self.open && self.inventory.is_some();
                true
            }
            RequestIdentity::Inventory(_) | RequestIdentity::Query(_) => false,
        };
        if current {
            self.record_error(&QuickOpenError::AllocationFailed);
        }
        current
    }

    pub(crate) fn admit(&mut self, output: QuickOpenWorkerOutput) -> QuickOpenAdmission {
        match output {
            QuickOpenWorkerOutput::Inventory { identity, result } => {
                if !self.open
                    || self.workspace != Some(identity.workspace)
                    || self.inventory_generation != identity.generation
                    || self.pending_inventory != Some(identity)
                {
                    return QuickOpenAdmission::Stale;
                }
                self.pending_inventory = None;
                match result {
                    Ok(inventory) => {
                        self.inventory = Some(inventory);
                        self.result = None;
                        self.error = None;
                        if self.bump_query().is_err() {
                            self.record_error(&QuickOpenError::GenerationExhausted);
                            return QuickOpenAdmission::Failed;
                        }
                        self.needs_query = true;
                        QuickOpenAdmission::Inventory
                    }
                    Err(error) => {
                        self.record_error(&error);
                        QuickOpenAdmission::Failed
                    }
                }
            }
            QuickOpenWorkerOutput::Query { identity, result } => {
                let inventory = self.inventory.as_ref().map(|value| value.generation);
                if !self.open
                    || self.workspace != Some(identity.workspace)
                    || inventory != Some(identity.inventory)
                    || self.query_generation != identity.query
                    || self.pending_query != Some(identity)
                {
                    return QuickOpenAdmission::Stale;
                }
                self.pending_query = None;
                match result {
                    Ok(result) => {
                        if result.identity != identity {
                            self.needs_query = true;
                            return QuickOpenAdmission::Stale;
                        }
                        self.result = Some(result);
                        self.error = None;
                        self.selected = 0;
                        self.first_visible = 0;
                        QuickOpenAdmission::Query
                    }
                    Err(error) => {
                        self.record_error(&error);
                        QuickOpenAdmission::Failed
                    }
                }
            }
        }
    }

    pub(crate) fn navigate(&mut self, forward: bool, visible_rows: usize) -> bool {
        let count = self.result.as_ref().map_or(0, |result| result.paths.len());
        if count == 0 {
            return false;
        }
        let previous = self.selected;
        self.selected = if forward {
            (self.selected + 1) % count
        } else if self.selected == 0 {
            count - 1
        } else {
            self.selected - 1
        };
        let visible_rows = visible_rows.clamp(1, MAX_VISIBLE_RESULTS);
        if self.selected < self.first_visible {
            self.first_visible = self.selected;
        } else if self.selected >= self.first_visible.saturating_add(visible_rows) {
            self.first_visible = self.selected.saturating_add(1).saturating_sub(visible_rows);
        }
        self.selected != previous
    }

    pub(crate) fn selected_path(&self) -> Result<Arc<str>, QuickOpenError> {
        let inventory = self
            .inventory
            .as_ref()
            .ok_or(QuickOpenError::MissingSelection)?;
        let ranked = self
            .result
            .as_ref()
            .and_then(|result| result.paths.get(self.selected))
            .ok_or(QuickOpenError::MissingSelection)?;
        inventory
            .paths
            .get(ranked.index)
            .cloned()
            .ok_or(QuickOpenError::MissingSelection)
    }

    pub(crate) fn visible_results(
        &self,
        visible_rows: usize,
        overscan: usize,
    ) -> Vec<(Arc<str>, bool)> {
        let (Some(inventory), Some(result)) = (&self.inventory, &self.result) else {
            return Vec::new();
        };
        let start = self.first_visible.saturating_sub(overscan);
        let end = self
            .first_visible
            .saturating_add(visible_rows.min(MAX_VISIBLE_RESULTS))
            .saturating_add(overscan)
            .min(result.paths.len())
            .min(start.saturating_add(MAX_VISIBLE_RESULTS));
        result.paths[start..end]
            .iter()
            .enumerate()
            .filter_map(|(offset, ranked)| {
                inventory
                    .paths
                    .get(ranked.index)
                    .map(|path| (Arc::clone(path), start + offset == self.selected))
            })
            .collect()
    }

    pub(crate) fn display_text(&self) -> Result<String, QuickOpenError> {
        let composition = self.composition.as_deref().unwrap_or_default();
        let count = self.result.as_ref().map_or(0, |result| result.paths.len());
        let total = self
            .inventory
            .as_ref()
            .map_or(0, |value| value.report.paths);
        let status = self.error.as_deref().or_else(|| {
            self.inventory
                .as_ref()
                .and_then(|value| value.first_error.as_deref())
        });
        let suffix = status.map_or_else(String::new, |message| format!(" | {message}"));
        let inventory_evidence = self
            .inventory
            .as_ref()
            .map_or_else(String::new, |inventory| {
                let report = inventory.report;
                let truncated = if report.truncated { ", truncated" } else { "" };
                format!(
                    " | index {}/{} entries, {} B, {} omitted, {} errors{}",
                    report.paths,
                    report.scanned,
                    report.path_bytes,
                    report.omitted,
                    report.errors,
                    truncated
                )
            });
        let result_evidence = self.result.as_ref().map_or_else(String::new, |result| {
            let truncated = if result.truncated { ", truncated" } else { "" };
            format!(
                " | result {} matched, {} B{}",
                result.matched, result.bytes, truncated
            )
        });
        let mut display = String::new();
        display
            .try_reserve(
                self.query
                    .len()
                    .saturating_add(composition.len())
                    .saturating_add(suffix.len())
                    .saturating_add(inventory_evidence.len())
                    .saturating_add(result_evidence.len())
                    .saturating_add(64),
            )
            .map_err(|_| QuickOpenError::AllocationFailed)?;
        write!(
            display,
            "Quick Open: {}{} ({count}/{total}){inventory_evidence}{result_evidence}{suffix}",
            self.query, composition,
        )
        .map_err(|_| QuickOpenError::AllocationFailed)?;
        Ok(display)
    }

    #[cfg(test)]
    pub(crate) fn inventory_report(&self) -> Option<InventoryReport> {
        self.inventory.as_ref().map(|inventory| inventory.report)
    }

    #[cfg(test)]
    pub(crate) fn result_report(&self) -> Option<(usize, usize, bool, QueryIdentity)> {
        self.result.as_ref().map(|result| {
            (
                result.matched,
                result.bytes,
                result.truncated,
                result.identity,
            )
        })
    }

    #[cfg(test)]
    pub(crate) fn query(&self) -> &str {
        &self.query
    }

    #[cfg(test)]
    pub(crate) fn exhaust_generations_for_test(&mut self) {
        self.inventory_generation = u64::MAX;
        self.query_generation = u64::MAX;
    }

    fn query_changed(&mut self) -> Result<(), QuickOpenError> {
        self.bump_query()?;
        self.needs_query = self.inventory.is_some();
        self.result = None;
        self.selected = 0;
        self.first_visible = 0;
        self.error = None;
        Ok(())
    }

    fn bump_query(&mut self) -> Result<(), QuickOpenError> {
        self.query_generation = self
            .query_generation
            .checked_add(1)
            .ok_or(QuickOpenError::GenerationExhausted)?;
        self.pending_query = None;
        Ok(())
    }

    fn check_query_len(length: usize) -> Result<(), QuickOpenError> {
        if length > MAX_QUERY_BYTES {
            Err(QuickOpenError::QueryTooLong {
                actual: length,
                limit: MAX_QUERY_BYTES,
            })
        } else {
            Ok(())
        }
    }

    fn record_error(&mut self, error: &QuickOpenError) {
        self.error = Some(Arc::from(error.to_string()));
    }
}

fn build_inventory(
    identity: InventoryIdentity,
    root: &Path,
    limits: QuickOpenLimits,
) -> Result<Arc<Inventory>, QuickOpenError> {
    if !limits.is_valid() {
        return Err(QuickOpenError::InvalidLimits);
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
    let mut scanned = 0_usize;
    let mut path_bytes = 0_usize;
    let mut omitted = 0_usize;
    let mut errors = 0_usize;
    let mut first_error = None;
    let mut truncated = false;
    for item in builder.build() {
        if scanned == limits.scanned {
            truncated = true;
            omitted = omitted.saturating_add(1);
            break;
        }
        let entry = match item {
            Ok(entry) if entry.depth() == 0 => continue,
            Ok(entry) => entry,
            Err(error) => {
                scanned = scanned.saturating_add(1);
                errors = errors.saturating_add(1);
                omitted = omitted.saturating_add(1);
                if first_error.is_none() {
                    first_error = Some(Arc::from(error.to_string()));
                }
                continue;
            }
        };
        scanned = scanned.saturating_add(1);
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        let Ok(relative) = entry.path().strip_prefix(root) else {
            omitted = omitted.saturating_add(1);
            continue;
        };
        let Some(relative) = relative.to_str() else {
            omitted = omitted.saturating_add(1);
            continue;
        };
        let length = relative.len();
        let next_bytes = path_bytes.checked_add(length);
        if length == 0
            || length > limits.path_bytes
            || paths.len() == limits.paths
            || next_bytes.is_none_or(|bytes| bytes > limits.total_path_bytes)
        {
            omitted = omitted.saturating_add(1);
            truncated = true;
            continue;
        }
        paths
            .try_reserve(1)
            .map_err(|_| QuickOpenError::AllocationFailed)?;
        paths.push(Arc::from(relative));
        path_bytes = next_bytes.ok_or(QuickOpenError::AllocationFailed)?;
    }
    paths.sort_unstable_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
    let before = paths.len();
    paths.dedup();
    omitted = omitted.saturating_add(before.saturating_sub(paths.len()));
    path_bytes = paths.iter().map(|path| path.len()).sum();
    paths.shrink_to_fit();
    let report = InventoryReport {
        scanned,
        paths: paths.len(),
        path_bytes,
        omitted,
        errors,
        truncated,
    };
    Ok(Arc::new(Inventory {
        generation: identity.generation,
        paths: paths.into_boxed_slice(),
        report,
        first_error,
    }))
}

fn rank_inventory(
    identity: QueryIdentity,
    inventory: &Inventory,
    query: &str,
    limits: QuickOpenLimits,
) -> Result<QueryResult, QuickOpenError> {
    if !limits.is_valid() {
        return Err(QuickOpenError::InvalidLimits);
    }
    if query.len() > MAX_QUERY_BYTES {
        return Err(QuickOpenError::QueryTooLong {
            actual: query.len(),
            limit: MAX_QUERY_BYTES,
        });
    }
    let capacity = limits.result_capacity();
    let mut heap = BinaryHeap::new();
    heap.try_reserve_exact(capacity)
        .map_err(|_| QuickOpenError::AllocationFailed)?;
    let mut matched = 0_usize;
    for (index, path) in inventory.paths.iter().enumerate() {
        let Some(score) = score_path(path, query)? else {
            continue;
        };
        matched = matched.saturating_add(1);
        let candidate = HeapEntry(RankedPath { index, score });
        if heap.len() < capacity {
            heap.push(candidate);
        } else if heap.peek().is_some_and(|worst| candidate < *worst) {
            let _ = heap.pop();
            heap.push(candidate);
        }
    }
    let mut paths: Vec<RankedPath> = heap.into_iter().map(|entry| entry.0).collect();
    paths.sort_unstable_by(|left, right| {
        left.score
            .cmp(&right.score)
            .then_with(|| left.index.cmp(&right.index))
    });
    let bytes = paths
        .len()
        .checked_mul(size_of::<RankedPath>())
        .ok_or(QuickOpenError::AllocationFailed)?;
    Ok(QueryResult {
        identity,
        truncated: matched > paths.len(),
        matched,
        bytes,
        paths: paths.into_boxed_slice(),
    })
}

fn score_path(path: &str, query: &str) -> Result<Option<u32>, QuickOpenError> {
    if query.is_empty() {
        return Ok(Some(0));
    }
    let path_bytes = path.as_bytes();
    let query_bytes = query.as_bytes();
    let basename = path_bytes
        .iter()
        .rposition(|byte| *byte == b'/')
        .map_or(0, |index| index.saturating_add(1));
    let mut query_index = 0_usize;
    let mut previous = None;
    let mut score =
        u32::try_from(path_bytes.len()).map_err(|_| QuickOpenError::AllocationFailed)?;
    for (index, byte) in path_bytes.iter().copied().enumerate() {
        let Some(query_byte) = query_bytes.get(query_index).copied() else {
            break;
        };
        if !byte.eq_ignore_ascii_case(&query_byte) {
            continue;
        }
        score = score.saturating_add(u32::from(index < basename) * 8);
        if let Some(previous) = previous {
            let gap = index.saturating_sub(previous).saturating_sub(1);
            let gap = u32::try_from(gap).map_err(|_| QuickOpenError::AllocationFailed)?;
            score = score.saturating_add(gap.saturating_mul(4));
            score = score.saturating_sub(u32::from(gap == 0));
        }
        previous = Some(index);
        query_index = query_index.saturating_add(1);
    }
    Ok((query_index == query_bytes.len()).then_some(score))
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        sync::atomic::{AtomicU64, Ordering},
    };

    use super::*;

    static NEXT_ROOT: AtomicU64 = AtomicU64::new(1);

    struct TestRoot(PathBuf);

    impl TestRoot {
        fn new() -> Result<Self, std::io::Error> {
            let id = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("alpine-quick-open-{}-{id}", std::process::id()));
            fs::create_dir(&path)?;
            Ok(Self(path))
        }

        fn write(&self, relative: &str) -> Result<(), std::io::Error> {
            let path = self.0.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(path, "")
        }
    }

    impl Drop for TestRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn inventory(root: &Path, limits: QuickOpenLimits) -> Result<Arc<Inventory>, QuickOpenError> {
        build_inventory(
            InventoryIdentity {
                workspace: 1,
                generation: 1,
            },
            root,
            limits,
        )
    }

    #[test]
    fn project_rules_hidden_files_and_git_exclusion_are_deterministic() -> Result<(), Box<dyn Error>>
    {
        let root = TestRoot::new()?;
        fs::write(root.0.join(".gitignore"), "ignored/\n*.tmp\n!keep.tmp\n")?;
        fs::write(root.0.join(".ignore"), "private.txt\n")?;
        for path in [
            "src/main.rs",
            "src/nested/mod.rs",
            "ignored/lost.rs",
            "drop.tmp",
            "keep.tmp",
            "private.txt",
            ".github/workflows/ci.yml",
            ".git/objects/private",
        ] {
            root.write(path)?;
        }
        let inventory = inventory(&root.0, QuickOpenLimits::default())?;
        let paths: Vec<&str> = inventory.paths.iter().map(AsRef::as_ref).collect();
        assert!(paths.contains(&"src/main.rs"));
        assert!(paths.contains(&"src/nested/mod.rs"));
        assert!(paths.contains(&"keep.tmp"));
        assert!(paths.contains(&".github/workflows/ci.yml"));
        assert!(!paths.contains(&"ignored/lost.rs"));
        assert!(!paths.contains(&"drop.tmp"));
        assert!(!paths.contains(&"private.txt"));
        assert!(!paths.iter().any(|path| path.starts_with(".git/")));
        assert!(
            paths
                .windows(2)
                .all(|pair| pair[0].as_bytes() < pair[1].as_bytes())
        );
        Ok(())
    }

    #[test]
    fn inventory_ranking_and_projection_enforce_caps() -> Result<(), Box<dyn Error>> {
        let root = TestRoot::new()?;
        for index in 0..12 {
            root.write(&format!("file-{index}.rs"))?;
        }
        let limits = QuickOpenLimits::new(32, 4, 64, 40, 8, 2, 2 * size_of::<RankedPath>());
        let inventory = inventory(&root.0, limits)?;
        assert!(inventory.report.paths <= 4);
        assert!(inventory.report.path_bytes <= 40);
        assert!(inventory.report.omitted >= 8);
        assert!(inventory.report.truncated);
        let identity = QueryIdentity {
            workspace: 1,
            inventory: 1,
            query: 1,
        };
        let result = rank_inventory(identity, &inventory, "file", limits)?;
        assert_eq!(result.paths.len(), 2);
        assert_eq!(result.bytes, 2 * size_of::<RankedPath>());
        assert!(result.truncated || inventory.paths.len() <= 2);
        Ok(())
    }

    #[test]
    fn state_rejects_stale_work_and_navigates_current_results() -> Result<(), Box<dyn Error>> {
        let root = TestRoot::new()?;
        root.write("alpha.rs")?;
        root.write("beta.rs")?;
        let mut state = QuickOpenState::default();
        assert!(state.open(1)?);
        let stale_request = state.take_request(&root.0).ok_or("stale request")?;
        let stale_identity = stale_request.identity();
        assert!(state.close());
        assert!(state.open(1)?);
        assert_eq!(
            state.admit(stale_request.execute()),
            QuickOpenAdmission::Stale
        );
        assert!(!state.reject_submission(stale_identity));

        let request = state.take_request(&root.0).ok_or("inventory")?;
        assert_eq!(
            state.admit(request.execute()),
            QuickOpenAdmission::Inventory
        );
        let request = state.take_request(&root.0).ok_or("query")?;
        assert_eq!(state.admit(request.execute()), QuickOpenAdmission::Query);
        assert_eq!(state.selected_path()?.as_ref(), "alpha.rs");
        assert!(state.navigate(true, 1));
        assert_eq!(state.selected_path()?.as_ref(), "beta.rs");
        assert!(state.commit_text("beta")?);
        let request = state.take_request(&root.0).ok_or("filtered query")?;
        assert_eq!(state.admit(request.execute()), QuickOpenAdmission::Query);
        assert_eq!(state.selected_path()?.as_ref(), "beta.rs");
        assert!(state.display_text()?.contains("1/2"));
        Ok(())
    }

    #[test]
    fn query_and_generation_failures_preserve_state() -> Result<(), Box<dyn Error>> {
        let mut state = QuickOpenState::default();
        assert!(state.open(1)?);
        assert!(state.begin_composition());
        assert!(!state.begin_composition());
        assert!(state.update_composition("alpha")?);
        assert!(state.cancel_composition());
        assert!(state.commit_text("alpha")?);
        assert!(state.delete_backward()?);
        assert_eq!(state.query(), "alph");
        let before = state.query().to_owned();
        assert!(matches!(
            state.commit_text(&"x".repeat(MAX_QUERY_BYTES)),
            Err(QuickOpenError::QueryTooLong { .. })
        ));
        assert_eq!(state.query(), before);

        let mut exhausted = QuickOpenState::default();
        exhausted.exhaust_generations_for_test();
        assert_eq!(exhausted.open(1), Err(QuickOpenError::GenerationExhausted));
        let mut invalid =
            QuickOpenState::with_test_limits(QuickOpenLimits::new(0, 0, 0, 0, 0, 0, 0));
        assert_eq!(invalid.open(1), Err(QuickOpenError::InvalidLimits));
        Ok(())
    }
}
