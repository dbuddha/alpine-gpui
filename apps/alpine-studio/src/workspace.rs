//! Bounded local-folder ownership for Alpine Studio.

#[cfg(test)]
use std::{cell::RefCell, ops::Range, sync::Arc};

use std::{
    error::Error,
    fmt, fs, io,
    path::{Component, Path, PathBuf},
};

#[cfg(test)]
type RevalidationHook = Box<dyn FnOnce(&Path)>;

#[cfg(test)]
thread_local! {
    static REVALIDATION_HOOK: RefCell<Option<RevalidationHook>> = RefCell::new(None);
}

#[cfg(test)]
const DEFAULT_MAX_SCANNED_ENTRIES: usize = 4_096;
#[cfg(test)]
const DEFAULT_MAX_RETAINED_ENTRIES: usize = 1_024;
#[cfg(test)]
const DEFAULT_MAX_NAME_BYTES: usize = 1_024;
#[cfg(test)]
const DEFAULT_MAX_RETAINED_NAME_BYTES: usize = 256 * 1_024;

/// A structured failure while admitting one local Studio workspace.
#[derive(Debug)]
pub enum WorkspaceError {
    /// One named filesystem operation failed.
    Io {
        /// Stable operation identity.
        operation: &'static str,
        /// Path on which the operation was attempted.
        path: PathBuf,
        /// Original operating-system error.
        source: io::Error,
    },
    /// The canonical launch target is not a directory.
    NotDirectory(PathBuf),
    /// The launch target is neither a regular file nor a directory.
    UnsupportedTarget(PathBuf),
    /// Enumeration exceeded the hard transient scan ceiling.
    ScanLimitExceeded {
        /// Canonical root that exceeded the ceiling.
        root: PathBuf,
        /// Maximum number of entries that may be inspected.
        limit: usize,
    },
    /// A fallible workspace allocation failed.
    AllocationFailed,
    /// The selected row is not retained by this workspace snapshot.
    EntryNotFound(usize),
    /// The selected row is not a regular file.
    NotRegularFile(PathBuf),
    /// Revalidation found that the selected target escaped the canonical root.
    EscapesRoot(PathBuf),
    /// A recursive selection contains a non-normal relative component.
    InvalidRelativePath(PathBuf),
    /// A recursive selection contains or became a symbolic link.
    Symlink(PathBuf),
    /// A recursive selection exceeds the admitted component ceiling.
    PathDepthExceeded {
        /// Number of normal path components observed.
        actual: usize,
        /// Maximum admitted component count.
        limit: usize,
    },
}

impl WorkspaceError {
    pub(crate) fn io(operation: &'static str, path: &Path, source: io::Error) -> Self {
        Self::Io {
            operation,
            path: path.to_path_buf(),
            source,
        }
    }
}

impl fmt::Display for WorkspaceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io {
                operation,
                path,
                source,
            } => {
                write!(
                    formatter,
                    "workspace {operation} failed for {}: {source}",
                    path.display()
                )
            }
            Self::NotDirectory(path) => {
                write!(
                    formatter,
                    "workspace root is not a directory: {}",
                    path.display()
                )
            }
            Self::UnsupportedTarget(path) => write!(
                formatter,
                "Studio path is neither a regular file nor a directory: {}",
                path.display()
            ),
            Self::ScanLimitExceeded { root, limit } => write!(
                formatter,
                "workspace {} exceeds the {limit}-entry scan ceiling",
                root.display()
            ),
            Self::AllocationFailed => formatter.write_str("workspace allocation failed"),
            Self::EntryNotFound(index) => {
                write!(formatter, "workspace entry {index} is unavailable")
            }
            Self::NotRegularFile(path) => {
                write!(
                    formatter,
                    "workspace target is not a regular file: {}",
                    path.display()
                )
            }
            Self::EscapesRoot(path) => write!(
                formatter,
                "workspace target escapes the canonical root: {}",
                path.display()
            ),
            Self::InvalidRelativePath(path) => write!(
                formatter,
                "workspace target is not a normal relative path: {}",
                path.display()
            ),
            Self::Symlink(path) => write!(
                formatter,
                "workspace target contains a symbolic link: {}",
                path.display()
            ),
            Self::PathDepthExceeded { actual, limit } => write!(
                formatter,
                "workspace target depth {actual} exceeds the {limit}-component ceiling"
            ),
        }
    }
}

