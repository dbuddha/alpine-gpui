//! Bounded multi-document ownership for Alpine Studio.

use std::{
    error::Error,
    fmt, mem,
    ops::Range,
    path::{Path, PathBuf},
    sync::Arc,
};

use alpine_text::{ByteOffset, Selection};

const DEFAULT_TAB_CAPACITY: usize = 32;
const DEFAULT_PATH_BYTE_BUDGET: usize = 64 * 1_024;
const DEFAULT_HISTORY_CAPACITY: usize = 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DocumentTabId(pub(crate) u64);

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct DocumentViewState {
    pub(crate) selection: Selection,
    pub(crate) scroll_y: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RestoredDocumentTab {
    pub(crate) path: Option<PathBuf>,
    pub(crate) view: DocumentViewState,
}

impl Default for DocumentViewState {
    fn default() -> Self {
        Self {
            selection: Selection::caret(ByteOffset::new(0)),
            scroll_y: 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DocumentTabLimits {
    tab_capacity: usize,
    path_byte_budget: usize,
    history_capacity: usize,
}

impl DocumentTabLimits {
    #[cfg(test)]
    pub(crate) const fn new(
        tab_capacity: usize,
        path_byte_budget: usize,
        history_capacity: usize,
    ) -> Self {
        Self {
            tab_capacity,
            path_byte_budget,
            history_capacity,
        }
    }
}

impl Default for DocumentTabLimits {
    fn default() -> Self {
        Self {
            tab_capacity: DEFAULT_TAB_CAPACITY,
            path_byte_budget: DEFAULT_PATH_BYTE_BUDGET,
            history_capacity: DEFAULT_HISTORY_CAPACITY,
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum DocumentTabError {
    InvalidLimits,
    AllocationFailed,
    CapacityReached(usize),
    PathBudgetExceeded(usize),
    IdentityExhausted,
    MissingTab(usize),
    DuplicatePath(PathBuf),
    LastTab,
    InvalidPayloadState,
}

impl fmt::Display for DocumentTabError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLimits => formatter.write_str("document tab limits must be non-zero"),
            Self::AllocationFailed => formatter.write_str("document tab allocation failed"),
            Self::CapacityReached(limit) => {
                write!(formatter, "document tab limit of {limit} was reached")
            }
            Self::PathBudgetExceeded(limit) => {
                write!(
                    formatter,
                    "document path budget of {limit} bytes was reached"
                )
            }
            Self::IdentityExhausted => formatter.write_str("document tab identity is exhausted"),
            Self::MissingTab(index) => write!(formatter, "document tab {index} is unavailable"),
            Self::DuplicatePath(path) => {
                write!(
                    formatter,
                    "document path is already open: {}",
                    path.display()
                )
            }
            Self::LastTab => formatter.write_str("the final document tab cannot be closed"),
            Self::InvalidPayloadState => {
                formatter.write_str("document tab payload ownership is inconsistent")
            }
        }
    }
}

impl Error for DocumentTabError {}

struct DocumentTab<T> {
    id: DocumentTabId,
    path: Option<PathBuf>,
    label: Arc<str>,
    retained_path_bytes: usize,
    workspace_entry: Option<usize>,
    document: Option<T>,
    deferred: bool,
    view: DocumentViewState,
}

pub(crate) struct DocumentTabs<T> {
    tabs: Vec<DocumentTab<T>>,
    active: usize,
    next_id: u64,
    retained_path_bytes: usize,
    history: Vec<DocumentTabId>,
    history_cursor: usize,
    limits: DocumentTabLimits,
}

impl<T> DocumentTabs<T> {
    pub(crate) fn new(
        path: Option<&Path>,
        workspace_entry: Option<usize>,
        limits: DocumentTabLimits,
    ) -> Result<Self, DocumentTabError> {
        if limits.tab_capacity == 0 || limits.path_byte_budget == 0 || limits.history_capacity == 0
        {
            return Err(DocumentTabError::InvalidLimits);
        }
        let (label, retained_path_bytes) = tab_metadata(path)?;
        if retained_path_bytes > limits.path_byte_budget {
            return Err(DocumentTabError::PathBudgetExceeded(
                limits.path_byte_budget,
            ));
        }
        let mut tabs = Vec::new();
        tabs.try_reserve(1)
            .map_err(|_| DocumentTabError::AllocationFailed)?;
        tabs.push(DocumentTab {
            id: DocumentTabId(1),
            path: path.map(Path::to_path_buf),
            label,
            retained_path_bytes,
            workspace_entry,
            document: None,
            deferred: false,
            view: DocumentViewState::default(),
        });
        let mut history = Vec::new();
        history
            .try_reserve_exact(limits.history_capacity)
            .map_err(|_| DocumentTabError::AllocationFailed)?;
        history.push(DocumentTabId(1));
        Ok(Self {
            tabs,
            active: 0,
            next_id: 2,
            retained_path_bytes,
            history,
            history_cursor: 0,
            limits,
        })
    }

    pub(crate) fn from_restored(
        restored: Vec<RestoredDocumentTab>,
        active: usize,
        limits: DocumentTabLimits,
    ) -> Result<Self, DocumentTabError> {
        if limits.tab_capacity == 0 || limits.path_byte_budget == 0 || limits.history_capacity == 0
        {
            return Err(DocumentTabError::InvalidLimits);
        }
        if restored.is_empty() || restored.len() > limits.tab_capacity || active >= restored.len() {
            return Err(DocumentTabError::InvalidPayloadState);
        }
        let restored_len = restored.len();
        let mut tabs = Vec::new();
        tabs.try_reserve_exact(restored_len)
            .map_err(|_| DocumentTabError::AllocationFailed)?;
        let mut retained_path_bytes = 0_usize;
        for (index, restored) in restored.into_iter().enumerate() {
            if let Some(path) = restored.path.as_deref()
                && tabs
                    .iter()
                    .any(|tab: &DocumentTab<T>| tab.path.as_deref() == Some(path))
            {
                return Err(DocumentTabError::DuplicatePath(path.to_path_buf()));
            }
            let (label, path_bytes) = tab_metadata(restored.path.as_deref())?;
            retained_path_bytes = retained_path_bytes.checked_add(path_bytes).ok_or(
                DocumentTabError::PathBudgetExceeded(limits.path_byte_budget),
            )?;
            if retained_path_bytes > limits.path_byte_budget {
                return Err(DocumentTabError::PathBudgetExceeded(
                    limits.path_byte_budget,
                ));
            }
            let id_value = u64::try_from(index)
                .ok()
                .and_then(|value| value.checked_add(1))
                .ok_or(DocumentTabError::IdentityExhausted)?;
            tabs.push(DocumentTab {
                id: DocumentTabId(id_value),
                path: restored.path,
                label,
                retained_path_bytes: path_bytes,
                workspace_entry: None,
                document: None,
                deferred: index != active,
                view: restored.view,
            });
        }
        let next_id = u64::try_from(restored_len)
            .ok()
            .and_then(|value| value.checked_add(1))
            .ok_or(DocumentTabError::IdentityExhausted)?;
        let active_id = tabs
            .get(active)
            .map(|tab| tab.id)
            .ok_or(DocumentTabError::InvalidPayloadState)?;
        let mut history = Vec::new();
        history
            .try_reserve_exact(limits.history_capacity)
            .map_err(|_| DocumentTabError::AllocationFailed)?;
        history.push(active_id);
        Ok(Self {
            tabs,
            active,
            next_id,
            retained_path_bytes,
            history,
            history_cursor: 0,
            limits,
        })
    }

    pub(crate) const fn len(&self) -> usize {
        self.tabs.len()
    }

    pub(crate) const fn active_index(&self) -> usize {
        self.active
    }

    pub(crate) fn active_id(&self) -> Result<DocumentTabId, DocumentTabError> {
        self.tabs
            .get(self.active)
            .map(|tab| tab.id)
            .ok_or(DocumentTabError::InvalidPayloadState)
    }

    pub(crate) fn index_for_id(&self, id: DocumentTabId) -> Option<usize> {
        self.tabs.iter().position(|tab| tab.id == id)
    }

    pub(crate) fn document_for_id<'a>(
        &'a self,
        id: DocumentTabId,
        active_document: &'a T,
    ) -> Result<&'a T, DocumentTabError> {
        let index = self
            .index_for_id(id)
            .ok_or(DocumentTabError::InvalidPayloadState)?;
        if index == self.active {
            return Ok(active_document);
        }
        self.tabs[index]
            .document
            .as_ref()
            .ok_or(DocumentTabError::InvalidPayloadState)
    }

    pub(crate) fn id_at(&self, index: usize) -> Option<DocumentTabId> {
        self.tabs.get(index).map(|tab| tab.id)
    }

    pub(crate) fn path_at(&self, index: usize) -> Option<&Path> {
        self.tabs.get(index).and_then(|tab| tab.path.as_deref())
    }

    pub(crate) fn is_deferred(&self, index: usize) -> Result<bool, DocumentTabError> {
        self.tabs
            .get(index)
            .map(|tab| tab.deferred)
            .ok_or(DocumentTabError::MissingTab(index))
    }

    pub(crate) fn materialize(
        &mut self,
        index: usize,
        document: T,
    ) -> Result<(), DocumentTabError> {
        if index == self.active {
            return Err(DocumentTabError::InvalidPayloadState);
        }
        let tab = self
            .tabs
            .get_mut(index)
            .ok_or(DocumentTabError::MissingTab(index))?;
        if !tab.deferred || tab.document.is_some() {
            return Err(DocumentTabError::InvalidPayloadState);
        }
        tab.document = Some(document);
        tab.deferred = false;
        Ok(())
    }

    pub(crate) fn navigation_target(&self, forward: bool) -> Option<usize> {
        if forward {
            self.history
                .get(self.history_cursor.saturating_add(1)..)?
                .iter()
                .find_map(|id| self.index_for_id(*id).filter(|index| *index != self.active))
        } else {
            self.history
                .get(..self.history_cursor)?
                .iter()
                .rev()
                .find_map(|id| self.index_for_id(*id).filter(|index| *index != self.active))
        }
    }

    pub(crate) fn close_target(&self) -> Result<usize, DocumentTabError> {
        if self.tabs.len() == 1 {
            return Err(DocumentTabError::LastTab);
        }
        Ok(if self.active + 1 < self.tabs.len() {
            self.active + 1
        } else {
            self.active - 1
        })
    }

    pub(crate) fn view_at(
        &self,
        index: usize,
        active_view: DocumentViewState,
    ) -> Result<DocumentViewState, DocumentTabError> {
        let tab = self
            .tabs
            .get(index)
            .ok_or(DocumentTabError::InvalidPayloadState)?;
        Ok(if index == self.active {
            active_view
        } else {
            tab.view
        })
    }

    pub(crate) fn document_at<'a>(
        &'a self,
        index: usize,
        active_document: &'a T,
    ) -> Result<&'a T, DocumentTabError> {
        let id = self
            .id_at(index)
            .ok_or(DocumentTabError::InvalidPayloadState)?;
        self.document_for_id(id, active_document)
    }

    pub(crate) fn active_workspace_entry(&self) -> Option<usize> {
        self.tabs
            .get(self.active)
            .and_then(|tab| tab.workspace_entry)
    }

    pub(crate) fn label(&self, index: usize) -> Option<Arc<str>> {
        self.tabs.get(index).map(|tab| Arc::clone(&tab.label))
    }

    pub(crate) fn index_for_path(&self, path: &Path) -> Option<usize> {
        self.tabs
            .iter()
            .position(|tab| tab.path.as_deref() == Some(path))
    }

    pub(crate) fn inactive_document_for_path(&self, path: &Path) -> Option<&T> {
        self.tabs
            .iter()
            .find(|tab| tab.path.as_deref() == Some(path))
            .and_then(|tab| tab.document.as_ref())
    }

    #[cfg(test)]
    pub(crate) fn clear_inactive_document_for_test(&mut self, path: &Path) -> bool {
        let Some(tab) = self
            .tabs
            .iter_mut()
            .find(|tab| tab.path.as_deref() == Some(path))
        else {
            return false;
        };
        tab.document = None;
        true
    }

    pub(crate) fn inactive_documents(&self) -> impl Iterator<Item = &T> {
        self.tabs.iter().filter_map(|tab| tab.document.as_ref())
    }

    pub(crate) fn can_navigate_back(&self) -> bool {
        self.history
            .get(..self.history_cursor)
            .unwrap_or_default()
            .iter()
            .rev()
            .any(|id| {
                self.index_for_id(*id)
                    .is_some_and(|index| index != self.active)
            })
    }

    pub(crate) fn can_navigate_forward(&self) -> bool {
        self.history
            .get(self.history_cursor.saturating_add(1)..)
            .unwrap_or_default()
            .iter()
            .any(|id| {
                self.index_for_id(*id)
                    .is_some_and(|index| index != self.active)
            })
    }

    pub(crate) fn visible_range(
        &self,
        first_visible: usize,
        visible_tabs: usize,
        overscan: usize,
    ) -> Range<usize> {
        let start = first_visible.saturating_sub(overscan).min(self.tabs.len());
        let end = first_visible
            .saturating_add(visible_tabs)
            .saturating_add(overscan)
            .min(self.tabs.len());
        start..end.max(start)
    }

    pub(crate) fn insert_and_activate(
        &mut self,
        path: &Path,
        workspace_entry: Option<usize>,
        new_document: T,
        active_document: &mut T,
        active_view: DocumentViewState,
    ) -> Result<(), DocumentTabError> {
        if self.index_for_path(path).is_some() {
            return Err(DocumentTabError::DuplicatePath(path.to_path_buf()));
        }
        if self.tabs.len() >= self.limits.tab_capacity {
            return Err(DocumentTabError::CapacityReached(self.limits.tab_capacity));
        }
        if self
            .tabs
            .get(self.active)
            .is_none_or(|tab| tab.document.is_some())
        {
            return Err(DocumentTabError::InvalidPayloadState);
        }
        let next_id = self
            .next_id
            .checked_add(1)
            .ok_or(DocumentTabError::IdentityExhausted)?;
        let (label, retained_path_bytes) = tab_metadata(Some(path))?;
        let next_path_bytes = self
            .retained_path_bytes
            .checked_add(retained_path_bytes)
            .ok_or(DocumentTabError::PathBudgetExceeded(
                self.limits.path_byte_budget,
            ))?;
        if next_path_bytes > self.limits.path_byte_budget {
            return Err(DocumentTabError::PathBudgetExceeded(
                self.limits.path_byte_budget,
            ));
        }
        self.tabs
            .try_reserve(1)
            .map_err(|_| DocumentTabError::AllocationFailed)?;
        let previous = mem::replace(active_document, new_document);
        let active_tab = &mut self.tabs[self.active];
        active_tab.document = Some(previous);
        active_tab.deferred = false;
        active_tab.view = active_view;
        let id = DocumentTabId(self.next_id);
        self.tabs.push(DocumentTab {
            id,
            path: Some(path.to_path_buf()),
            label,
            retained_path_bytes,
            workspace_entry,
            document: None,
            deferred: false,
            view: DocumentViewState::default(),
        });
        self.active = self.tabs.len() - 1;
        self.next_id = next_id;
        self.retained_path_bytes = next_path_bytes;
        self.record_navigation(id);
        Ok(())
    }

    pub(crate) fn activate(
        &mut self,
        index: usize,
        active_document: &mut T,
        active_view: DocumentViewState,
    ) -> Result<Option<DocumentViewState>, DocumentTabError> {
        if index == self.active {
            return Ok(None);
        }
        let id = self
            .tabs
            .get(index)
            .ok_or(DocumentTabError::MissingTab(index))?
            .id;
        let view = self.switch_to(index, active_document, active_view)?;
        self.record_navigation(id);
        Ok(Some(view))
    }

    pub(crate) fn navigate_back(
        &mut self,
        active_document: &mut T,
        active_view: DocumentViewState,
    ) -> Result<Option<DocumentViewState>, DocumentTabError> {
        let mut cursor = self.history_cursor;
        while let Some(previous) = cursor.checked_sub(1) {
            cursor = previous;
            let id = self.history[cursor];
            if let Some(index) = self.index_for_id(id)
                && index != self.active
            {
                let view = self.switch_to(index, active_document, active_view)?;
                self.history_cursor = cursor;
                return Ok(Some(view));
            }
        }
        Ok(None)
    }

    pub(crate) fn navigate_forward(
        &mut self,
        active_document: &mut T,
        active_view: DocumentViewState,
    ) -> Result<Option<DocumentViewState>, DocumentTabError> {
        let mut cursor = self.history_cursor;
        while let Some(next) = cursor.checked_add(1) {
            if next >= self.history.len() {
                break;
            }
            cursor = next;
            let id = self.history[cursor];
            if let Some(index) = self.index_for_id(id)
                && index != self.active
            {
                let view = self.switch_to(index, active_document, active_view)?;
                self.history_cursor = cursor;
                return Ok(Some(view));
            }
        }
        Ok(None)
    }

    pub(crate) fn close_active(
        &mut self,
        active_document: &mut T,
    ) -> Result<DocumentViewState, DocumentTabError> {
        if self.tabs.len() == 1 {
            return Err(DocumentTabError::LastTab);
        }
        let closing = self.active;
        let target = if closing + 1 < self.tabs.len() {
            closing + 1
        } else {
            closing - 1
        };
        let target_id = self.tabs[target].id;
        let target_view = self.tabs[target].view;
        let target_document = self.tabs[target]
            .document
            .take()
            .ok_or(DocumentTabError::InvalidPayloadState)?;
        let closed_document = mem::replace(active_document, target_document);
        drop(closed_document);
        let removed = self.tabs.remove(closing);
        self.retained_path_bytes = self
            .retained_path_bytes
            .saturating_sub(removed.retained_path_bytes);
        self.active = self
            .index_for_id(target_id)
            .ok_or(DocumentTabError::InvalidPayloadState)?;
        self.record_navigation(target_id);
        Ok(target_view)
    }

    #[cfg(test)]
    pub(crate) const fn retained_path_bytes(&self) -> usize {
        self.retained_path_bytes
    }

    #[cfg(test)]
    pub(crate) const fn history_len(&self) -> usize {
        self.history.len()
    }

    #[cfg(test)]
    pub(crate) fn inject_active_payload_for_test(&mut self, payload: T) {
        self.tabs[self.active].document = Some(payload);
    }

    fn switch_to(
        &mut self,
        index: usize,
        active_document: &mut T,
        active_view: DocumentViewState,
    ) -> Result<DocumentViewState, DocumentTabError> {
        if self
            .tabs
            .get(self.active)
            .is_none_or(|tab| tab.document.is_some() || tab.deferred)
        {
            return Err(DocumentTabError::InvalidPayloadState);
        }
        let target = self
            .tabs
            .get_mut(index)
            .ok_or(DocumentTabError::MissingTab(index))?;
        if target.deferred {
            return Err(DocumentTabError::InvalidPayloadState);
        }
        let target_document = target
            .document
            .take()
            .ok_or(DocumentTabError::InvalidPayloadState)?;
        let target_view = target.view;
        let previous = mem::replace(active_document, target_document);
        let active = &mut self.tabs[self.active];
        active.document = Some(previous);
        active.deferred = false;
        active.view = active_view;
        self.active = index;
        Ok(target_view)
    }

    fn record_navigation(&mut self, id: DocumentTabId) {
        if self.history.get(self.history_cursor) == Some(&id) {
            return;
        }
        self.history.truncate(self.history_cursor.saturating_add(1));
        if self.history.len() == self.limits.history_capacity {
            self.history.remove(0);
            self.history_cursor = self.history_cursor.saturating_sub(1);
        }
        self.history.push(id);
        self.history_cursor = self.history.len().saturating_sub(1);
    }
}

