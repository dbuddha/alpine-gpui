//! Lazy, bounded local workspace tree state.

use std::{
    collections::BinaryHeap,
    error::Error,
    ffi::OsStr,
    fmt, fs, io,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use ignore::{
    Match,
    gitignore::{Gitignore, GitignoreBuilder},
};

pub(crate) const MAX_SCANNED_PER_DIRECTORY: usize = 16_384;
pub(crate) const MAX_CHILDREN_PER_DIRECTORY: usize = 4_096;
pub(crate) const MAX_DIRECTORY_PATH_BYTES: usize = 1_024 * 1_024;
pub(crate) const MAX_CACHED_DIRECTORIES: usize = 4_096;
pub(crate) const MAX_CACHED_ENTRIES: usize = 65_536;
pub(crate) const MAX_CACHED_PATH_BYTES: usize = 8 * 1_024 * 1_024;
pub(crate) const MAX_PATH_BYTES: usize = 4 * 1_024;
pub(crate) const MAX_DEPTH: usize = 256;
pub(crate) const MAX_VISIBLE_ROWS: usize = 512;
const MAX_ERROR_BYTES: usize = 512;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FileTreeLimits {
    scanned_per_directory: usize,
    children_per_directory: usize,
    directory_path_bytes: usize,
    cached_directories: usize,
    cached_entries: usize,
    cached_path_bytes: usize,
    path_bytes: usize,
    depth: usize,
    visible_rows: usize,
}

impl Default for FileTreeLimits {
    fn default() -> Self {
        Self {
            scanned_per_directory: MAX_SCANNED_PER_DIRECTORY,
            children_per_directory: MAX_CHILDREN_PER_DIRECTORY,
            directory_path_bytes: MAX_DIRECTORY_PATH_BYTES,
            cached_directories: MAX_CACHED_DIRECTORIES,
            cached_entries: MAX_CACHED_ENTRIES,
            cached_path_bytes: MAX_CACHED_PATH_BYTES,
            path_bytes: MAX_PATH_BYTES,
            depth: MAX_DEPTH,
            visible_rows: MAX_VISIBLE_ROWS,
        }
    }
}

impl FileTreeLimits {
    #[cfg(test)]
    #[allow(
        clippy::too_many_arguments,
        reason = "the test constructor varies each independent hard ceiling"
    )]
    pub(crate) const fn new(
        scanned_per_directory: usize,
        children_per_directory: usize,
        directory_path_bytes: usize,
        cached_directories: usize,
        cached_entries: usize,
        cached_path_bytes: usize,
        path_bytes: usize,
        depth: usize,
        visible_rows: usize,
    ) -> Self {
        Self {
            scanned_per_directory,
            children_per_directory,
            directory_path_bytes,
            cached_directories,
            cached_entries,
            cached_path_bytes,
            path_bytes,
            depth,
            visible_rows,
        }
    }

    const fn is_valid(self) -> bool {
        self.scanned_per_directory > 0
            && self.children_per_directory > 0
            && self.directory_path_bytes > 0
            && self.cached_directories > 0
            && self.cached_entries > 0
            && self.cached_path_bytes > 0
            && self.path_bytes > 0
            && self.depth > 0
            && self.visible_rows > 0
    }
}

#[derive(Debug)]
pub(crate) enum FileTreeError {
    NoWorkspace,
    InvalidLimits,
    GenerationExhausted,
    AllocationFailed,
    MissingSelection,
    InvalidRelativePath(PathBuf),
    PathDepthExceeded {
        actual: usize,
        limit: usize,
    },
    Symlink(PathBuf),
    NotDirectory(PathBuf),
    CacheLimitExceeded {
        resource: &'static str,
        limit: usize,
    },
    Ignore(Arc<str>),
    Io {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
}

impl FileTreeError {
    fn io(operation: &'static str, path: &Path, source: io::Error) -> Self {
        Self::Io {
            operation,
            path: path.to_path_buf(),
            source,
        }
    }
}

impl fmt::Display for FileTreeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoWorkspace => formatter.write_str("file tree requires one local workspace"),
            Self::InvalidLimits => formatter.write_str("file-tree limits must be non-zero"),
            Self::GenerationExhausted => {
                formatter.write_str("file-tree request generation is exhausted")
            }
            Self::AllocationFailed => formatter.write_str("file-tree allocation failed"),
            Self::MissingSelection => formatter.write_str("file tree has no selected row"),
            Self::InvalidRelativePath(path) => {
                write!(
                    formatter,
                    "invalid file-tree relative path: {}",
                    path.display()
                )
            }
            Self::PathDepthExceeded { actual, limit } => write!(
                formatter,
                "file-tree path depth {actual} exceeds the {limit}-component ceiling"
            ),
            Self::Symlink(path) => {
                write!(
                    formatter,
                    "file-tree path contains a symlink: {}",
                    path.display()
                )
            }
            Self::NotDirectory(path) => {
                write!(
                    formatter,
                    "file-tree path is not a directory: {}",
                    path.display()
                )
            }
            Self::CacheLimitExceeded { resource, limit } => {
                write!(
                    formatter,
                    "file-tree {resource} exceeds the {limit} ceiling"
                )
            }
            Self::Ignore(message) => write!(formatter, "file-tree ignore rules failed: {message}"),
            Self::Io {
                operation,
                path,
                source,
            } => write!(
                formatter,
                "file-tree {operation} failed for {}: {source}",
                path.display()
            ),
        }
    }
}

