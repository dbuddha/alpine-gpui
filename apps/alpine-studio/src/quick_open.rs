//! Lazy, bounded workspace inventory and quick-open state.

use std::{
    cmp::Ordering,
    collections::BinaryHeap,
    error::Error,
    ffi::OsStr,
    fmt,
    mem::size_of,
    path::{Component, Path, PathBuf},
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

    fn result_capacity(self) -> usize {
        let by_bytes = self.result_bytes / size_of::<RankedPath>();
        self.results.min(by_bytes)
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
        if self.selected >= self.first_visible.saturating_add(visible_rows) {
            self.first_visible = self.selected.saturating_add(1).saturating_sub(visible_rows);
        }
        self.first_visible = self.first_visible.min(self.selected);
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
        let Some(relative) = portable_relative_path(root, entry.path())? else {
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

fn portable_relative_path(root: &Path, path: &Path) -> Result<Option<String>, QuickOpenError> {
    let Ok(relative) = path.strip_prefix(root) else {
        return Ok(None);
    };
    let mut rendered = String::new();
    rendered
        .try_reserve(relative.as_os_str().as_encoded_bytes().len())
        .map_err(|_| QuickOpenError::AllocationFailed)?;
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
        } else if heap
            .peek()
            .is_some_and(|worst| candidate.cmp(worst).is_lt())
        {
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
            fs::create_dir_all(path.parent().unwrap_or(Path::new(".")))?;
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

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "one defensive state corpus proves each independent admission and failure branch"
    )]
    fn defensive_state_and_worker_failures_are_structured() -> Result<(), Box<dyn Error>> {
        for error in [
            QuickOpenError::NoWorkspace,
            QuickOpenError::InvalidLimits,
            QuickOpenError::GenerationExhausted,
            QuickOpenError::QueryTooLong {
                actual: MAX_QUERY_BYTES + 1,
                limit: MAX_QUERY_BYTES,
            },
            QuickOpenError::AllocationFailed,
            QuickOpenError::MissingSelection,
        ] {
            assert!(!error.to_string().is_empty());
            assert!(error.source().is_none());
        }

        let root = TestRoot::new()?;
        root.write("alpha.rs")?;
        root.write("beta.rs")?;
        let invalid_limits = QuickOpenLimits::new(0, 0, 0, 0, 0, 0, 0);
        let inventory_identity = InventoryIdentity {
            workspace: 1,
            generation: 1,
        };
        assert!(matches!(
            build_inventory(inventory_identity, &root.0, invalid_limits),
            Err(QuickOpenError::InvalidLimits)
        ));
        let empty_inventory = Inventory {
            generation: 1,
            paths: Vec::new().into_boxed_slice(),
            report: InventoryReport {
                scanned: 0,
                paths: 0,
                path_bytes: 0,
                omitted: 0,
                errors: 0,
                truncated: false,
            },
            first_error: None,
        };
        let query_identity = QueryIdentity {
            workspace: 1,
            inventory: 1,
            query: 1,
        };
        assert!(matches!(
            rank_inventory(query_identity, &empty_inventory, "", invalid_limits),
            Err(QuickOpenError::InvalidLimits)
        ));
        assert!(matches!(
            rank_inventory(
                query_identity,
                &empty_inventory,
                &"x".repeat(MAX_QUERY_BYTES + 1),
                QuickOpenLimits::default(),
            ),
            Err(QuickOpenError::QueryTooLong { .. })
        ));

        let mut state = QuickOpenState::default();
        assert!(!state.close());
        assert!(!state.delete_backward()?);
        assert!(!state.commit_text("")?);
        assert!(!state.navigate(true, 0));
        assert_eq!(state.selected_path(), Err(QuickOpenError::MissingSelection));
        assert!(state.visible_results(1, 1).is_empty());
        assert!(state.open(1)?);
        assert!(!state.open(1)?);
        let request = state.take_request(&root.0).ok_or("inventory request")?;
        let identity = request.identity();
        assert!(
            !state.reject_submission(RequestIdentity::Query(QueryIdentity {
                workspace: 9,
                inventory: 9,
                query: 9,
            }))
        );
        assert!(state.reject_submission(identity));
        let retry = state.take_request(&root.0).ok_or("inventory retry")?;
        assert_eq!(state.admit(retry.execute()), QuickOpenAdmission::Inventory);
        let query = state.take_request(&root.0).ok_or("query request")?;
        let query_identity = query.identity();
        assert!(state.reject_submission(query_identity));
        let query = state.take_request(&root.0).ok_or("query retry")?;
        assert_eq!(state.admit(query.execute()), QuickOpenAdmission::Query);
        assert_eq!(
            state.admit(QuickOpenWorkerOutput::Query {
                identity: QueryIdentity {
                    workspace: 9,
                    inventory: 9,
                    query: 9,
                },
                result: Err(QuickOpenError::AllocationFailed),
            }),
            QuickOpenAdmission::Stale
        );
        assert!(state.navigate(false, 1));
        assert_eq!(state.selected_path()?.as_ref(), "beta.rs");
        assert!(state.navigate(false, 1));
        assert_eq!(state.selected_path()?.as_ref(), "alpha.rs");
        assert!(state.close());
        assert!(state.open(1)?);
        assert!(matches!(
            state.take_request(&root.0),
            Some(QuickOpenRequest::Query { .. })
        ));

        let mut failed_inventory = QuickOpenState::default();
        assert!(failed_inventory.open(1)?);
        let request = failed_inventory
            .take_request(&root.0)
            .ok_or("failed inventory request")?;
        assert!(matches!(request.identity(), RequestIdentity::Inventory(_)));
        let identity = InventoryIdentity {
            workspace: 1,
            generation: 1,
        };
        assert_eq!(
            failed_inventory.admit(QuickOpenWorkerOutput::Inventory {
                identity,
                result: Err(QuickOpenError::AllocationFailed),
            }),
            QuickOpenAdmission::Failed
        );
        assert!(
            failed_inventory
                .display_text()?
                .contains("allocation failed")
        );

        let mut exhausted_query = QuickOpenState::default();
        assert!(exhausted_query.open(1)?);
        let request = exhausted_query
            .take_request(&root.0)
            .ok_or("exhausted inventory request")?;
        exhausted_query.query_generation = u64::MAX;
        assert_eq!(
            exhausted_query.admit(request.execute()),
            QuickOpenAdmission::Failed
        );

        let mut failed_query = QuickOpenState::default();
        assert!(failed_query.open(1)?);
        let request = failed_query
            .take_request(&root.0)
            .ok_or("query inventory request")?;
        assert_eq!(
            failed_query.admit(request.execute()),
            QuickOpenAdmission::Inventory
        );
        let request = failed_query
            .take_request(&root.0)
            .ok_or("failed query request")?;
        assert!(matches!(request.identity(), RequestIdentity::Query(_)));
        let identity = QueryIdentity {
            workspace: 1,
            inventory: 1,
            query: 1,
        };
        assert_eq!(
            failed_query.admit(QuickOpenWorkerOutput::Query {
                identity,
                result: Err(QuickOpenError::AllocationFailed),
            }),
            QuickOpenAdmission::Failed
        );

        assert!(failed_query.take_request(&root.0).is_none());
        assert!(failed_query.commit_text("x")?);
        let request = failed_query
            .take_request(&root.0)
            .ok_or("query after input change")?;
        assert!(matches!(request.identity(), RequestIdentity::Query(_)));
        let identity = QueryIdentity {
            workspace: 1,
            inventory: 1,
            query: 2,
        };
        let inventory = Arc::clone(failed_query.inventory.as_ref().ok_or("inventory")?);
        let wrong_identity = QueryIdentity {
            query: identity.query.saturating_add(1),
            ..identity
        };
        let result = rank_inventory(wrong_identity, &inventory, "x", QuickOpenLimits::default())?;
        assert_eq!(
            failed_query.admit(QuickOpenWorkerOutput::Query {
                identity,
                result: Ok(result),
            }),
            QuickOpenAdmission::Stale
        );
        Ok(())
    }

    #[test]
    fn portable_paths_and_scan_caps_are_explicit() -> Result<(), Box<dyn Error>> {
        let root = TestRoot::new()?;
        root.write("src/main.rs")?;
        root.write("src/lib.rs")?;
        assert_eq!(
            portable_relative_path(&root.0, &root.0.join("src").join("main.rs"))?,
            Some("src/main.rs".to_owned())
        );
        assert_eq!(portable_relative_path(&root.0, &root.0)?, None);
        assert_eq!(
            portable_relative_path(&root.0, &root.0.join("src/../src/main.rs"))?,
            None
        );
        assert_eq!(
            portable_relative_path(&root.0, &std::env::temp_dir().join("outside.rs"))?,
            None
        );
        let limits = QuickOpenLimits::new(1, 8, 128, 1_024, 8, 8, 1_024);
        let inventory = inventory(&root.0, limits)?;
        assert_eq!(inventory.report.scanned, 1);
        assert!(inventory.report.truncated);
        assert!(inventory.report.omitted >= 1);
        let ranked_inventory = Inventory {
            generation: 1,
            paths: vec![Arc::from("zzzzm.rs"), Arc::from("m.rs")].into_boxed_slice(),
            report: InventoryReport {
                scanned: 2,
                paths: 2,
                path_bytes: 12,
                omitted: 0,
                errors: 0,
                truncated: false,
            },
            first_error: None,
        };
        let rank_limits = QuickOpenLimits::new(8, 8, 128, 1_024, 8, 1, 1_024);
        let identity = QueryIdentity {
            workspace: 1,
            inventory: 1,
            query: 1,
        };
        let ranked = rank_inventory(identity, &ranked_inventory, "m", rank_limits)?;
        assert_eq!(ranked.paths[0].index, 1);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn invalid_utf8_and_unreadable_entries_are_omitted() -> Result<(), Box<dyn Error>> {
        use std::os::unix::ffi::OsStringExt;
        #[cfg(not(miri))]
        use std::os::unix::fs::PermissionsExt;

        let root = TestRoot::new()?;
        let invalid = std::ffi::OsString::from_vec(vec![b'b', b'a', b'd', 0xff]);
        let invalid = root.0.join(invalid);
        assert_eq!(portable_relative_path(&root.0, &invalid)?, None);
        #[cfg(all(target_os = "linux", not(miri)))]
        fs::write(&invalid, "")?;
        #[cfg(not(miri))]
        {
            let blocked = root.0.join("blocked");
            fs::create_dir(&blocked)?;
            fs::write(blocked.join("hidden.rs"), "")?;
            fs::set_permissions(&blocked, fs::Permissions::from_mode(0o000))?;
            let inventory = inventory(&root.0, QuickOpenLimits::default());
            fs::set_permissions(&blocked, fs::Permissions::from_mode(0o700))?;
            let inventory = inventory?;
            assert!(inventory.report.omitted >= 1);
            assert!(inventory.report.errors >= 1);
            assert!(inventory.first_error.is_some());
        }
        Ok(())
    }

    fn current_inventory_state(identity: InventoryIdentity) -> QuickOpenState {
        QuickOpenState {
            open: true,
            workspace: Some(identity.workspace),
            inventory_generation: identity.generation,
            pending_inventory: Some(identity),
            ..QuickOpenState::default()
        }
    }

    fn failed_inventory_output(identity: InventoryIdentity) -> QuickOpenWorkerOutput {
        QuickOpenWorkerOutput::Inventory {
            identity,
            result: Err(QuickOpenError::AllocationFailed),
        }
    }

    fn inventory_for_state(generation: u64) -> Arc<Inventory> {
        Arc::new(Inventory {
            generation,
            paths: vec![Arc::from("alpha.rs")].into_boxed_slice(),
            report: InventoryReport {
                scanned: 1,
                paths: 1,
                path_bytes: 8,
                omitted: 0,
                errors: 0,
                truncated: false,
            },
            first_error: None,
        })
    }

    fn current_query_state(identity: QueryIdentity) -> QuickOpenState {
        QuickOpenState {
            open: true,
            workspace: Some(identity.workspace),
            inventory: Some(inventory_for_state(identity.inventory)),
            query_generation: identity.query,
            pending_query: Some(identity),
            ..QuickOpenState::default()
        }
    }

    fn failed_query_output(identity: QueryIdentity) -> QuickOpenWorkerOutput {
        QuickOpenWorkerOutput::Query {
            identity,
            result: Err(QuickOpenError::AllocationFailed),
        }
    }

    #[test]
    fn locked_limits_and_capacity_boundaries_are_exact() -> Result<(), Box<dyn Error>> {
        assert_eq!(MAX_QUERY_BYTES, 4_096);
        assert_eq!(MAX_SCANNED_ENTRIES, 250_000);
        assert_eq!(MAX_RETAINED_PATHS, 100_000);
        assert_eq!(MAX_PATH_BYTES, 4_096);
        assert_eq!(MAX_RETAINED_PATH_BYTES, 16_777_216);
        assert_eq!(MAX_DEPTH, 256);
        assert_eq!(MAX_RESULTS, 1_024);
        assert_eq!(MAX_RESULT_METADATA_BYTES, 1_048_576);
        assert_eq!(MAX_VISIBLE_RESULTS, 256);

        let ranked_bytes = size_of::<RankedPath>();
        let valid = QuickOpenLimits::new(1, 1, 1, 1, 1, 1, ranked_bytes);
        assert!(valid.is_valid());
        for invalid in [
            QuickOpenLimits::new(0, 1, 1, 1, 1, 1, ranked_bytes),
            QuickOpenLimits::new(1, 0, 1, 1, 1, 1, ranked_bytes),
            QuickOpenLimits::new(1, 1, 0, 1, 1, 1, ranked_bytes),
            QuickOpenLimits::new(1, 1, 1, 0, 1, 1, ranked_bytes),
            QuickOpenLimits::new(1, 1, 1, 1, 0, 1, ranked_bytes),
            QuickOpenLimits::new(1, 1, 1, 1, 1, 0, ranked_bytes),
            QuickOpenLimits::new(1, 1, 1, 1, 1, 1, ranked_bytes - 1),
        ] {
            assert!(!invalid.is_valid());
        }
        assert_eq!(
            QuickOpenLimits::new(1, 1, 1, 1, 1, 3, 2 * ranked_bytes).result_capacity(),
            2
        );
        assert_eq!(
            QuickOpenLimits::new(1, 1, 1, 1, 1, 2, 3 * ranked_bytes).result_capacity(),
            2
        );

        let mut exact_query = QuickOpenState::default();
        assert!(exact_query.commit_text(&"x".repeat(MAX_QUERY_BYTES))?);
        assert_eq!(exact_query.query().len(), MAX_QUERY_BYTES);
        assert!(exact_query.begin_composition());
        assert!(!exact_query.update_composition("")?);
        assert!(exact_query.cancel_composition());
        assert!(!exact_query.cancel_composition());
        Ok(())
    }

    #[test]
    fn each_admission_identity_guard_is_independent() {
        let inventory_identity = InventoryIdentity {
            workspace: 7,
            generation: 11,
        };
        let mut closed_inventory = current_inventory_state(inventory_identity);
        closed_inventory.open = false;
        assert_eq!(
            closed_inventory.admit(failed_inventory_output(inventory_identity)),
            QuickOpenAdmission::Stale
        );
        let mut wrong_workspace = current_inventory_state(inventory_identity);
        wrong_workspace.workspace = Some(8);
        assert_eq!(
            wrong_workspace.admit(failed_inventory_output(inventory_identity)),
            QuickOpenAdmission::Stale
        );
        let mut wrong_generation = current_inventory_state(inventory_identity);
        wrong_generation.inventory_generation = 12;
        assert_eq!(
            wrong_generation.admit(failed_inventory_output(inventory_identity)),
            QuickOpenAdmission::Stale
        );
        let mut no_pending_inventory = current_inventory_state(inventory_identity);
        no_pending_inventory.pending_inventory = None;
        assert_eq!(
            no_pending_inventory.admit(failed_inventory_output(inventory_identity)),
            QuickOpenAdmission::Stale
        );

        let query_identity = QueryIdentity {
            workspace: 7,
            inventory: 11,
            query: 13,
        };
        let mut closed_query = current_query_state(query_identity);
        closed_query.open = false;
        assert_eq!(
            closed_query.admit(failed_query_output(query_identity)),
            QuickOpenAdmission::Stale
        );
        assert!(closed_query.reject_submission(RequestIdentity::Query(query_identity)));
        assert!(!closed_query.needs_query);
        let mut wrong_query_workspace = current_query_state(query_identity);
        wrong_query_workspace.workspace = Some(8);
        assert_eq!(
            wrong_query_workspace.admit(failed_query_output(query_identity)),
            QuickOpenAdmission::Stale
        );
        let mut wrong_inventory = current_query_state(query_identity);
        wrong_inventory.inventory = Some(inventory_for_state(12));
        assert_eq!(
            wrong_inventory.admit(failed_query_output(query_identity)),
            QuickOpenAdmission::Stale
        );
        let mut wrong_query_generation = current_query_state(query_identity);
        wrong_query_generation.query_generation = 14;
        assert_eq!(
            wrong_query_generation.admit(failed_query_output(query_identity)),
            QuickOpenAdmission::Stale
        );
        let mut no_pending_query = current_query_state(query_identity);
        no_pending_query.pending_query = None;
        assert_eq!(
            no_pending_query.admit(failed_query_output(query_identity)),
            QuickOpenAdmission::Stale
        );
    }

    #[test]
    fn navigation_window_edges_are_exact() {
        let identity = QueryIdentity {
            workspace: 1,
            inventory: 1,
            query: 1,
        };
        let result = || QueryResult {
            identity,
            paths: (0..5)
                .map(|index| RankedPath { index, score: 0 })
                .collect::<Vec<_>>()
                .into_boxed_slice(),
            matched: 5,
            bytes: 5 * size_of::<RankedPath>(),
            truncated: false,
        };
        let mut downward = QuickOpenState {
            result: Some(result()),
            selected: 3,
            first_visible: 3,
            ..QuickOpenState::default()
        };
        assert!(downward.navigate(false, 2));
        assert_eq!(downward.selected, 2);
        assert_eq!(downward.first_visible, 2);

        let mut lower_edge = QuickOpenState {
            result: Some(result()),
            selected: 1,
            ..QuickOpenState::default()
        };
        assert!(lower_edge.navigate(true, 2));
        assert_eq!(lower_edge.selected, 2);
        assert_eq!(lower_edge.first_visible, 1);
    }

    #[test]
    fn inventory_limits_are_independent_and_inclusive() -> Result<(), Box<dyn Error>> {
        let metadata_bytes = 8 * size_of::<RankedPath>();

        let scanned_root = TestRoot::new()?;
        scanned_root.write("only.rs")?;
        let scanned = inventory(&scanned_root.0, QuickOpenLimits::default())?;
        assert_eq!(scanned.report.scanned, 1);

        let exact_path = TestRoot::new()?;
        exact_path.write("four")?;
        let path_limits = QuickOpenLimits::new(8, 8, 4, 32, 8, 8, metadata_bytes);
        let exact = inventory(&exact_path.0, path_limits)?;
        assert_eq!(exact.paths.len(), 1);
        assert_eq!(exact.report.path_bytes, 4);

        let long_path = TestRoot::new()?;
        long_path.write("five5")?;
        let long = inventory(&long_path.0, path_limits)?;
        assert!(long.paths.is_empty());
        assert!(long.report.truncated);

        let path_cap = TestRoot::new()?;
        path_cap.write("a")?;
        path_cap.write("b")?;
        let cap_limits = QuickOpenLimits::new(8, 1, 8, 32, 8, 8, metadata_bytes);
        let capped = inventory(&path_cap.0, cap_limits)?;
        assert_eq!(capped.paths.len(), 1);
        assert!(capped.report.truncated);

        let total_exact = TestRoot::new()?;
        total_exact.write("aa")?;
        total_exact.write("bb")?;
        let exact_total_limits = QuickOpenLimits::new(8, 8, 8, 4, 8, 8, metadata_bytes);
        let exact_total = inventory(&total_exact.0, exact_total_limits)?;
        assert_eq!(exact_total.paths.len(), 2);
        assert_eq!(exact_total.report.path_bytes, 4);
        let over_total_limits = QuickOpenLimits::new(8, 8, 8, 3, 8, 8, metadata_bytes);
        let over_total = inventory(&total_exact.0, over_total_limits)?;
        assert_eq!(over_total.paths.len(), 1);
        assert!(over_total.report.truncated);
        Ok(())
    }

    #[test]
    fn ranking_boundaries_and_scores_are_exact() -> Result<(), Box<dyn Error>> {
        let lower = HeapEntry(RankedPath { index: 1, score: 2 });
        let higher = HeapEntry(RankedPath { index: 2, score: 3 });
        assert_eq!(lower.partial_cmp(&higher), Some(std::cmp::Ordering::Less));
        assert_eq!(score_path("src/main.rs", "m")?, Some(11));
        assert_eq!(score_path("src/main.rs", "sm")?, Some(31));
        assert_eq!(score_path("src/main.rs", "ma")?, Some(10));
        assert_eq!(score_path("src/main.rs", "z")?, None);

        let identity = QueryIdentity {
            workspace: 1,
            inventory: 1,
            query: 1,
        };
        let paths = vec![Arc::from("alpha.rs"), Arc::from("beta.rs")].into_boxed_slice();
        let inventory = Inventory {
            generation: 1,
            report: InventoryReport {
                scanned: 2,
                paths: 2,
                path_bytes: 15,
                omitted: 0,
                errors: 0,
                truncated: false,
            },
            paths,
            first_error: None,
        };
        let limits = QuickOpenLimits::new(8, 8, 32, 64, 8, 2, 2 * size_of::<RankedPath>());
        let exact = rank_inventory(identity, &inventory, "", limits)?;
        assert_eq!(exact.matched, 2);
        assert_eq!(exact.paths.len(), 2);
        assert!(!exact.truncated);
        let maximum_query = "x".repeat(MAX_QUERY_BYTES);
        let maximum = rank_inventory(identity, &inventory, &maximum_query, limits)?;
        assert_eq!(maximum.matched, 0);
        assert!(!maximum.truncated);
        Ok(())
    }
}