impl Error for WorkspaceError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::NotDirectory(_)
            | Self::UnsupportedTarget(_)
            | Self::ScanLimitExceeded { .. }
            | Self::AllocationFailed
            | Self::EntryNotFound(_)
            | Self::NotRegularFile(_)
            | Self::EscapesRoot(_)
            | Self::InvalidRelativePath(_)
            | Self::Symlink(_)
            | Self::PathDepthExceeded { .. } => None,
        }
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum WorkspaceEntryKind {
    Directory,
    File,
}

#[cfg(test)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WorkspaceEntry {
    name: Arc<str>,
    kind: WorkspaceEntryKind,
}

#[cfg(test)]
impl WorkspaceEntry {
    pub(crate) fn name(&self) -> Arc<str> {
        Arc::clone(&self.name)
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct WorkspaceLimits {
    scan_ceiling: usize,
    retained_capacity: usize,
    per_name_bytes: usize,
    aggregate_name_budget: usize,
}

#[cfg(test)]
impl WorkspaceLimits {
    pub(crate) const fn new(
        max_scanned_entries: usize,
        max_retained_entries: usize,
        max_name_bytes: usize,
        max_retained_name_bytes: usize,
    ) -> Self {
        Self {
            scan_ceiling: max_scanned_entries,
            retained_capacity: max_retained_entries,
            per_name_bytes: max_name_bytes,
            aggregate_name_budget: max_retained_name_bytes,
        }
    }
}

#[cfg(test)]
impl Default for WorkspaceLimits {
    fn default() -> Self {
        Self {
            scan_ceiling: DEFAULT_MAX_SCANNED_ENTRIES,
            retained_capacity: DEFAULT_MAX_RETAINED_ENTRIES,
            per_name_bytes: DEFAULT_MAX_NAME_BYTES,
            aggregate_name_budget: DEFAULT_MAX_RETAINED_NAME_BYTES,
        }
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct WorkspaceSnapshot {
    pub(crate) scanned_entries: usize,
    pub(crate) retained_entries: usize,
    pub(crate) retained_name_bytes: usize,
    pub(crate) omitted_entries: usize,
    pub(crate) scan_limit: usize,
    pub(crate) entry_limit: usize,
    pub(crate) name_byte_limit: usize,
}

pub(crate) struct Workspace {
    root: PathBuf,
    #[cfg(test)]
    entries: Box<[WorkspaceEntry]>,
    #[cfg(test)]
    snapshot: WorkspaceSnapshot,
}

impl Workspace {
    pub(crate) fn open_root(root: &Path) -> Result<Self, WorkspaceError> {
        let canonical = fs::canonicalize(root)
            .map_err(|source| WorkspaceError::io("canonicalize", root, source))?;
        let metadata = fs::metadata(&canonical)
            .map_err(|source| WorkspaceError::io("read metadata", &canonical, source))?;
        if !metadata.is_dir() {
            return Err(WorkspaceError::NotDirectory(canonical));
        }
        Ok(Self {
            root: canonical,
            #[cfg(test)]
            entries: Box::default(),
            #[cfg(test)]
            snapshot: WorkspaceSnapshot {
                scanned_entries: 0,
                retained_entries: 0,
                retained_name_bytes: 0,
                omitted_entries: 0,
                scan_limit: DEFAULT_MAX_SCANNED_ENTRIES,
                entry_limit: DEFAULT_MAX_RETAINED_ENTRIES,
                name_byte_limit: DEFAULT_MAX_RETAINED_NAME_BYTES,
            },
        })
    }

    #[cfg(test)]
    pub(crate) fn open(root: &Path, limits: WorkspaceLimits) -> Result<Self, WorkspaceError> {
        let canonical = fs::canonicalize(root)
            .map_err(|source| WorkspaceError::io("canonicalize", root, source))?;
        let metadata = fs::metadata(&canonical)
            .map_err(|source| WorkspaceError::io("read metadata", &canonical, source))?;
        if !metadata.is_dir() {
            return Err(WorkspaceError::NotDirectory(canonical));
        }

        let reader = fs::read_dir(&canonical)
            .map_err(|source| WorkspaceError::io("enumerate", &canonical, source))?;
        let mut candidates = Vec::new();
        let mut scanned_entries = 0_usize;
        let mut omitted_entries = 0_usize;
        for result in reader {
            if scanned_entries == limits.scan_ceiling {
                return Err(WorkspaceError::ScanLimitExceeded {
                    root: canonical,
                    limit: limits.scan_ceiling,
                });
            }
            scanned_entries += 1;
            let entry = result
                .map_err(|source| WorkspaceError::io("read directory entry", &canonical, source))?;
            let file_type = entry
                .file_type()
                .map_err(|source| WorkspaceError::io("read entry type", &entry.path(), source))?;
            let kind = if file_type.is_dir() {
                WorkspaceEntryKind::Directory
            } else if file_type.is_file() {
                WorkspaceEntryKind::File
            } else {
                omitted_entries = omitted_entries.saturating_add(1);
                continue;
            };
            let file_name = entry.file_name();
            let Some(name) = file_name.to_str() else {
                omitted_entries = omitted_entries.saturating_add(1);
                continue;
            };
            if name.len() > limits.per_name_bytes {
                omitted_entries = omitted_entries.saturating_add(1);
                continue;
            }
            candidates
                .try_reserve(1)
                .map_err(|_| WorkspaceError::AllocationFailed)?;
            candidates.push(WorkspaceEntry {
                name: Arc::from(name),
                kind,
            });
        }

        candidates.sort_unstable_by(|left, right| {
            left.kind
                .cmp(&right.kind)
                .then_with(|| left.name.as_bytes().cmp(right.name.as_bytes()))
        });
        let mut entries = Vec::new();
        let mut retained_name_bytes = 0_usize;
        for candidate in candidates {
            if candidate.name.len()
                > limits
                    .aggregate_name_budget
                    .saturating_sub(retained_name_bytes)
            {
                omitted_entries = omitted_entries.saturating_add(1);
                continue;
            }
            if entries.len() >= limits.retained_capacity {
                omitted_entries = omitted_entries.saturating_add(1);
                continue;
            }
            entries
                .try_reserve(1)
                .map_err(|_| WorkspaceError::AllocationFailed)?;
            retained_name_bytes += candidate.name.len();
            entries.push(candidate);
        }
        entries.shrink_to_fit();
        let snapshot = WorkspaceSnapshot {
            scanned_entries,
            retained_entries: entries.len(),
            retained_name_bytes,
            omitted_entries,
            scan_limit: limits.scan_ceiling,
            entry_limit: limits.retained_capacity,
            name_byte_limit: limits.aggregate_name_budget,
        };
        Ok(Self {
            root: canonical,
            entries: entries.into_boxed_slice(),
            snapshot,
        })
    }

    #[cfg(test)]
    pub(crate) const fn len(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    #[cfg(test)]
    pub(crate) fn entry(&self, index: usize) -> Option<&WorkspaceEntry> {
        self.entries.get(index)
    }

    #[cfg(test)]
    pub(crate) const fn snapshot(&self) -> WorkspaceSnapshot {
        self.snapshot
    }

    #[cfg(test)]
    pub(crate) fn visible_range(
        &self,
        first_visible: usize,
        visible_rows: usize,
        overscan: usize,
    ) -> Range<usize> {
        let start = first_visible
            .saturating_sub(overscan)
            .min(self.entries.len());
        let end = first_visible
            .saturating_add(visible_rows)
            .saturating_add(overscan)
            .min(self.entries.len());
        start..end.max(start)
    }

    #[cfg(test)]
    pub(crate) fn path_for_file(&self, index: usize) -> Result<PathBuf, WorkspaceError> {
        let entry = self
            .entries
            .get(index)
            .ok_or(WorkspaceError::EntryNotFound(index))?;
        if entry.kind != WorkspaceEntryKind::File {
            return Err(WorkspaceError::NotRegularFile(
                self.root.join(entry.name.as_ref()),
            ));
        }
        let candidate = self.root.join(entry.name.as_ref());
        let metadata = fs::symlink_metadata(&candidate)
            .map_err(|source| WorkspaceError::io("revalidate target", &candidate, source))?;
        if !metadata.file_type().is_file() {
            return Err(WorkspaceError::NotRegularFile(candidate));
        }
        #[cfg(test)]
        run_revalidation_hook(&candidate);
        let canonical = fs::canonicalize(&candidate)
            .map_err(|source| WorkspaceError::io("canonicalize target", &candidate, source))?;
        if canonical.parent() != Some(self.root.as_path()) {
            return Err(WorkspaceError::EscapesRoot(canonical));
        }
        Ok(canonical)
    }

    pub(crate) fn path_for_relative_file(
        &self,
        relative: &Path,
    ) -> Result<PathBuf, WorkspaceError> {
        let mut components = relative.components().peekable();
        let mut depth = 0_usize;
        let mut candidate = self.root.clone();
        while let Some(component) = components.next() {
            let Component::Normal(component) = component else {
                return Err(WorkspaceError::InvalidRelativePath(relative.to_path_buf()));
            };
            depth = depth.saturating_add(1);
            if depth > crate::quick_open::MAX_DEPTH {
                return Err(WorkspaceError::PathDepthExceeded {
                    actual: depth,
                    limit: crate::quick_open::MAX_DEPTH,
                });
            }
            candidate.push(component);
            let metadata = fs::symlink_metadata(&candidate).map_err(|source| {
                WorkspaceError::io("revalidate path component", &candidate, source)
            })?;
            if metadata.file_type().is_symlink() {
                return Err(WorkspaceError::Symlink(candidate));
            }
            if components.peek().is_some() && !metadata.is_dir() {
                return Err(WorkspaceError::NotRegularFile(candidate));
            }
        }
        if depth == 0 {
            return Err(WorkspaceError::InvalidRelativePath(relative.to_path_buf()));
        }
        #[cfg(test)]
        run_revalidation_hook(&candidate);
        let metadata = fs::symlink_metadata(&candidate)
            .map_err(|source| WorkspaceError::io("revalidate target", &candidate, source))?;
        if metadata.file_type().is_symlink() {
            return Err(WorkspaceError::Symlink(candidate));
        }
        if !metadata.is_file() {
            return Err(WorkspaceError::NotRegularFile(candidate));
        }
        #[cfg(test)]
        run_revalidation_hook(&candidate);
        let final_metadata = fs::symlink_metadata(&candidate)
            .map_err(|source| WorkspaceError::io("revalidate target", &candidate, source))?;
        if final_metadata.file_type().is_symlink() {
            return Err(WorkspaceError::Symlink(candidate));
        }
        #[cfg(test)]
        run_revalidation_hook(&candidate);
        let canonical = fs::canonicalize(&candidate)
            .map_err(|source| WorkspaceError::io("canonicalize target", &candidate, source))?;
        if !canonical.starts_with(&self.root) {
            return Err(WorkspaceError::EscapesRoot(canonical));
        }
        if canonical != candidate {
            return Err(WorkspaceError::Symlink(candidate));
        }
        Ok(canonical)
    }

    #[cfg(test)]
    pub(crate) fn index_named(&self, name: &str) -> Option<usize> {
        self.entries
            .iter()
            .position(|entry| entry.name.as_ref() == name)
    }
}

#[cfg(test)]
pub(crate) fn set_revalidation_hook(hook: impl FnOnce(&Path) + 'static) {
    REVALIDATION_HOOK.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(hook));
    });
}

#[cfg(test)]
fn run_revalidation_hook(path: &Path) {
    let hook = REVALIDATION_HOOK.with(|slot| slot.borrow_mut().take());
    if let Some(hook) = hook {
        hook(path);
    }
}