fn tab_metadata(path: Option<&Path>) -> Result<(Arc<str>, usize), DocumentTabError> {
    let Some(path) = path else {
        return Ok((Arc::from("Untitled"), 0));
    };
    let label: Arc<str> = path.file_name().map_or_else(
        || Arc::from("Untitled"),
        |name| Arc::from(name.to_string_lossy().as_ref()),
    );
    let retained = path
        .as_os_str()
        .as_encoded_bytes()
        .len()
        .checked_add(label.len())
        .ok_or(DocumentTabError::AllocationFailed)?;
    Ok((label, retained))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(offset: usize, scroll_y: f32) -> DocumentViewState {
        DocumentViewState {
            selection: Selection::caret(ByteOffset::new(offset)),
            scroll_y,
        }
    }

    #[test]
    fn tabs_preserve_payload_view_identity_and_duplicate_lookup()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut tabs = DocumentTabs::new(None, None, DocumentTabLimits::new(4, 1_024, 4))?;
        let mut active = String::from("scratch");
        let first_insertion = tabs.insert_and_activate(
            Path::new("/root/a.rs"),
            Some(2),
            "alpha".into(),
            &mut active,
            view(1, 2.0),
        );
        first_insertion?;
        let second_insertion = tabs.insert_and_activate(
            Path::new("/root/b.rs"),
            Some(3),
            "beta".into(),
            &mut active,
            view(3, 4.0),
        );
        second_insertion?;
        assert_eq!(active, "beta");
        assert_eq!(tabs.len(), 3);
        assert_eq!(tabs.active_workspace_entry(), Some(3));
        assert_eq!(tabs.index_for_path(Path::new("/root/a.rs")), Some(1));
        let restored = tabs
            .activate(1, &mut active, view(5, 6.0))?
            .ok_or("activation")?;
        assert_eq!(active, "alpha");
        assert_eq!(restored, view(3, 4.0));
        assert_eq!(tabs.active_index(), 1);
        assert_eq!(tabs.label(1).as_deref(), Some("a.rs"));
        assert!(tabs.retained_path_bytes() > 0);
        Ok(())
    }

    #[test]
    fn limits_refuse_without_mutating_active_payload() -> Result<(), Box<dyn std::error::Error>> {
        let defaults = DocumentTabLimits::default();
        assert_eq!(defaults.tab_capacity, 32);
        assert_eq!(defaults.path_byte_budget, 65_536);
        assert_eq!(defaults.history_capacity, 256);
        assert!(matches!(
            DocumentTabs::<String>::new(None, None, DocumentTabLimits::new(0, 1, 1)),
            Err(DocumentTabError::InvalidLimits)
        ));
        let mut tabs = DocumentTabs::new(None, None, DocumentTabLimits::new(2, 20, 2))?;
        let mut active = String::from("scratch");
        let insertion = tabs.insert_and_activate(
            Path::new("a"),
            Some(0),
            "a".into(),
            &mut active,
            view(0, 0.0),
        );
        insertion?;
        let before = active.clone();
        assert!(matches!(
            tabs.insert_and_activate(
                Path::new("b"),
                Some(1),
                "b".into(),
                &mut active,
                view(0, 0.0)
            ),
            Err(DocumentTabError::CapacityReached(2))
        ));
        assert_eq!(active, before);
        assert_eq!(tabs.len(), 2);

        let mut limited = DocumentTabs::new(None, None, DocumentTabLimits::new(3, 4, 2))?;
        let mut limited_active = String::from("scratch");
        assert!(matches!(
            limited.insert_and_activate(
                Path::new("long-name"),
                Some(0),
                "x".into(),
                &mut limited_active,
                view(0, 0.0)
            ),
            Err(DocumentTabError::PathBudgetExceeded(4))
        ));
        assert_eq!(limited_active, "scratch");
        assert_eq!(limited.len(), 1);

        let mut exact = DocumentTabs::new(None, None, DocumentTabLimits::new(2, 2, 2))?;
        let mut exact_active = String::from("scratch");
        let insertion = exact.insert_and_activate(
            Path::new("a"),
            Some(0),
            "a".into(),
            &mut exact_active,
            view(0, 0.0),
        );
        insertion?;
        assert_eq!(exact.retained_path_bytes(), 2);
        Ok(())
    }

    #[test]
    fn every_tab_error_has_a_nonempty_structured_message() {
        let errors = [
            DocumentTabError::InvalidLimits,
            DocumentTabError::AllocationFailed,
            DocumentTabError::CapacityReached(2),
            DocumentTabError::PathBudgetExceeded(3),
            DocumentTabError::IdentityExhausted,
            DocumentTabError::MissingTab(4),
            DocumentTabError::DuplicatePath(PathBuf::from("duplicate")),
            DocumentTabError::LastTab,
            DocumentTabError::InvalidPayloadState,
        ];
        for error in errors {
            assert!(!error.to_string().is_empty());
            assert!(std::error::Error::source(&error).is_none());
        }
    }

    #[test]
    fn bounded_history_branches_and_close_are_deterministic()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut tabs = DocumentTabs::new(None, None, DocumentTabLimits::new(5, 1_024, 3))?;
        let mut active = String::from("scratch");
        assert!(tabs.navigate_back(&mut active, view(0, 0.0))?.is_none());
        assert!(matches!(
            tabs.close_active(&mut active),
            Err(DocumentTabError::LastTab)
        ));
        let a_insertion = tabs.insert_and_activate(
            Path::new("/a"),
            Some(0),
            "a".into(),
            &mut active,
            view(0, 0.0),
        );
        a_insertion?;
        let b_insertion = tabs.insert_and_activate(
            Path::new("/b"),
            Some(1),
            "b".into(),
            &mut active,
            view(0, 0.0),
        );
        b_insertion?;
        let c_insertion = tabs.insert_and_activate(
            Path::new("/c"),
            Some(2),
            "c".into(),
            &mut active,
            view(0, 0.0),
        );
        c_insertion?;
        assert!(tabs.can_navigate_back());
        assert!(!tabs.can_navigate_forward());
        assert_eq!(tabs.history_len(), 3);
        assert!(tabs.navigate_back(&mut active, view(1, 1.0))?.is_some());
        assert!(tabs.can_navigate_back());
        assert!(tabs.can_navigate_forward());
        assert_eq!(active, "b");
        assert!(tabs.navigate_back(&mut active, view(2, 2.0))?.is_some());
        assert!(!tabs.can_navigate_back());
        assert!(tabs.can_navigate_forward());
        assert_eq!(active, "a");
        assert!(tabs.navigate_forward(&mut active, view(3, 3.0))?.is_some());
        assert_eq!(active, "b");
        let a = tabs.index_for_path(Path::new("/a")).ok_or("a")?;
        assert!(tabs.activate(a, &mut active, view(4, 4.0))?.is_some());
        assert!(tabs.navigate_forward(&mut active, view(5, 5.0))?.is_none());
        let restored = tabs.close_active(&mut active)?;
        assert_eq!(active, "b");
        assert_eq!(restored, view(4, 4.0));

        let c = tabs.index_for_path(Path::new("/c")).ok_or("c")?;
        assert!(tabs.activate(c, &mut active, view(6, 6.0))?.is_some());
        assert_eq!(active, "c");
        let restored_last = tabs.close_active(&mut active)?;
        assert_eq!(active, "b");
        assert_eq!(restored_last, view(6, 6.0));
        assert_eq!(tabs.active_workspace_entry(), Some(1));

        let active_id = tabs.tabs[tabs.active].id;
        let scratch_id = tabs.tabs[0].id;
        tabs.history.clear();
        tabs.history
            .extend([scratch_id, DocumentTabId(u64::MAX), active_id]);
        tabs.history_cursor = 2;
        assert!(tabs.navigate_back(&mut active, view(7, 7.0))?.is_some());
        assert_eq!(active, "scratch");
        assert!(tabs.navigate_forward(&mut active, view(8, 8.0))?.is_some());
        assert_eq!(active, "b");
        Ok(())
    }

    #[test]
    fn defensive_transitions_fail_without_losing_the_active_payload()
    -> Result<(), Box<dyn std::error::Error>> {
        let tiny = DocumentTabLimits::new(2, 1, 2);
        assert!(matches!(
            DocumentTabs::<String>::new(Some(Path::new("long")), None, tiny),
            Err(DocumentTabError::PathBudgetExceeded(1))
        ));
        let (root_label, root_bytes) =
            tab_metadata(Some(Path::new(std::path::MAIN_SEPARATOR_STR)))?;
        assert_eq!(&*root_label, "Untitled");
        assert!(root_bytes > 0);

        let mut tabs = DocumentTabs::new(None, None, DocumentTabLimits::new(4, 1_024, 4))?;
        let mut active = String::from("scratch");
        let insertion = tabs.insert_and_activate(
            Path::new("a"),
            Some(0),
            "alpha".into(),
            &mut active,
            view(1, 1.0),
        );
        insertion?;
        let before = active.clone();
        assert!(matches!(
            tabs.insert_and_activate(
                Path::new("a"),
                Some(0),
                "duplicate".into(),
                &mut active,
                view(2, 2.0)
            ),
            Err(DocumentTabError::DuplicatePath(path)) if path == Path::new("a")
        ));
        assert_eq!(active, before);
        let same_tab = tabs.activate(tabs.active_index(), &mut active, view(3, 3.0))?;
        assert!(same_tab.is_none());
        let history_len = tabs.history_len();
        tabs.record_navigation(tabs.tabs[tabs.active].id);
        assert_eq!(tabs.history_len(), history_len);

        tabs.inject_active_payload_for_test("invalid duplicate payload".into());
        assert!(matches!(
            tabs.insert_and_activate(
                Path::new("b"),
                Some(1),
                "beta".into(),
                &mut active,
                view(4, 4.0)
            ),
            Err(DocumentTabError::InvalidPayloadState)
        ));
        assert!(matches!(
            tabs.activate(0, &mut active, view(5, 5.0)),
            Err(DocumentTabError::InvalidPayloadState)
        ));
        assert_eq!(active, before);
        Ok(())
    }

    #[test]
    fn restored_tabs_defer_payloads_until_checked_materialization()
    -> Result<(), Box<dyn std::error::Error>> {
        let restored = vec![
            RestoredDocumentTab {
                path: Some(PathBuf::from("/alpha")),
                view: view(1, 1.0),
            },
            RestoredDocumentTab {
                path: Some(PathBuf::from("/beta")),
                view: view(2, 2.0),
            },
            RestoredDocumentTab {
                path: Some(PathBuf::from("/gamma")),
                view: view(3, 3.0),
            },
        ];
        let mut tabs =
            DocumentTabs::from_restored(restored, 1, DocumentTabLimits::new(3, 1_024, 3))?;
        let mut active = String::from("beta");

        assert_eq!(tabs.active_index(), 1);
        assert_eq!(tabs.is_deferred(0), Ok(true));
        assert_eq!(tabs.is_deferred(1), Ok(false));
        assert_eq!(tabs.is_deferred(2), Ok(true));
        assert!(matches!(
            tabs.document_at(0, &active),
            Err(DocumentTabError::InvalidPayloadState)
        ));

        tabs.materialize(0, String::from("alpha"))?;
        assert_eq!(tabs.is_deferred(0), Ok(false));
        assert!(tabs.activate(0, &mut active, view(4, 4.0))?.is_some());
        assert_eq!(active, "alpha");
        assert_eq!(tabs.document_at(1, &active)?, "beta");
        assert!(matches!(
            tabs.materialize(0, String::from("duplicate")),
            Err(DocumentTabError::InvalidPayloadState)
        ));
        assert_eq!(tabs.navigation_target(false), Some(1));
        assert_eq!(tabs.close_target(), Ok(1));
        Ok(())
    }

    #[test]
    fn visible_projection_is_bounded_to_range_and_overscan()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut tabs = DocumentTabs::new(None, None, DocumentTabLimits::new(8, 1_024, 8))?;
        let mut active = String::from("scratch");
        for index in 0..5 {
            let path = PathBuf::from(format!("/{index}"));
            let insertion = tabs.insert_and_activate(
                &path,
                Some(index),
                index.to_string(),
                &mut active,
                view(0, 0.0),
            );
            insertion?;
        }
        assert_eq!(tabs.visible_range(3, 1, 1), 2..5);
        assert_eq!(tabs.visible_range(99, usize::MAX, 2), 6..6);
        Ok(())
    }
}
