//! Paths macOS asks the editor to open: Finder's Open With and the Dock.

use std::{
    fmt::Display,
    path::{Path, PathBuf},
    sync::Arc,
};

use alpine_platform_macos::{KeyState, PointerAction, SurfaceEvent};

use crate::{
    DocumentViewState, EditorApp, EditorDocument, EventEffect, FileTreeState, LocalStatus,
    WorkspaceError, WorkspaceSelectionError, record_project_settings_result,
};

/// Whether paths macOS opens may still replace the restored session.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum RestoredLaunch {
    /// Set by `run()` until the first deliberate input or loss of focus.
    Pending,
    #[default]
    Settled,
}

/// Counts the paths of one batch that did not open, keeping the first reason.
#[derive(Default)]
struct OpenFailures {
    count: usize,
    first: Option<String>,
}

impl EditorApp {
    /// Opens folders first, since a new workspace closes tabs, then files in
    /// order as File > Open does. At a restored launch, the first batch
    /// replaces a session with no unsaved text, as launch arguments would.
    pub(crate) fn open_paths(&mut self, paths: &[PathBuf], dropped: usize) -> EventEffect {
        let mut replace_session = std::mem::take(&mut self.restored_launch)
            == RestoredLaunch::Pending
            && !self.holds_unsaved_work();
        let recovered = match &self.local_status {
            Some(LocalStatus::Recovery(warning)) => Some(Arc::clone(warning)),
            _ => None,
        };
        let mut failures = OpenFailures::default();
        let mut folders = Vec::new();
        let mut files = Vec::new();
        for path in paths {
            match std::fs::metadata(path) {
                Ok(metadata) if metadata.is_dir() => folders.push(path.as_path()),
                Ok(_) => files.push(path.as_path()),
                Err(source) => self.note_open_failure(
                    &mut failures,
                    &WorkspaceSelectionError::Workspace(WorkspaceError::io(
                        "read opened path",
                        path,
                        source,
                    )),
                ),
            }
        }
        let mut effect = EventEffect::default();
        for folder in folders {
            // A new workspace closes every tab, unsaved text included.
            if self.holds_unsaved_work() {
                let refused = format!(
                    "save changes before opening the folder {}",
                    folder.display()
                );
                self.note_open_failure(&mut failures, &refused);
                continue;
            }
            match self.try_open_workspace_root(folder) {
                Ok(opened) => {
                    effect = effect.merge(opened);
                    replace_session = false;
                }
                Err(error) => self.note_open_failure(&mut failures, &error),
            }
        }
        for file in files {
            let opened = if replace_session {
                self.replace_session_with_file(file)
            } else {
                self.open_workspace_path(file, None)
            };
            match opened {
                Ok(opened) => {
                    effect = effect.merge(opened);
                    replace_session = false;
                }
                Err(error) => self.note_open_failure(&mut failures, &error),
            }
        }
        effect.merge(self.report_open_outcome(failures, dropped, recovered))
    }

    /// Deliberate input or losing focus ends the window in which opened
    /// paths replace a restored session: the user has moved on.
    pub(crate) fn settle_launch_on_input(&mut self, event: &SurfaceEvent) {
        if matches!(
            event,
            SurfaceEvent::Keyboard {
                state: KeyState::Down,
                ..
            } | SurfaceEvent::Pointer {
                action: PointerAction::Down,
                ..
            } | SurfaceEvent::Focus { focused: false, .. }
                | SurfaceEvent::Scroll { .. }
                | SurfaceEvent::Ime { .. }
                | SurfaceEvent::Menu { .. }
        ) {
            self.restored_launch = RestoredLaunch::Settled;
        }
    }

    /// Whether any loaded tab holds text that is not on disk.
    fn holds_unsaved_work(&self) -> bool {
        self.pending_recovery.iter().any(Option::is_some)
            || (0..self.tabs.len()).any(|index| match self.tabs.is_deferred(index) {
                Ok(true) => false,
                Ok(false) => self
                    .tabs
                    .document_at(index, &self.document)
                    .map_or(true, |document| {
                        document.is_dirty() || document.has_recovery_conflict()
                    }),
                Err(_) => true,
            })
    }

    /// Replaces a clean restored session with one file tab and no workspace.
    fn replace_session_with_file(
        &mut self,
        path: &Path,
    ) -> Result<EventEffect, WorkspaceSelectionError> {
        let document = EditorDocument::open(path).map_err(WorkspaceSelectionError::File)?;
        let next_document = self
            .runtime_document_revision
            .checked_add(1)
            .ok_or(WorkspaceSelectionError::RevisionExhausted)?;
        let next_workspace = self
            .runtime_workspace_revision
            .checked_add(1)
            .ok_or(WorkspaceSelectionError::RevisionExhausted)?;
        let (tabs, panes) = Self::initial_tabs_and_panes(Some(path))
            .map_err(|_| WorkspaceSelectionError::RevisionExhausted)?;
        self.tabs = tabs;
        self.panes = panes;
        self.document = document;
        self.pending_recovery.clear();
        self.runtime_document_revision = next_document;
        self.runtime_workspace_revision = next_workspace;
        self.workspace = None;
        self.active_workspace_entry = None;
        self.file_tree = FileTreeState::default();
        self.apply_document_view(DocumentViewState::default());
        let result = self.settings_reload.replace_project(None);
        record_project_settings_result(self, result);
        Ok(EventEffect::document_replacement())
    }

    fn note_open_failure(&mut self, failures: &mut OpenFailures, error: &impl Display) {
        self.workspace_failures = self.workspace_failures.saturating_add(1);
        failures.count = failures.count.saturating_add(1);
        failures.first.get_or_insert_with(|| error.to_string());
    }

    /// Shows what did not open, after a recovery warning that was showing;
    /// each successful open clears the status line.
    fn report_open_outcome(
        &mut self,
        failures: OpenFailures,
        dropped: usize,
        recovered: Option<Arc<str>>,
    ) -> EventEffect {
        let failed = match (failures.count, failures.first) {
            (1, Some(first)) => Some(first),
            (count, Some(first)) => Some(format!("{count} paths did not open; the first: {first}")),
            (_, None) => None,
        };
        let skipped = (dropped > 0).then(|| format!("{dropped} more paths were not opened."));
        let message = match (failed, skipped) {
            (Some(failed), Some(skipped)) => Some(format!("{failed}. {skipped}")),
            (Some(message), None) | (None, Some(message)) => Some(message),
            (None, None) => None,
        };
        if let Some(message) = &message {
            self.last_workspace_error = Some(Arc::from(message.as_str()));
        }
        let status = match (recovered, message) {
            (Some(warning), Some(message)) => {
                LocalStatus::Recovery(Arc::from(format!("{warning} {message}")))
            }
            (Some(warning), None) => LocalStatus::Recovery(warning),
            (None, Some(message)) => LocalStatus::Workspace(Arc::from(message)),
            (None, None) => return EventEffect::default(),
        };
        self.set_local_status(status)
    }
}