impl Error for FileTreeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum FileTreeEntryKind {
    Directory,
    File,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct FileTreeEntry {
    kind: FileTreeEntryKind,
    path: Arc<str>,
    name_start: u16,
    depth: u16,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct DirectoryReport {
    pub(crate) scanned: usize,
    pub(crate) retained: usize,
    pub(crate) path_bytes: usize,
    pub(crate) omitted: usize,
    pub(crate) errors: usize,
    pub(crate) truncated: bool,
}

#[derive(Debug)]
struct DirectoryResult {
    entries: Box<[FileTreeEntry]>,
    report: DirectoryReport,
    first_error: Option<Arc<str>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FileTreeRequestIdentity {
    workspace: u64,
    tree: u64,
    directory: u64,
    request: u64,
}

#[derive(Debug)]
pub(crate) struct FileTreeRequest {
    identity: FileTreeRequestIdentity,
    root: PathBuf,
    relative: Arc<str>,
    limits: FileTreeLimits,
}

impl FileTreeRequest {
    pub(crate) const fn identity(&self) -> FileTreeRequestIdentity {
        self.identity
    }

    pub(crate) fn execute(self) -> FileTreeWorkerOutput {
        let result = read_directory(&self.root, &self.relative, self.limits);
        FileTreeWorkerOutput {
            identity: self.identity,
            relative: self.relative,
            result,
        }
    }
}

#[derive(Debug)]
pub(crate) struct FileTreeWorkerOutput {
    identity: FileTreeRequestIdentity,
    relative: Arc<str>,
    result: Result<DirectoryResult, FileTreeError>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FileTreeAdmission {
    Directory,
    Failed,
    Stale,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum DirectoryLoad {
    Dormant,
    Loading(FileTreeRequestIdentity),
    Ready,
    Failed,
}

#[derive(Debug)]
struct DirectoryNode {
    path: Arc<str>,
    generation: u64,
    expanded: bool,
    load: DirectoryLoad,
    entries: Box<[FileTreeEntry]>,
    prefix_rows: Box<[usize]>,
    report: DirectoryReport,
    first_error: Option<Arc<str>>,
}

impl DirectoryNode {
    fn new(path: Arc<str>, generation: u64) -> Self {
        Self {
            path,
            generation,
            expanded: true,
            load: DirectoryLoad::Dormant,
            entries: Box::default(),
            prefix_rows: Box::default(),
            report: DirectoryReport::default(),
            first_error: None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct VisibleFileTreeRow {
    pub(crate) index: usize,
    pub(crate) path: Arc<str>,
    name_start: u16,
    pub(crate) depth: usize,
    pub(crate) kind: FileTreeEntryKind,
    pub(crate) expanded: bool,
    pub(crate) selected: bool,
}

impl VisibleFileTreeRow {
    pub(crate) fn label(&self) -> &str {
        &self.path[usize::from(self.name_start)..]
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum FileTreeAction {
    Changed,
    Open(Arc<str>),
}

#[derive(Debug)]
pub(crate) struct FileTreeState {
    visible: bool,
    active: bool,
    focused: bool,
    workspace: Option<u64>,
    tree_generation: u64,
    directory_generation: u64,
    request_generation: u64,
    pending: Option<FileTreeRequestIdentity>,
    nodes: Vec<DirectoryNode>,
    selected: Option<usize>,
    selected_path: Option<Arc<str>>,
    error: Option<Arc<str>>,
    retained_entries: usize,
    retained_path_bytes: usize,
    limits: FileTreeLimits,
}

impl Default for FileTreeState {
    fn default() -> Self {
        Self::with_limits(FileTreeLimits::default())
    }
}

impl FileTreeState {
    fn with_limits(limits: FileTreeLimits) -> Self {
        Self {
            visible: true,
            active: false,
            focused: false,
            workspace: None,
            tree_generation: 0,
            directory_generation: 0,
            request_generation: 0,
            pending: None,
            nodes: Vec::new(),
            selected: None,
            selected_path: None,
            error: None,
            retained_entries: 0,
            retained_path_bytes: 0,
            limits,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_test_limits(limits: FileTreeLimits) -> Self {
        Self::with_limits(limits)
    }

    #[cfg(test)]
    pub(crate) fn exhaust_tree_generation(&mut self) {
        self.tree_generation = u64::MAX;
    }

    pub(crate) const fn is_visible(&self) -> bool {
        self.visible
    }

    pub(crate) const fn is_active(&self) -> bool {
        self.active
    }

    pub(crate) const fn is_focused(&self) -> bool {
        self.focused
    }

    pub(crate) fn activate(&mut self, workspace: u64) -> Result<bool, FileTreeError> {
        if !self.limits.is_valid() {
            return Err(FileTreeError::InvalidLimits);
        }
        let changed = !self.visible || !self.active || !self.focused;
        self.visible = true;
        self.active = true;
        self.focused = true;
        if self.workspace != Some(workspace) {
            self.reset_for_workspace(workspace)?;
        } else if self.nodes.is_empty() {
            self.insert_root()?;
        }
        Ok(changed)
    }

    pub(crate) fn hide(&mut self) -> bool {
        if !self.visible {
            return false;
        }
        self.visible = false;
        self.focused = false;
        self.cancel_pending();
        true
    }

    pub(crate) fn unfocus(&mut self) -> bool {
        let changed = self.focused;
        self.focused = false;
        changed
    }

    fn reset_for_workspace(&mut self, workspace: u64) -> Result<(), FileTreeError> {
        self.tree_generation = self
            .tree_generation
            .checked_add(1)
            .ok_or(FileTreeError::GenerationExhausted)?;
        self.workspace = Some(workspace);
        self.pending = None;
        self.nodes.clear();
        self.selected = None;
        self.selected_path = None;
        self.error = None;
        self.retained_entries = 0;
        self.retained_path_bytes = 0;
        self.insert_root()
    }

    fn insert_root(&mut self) -> Result<(), FileTreeError> {
        self.directory_generation = self
            .directory_generation
            .checked_add(1)
            .ok_or(FileTreeError::GenerationExhausted)?;
        self.nodes
            .try_reserve(1)
            .map_err(|_| FileTreeError::AllocationFailed)?;
        self.nodes
            .push(DirectoryNode::new(Arc::from(""), self.directory_generation));
        Ok(())
    }

    pub(crate) fn restore_session(
        &mut self,
        workspace: u64,
        state: &crate::session::SessionFileTree,
    ) -> Result<(), FileTreeError> {
        let mut restored = Self::with_limits(self.limits);
        restored.visible = self.visible;
        restored.tree_generation = self.tree_generation;
        restored.directory_generation = self.directory_generation;
        restored.request_generation = self.request_generation;
        restored.restore_session_in_place(workspace, state)?;
        *self = restored;
        Ok(())
    }

    fn restore_session_in_place(
        &mut self,
        workspace: u64,
        state: &crate::session::SessionFileTree,
    ) -> Result<(), FileTreeError> {
        self.reset_for_workspace(workspace)?;
        if state.expanded.len() >= self.limits.cached_directories {
            return Err(FileTreeError::CacheLimitExceeded {
                resource: "directory count",
                limit: self.limits.cached_directories,
            });
        }
        self.nodes
            .try_reserve_exact(state.expanded.len())
            .map_err(|_| FileTreeError::AllocationFailed)?;
        for path in &state.expanded {
            let path = restored_identity(path, self.limits)?;
            if self.node_index(&path).is_some() {
                return Err(FileTreeError::InvalidRelativePath(PathBuf::from(
                    path.as_ref(),
                )));
            }
            self.directory_generation = self
                .directory_generation
                .checked_add(1)
                .ok_or(FileTreeError::GenerationExhausted)?;
            let insertion = self
                .nodes
                .binary_search_by(|node| node.path.as_ref().cmp(path.as_ref()))
                .unwrap_or_else(|index| index);
            self.nodes.insert(
                insertion,
                DirectoryNode::new(path, self.directory_generation),
            );
        }
        self.selected_path = state
            .selected
            .as_deref()
            .map(|path| restored_identity(path, self.limits))
            .transpose()?;
        self.selected = None;
        Ok(())
    }

    pub(crate) fn session_state(&self) -> Result<crate::session::SessionFileTree, FileTreeError> {
        if self.workspace.is_none() {
            return Ok(crate::session::SessionFileTree::default());
        }
        let mut expanded = Vec::new();
        for node in &self.nodes {
            if node.expanded && !node.path.is_empty() {
                expanded
                    .try_reserve(1)
                    .map_err(|_| FileTreeError::AllocationFailed)?;
                expanded.push(PathBuf::from(node.path.as_ref()));
            }
        }
        Ok(crate::session::SessionFileTree {
            expanded,
            selected: self.selected_path.as_deref().map(PathBuf::from),
        })
    }

    pub(crate) fn take_request(&mut self, root: &Path) -> Option<FileTreeRequest> {
        if !self.visible || !self.active || self.pending.is_some() {
            return None;
        }
        let workspace = self.workspace?;
        let index = self
            .nodes
            .iter()
            .position(|node| node.expanded && node.load == DirectoryLoad::Dormant)?;
        self.request_generation = self.request_generation.checked_add(1)?;
        let identity = FileTreeRequestIdentity {
            workspace,
            tree: self.tree_generation,
            directory: self.nodes[index].generation,
            request: self.request_generation,
        };
        self.nodes[index].load = DirectoryLoad::Loading(identity);
        self.pending = Some(identity);
        Some(FileTreeRequest {
            identity,
            root: root.to_path_buf(),
            relative: Arc::clone(&self.nodes[index].path),
            limits: self.limits,
        })
    }

    pub(crate) fn reject_submission(&mut self, identity: FileTreeRequestIdentity) -> bool {
        if self.pending != Some(identity) {
            return false;
        }
        self.pending = None;
        if let Some(node) = self
            .nodes
            .iter_mut()
            .find(|node| node.generation == identity.directory)
        {
            node.load = DirectoryLoad::Dormant;
        }
        self.record_error(&FileTreeError::AllocationFailed);
        true
    }

    pub(crate) fn admit(&mut self, output: FileTreeWorkerOutput) -> FileTreeAdmission {
        let identity = output.identity;
        if !self.visible
            || !self.active
            || self.workspace != Some(identity.workspace)
            || self.tree_generation != identity.tree
            || self.pending != Some(identity)
        {
            return FileTreeAdmission::Stale;
        }
        let Some(index) = self.nodes.iter().position(|node| {
            node.generation == identity.directory
                && node.path.as_ref() == output.relative.as_ref()
                && node.load == DirectoryLoad::Loading(identity)
        }) else {
            return FileTreeAdmission::Stale;
        };
        self.pending = None;
        match output.result {
            Ok(result) => match self.publish_directory(index, result) {
                Ok(()) => FileTreeAdmission::Directory,
                Err(error) => {
                    self.nodes[index].load = DirectoryLoad::Failed;
                    self.record_error(&error);
                    self.rebind_selected();
                    FileTreeAdmission::Failed
                }
            },
            Err(error) => {
                self.nodes[index].load = DirectoryLoad::Failed;
                self.nodes[index].first_error = Some(bounded_message(&error));
                self.record_error(&error);
                self.rebind_selected();
                FileTreeAdmission::Failed
            }
        }
    }

    fn publish_directory(
        &mut self,
        index: usize,
        result: DirectoryResult,
    ) -> Result<(), FileTreeError> {
        #[cfg(test)]
        if take_test_fault(FileTreeFault::Publish) {
            return Err(FileTreeError::AllocationFailed);
        }
        let available_entries = self
            .limits
            .cached_entries
            .saturating_sub(self.retained_entries);
        let available_bytes = self
            .limits
            .cached_path_bytes
            .saturating_sub(self.retained_path_bytes);
        let mut retained = Vec::new();
        retained
            .try_reserve(result.entries.len().min(available_entries))
            .map_err(|_| FileTreeError::AllocationFailed)?;
        let mut path_bytes = 0_usize;
        let mut omitted = result.report.omitted;
        for entry in Vec::from(result.entries) {
            if retained.len() == available_entries
                || entry.path.len() > available_bytes.saturating_sub(path_bytes)
            {
                omitted = omitted.saturating_add(1);
                continue;
            }
            path_bytes = path_bytes.saturating_add(entry.path.len());
            retained.push(entry);
        }
        retained.shrink_to_fit();
        let report = DirectoryReport {
            retained: retained.len(),
            path_bytes,
            omitted,
            truncated: result.report.truncated || omitted > result.report.omitted,
            ..result.report
        };
        self.retained_entries = self.retained_entries.saturating_add(retained.len());
        self.retained_path_bytes = self.retained_path_bytes.saturating_add(path_bytes);
        {
            let node = &mut self.nodes[index];
            node.entries = retained.into_boxed_slice();
            node.report = report;
            node.first_error = result.first_error;
            node.load = DirectoryLoad::Ready;
        }
        self.recompute_prefixes()?;
        self.rebind_selected();
        self.error = self.nodes[index].first_error.clone();
        Ok(())
    }

    pub(crate) fn visible_rows(
        &self,
        first_visible: usize,
        visible_rows: usize,
        overscan: usize,
    ) -> Result<Vec<VisibleFileTreeRow>, FileTreeError> {
        let start = first_visible.saturating_sub(overscan);
        let count = visible_rows
            .min(self.limits.visible_rows)
            .saturating_add(overscan.saturating_mul(2))
            .min(self.limits.visible_rows);
        let mut rows = Vec::new();
        rows.try_reserve(count)
            .map_err(|_| FileTreeError::AllocationFailed)?;
        self.collect_rows("", start, count, start, &mut rows);
        Ok(rows)
    }

    fn collect_rows(
        &self,
        directory: &str,
        mut skip: usize,
        limit: usize,
        base: usize,
        rows: &mut Vec<VisibleFileTreeRow>,
    ) {
        if rows.len() >= limit {
            return;
        }
        let Some(node_index) = self.node_index(directory) else {
            return;
        };
        let node = &self.nodes[node_index];
        if !node.expanded || node.load != DirectoryLoad::Ready {
            return;
        }
        let mut entry_index = node.prefix_rows.partition_point(|end| *end <= skip);
        let mut previous = entry_index
            .checked_sub(1)
            .and_then(|index| node.prefix_rows.get(index).copied())
            .unwrap_or(0);
        while entry_index < node.entries.len() && rows.len() < limit {
            let entry = node.entries[entry_index].clone();
            let local_skip = skip.saturating_sub(previous);
            if local_skip == 0 {
                let expanded = entry.kind == FileTreeEntryKind::Directory
                    && self
                        .node_index(&entry.path)
                        .is_some_and(|index| self.nodes[index].expanded);
                let index = base.saturating_add(rows.len());
                rows.push(VisibleFileTreeRow {
                    index,
                    path: Arc::clone(&entry.path),
                    name_start: entry.name_start,
                    depth: usize::from(entry.depth),
                    kind: entry.kind,
                    expanded,
                    selected: self.selected == Some(index),
                });
                if expanded {
                    self.collect_rows(&entry.path, 0, limit, base, rows);
                }
            } else if entry.kind == FileTreeEntryKind::Directory {
                self.collect_rows(&entry.path, local_skip.saturating_sub(1), limit, base, rows);
            }
            previous = node.prefix_rows[entry_index];
            skip = previous;
            entry_index += 1;
        }
    }

    pub(crate) fn navigate(&mut self, forward: bool, visible_rows: usize) -> bool {
        let count = self.total_rows();
        if count == 0 {
            return false;
        }
        let previous = self.selected;
        let selected = previous.unwrap_or(0);
        let selected = if forward {
            selected.saturating_add(1).min(count - 1)
        } else {
            selected.saturating_sub(1)
        };
        self.selected = Some(selected);
        self.selected_path = self.visible_path_at(selected);
        let _ = visible_rows.min(self.limits.visible_rows);
        self.selected != previous
    }

    pub(crate) fn activate_row(&mut self, index: usize) -> Result<FileTreeAction, FileTreeError> {
        let index = index.min(self.total_rows().saturating_sub(1));
        let row = self.row_at(index)?.ok_or(FileTreeError::MissingSelection)?;
        self.selected = Some(index);
        self.selected_path = Some(row.path);
        self.activate_selected()
    }

    pub(crate) fn activate_selected(&mut self) -> Result<FileTreeAction, FileTreeError> {
        let index = self.selected.ok_or(FileTreeError::MissingSelection)?;
        let row = self.row_at(index)?.ok_or(FileTreeError::MissingSelection)?;
        match row.kind {
            FileTreeEntryKind::File => Ok(FileTreeAction::Open(row.path)),
            FileTreeEntryKind::Directory => {
                self.toggle_directory(row.path)?;
                Ok(FileTreeAction::Changed)
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn select_path(&mut self, path: &str) -> Result<bool, FileTreeError> {
        let total = self.total_rows();
        for index in 0..total {
            if self
                .row_at(index)?
                .is_some_and(|row| row.path.as_ref() == path)
            {
                let changed = self.selected != Some(index);
                self.selected = Some(index);
                self.selected_path = Some(Arc::from(path));
                return Ok(changed);
            }
        }
        Ok(false)
    }

    fn row_at(&self, index: usize) -> Result<Option<VisibleFileTreeRow>, FileTreeError> {
        Ok(self.visible_rows(index, 1, 0)?.into_iter().next())
    }

    fn toggle_directory(&mut self, path: Arc<str>) -> Result<(), FileTreeError> {
        if let Some(index) = self.node_index(&path) {
            let expanded = self.nodes[index].expanded;
            self.nodes[index].expanded = !expanded;
            if expanded {
                self.remove_descendants(&path);
            } else if self.nodes[index].load == DirectoryLoad::Failed {
                self.nodes[index].load = DirectoryLoad::Dormant;
            }
        } else {
            if self.nodes.len() == self.limits.cached_directories {
                return Err(FileTreeError::CacheLimitExceeded {
                    resource: "directory count",
                    limit: self.limits.cached_directories,
                });
            }
            self.directory_generation = self
                .directory_generation
                .checked_add(1)
                .ok_or(FileTreeError::GenerationExhausted)?;
            let insertion = self
                .nodes
                .binary_search_by(|node| node.path.as_ref().cmp(path.as_ref()))
                .unwrap_or_else(|index| index);
            self.nodes
                .try_reserve(1)
                .map_err(|_| FileTreeError::AllocationFailed)?;
            self.nodes.insert(
                insertion,
                DirectoryNode::new(path, self.directory_generation),
            );
        }
        self.recompute_prefixes()?;
        self.rebind_selected();
        Ok(())
    }

    fn remove_descendants(&mut self, path: &str) {
        let prefix = if path.is_empty() {
            String::new()
        } else {
            format!("{path}/")
        };
        let mut removed_entries = 0_usize;
        let mut removed_bytes = 0_usize;
        self.nodes.retain(|node| {
            let remove =
                node.path.as_ref().starts_with(prefix.as_str()) && node.path.as_ref() != path;
            if remove {
                removed_entries = removed_entries.saturating_add(node.entries.len());
                removed_bytes = removed_bytes.saturating_add(
                    node.entries
                        .iter()
                        .map(|entry| entry.path.len())
                        .sum::<usize>(),
                );
            }
            !remove
        });
        self.retained_entries = self.retained_entries.saturating_sub(removed_entries);
        self.retained_path_bytes = self.retained_path_bytes.saturating_sub(removed_bytes);
    }

    fn recompute_prefixes(&mut self) -> Result<(), FileTreeError> {
        let _ = self.recompute_node("")?;
        Ok(())
    }

    fn recompute_node(&mut self, path: &str) -> Result<usize, FileTreeError> {
        let Some(index) = self.node_index(path) else {
            return Ok(0);
        };
        if !self.nodes[index].expanded || self.nodes[index].load != DirectoryLoad::Ready {
            self.nodes[index].prefix_rows = Box::default();
            return Ok(0);
        }
        let entries = self.nodes[index].entries.to_vec();
        let mut prefix = Vec::new();
        prefix
            .try_reserve(entries.len())
            .map_err(|_| FileTreeError::AllocationFailed)?;
        let mut total = 0_usize;
        for entry in entries {
            let descendants = if entry.kind == FileTreeEntryKind::Directory {
                self.recompute_node(&entry.path)?
            } else {
                0
            };
            total = total.saturating_add(1).saturating_add(descendants);
            prefix.push(total);
        }
        self.nodes[index].prefix_rows = prefix.into_boxed_slice();
        Ok(total)
    }

    pub(crate) fn total_rows(&self) -> usize {
        self.node_index("")
            .and_then(|index| self.nodes[index].prefix_rows.last().copied())
            .unwrap_or(0)
    }

    fn node_index(&self, path: &str) -> Option<usize> {
        self.nodes
            .binary_search_by(|node| node.path.as_ref().cmp(path))
            .ok()
    }

    fn visible_path_at(&self, target: usize) -> Option<Arc<str>> {
        let mut current = 0_usize;
        self.visible_path_at_from("", target, &mut current)
    }

    fn visible_path_at_from(
        &self,
        directory: &str,
        target: usize,
        current: &mut usize,
    ) -> Option<Arc<str>> {
        let node = self.node_index(directory).map(|index| &self.nodes[index])?;
        if !node.expanded || node.load != DirectoryLoad::Ready {
            return None;
        }
        for entry in &node.entries {
            if *current == target {
                return Some(Arc::clone(&entry.path));
            }
            *current = current.saturating_add(1);
            if entry.kind == FileTreeEntryKind::Directory
                && let Some(path) = self.visible_path_at_from(&entry.path, target, current)
            {
                return Some(path);
            }
        }
        None
    }

    fn visible_index_of(&self, target: &str) -> Option<usize> {
        let mut current = 0_usize;
        self.visible_index_of_from("", target, &mut current)
    }

    fn visible_index_of_from(
        &self,
        directory: &str,
        target: &str,
        current: &mut usize,
    ) -> Option<usize> {
        let node = self.node_index(directory).map(|index| &self.nodes[index])?;
        if !node.expanded || node.load != DirectoryLoad::Ready {
            return None;
        }
        for entry in &node.entries {
            if entry.path.as_ref() == target {
                return Some(*current);
            }
            *current = current.saturating_add(1);
            if entry.kind == FileTreeEntryKind::Directory
                && let Some(index) = self.visible_index_of_from(&entry.path, target, current)
            {
                return Some(index);
            }
        }
        None
    }

    fn rebind_selected(&mut self) {
        self.selected = self
            .selected_path
            .as_deref()
            .and_then(|path| self.visible_index_of(path));
        let has_future_load = self.nodes.iter().any(|node| {
            node.expanded
                && matches!(
                    node.load,
                    DirectoryLoad::Dormant | DirectoryLoad::Loading(_)
                )
        });
        if self.selected.is_none() && !has_future_load {
            self.selected_path = None;
        }
    }

    fn cancel_pending(&mut self) {
        let Some(identity) = self.pending.take() else {
            return;
        };
        if let Some(node) = self
            .nodes
            .iter_mut()
            .find(|node| node.generation == identity.directory)
        {
            node.load = DirectoryLoad::Dormant;
        }
    }

    fn record_error(&mut self, error: &FileTreeError) {
        self.error = Some(bounded_message(error));
    }

    pub(crate) fn error_message(&self) -> Option<Arc<str>> {
        self.error
            .clone()
            .or_else(|| self.nodes.iter().find_map(|node| node.first_error.clone()))
    }

    #[cfg(test)]
    pub(crate) fn snapshot(&self) -> (usize, usize, usize, Option<usize>) {
        (
            self.nodes.len(),
            self.retained_entries,
            self.retained_path_bytes,
            self.selected,
        )
    }
}

fn restored_identity(path: &Path, limits: FileTreeLimits) -> Result<Arc<str>, FileTreeError> {
    let relative = path
        .to_str()
        .ok_or_else(|| FileTreeError::InvalidRelativePath(path.to_path_buf()))?;
    if path.is_absolute()
        || relative.len() > limits.path_bytes
        || relative.as_bytes().contains(&0)
        || relative
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
        || relative.split('/').count() > limits.depth
    {
        return Err(FileTreeError::InvalidRelativePath(path.to_path_buf()));
    }
    Ok(Arc::from(relative))
}

fn admit_io<T>(
    result: io::Result<T>,
    report: &mut DirectoryReport,
    first_error: &mut Option<Arc<str>>,
) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(error) => {
            report.errors = report.errors.saturating_add(1);
            report.omitted = report.omitted.saturating_add(1);
            first_error.get_or_insert_with(|| bounded_text(&error.to_string()));
            None
        }
    }
}

fn admit_name<'a>(name: &'a OsStr, report: &mut DirectoryReport) -> Option<&'a str> {
    let Some(name) = name.to_str() else {
        report.omitted = report.omitted.saturating_add(1);
        return None;
    };
    Some(name)
}

fn admit_file_tree_name<'name>(
    name: &'name OsStr,
    report: &mut DirectoryReport,
) -> Option<&'name str> {
    #[cfg(test)]
    if take_test_fault(FileTreeFault::InvalidName) {
        report.omitted = report.omitted.saturating_add(1);
        return None;
    }
    admit_name(name, report)
}

fn file_tree_file_type(entry: &fs::DirEntry) -> io::Result<fs::FileType> {
    #[cfg(test)]
    if take_test_fault(FileTreeFault::FileType) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "file-type fault",
        ));
    }
    entry.file_type()
}

#[allow(
    clippy::too_many_lines,
    reason = "one immediate-directory transaction keeps scan and retention accounting atomic"
)]
fn read_directory(
    root: &Path,
    relative: &str,
    limits: FileTreeLimits,
) -> Result<DirectoryResult, FileTreeError> {
    if !limits.is_valid() {
        return Err(FileTreeError::InvalidLimits);
    }
    let directory = validate_directory(root, relative, limits.depth)?;
    let (matchers, mut report, mut first_error) = ignore_stack(root, &directory)?;
    let reader = fs::read_dir(&directory)
        .map_err(|source| FileTreeError::io("enumerate", &directory, source))?;
    let depth = if relative.is_empty() {
        0
    } else {
        relative.split('/').count()
    };
    let mut entries = BinaryHeap::new();
    let mut path_bytes = 0_usize;
    for result in reader {
        if report.scanned == limits.scanned_per_directory {
            report.truncated = true;
            report.omitted = report.omitted.saturating_add(1);
            break;
        }
        report.scanned = report.scanned.saturating_add(1);
        #[cfg(test)]
        let result = if take_test_fault(FileTreeFault::DirectoryEntry) {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "entry fault",
            ))
        } else {
            result
        };
        let Some(entry) = admit_io(result, &mut report, &mut first_error) else {
            continue;
        };
        let name = entry.file_name();
        let name = admit_file_tree_name(&name, &mut report);
        let Some(name) = name else {
            continue;
        };
        if name == ".git" {
            report.omitted = report.omitted.saturating_add(1);
            continue;
        }
        let file_type = file_tree_file_type(&entry);
        let Some(file_type) = admit_io(file_type, &mut report, &mut first_error) else {
            continue;
        };
        let kind = if file_type.is_dir() {
            FileTreeEntryKind::Directory
        } else if file_type.is_file() {
            FileTreeEntryKind::File
        } else {
            report.omitted = report.omitted.saturating_add(1);
            continue;
        };
        if ignored(
            &matchers,
            &entry.path(),
            kind == FileTreeEntryKind::Directory,
        ) {
            report.omitted = report.omitted.saturating_add(1);
            continue;
        }
        let path = portable_child(relative, name)?;
        if path.len() > limits.path_bytes || depth.saturating_add(1) > limits.depth {
            report.omitted = report.omitted.saturating_add(1);
            report.truncated = true;
            continue;
        }
        entries
            .try_reserve(1)
            .map_err(|_| FileTreeError::AllocationFailed)?;
        let name_start = path.len().saturating_sub(name.len());
        let name_start = u16::try_from(name_start).map_err(|_| FileTreeError::AllocationFailed)?;
        let entry_depth = u16::try_from(depth).map_err(|_| FileTreeError::AllocationFailed)?;
        path_bytes = path_bytes.saturating_add(path.len());
        entries.push((
            path.len(),
            FileTreeEntry {
                kind,
                path: Arc::from(path),
                name_start,
                depth: entry_depth,
            },
        ));
        if entries.len() > limits.children_per_directory || path_bytes > limits.directory_path_bytes
        {
            let omitted = entries.pop().ok_or(FileTreeError::AllocationFailed)?.1;
            path_bytes = path_bytes.saturating_sub(omitted.path.len());
            report.omitted = report.omitted.saturating_add(1);
            report.truncated = true;
        }
    }
    let mut entries: Vec<_> = entries
        .into_vec()
        .into_iter()
        .map(|(_, entry)| entry)
        .collect();
    entries.sort_unstable();
    entries.shrink_to_fit();
    report.retained = entries.len();
    report.path_bytes = path_bytes;
    Ok(DirectoryResult {
        entries: entries.into_boxed_slice(),
        report,
        first_error,
    })
}

fn validate_directory(
    root: &Path,
    relative: &str,
    max_depth: usize,
) -> Result<PathBuf, FileTreeError> {
    let mut directory = root.to_path_buf();
    if relative.is_empty() {
        return Ok(directory);
    }
    let relative_path = Path::new(relative);
    let mut depth = 0_usize;
    for component in relative_path.components() {
        let Component::Normal(component) = component else {
            return Err(FileTreeError::InvalidRelativePath(
                relative_path.to_path_buf(),
            ));
        };
        depth = depth.saturating_add(1);
        if depth > max_depth {
            return Err(FileTreeError::PathDepthExceeded {
                actual: depth,
                limit: max_depth,
            });
        }
        directory.push(component);
        let metadata = fs::symlink_metadata(&directory)
            .map_err(|source| FileTreeError::io("revalidate directory", &directory, source))?;
        if metadata.file_type().is_symlink() {
            return Err(FileTreeError::Symlink(directory));
        }
        if !metadata.is_dir() {
            return Err(FileTreeError::NotDirectory(directory));
        }
    }
    let canonical = fs::canonicalize(&directory)
        .map_err(|source| FileTreeError::io("canonicalize directory", &directory, source))?;
    if canonical != directory {
        return Err(FileTreeError::Symlink(directory));
    }
    Ok(directory)
}

type IgnoreStack = (Vec<Gitignore>, DirectoryReport, Option<Arc<str>>);

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FileTreeFault {
    Publish,
    GitExclude,
    Gitignore,
    DirectoryEntry,
    InvalidName,
    FileType,
}

#[cfg(test)]
std::thread_local! {
    static FILE_TREE_FAULT: std::cell::Cell<Option<FileTreeFault>> = const {
        std::cell::Cell::new(None)
    };
}

#[cfg(test)]
fn set_test_fault(fault: FileTreeFault) {
    FILE_TREE_FAULT.set(Some(fault));
}

#[cfg(test)]
fn take_test_fault(fault: FileTreeFault) -> bool {
    if FILE_TREE_FAULT.get() == Some(fault) {
        FILE_TREE_FAULT.set(None);
        true
    } else {
        false
    }
}

#[cfg(test)]
fn take_ignore_fault(path: &Path) -> bool {
    match path.file_name().and_then(OsStr::to_str) {
        Some("exclude") => take_test_fault(FileTreeFault::GitExclude),
        Some(".gitignore") => take_test_fault(FileTreeFault::Gitignore),
        _ => false,
    }
}

fn ignore_stack(root: &Path, directory: &Path) -> Result<IgnoreStack, FileTreeError> {
    let mut matchers = Vec::new();
    let mut report = DirectoryReport::default();
    let mut first_error = None;
    let git = root.join(".git");
    if fs::symlink_metadata(&git).is_ok_and(|metadata| metadata.is_dir()) {
        add_ignore_file(
            root,
            &git.join("info").join("exclude"),
            &mut matchers,
            &mut report,
            &mut first_error,
        )?;
    }
    let relative = directory
        .strip_prefix(root)
        .map_err(|_| FileTreeError::InvalidRelativePath(directory.to_path_buf()))?;
    let mut current = root.to_path_buf();
    add_directory_ignores(&current, &mut matchers, &mut report, &mut first_error)?;
    for component in relative.components() {
        let Component::Normal(component) = component else {
            return Err(FileTreeError::InvalidRelativePath(relative.to_path_buf()));
        };
        current.push(component);
        add_directory_ignores(&current, &mut matchers, &mut report, &mut first_error)?;
    }
    Ok((matchers, report, first_error))
}

fn add_directory_ignores(
    directory: &Path,
    matchers: &mut Vec<Gitignore>,
    report: &mut DirectoryReport,
    first_error: &mut Option<Arc<str>>,
) -> Result<(), FileTreeError> {
    add_ignore_file(
        directory,
        &directory.join(".gitignore"),
        matchers,
        report,
        first_error,
    )?;
    add_ignore_file(
        directory,
        &directory.join(".ignore"),
        matchers,
        report,
        first_error,
    )
}

fn add_ignore_file(
    context: &Path,
    path: &Path,
    matchers: &mut Vec<Gitignore>,
    report: &mut DirectoryReport,
    first_error: &mut Option<Arc<str>>,
) -> Result<(), FileTreeError> {
    #[cfg(test)]
    if take_ignore_fault(path) {
        return Err(FileTreeError::AllocationFailed);
    }
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            report.errors = report.errors.saturating_add(1);
            first_error.get_or_insert_with(|| bounded_text(&error.to_string()));
            return Ok(());
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        report.errors = report.errors.saturating_add(1);
        first_error.get_or_insert_with(|| bounded_text("ignore file is not a regular file"));
        return Ok(());
    }
    let mut builder = GitignoreBuilder::new(context);
    if let Some(error) = builder.add(path) {
        report.errors = report.errors.saturating_add(1);
        first_error.get_or_insert_with(|| bounded_text(&error.to_string()));
    }
    let matcher = builder
        .build()
        .map_err(|error| FileTreeError::Ignore(bounded_text(&error.to_string())))?;
    matchers
        .try_reserve(1)
        .map_err(|_| FileTreeError::AllocationFailed)?;
    matchers.push(matcher);
    Ok(())
}

fn ignored(matchers: &[Gitignore], path: &Path, is_dir: bool) -> bool {
    let mut ignored = false;
    for matcher in matchers {
        match matcher.matched(path, is_dir) {
            Match::None => {}
            Match::Ignore(_) => ignored = true,
            Match::Whitelist(_) => ignored = false,
        }
    }
    ignored
}

fn portable_child(relative: &str, name: &str) -> Result<String, FileTreeError> {
    let capacity = relative
        .len()
        .checked_add(1)
        .and_then(|length| length.checked_add(name.len()))
        .ok_or(FileTreeError::AllocationFailed)?;
    let mut path = String::new();
    path.try_reserve(capacity)
        .map_err(|_| FileTreeError::AllocationFailed)?;
    if !relative.is_empty() {
        path.push_str(relative);
        path.push('/');
    }
    path.push_str(name);
    Ok(path)
}

fn bounded_message(error: &FileTreeError) -> Arc<str> {
    bounded_text(&error.to_string())
}

fn bounded_text(text: &str) -> Arc<str> {
    if text.len() <= MAX_ERROR_BYTES {
        return Arc::from(text);
    }
    let mut end = MAX_ERROR_BYTES;
    while !text.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    Arc::from(&text[..end])
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        mem::size_of,
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
    };

    use super::*;

    static NEXT_ROOT: AtomicU64 = AtomicU64::new(1);

    struct TestRoot(PathBuf);

    impl TestRoot {
        fn new() -> io::Result<Self> {
            let sequence = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "alpine-file-tree-{}-{sequence}",
                std::process::id()
            ));
            fs::create_dir(&path)?;
            Ok(Self(fs::canonicalize(path)?))
        }

        fn write(&self, relative: &str) -> io::Result<()> {
            let path = self.0.join(relative);
            fs::create_dir_all(path.parent().unwrap_or(&self.0))?;
            fs::write(path, "x")
        }
    }

    impl Drop for TestRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn admit_next(
        state: &mut FileTreeState,
        root: &Path,
    ) -> Result<FileTreeAdmission, FileTreeError> {
        let request = state
            .take_request(root)
            .ok_or(FileTreeError::MissingSelection)?;
        Ok(state.admit(request.execute()))
    }

    #[test]
    fn activation_and_directory_caps_are_exact() -> Result<(), Box<dyn Error>> {
        let root = TestRoot::new()?;
        root.write("a.rs")?;
        root.write("b.rs")?;
        root.write("c.rs")?;
        let limits = FileTreeLimits::new(16, 1, 8, 2, 1, 8, 8, 2, 2);
        let mut state = FileTreeState::with_test_limits(limits);
        assert!(state.take_request(&root.0).is_none());
        assert!(state.activate(7)?);
        assert_eq!(
            admit_next(&mut state, &root.0)?,
            FileTreeAdmission::Directory
        );
        assert_eq!(state.snapshot(), (1, 1, 4, None));
        assert_eq!(state.total_rows(), 1);
        let row = state.visible_rows(0, 1, 0)?.remove(0);
        assert_eq!(row.path.as_ref(), "a.rs");
        assert_eq!(row.label(), "a.rs");
        assert_eq!(MAX_SCANNED_PER_DIRECTORY, 16_384);
        assert_eq!(MAX_CHILDREN_PER_DIRECTORY, 4_096);
        assert_eq!(MAX_DIRECTORY_PATH_BYTES, 1_048_576);
        assert_eq!(MAX_CACHED_DIRECTORIES, 4_096);
        assert_eq!(MAX_CACHED_ENTRIES, 65_536);
        assert_eq!(MAX_CACHED_PATH_BYTES, 8_388_608);
        assert_eq!(MAX_PATH_BYTES, 4_096);
        assert_eq!(MAX_DEPTH, 256);
        assert_eq!(MAX_VISIBLE_ROWS, 512);
        assert!(size_of::<FileTreeEntry>() > 0);
        Ok(())
    }

    #[test]
    fn project_ignore_stack_and_symlink_policy_are_deterministic() -> Result<(), Box<dyn Error>> {
        let root = TestRoot::new()?;
        root.write("keep.rs")?;
        root.write("skip.log")?;
        root.write("src/keep.log")?;
        root.write("src/skip.tmp")?;
        fs::write(root.0.join(".gitignore"), "*.log\n!src/keep.log\n")?;
        fs::write(root.0.join("src/.ignore"), "*.tmp\n")?;
        fs::create_dir(root.0.join(".git"))?;
        root.write(".git/config")?;
        let root_result = read_directory(&root.0, "", FileTreeLimits::default())?;
        let root_paths: Vec<&str> = root_result
            .entries
            .iter()
            .map(|entry| entry.path.as_ref())
            .collect();
        assert_eq!(root_paths, ["src", ".gitignore", "keep.rs"]);
        let nested = read_directory(&root.0, "src", FileTreeLimits::default())?;
        let nested_paths: Vec<&str> = nested
            .entries
            .iter()
            .map(|entry| entry.path.as_ref())
            .collect();
        assert_eq!(nested_paths, ["src/.ignore", "src/keep.log"]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            symlink(root.0.join("src"), root.0.join("linked"))?;
            let result = read_directory(&root.0, "", FileTreeLimits::default())?;
            assert!(
                result
                    .entries
                    .iter()
                    .all(|entry| entry.path.as_ref() != "linked")
            );
        }
        Ok(())
    }

    #[test]
    fn state_rejects_stale_directory_results_and_bounds_projection() -> Result<(), Box<dyn Error>> {
        let root = TestRoot::new()?;
        for index in 0..20 {
            root.write(&format!("file-{index:02}.rs"))?;
        }
        let mut state = FileTreeState::default();
        state.activate(1)?;
        let request = state.take_request(&root.0).ok_or("request")?;
        let identity = request.identity();
        assert!(state.hide());
        assert_eq!(state.admit(request.execute()), FileTreeAdmission::Stale);
        state.activate(1)?;
        let retry = state.take_request(&root.0).ok_or("retry")?;
        assert!(!state.reject_submission(identity));
        assert_eq!(state.admit(retry.execute()), FileTreeAdmission::Directory);
        assert_eq!(state.visible_rows(0, 3, 1)?.len(), 5);
        assert!(state.visible_rows(0, usize::MAX, usize::MAX)?.len() <= MAX_VISIBLE_ROWS);
        Ok(())
    }

    #[test]
    fn tree_navigation_opens_only_current_file_rows() -> Result<(), Box<dyn Error>> {
        let root = TestRoot::new()?;
        root.write("src/main.rs")?;
        root.write("top.rs")?;
        let mut state = FileTreeState::default();
        state.activate(1)?;
        assert_eq!(
            admit_next(&mut state, &root.0)?,
            FileTreeAdmission::Directory
        );
        assert!(matches!(state.activate_row(0)?, FileTreeAction::Changed));
        assert_eq!(
            admit_next(&mut state, &root.0)?,
            FileTreeAdmission::Directory
        );
        assert_eq!(state.total_rows(), 3);
        assert!(state.navigate(true, 2));
        assert!(matches!(
            state.activate_selected()?,
            FileTreeAction::Open(path) if path.as_ref() == "src/main.rs"
        ));
        assert!(state.select_path("top.rs")?);
        assert!(matches!(
            state.activate_selected()?,
            FileTreeAction::Open(path) if path.as_ref() == "top.rs"
        ));
        Ok(())
    }

    #[test]
    fn restored_expansion_and_selection_remain_dormant_then_rebind_by_path()
    -> Result<(), Box<dyn Error>> {
        let root = TestRoot::new()?;
        root.write("src/nested/main.rs")?;
        root.write("top.rs")?;
        let session = crate::session::SessionFileTree {
            expanded: vec![PathBuf::from("src"), PathBuf::from("src/nested")],
            selected: Some(PathBuf::from("src/nested/main.rs")),
        };
        let mut state = FileTreeState::default();
        state.restore_session(9, &session)?;
        assert_eq!(state.snapshot(), (3, 0, 0, None));
        assert!(state.take_request(&root.0).is_none());
        assert_eq!(state.session_state()?, session);

        state.activate(9)?;
        assert_eq!(
            admit_next(&mut state, &root.0)?,
            FileTreeAdmission::Directory
        );
        assert_eq!(state.snapshot().3, None);
        assert_eq!(
            admit_next(&mut state, &root.0)?,
            FileTreeAdmission::Directory
        );
        assert_eq!(state.snapshot().3, None);
        assert_eq!(
            admit_next(&mut state, &root.0)?,
            FileTreeAdmission::Directory
        );
        assert!(matches!(
            state.activate_selected()?,
            FileTreeAction::Open(path) if path.as_ref() == "src/nested/main.rs"
        ));
        assert_eq!(state.session_state()?, session);
        Ok(())
    }

    #[test]
    fn restored_identity_and_duplicate_boundaries_are_independent() -> Result<(), Box<dyn Error>> {
        let limits = FileTreeLimits::default();
        let exact_path = PathBuf::from("a".repeat(limits.path_bytes));
        assert_eq!(
            restored_identity(&exact_path, limits)?.len(),
            limits.path_bytes
        );
        assert!(matches!(
            restored_identity(&PathBuf::from("a".repeat(limits.path_bytes + 1)), limits),
            Err(FileTreeError::InvalidRelativePath(_))
        ));

        let exact_depth = (0..limits.depth).map(|_| "a").collect::<Vec<_>>().join("/");
        assert_eq!(
            restored_identity(Path::new(&exact_depth), limits)?
                .split('/')
                .count(),
            limits.depth
        );
        let excessive_depth = format!("{exact_depth}/a");
        for invalid in [
            PathBuf::new(),
            std::env::current_dir()?.join("absolute"),
            PathBuf::from("a\0b"),
            PathBuf::from("a//b"),
            PathBuf::from("a/./b"),
            PathBuf::from("a/../b"),
            PathBuf::from(excessive_depth),
        ] {
            assert!(matches!(
                restored_identity(&invalid, limits),
                Err(FileTreeError::InvalidRelativePath(_))
            ));
        }

        let mut state = FileTreeState::default();
        assert!(matches!(
            state.restore_session(
                1,
                &crate::session::SessionFileTree {
                    expanded: vec![PathBuf::from("src"), PathBuf::from("src")],
                    selected: None,
                }
            ),
            Err(FileTreeError::InvalidRelativePath(_))
        ));
        assert_eq!(state.snapshot(), (0, 0, 0, None));
        Ok(())
    }

    #[test]
    fn visible_path_recursion_distinguishes_ready_expanded_nodes() -> Result<(), Box<dyn Error>> {
        let root = TestRoot::new()?;
        root.write("src/main.rs")?;
        root.write("top.rs")?;
        let mut state = FileTreeState::default();
        state.activate(1)?;
        assert_eq!(
            admit_next(&mut state, &root.0)?,
            FileTreeAdmission::Directory
        );
        assert!(matches!(state.activate_row(0)?, FileTreeAction::Changed));
        assert_eq!(
            admit_next(&mut state, &root.0)?,
            FileTreeAdmission::Directory
        );

        assert_eq!(state.visible_path_at(0).as_deref(), Some("src"));
        assert_eq!(state.visible_path_at(1).as_deref(), Some("src/main.rs"));
        assert_eq!(state.visible_path_at(2).as_deref(), Some("top.rs"));
        assert_eq!(state.visible_path_at(3), None);
        assert_eq!(state.visible_index_of("src"), Some(0));
        assert_eq!(state.visible_index_of("src/main.rs"), Some(1));
        assert_eq!(state.visible_index_of("top.rs"), Some(2));
        assert_eq!(state.visible_index_of("missing"), None);

        let src = state.node_index("src").ok_or("src node")?;
        state.nodes[src].expanded = false;
        let mut current = 0;
        assert_eq!(state.visible_path_at_from("src", 0, &mut current), None);
        let mut current = 0;
        assert_eq!(
            state.visible_index_of_from("src", "src/main.rs", &mut current),
            None
        );
        state.nodes[src].expanded = true;
        state.nodes[src].load = DirectoryLoad::Dormant;
        let mut current = 0;
        assert_eq!(state.visible_path_at_from("src", 0, &mut current), None);
        let mut current = 0;
        assert_eq!(
            state.visible_index_of_from("src", "src/main.rs", &mut current),
            None
        );
        Ok(())
    }

    #[test]
    fn missing_restored_directory_fails_without_retargeting_selection() -> Result<(), Box<dyn Error>>
    {
        let root = TestRoot::new()?;
        root.write("top.rs")?;
        let mut state = FileTreeState::default();
        state.restore_session(
            3,
            &crate::session::SessionFileTree {
                expanded: vec![PathBuf::from("missing")],
                selected: Some(PathBuf::from("missing/file.rs")),
            },
        )?;
        state.activate(3)?;
        assert_eq!(
            admit_next(&mut state, &root.0)?,
            FileTreeAdmission::Directory
        );
        assert_eq!(admit_next(&mut state, &root.0)?, FileTreeAdmission::Failed);
        assert!(matches!(
            state.activate_selected(),
            Err(FileTreeError::MissingSelection)
        ));
        assert_eq!(state.session_state()?.selected, None);

        let accepted = state.session_state()?;
        let snapshot = state.snapshot();
        assert!(matches!(
            state.restore_session(
                4,
                &crate::session::SessionFileTree {
                    expanded: vec![PathBuf::from("../escape")],
                    selected: None,
                },
            ),
            Err(FileTreeError::InvalidRelativePath(_))
        ));
        assert_eq!(state.snapshot(), snapshot);
        assert_eq!(state.session_state()?, accepted);
        Ok(())
    }

    #[test]
    fn error_contracts_generation_limits_and_diagnostics_are_bounded() -> Result<(), Box<dyn Error>>
    {
        let invalid_limits = FileTreeLimits::new(0, 1, 1, 1, 1, 1, 1, 1, 1);
        let mut invalid = FileTreeState::with_test_limits(invalid_limits);
        assert!(matches!(
            invalid.activate(1),
            Err(FileTreeError::InvalidLimits)
        ));
        assert!(matches!(
            read_directory(Path::new("."), "", invalid_limits),
            Err(FileTreeError::InvalidLimits)
        ));

        let errors = [
            FileTreeError::NoWorkspace,
            FileTreeError::InvalidLimits,
            FileTreeError::GenerationExhausted,
            FileTreeError::AllocationFailed,
            FileTreeError::MissingSelection,
            FileTreeError::InvalidRelativePath(PathBuf::from("../escape")),
            FileTreeError::PathDepthExceeded {
                actual: 3,
                limit: 2,
            },
            FileTreeError::Symlink(PathBuf::from("linked")),
            FileTreeError::NotDirectory(PathBuf::from("file.rs")),
            FileTreeError::CacheLimitExceeded {
                resource: "directory count",
                limit: 1,
            },
            FileTreeError::Ignore(Arc::from("bad rule")),
        ];
        for error in errors {
            assert!(!error.to_string().is_empty());
            assert!(error.source().is_none());
        }
        let source = io::Error::new(io::ErrorKind::PermissionDenied, "denied");
        let io_error = FileTreeError::io("enumerate", Path::new("root"), source);
        assert_eq!(
            io_error.to_string(),
            "file-tree enumerate failed for root: denied"
        );
        assert!(io_error.source().and_then(Error::source).is_none());
        assert!(io_error.source().is_some());

        let long = format!("{}é", "a".repeat(MAX_ERROR_BYTES - 1));
        let bounded = bounded_text(&long);
        assert_eq!(bounded.len(), MAX_ERROR_BYTES - 1);
        assert!(long.starts_with(bounded.as_ref()));
        assert_eq!(bounded_text("short").as_ref(), "short");
        assert_eq!(
            bounded_message(&FileTreeError::AllocationFailed).as_ref(),
            "file-tree allocation failed"
        );

        let mut tree_generation = FileTreeState {
            tree_generation: u64::MAX,
            ..FileTreeState::default()
        };
        assert!(matches!(
            tree_generation.activate(1),
            Err(FileTreeError::GenerationExhausted)
        ));
        let mut directory_generation = FileTreeState {
            workspace: Some(1),
            directory_generation: u64::MAX,
            ..FileTreeState::default()
        };
        assert!(matches!(
            directory_generation.activate(1),
            Err(FileTreeError::GenerationExhausted)
        ));
        let root = TestRoot::new()?;
        let mut request_generation = FileTreeState::default();
        request_generation.activate(1)?;
        request_generation.request_generation = u64::MAX;
        assert!(request_generation.take_request(&root.0).is_none());
        Ok(())
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "one contiguous state-machine journey preserves request and selection identity"
    )]
    fn state_authority_cache_caps_and_nested_collapse_are_exact() -> Result<(), Box<dyn Error>> {
        let root = TestRoot::new()?;
        root.write("src/nested/deep.rs")?;
        root.write("top.rs")?;
        let mut state = FileTreeState::default();
        assert!(state.activate(7)?);
        assert!(!state.activate(7)?);
        let request = state.take_request(&root.0).ok_or("root request")?;
        let identity = request.identity();
        assert!(state.take_request(&root.0).is_none());
        assert_eq!(
            state.admit(FileTreeWorkerOutput {
                identity,
                relative: Arc::from("wrong"),
                result: Err(FileTreeError::NoWorkspace),
            }),
            FileTreeAdmission::Stale
        );
        assert!(state.reject_submission(identity));
        assert_eq!(
            state.error_message().as_deref(),
            Some("file-tree allocation failed")
        );
        assert!(!state.reject_submission(identity));
        assert_eq!(
            admit_next(&mut state, &root.0)?,
            FileTreeAdmission::Directory
        );
        assert!(state.navigate(true, 2));
        assert!(state.navigate(false, 2));
        assert!(!state.navigate(false, 2));
        assert!(!state.select_path("src")?);
        assert!(matches!(
            state.activate_selected()?,
            FileTreeAction::Changed
        ));
        assert_eq!(
            admit_next(&mut state, &root.0)?,
            FileTreeAdmission::Directory
        );
        assert!(state.select_path("src/nested")?);
        assert!(matches!(
            state.activate_selected()?,
            FileTreeAction::Changed
        ));
        assert_eq!(
            admit_next(&mut state, &root.0)?,
            FileTreeAdmission::Directory
        );
        assert_eq!(state.total_rows(), 4);
        assert_eq!(state.visible_rows(0, usize::MAX, 0)?.len(), 4);
        assert_eq!(
            state
                .visible_rows(2, 1, 0)?
                .first()
                .map(|row| (row.index, row.path.as_ref())),
            Some((2, "src/nested/deep.rs"))
        );
        assert!(state.select_path("src")?);
        assert!(matches!(
            state.activate_selected()?,
            FileTreeAction::Changed
        ));
        assert_eq!(state.snapshot().0, 2);
        assert_eq!(state.total_rows(), 2);
        assert!(state.visible_rows(usize::MAX, 1, 0)?.is_empty());
        assert!(!state.select_path("src/nested/deep.rs")?);
        assert!(state.unfocus());
        assert!(!state.unfocus());
        assert!(state.hide());
        assert!(!state.hide());

        let failed_root = TestRoot::new()?;
        failed_root.write("file.rs")?;
        let mut failed = FileTreeState::default();
        failed.activate(9)?;
        let failed_request = failed.take_request(&failed_root.0).ok_or("request")?;
        fs::remove_dir_all(&failed_root.0)?;
        assert_eq!(
            failed.admit(failed_request.execute()),
            FileTreeAdmission::Failed
        );
        assert!(
            failed
                .error_message()
                .is_some_and(|message| message.contains("enumerate failed"))
        );

        let capped_root = TestRoot::new()?;
        capped_root.write("dir/a.rs")?;
        capped_root.write("other.rs")?;
        let limits = FileTreeLimits::new(8, 8, 64, 1, 1, 8, 16, 4, 8);
        let mut capped = FileTreeState::with_test_limits(limits);
        capped.activate(11)?;
        assert_eq!(
            admit_next(&mut capped, &capped_root.0)?,
            FileTreeAdmission::Directory
        );
        assert_eq!(capped.snapshot().1, 1);
        assert!(matches!(
            capped.activate_row(0),
            Err(FileTreeError::CacheLimitExceeded {
                resource: "directory count",
                limit: 1
            })
        ));

        let mut empty = FileTreeState::default();
        assert!(!empty.navigate(true, 1));
        assert!(matches!(
            empty.activate_selected(),
            Err(FileTreeError::MissingSelection)
        ));
        assert!(matches!(
            empty.activate_row(4),
            Err(FileTreeError::MissingSelection)
        ));
        Ok(())
    }

    #[test]
    fn filesystem_validation_ignore_errors_and_scan_ceilings_are_explicit()
    -> Result<(), Box<dyn Error>> {
        let root = TestRoot::new()?;
        root.write("src/file.rs")?;
        root.write("a.rs")?;
        root.write("b.rs")?;
        assert!(matches!(
            validate_directory(&root.0, "../escape", MAX_DEPTH),
            Err(FileTreeError::InvalidRelativePath(_))
        ));
        assert!(matches!(
            validate_directory(&root.0, "src/file.rs", MAX_DEPTH),
            Err(FileTreeError::NotDirectory(_))
        ));
        assert!(matches!(
            validate_directory(&root.0, "src", 0),
            Err(FileTreeError::PathDepthExceeded {
                actual: 1,
                limit: 0
            })
        ));
        assert!(matches!(
            ignore_stack(&root.0, &root.0.join("../outside")),
            Err(FileTreeError::InvalidRelativePath(_))
        ));

        let scan_limits = FileTreeLimits::new(1, 8, 64, 8, 8, 64, 16, 4, 8);
        let scanned = read_directory(&root.0, "", scan_limits)?;
        assert_eq!(scanned.report.scanned, 1);
        assert!(scanned.report.truncated);
        assert!(scanned.report.omitted >= 1);

        let child_limits = FileTreeLimits::new(16, 1, 64, 8, 8, 64, 16, 4, 8);
        let children = read_directory(&root.0, "", child_limits)?;
        assert_eq!(children.report.retained, 1);
        assert!(children.report.truncated);

        let byte_limits = FileTreeLimits::new(16, 16, 1, 8, 8, 64, 16, 4, 8);
        let bytes = read_directory(&root.0, "", byte_limits)?;
        assert_eq!(bytes.report.retained, 0);
        assert!(bytes.report.truncated);

        let depth_limits = FileTreeLimits::new(16, 16, 64, 8, 8, 64, 16, 1, 8);
        let depth = read_directory(&root.0, "src", depth_limits)?;
        assert_eq!(depth.report.retained, 0);
        assert!(depth.report.truncated);

        fs::create_dir(root.0.join(".gitignore"))?;
        fs::write(root.0.join(".ignore"), "[z-a]\n")?;
        let ignored = read_directory(&root.0, "", FileTreeLimits::default())?;
        assert!(ignored.report.errors >= 2);
        assert!(ignored.first_error.is_some());

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;

            symlink(root.0.join("src"), root.0.join("linked-src"))?;
            assert!(matches!(
                validate_directory(&root.0, "linked-src", MAX_DEPTH),
                Err(FileTreeError::Symlink(_))
            ));
        }
        Ok(())
    }

    #[test]
    fn directory_byte_cap_is_independent_of_enumeration_order() -> Result<(), Box<dyn Error>> {
        for order in [
            ["z", "yy", "aaa"],
            ["z", "aaa", "yy"],
            ["yy", "z", "aaa"],
            ["yy", "aaa", "z"],
            ["aaa", "z", "yy"],
            ["aaa", "yy", "z"],
        ] {
            let root = TestRoot::new()?;
            for path in order {
                root.write(path)?;
            }
            let limits = FileTreeLimits::new(16, 16, 3, 8, 8, 64, 16, 4, 8);
            let result = read_directory(&root.0, "", limits)?;
            let paths: Vec<_> = result
                .entries
                .iter()
                .map(|entry| entry.path.as_ref())
                .collect();
            assert_eq!(paths, ["yy", "z"]);
            assert_eq!(result.report.retained, 2);
            assert_eq!(result.report.path_bytes, 3);
            assert_eq!(result.report.omitted, 1);
            assert!(result.report.truncated);
        }
        Ok(())
    }

    #[test]
    fn projection_retry_containment_and_metadata_faults_fail_closed() -> Result<(), Box<dyn Error>>
    {
        let root = TestRoot::new()?;
        root.write("src/file.rs")?;
        let mut state = FileTreeState::default();
        state.activate(1)?;
        assert_eq!(
            admit_next(&mut state, &root.0)?,
            FileTreeAdmission::Directory
        );

        let mut rows = Vec::new();
        state.collect_rows("missing", 0, 1, 0, &mut rows);
        state.collect_rows("", 0, 0, 0, &mut rows);
        assert!(rows.is_empty());
        let root_index = state.node_index("").ok_or("root node")?;
        state.nodes[root_index].expanded = false;
        state.collect_rows("", 0, 1, 0, &mut rows);
        assert!(rows.is_empty());
        state.nodes[root_index].expanded = true;

        assert!(state.select_path("src")?);
        assert!(matches!(
            state.activate_selected()?,
            FileTreeAction::Changed
        ));
        let request = state.take_request(&root.0).ok_or("src request")?;
        fs::remove_dir_all(root.0.join("src"))?;
        assert_eq!(state.admit(request.execute()), FileTreeAdmission::Failed);
        assert!(matches!(
            state.activate_selected()?,
            FileTreeAction::Changed
        ));
        assert!(matches!(
            state.activate_selected()?,
            FileTreeAction::Changed
        ));
        assert!(state.take_request(&root.0).is_some());
        state.remove_descendants("");
        assert_eq!(state.snapshot().0, 1);

        fs::create_dir(root.0.join("src"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;

            let parent = root.0.parent().ok_or("root parent")?;
            let name = root.0.file_name().ok_or("root name")?;
            let alias = parent.join(format!("{}-alias", name.to_string_lossy()));
            symlink(&root.0, &alias)?;
            assert!(matches!(
                validate_directory(&alias, "src", MAX_DEPTH),
                Err(FileTreeError::Symlink(_))
            ));
            fs::remove_file(alias)?;
        }
        let other = TestRoot::new()?;
        assert!(matches!(
            ignore_stack(&root.0, &other.0),
            Err(FileTreeError::InvalidRelativePath(_))
        ));

        let mut io_report = DirectoryReport::default();
        let mut io_error = None;
        let missing: Option<()> = admit_io(
            Err(io::Error::new(io::ErrorKind::PermissionDenied, "denied")),
            &mut io_report,
            &mut io_error,
        );
        assert!(missing.is_none());
        assert_eq!((io_report.errors, io_report.omitted), (1, 1));
        assert_eq!(io_error.as_deref(), Some("denied"));
        assert_eq!(admit_io(Ok(7), &mut io_report, &mut io_error), Some(7));

        #[cfg(unix)]
        {
            use std::{ffi::OsString, os::unix::ffi::OsStringExt};

            let mut name_report = DirectoryReport::default();
            let invalid = OsString::from_vec(vec![0xff]);
            assert!(admit_name(&invalid, &mut name_report).is_none());
            assert_eq!(name_report.omitted, 1);
            assert_eq!(
                admit_name(OsStr::new("valid"), &mut name_report),
                Some("valid")
            );
        }

        let mut matchers = Vec::new();
        let mut report = DirectoryReport::default();
        let mut first_error = None;
        assert!(
            add_ignore_file(
                &root.0,
                &root.0.join("x".repeat(300)),
                &mut matchers,
                &mut report,
                &mut first_error,
            )
            .is_ok()
        );
        assert_eq!(report.errors, 1);
        assert!(first_error.is_some());
        assert!(matchers.is_empty());
        Ok(())
    }

    fn admitted_report(
        root: &Path,
        limits: FileTreeLimits,
    ) -> Result<DirectoryReport, Box<dyn Error>> {
        let mut state = FileTreeState::with_test_limits(limits);
        state.activate(1)?;
        assert_eq!(admit_next(&mut state, root)?, FileTreeAdmission::Directory);
        let index = state.node_index("").ok_or("root node")?;
        Ok(state.nodes[index].report)
    }

    #[derive(Clone, Copy)]
    enum AdmissionGuard {
        Visible,
        Active,
        Workspace,
        Tree,
        Pending,
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "each independent limit and admission identity is a separate policy boundary"
    )]
    fn every_limit_and_state_authority_guard_is_independent() -> Result<(), Box<dyn Error>> {
        let invalid = [
            FileTreeLimits::new(0, 1, 1, 1, 1, 1, 1, 1, 1),
            FileTreeLimits::new(1, 0, 1, 1, 1, 1, 1, 1, 1),
            FileTreeLimits::new(1, 1, 0, 1, 1, 1, 1, 1, 1),
            FileTreeLimits::new(1, 1, 1, 0, 1, 1, 1, 1, 1),
            FileTreeLimits::new(1, 1, 1, 1, 0, 1, 1, 1, 1),
            FileTreeLimits::new(1, 1, 1, 1, 1, 0, 1, 1, 1),
            FileTreeLimits::new(1, 1, 1, 1, 1, 1, 0, 1, 1),
            FileTreeLimits::new(1, 1, 1, 1, 1, 1, 1, 0, 1),
            FileTreeLimits::new(1, 1, 1, 1, 1, 1, 1, 1, 0),
        ];
        for limits in invalid {
            assert!(!limits.is_valid());
            assert!(matches!(
                FileTreeState::with_test_limits(limits).activate(1),
                Err(FileTreeError::InvalidLimits)
            ));
        }

        for (visible, active, focused) in [
            (false, true, true),
            (true, false, true),
            (true, true, false),
        ] {
            let mut state = FileTreeState {
                visible,
                active,
                focused,
                workspace: Some(1),
                ..FileTreeState::default()
            };
            state.insert_root()?;
            assert!(state.activate(1)?);
        }

        let root = TestRoot::new()?;
        root.write("a.rs")?;
        let mut hidden = FileTreeState::default();
        hidden.activate(1)?;
        hidden.visible = false;
        assert!(!hidden.is_visible());
        assert!(hidden.take_request(&root.0).is_none());
        let mut inactive = FileTreeState::default();
        inactive.activate(1)?;
        inactive.active = false;
        assert!(inactive.take_request(&root.0).is_none());
        let mut pending = FileTreeState::default();
        pending.activate(1)?;
        let _request = pending.take_request(&root.0).ok_or("pending request")?;
        pending.nodes[0].load = DirectoryLoad::Dormant;
        assert!(pending.take_request(&root.0).is_none());

        for guard in [
            AdmissionGuard::Visible,
            AdmissionGuard::Active,
            AdmissionGuard::Workspace,
            AdmissionGuard::Tree,
            AdmissionGuard::Pending,
        ] {
            let mut state = FileTreeState::default();
            state.activate(7)?;
            let request = state.take_request(&root.0).ok_or("guard request")?;
            let identity = request.identity();
            let output = request.execute();
            match guard {
                AdmissionGuard::Visible => state.visible = false,
                AdmissionGuard::Active => state.active = false,
                AdmissionGuard::Workspace => state.workspace = Some(8),
                AdmissionGuard::Tree => {
                    state.tree_generation = state.tree_generation.saturating_add(1);
                }
                AdmissionGuard::Pending => {
                    state.pending = Some(FileTreeRequestIdentity {
                        request: identity.request.saturating_add(1),
                        ..identity
                    });
                }
            }
            assert_eq!(state.admit(output), FileTreeAdmission::Stale);
            assert_eq!(state.snapshot().1, 0);
        }
        Ok(())
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "cache accounting, projection, and navigation share one admitted tree fixture"
    )]
    fn cache_reports_projection_and_navigation_boundaries_are_exact() -> Result<(), Box<dyn Error>>
    {
        let root = TestRoot::new()?;
        root.write("a")?;
        root.write("bb")?;
        root.write("ccc")?;
        let exact = FileTreeLimits::new(8, 8, 32, 8, 8, 32, 8, 8, 8);
        assert_eq!(
            admitted_report(&root.0, exact)?,
            DirectoryReport {
                scanned: 3,
                retained: 3,
                path_bytes: 6,
                omitted: 0,
                errors: 0,
                truncated: false,
            }
        );
        let entry_capped = FileTreeLimits::new(8, 8, 32, 8, 1, 32, 8, 8, 8);
        assert_eq!(
            admitted_report(&root.0, entry_capped)?,
            DirectoryReport {
                scanned: 3,
                retained: 1,
                path_bytes: 1,
                omitted: 2,
                errors: 0,
                truncated: true,
            }
        );
        let byte_capped = FileTreeLimits::new(8, 8, 32, 8, 8, 3, 8, 8, 8);
        assert_eq!(
            admitted_report(&root.0, byte_capped)?,
            DirectoryReport {
                scanned: 3,
                retained: 2,
                path_bytes: 3,
                omitted: 1,
                errors: 0,
                truncated: true,
            }
        );
        let source_capped = FileTreeLimits::new(1, 8, 32, 8, 8, 32, 8, 8, 8);
        let source_report = admitted_report(&root.0, source_capped)?;
        assert_eq!((source_report.scanned, source_report.retained), (1, 1));
        assert_eq!(source_report.omitted, 1);
        assert!(source_report.truncated);

        let directory_root = TestRoot::new()?;
        directory_root.write("dir/child")?;
        let mut state = FileTreeState::default();
        state.activate(1)?;
        assert_eq!(
            admit_next(&mut state, &directory_root.0)?,
            FileTreeAdmission::Directory
        );
        let row = state.visible_rows(0, 1, 0)?.remove(0);
        assert_eq!(row.kind, FileTreeEntryKind::Directory);
        assert!(!row.expanded);

        let mut navigation = FileTreeState::default();
        navigation.activate(1)?;
        assert_eq!(
            admit_next(&mut navigation, &root.0)?,
            FileTreeAdmission::Directory
        );
        assert!(navigation.navigate(true, 3));
        assert!(navigation.navigate(true, 3));
        assert!(!navigation.navigate(true, 3));
        assert_eq!(navigation.snapshot().3, Some(2));
        assert!(matches!(
            navigation.activate_selected()?,
            FileTreeAction::Open(path) if path.as_ref() == "ccc"
        ));
        Ok(())
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "filesystem threshold and ignore-type cases prove independent admission classes"
    )]
    fn filesystem_thresholds_and_ignore_types_are_independent() -> Result<(), Box<dyn Error>> {
        let root = TestRoot::new()?;
        root.write("a")?;
        root.write("bb")?;
        root.write("ccc")?;
        let path_limits = FileTreeLimits::new(8, 8, 32, 8, 8, 32, 2, 8, 8);
        let path_report = read_directory(&root.0, "", path_limits)?.report;
        assert_eq!((path_report.retained, path_report.path_bytes), (2, 3));
        assert_eq!(path_report.omitted, 1);
        assert!(path_report.truncated);
        let depth_limits = FileTreeLimits::new(8, 8, 32, 8, 8, 32, 8, 1, 8);
        assert_eq!(
            read_directory(&root.0, "", depth_limits)?.report.retained,
            3
        );
        let child_limits = FileTreeLimits::new(8, 1, 32, 8, 8, 32, 8, 8, 8);
        let child_report = read_directory(&root.0, "", child_limits)?.report;
        assert_eq!((child_report.retained, child_report.omitted), (1, 2));
        let byte_limits = FileTreeLimits::new(8, 8, 3, 8, 8, 32, 8, 8, 8);
        let byte_report = read_directory(&root.0, "", byte_limits)?.report;
        assert_eq!((byte_report.retained, byte_report.path_bytes), (2, 3));
        assert_eq!(byte_report.omitted, 1);

        let nested = TestRoot::new()?;
        nested.write("dir/a")?;
        let nested_report = read_directory(&nested.0, "dir", depth_limits)?.report;
        assert_eq!((nested_report.retained, nested_report.omitted), (0, 1));
        assert!(nested_report.truncated);

        let ignored_root = TestRoot::new()?;
        fs::write(ignored_root.0.join(".gitignore"), "ignored/\n")?;
        ignored_root.write("ignored/lost")?;
        ignored_root.write("keep")?;
        let ignored_result = read_directory(&ignored_root.0, "", FileTreeLimits::default())?;
        assert!(
            ignored_result
                .entries
                .iter()
                .all(|entry| entry.path.as_ref() != "ignored")
        );
        assert!(
            ignored_result
                .entries
                .iter()
                .any(|entry| entry.path.as_ref() == "keep")
        );

        let mut matchers = Vec::new();
        let mut report = DirectoryReport::default();
        let mut first_error = None;
        assert!(
            add_ignore_file(
                &ignored_root.0,
                &ignored_root.0.join("missing-ignore"),
                &mut matchers,
                &mut report,
                &mut first_error,
            )
            .is_ok()
        );
        assert_eq!(report.errors, 0);
        assert!(matchers.is_empty());
        let directory_ignore = ignored_root.0.join("directory-ignore");
        fs::create_dir(&directory_ignore)?;
        assert!(
            add_ignore_file(
                &ignored_root.0,
                &directory_ignore,
                &mut matchers,
                &mut report,
                &mut first_error,
            )
            .is_ok()
        );
        assert_eq!(report.errors, 1);
        assert!(matchers.is_empty());
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;

            let target = ignored_root.0.join("real-ignore");
            fs::write(&target, "*.tmp\n")?;
            let link = ignored_root.0.join("linked-ignore");
            symlink(target, &link)?;
            assert!(
                add_ignore_file(
                    &ignored_root.0,
                    &link,
                    &mut matchers,
                    &mut report,
                    &mut first_error,
                )
                .is_ok()
            );
            assert_eq!(report.errors, 2);
            assert!(matchers.is_empty());
        }
        assert_eq!(portable_child("", "a")?, "a");
        assert_eq!(portable_child("dir", "a")?, "dir/a");
        Ok(())
    }

    #[test]
    fn randomized_expand_collapse_selection_soak_releases_on_replacement()
    -> Result<(), Box<dyn Error>> {
        let root = TestRoot::new()?;
        for directory in 0..8 {
            root.write(&format!("d{directory}/file.rs"))?;
        }
        let mut state = FileTreeState::default();
        state.activate(1)?;
        assert_eq!(
            admit_next(&mut state, &root.0)?,
            FileTreeAdmission::Directory
        );
        for directory in 0..8 {
            assert!(state.select_path(&format!("d{directory}"))?);
            assert!(matches!(
                state.activate_selected()?,
                FileTreeAction::Changed
            ));
            assert_eq!(
                admit_next(&mut state, &root.0)?,
                FileTreeAdmission::Directory
            );
        }
        assert_eq!(state.snapshot(), (9, 16, 96, Some(14)));

        let mut random = 0x9E37_79B9_u64;
        for _ in 0..4_096 {
            random = random
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let total = state.total_rows();
            match random & 3 {
                0 => {
                    let _changed = state.navigate(random & 4 != 0, 32);
                }
                1 if total > 0 => {
                    let index = usize::try_from(random % u64::try_from(total)?)?;
                    let _action = state.activate_row(index)?;
                }
                2 => {
                    let first = usize::try_from(random & 31)?;
                    assert!(state.visible_rows(first, 16, 3)?.len() <= 22);
                }
                _ => {
                    assert!(state.hide());
                    assert!(!state.is_visible());
                    assert!(state.activate(1)?);
                }
            }
            let (nodes, entries, bytes, selected) = state.snapshot();
            assert!(nodes <= MAX_CACHED_DIRECTORIES);
            assert!(entries <= MAX_CACHED_ENTRIES);
            assert!(bytes <= MAX_CACHED_PATH_BYTES);
            assert!(selected.is_none_or(|index| index < state.total_rows()));
        }

        let _changed = state.activate(2)?;
        assert_eq!(state.snapshot(), (1, 0, 0, None));
        assert_eq!(state.total_rows(), 0);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn permission_denial_is_structured_and_does_not_publish() -> Result<(), Box<dyn Error>> {
        use std::os::unix::fs::PermissionsExt;

        let root = TestRoot::new()?;
        root.write("blocked/file.rs")?;
        let blocked = root.0.join("blocked");
        fs::set_permissions(&blocked, fs::Permissions::from_mode(0o000))?;
        let result = read_directory(&root.0, "blocked", FileTreeLimits::default());
        fs::set_permissions(&blocked, fs::Permissions::from_mode(0o700))?;
        assert!(matches!(
            result,
            Err(FileTreeError::Io {
                operation: "enumerate",
                source,
                ..
            }) if source.kind() == io::ErrorKind::PermissionDenied
        ));
        Ok(())
    }

    #[test]
    fn publication_and_ignore_propagation_fail_closed() -> Result<(), Box<dyn Error>> {
        let root = TestRoot::new()?;
        root.write("a.rs")?;
        let mut state = FileTreeState::default();
        assert!(state.activate(1)?);
        let request = state.take_request(&root.0).ok_or("root request")?;
        set_test_fault(FileTreeFault::Publish);
        assert_eq!(state.admit(request.execute()), FileTreeAdmission::Failed);
        assert!(state.error_message().is_some());

        root.write(".git/info/exclude")?;
        set_test_fault(FileTreeFault::GitExclude);
        assert!(matches!(
            read_directory(&root.0, "", FileTreeLimits::default()),
            Err(FileTreeError::AllocationFailed)
        ));
        root.write(".gitignore")?;
        set_test_fault(FileTreeFault::Gitignore);
        assert!(matches!(
            read_directory(&root.0, "", FileTreeLimits::default()),
            Err(FileTreeError::AllocationFailed)
        ));

        let fault_root = TestRoot::new()?;
        fault_root.write("only.rs")?;
        for (fault, errors) in [
            (FileTreeFault::DirectoryEntry, 1),
            (FileTreeFault::InvalidName, 0),
            (FileTreeFault::FileType, 1),
        ] {
            set_test_fault(fault);
            let result = read_directory(&fault_root.0, "", FileTreeLimits::default())?;
            assert_eq!(
                (
                    result.report.retained,
                    result.report.omitted,
                    result.report.errors
                ),
                (0, 1, errors)
            );
        }
        Ok(())
    }

    #[test]
    fn projection_skips_into_an_expanded_descendant() -> Result<(), Box<dyn Error>> {
        let root = TestRoot::new()?;
        root.write("dir/a.rs")?;
        root.write("dir/b.rs")?;
        root.write("z.rs")?;
        let mut state = FileTreeState::default();
        assert!(state.activate(1)?);
        assert_eq!(
            admit_next(&mut state, &root.0)?,
            FileTreeAdmission::Directory
        );
        assert_eq!(state.activate_row(0)?, FileTreeAction::Changed);
        assert_eq!(
            admit_next(&mut state, &root.0)?,
            FileTreeAdmission::Directory
        );
        let rows = state.visible_rows(2, 1, 0)?;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].path.as_ref(), "dir/b.rs");
        Ok(())
    }
}
