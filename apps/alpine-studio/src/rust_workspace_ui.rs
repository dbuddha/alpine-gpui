//! Bounded rename input and prepared workspace-edit preview state.

use std::{error::Error, fmt, mem, sync::Arc};

use crate::{
    rust_diagnostics::{
        LanguageIdentity, RustDiagnosticsSnapshot, WorkspaceEditIdentity, WorkspaceEditKind,
        WorkspaceEditPreparationOutput,
    },
    rust_workspace_edit::{PreparedWorkspaceEdit, WorkspaceEditError},
};

pub(crate) const MAX_RENAME_INPUT_BYTES: usize = 256;
pub(crate) const MAX_WORKSPACE_PREVIEW_FILES: usize = 8;
const MAX_PREVIEW_LINE_BYTES: usize = 4_096;
const MAX_PREVIEW_LINES: usize = MAX_WORKSPACE_PREVIEW_FILES + 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WorkspaceEditPanelOutcome {
    Ignored,
    Empty(WorkspaceEditKind),
    Preview {
        kind: WorkspaceEditKind,
        files: usize,
        edits: usize,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WorkspaceEditPanelError {
    InputTooLong,
    InvalidName,
    InvalidComposition,
    AllocationFailed,
    StaleResponse,
    Preparation(WorkspaceEditError),
}

impl fmt::Display for WorkspaceEditPanelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Rust workspace edit unavailable: {self:?}")
    }
}

impl Error for WorkspaceEditPanelError {}

#[derive(Default)]
struct RenameInput {
    text: String,
    composition: Option<String>,
}

#[derive(Default)]
enum PanelState {
    #[default]
    Closed,
    Rename(RenameInput),
    Waiting(WorkspaceEditKind),
    Preparing(WorkspaceEditIdentity),
    Preview {
        identity: WorkspaceEditIdentity,
        prepared: PreparedWorkspaceEdit,
    },
}

#[derive(Default)]
pub(crate) struct WorkspaceEditPanel {
    state: PanelState,
    lines: Box<[Box<str>]>,
    accessibility_label: Option<Arc<str>>,
    peak_retained_bytes: usize,
}

impl WorkspaceEditPanel {
    pub(crate) fn is_open(&self) -> bool {
        !matches!(self.state, PanelState::Closed)
    }

    pub(crate) fn is_rename_input(&self) -> bool {
        matches!(self.state, PanelState::Rename(_))
    }

    pub(crate) fn open_rename(&mut self) -> Result<bool, WorkspaceEditPanelError> {
        if self.is_rename_input() {
            return Ok(false);
        }
        self.state = PanelState::Rename(RenameInput::default());
        self.rebuild_rename_lines()?;
        Ok(true)
    }

    pub(crate) fn begin_composition(&mut self) -> bool {
        let PanelState::Rename(input) = &mut self.state else {
            return false;
        };
        if input.composition.is_some() {
            return false;
        }
        input.composition = Some(String::new());
        true
    }

    pub(crate) fn update_composition(
        &mut self,
        text: &str,
        selected_start_utf16: u32,
        selected_length_utf16: u32,
    ) -> Result<bool, WorkspaceEditPanelError> {
        let PanelState::Rename(input) = &mut self.state else {
            return Ok(false);
        };
        validate_composition(text, selected_start_utf16, selected_length_utf16)?;
        checked_input_length(input.text.len(), text.len())?;
        if text.chars().any(char::is_control) {
            return Err(WorkspaceEditPanelError::InvalidName);
        }
        if input.composition.as_deref() == Some(text) {
            return Ok(false);
        }
        let mut composition = String::new();
        composition
            .try_reserve_exact(text.len())
            .map_err(|_| WorkspaceEditPanelError::AllocationFailed)?;
        composition.push_str(text);
        input.composition = Some(composition);
        self.rebuild_rename_lines()?;
        Ok(true)
    }

    pub(crate) fn cancel_composition(&mut self) -> bool {
        let PanelState::Rename(input) = &mut self.state else {
            return false;
        };
        let changed = input.composition.take().is_some();
        if changed {
            let _ = self.rebuild_rename_lines();
        }
        changed
    }

    pub(crate) fn commit_text(&mut self, text: &str) -> Result<bool, WorkspaceEditPanelError> {
        let PanelState::Rename(input) = &mut self.state else {
            return Ok(false);
        };
        if text.chars().any(char::is_control) {
            return Err(WorkspaceEditPanelError::InvalidName);
        }
        let next_length = checked_input_length(input.text.len(), text.len())?;
        input.composition = None;
        if text.is_empty() {
            return Ok(false);
        }
        input
            .text
            .try_reserve(next_length.saturating_sub(input.text.len()))
            .map_err(|_| WorkspaceEditPanelError::AllocationFailed)?;
        input.text.push_str(text);
        self.rebuild_rename_lines()?;
        Ok(true)
    }

    pub(crate) fn delete_backward(&mut self) -> Result<bool, WorkspaceEditPanelError> {
        let PanelState::Rename(input) = &mut self.state else {
            return Ok(false);
        };
        input.composition = None;
        if input.text.pop().is_none() {
            return Ok(false);
        }
        self.rebuild_rename_lines()?;
        Ok(true)
    }

    pub(crate) fn take_rename_for_request(&mut self) -> Result<Box<str>, WorkspaceEditPanelError> {
        let PanelState::Rename(input) = &mut self.state else {
            return Err(WorkspaceEditPanelError::InvalidName);
        };
        if input.composition.is_some()
            || input.text.is_empty()
            || input.text.chars().any(char::is_control)
        {
            return Err(WorkspaceEditPanelError::InvalidName);
        }
        let name: Box<str> = input.text.clone().into_boxed_str();
        self.wait(WorkspaceEditKind::Rename)?;
        Ok(name)
    }

    pub(crate) fn wait(
        &mut self,
        kind: WorkspaceEditKind,
    ) -> Result<bool, WorkspaceEditPanelError> {
        self.state = PanelState::Waiting(kind);
        self.replace_lines(
            vec![format!("{} requested. Escape cancels.", kind.label()).into_boxed_str()],
            Arc::from(format!("{} requested", kind.label())),
        )?;
        Ok(true)
    }

    pub(crate) fn preparation_started(
        &mut self,
        identity: WorkspaceEditIdentity,
    ) -> Result<bool, WorkspaceEditPanelError> {
        let expected = matches!(self.state, PanelState::Waiting(kind) if kind == identity.kind());
        if !expected {
            return Ok(false);
        }
        self.state = PanelState::Preparing(identity);
        self.replace_lines(
            vec![format!("Preparing {} preview...", identity.kind().label()).into_boxed_str()],
            Arc::from(format!("Preparing {} preview", identity.kind().label())),
        )?;
        Ok(true)
    }

    pub(crate) fn preparation_failed(&mut self, identity: WorkspaceEditIdentity) -> bool {
        if !matches!(self.state, PanelState::Preparing(current) if current == identity) {
            return false;
        }
        self.release();
        true
    }

    pub(crate) fn complete(
        &mut self,
        output: WorkspaceEditPreparationOutput,
        current: LanguageIdentity,
        language: &RustDiagnosticsSnapshot,
    ) -> Result<WorkspaceEditPanelOutcome, WorkspaceEditPanelError> {
        if !matches!(self.state, PanelState::Preparing(identity) if identity == output.identity) {
            return Ok(WorkspaceEditPanelOutcome::Ignored);
        }
        if !output.identity.matches(current, language) {
            self.release();
            return Err(WorkspaceEditPanelError::StaleResponse);
        }
        let prepared = match output.result {
            Ok(prepared) => prepared,
            Err(error) => {
                self.release();
                return Err(WorkspaceEditPanelError::Preparation(error));
            }
        };
        let kind = output.identity.kind();
        let files = prepared.file_count();
        let edits = prepared.edit_count();
        if files == 0 || edits == 0 {
            self.release();
            return Ok(WorkspaceEditPanelOutcome::Empty(kind));
        }
        let lines = preview_lines(kind, &prepared)?;
        let label = Arc::from(format!(
            "{} preview: {files} file(s), {edits} edit(s). Publication is not enabled.",
            kind.label()
        ));
        self.state = PanelState::Preview {
            identity: output.identity,
            prepared,
        };
        self.replace_lines(lines, label)?;
        Ok(WorkspaceEditPanelOutcome::Preview { kind, files, edits })
    }

    pub(crate) fn cancel(&mut self) -> bool {
        if !self.is_open() {
            return false;
        }
        self.release();
        true
    }

    pub(crate) fn line_count(&self) -> usize {
        self.lines.len()
    }

    pub(crate) fn line(&self, index: usize) -> Option<&str> {
        self.lines.get(index).map(AsRef::as_ref)
    }

    pub(crate) fn accessibility_label(&self) -> Option<Arc<str>> {
        self.accessibility_label.clone()
    }

    #[allow(
        dead_code,
        reason = "the accepted publication slice will consume the retained prepared edit"
    )]
    pub(crate) fn preview(&self) -> Option<(WorkspaceEditIdentity, &PreparedWorkspaceEdit)> {
        match &self.state {
            PanelState::Preview { identity, prepared } => Some((*identity, prepared)),
            _ => None,
        }
    }

    #[cfg(test)]
    pub(crate) fn retained_bytes(&self) -> usize {
        self.current_retained_bytes()
    }

    #[cfg(test)]
    pub(crate) const fn peak_retained_bytes(&self) -> usize {
        self.peak_retained_bytes
    }

    fn rebuild_rename_lines(&mut self) -> Result<(), WorkspaceEditPanelError> {
        let PanelState::Rename(input) = &self.state else {
            return Ok(());
        };
        let composition = input.composition.as_deref().unwrap_or_default();
        let mut line = String::new();
        line.try_reserve("Rename Rust symbol: ".len() + input.text.len() + composition.len() + 16)
            .map_err(|_| WorkspaceEditPanelError::AllocationFailed)?;
        line.push_str("Rename Rust symbol: ");
        line.push_str(&input.text);
        line.push_str(composition);
        line.push_str(" | Enter submits");
        let label = Arc::from(line.clone());
        self.replace_lines(vec![line.into_boxed_str()], label)
    }

    fn replace_lines(
        &mut self,
        lines: Vec<Box<str>>,
        label: Arc<str>,
    ) -> Result<(), WorkspaceEditPanelError> {
        if lines.len() > MAX_PREVIEW_LINES
            || lines.iter().any(|line| line.len() > MAX_PREVIEW_LINE_BYTES)
        {
            return Err(WorkspaceEditPanelError::AllocationFailed);
        }
        self.lines = lines.into_boxed_slice();
        self.accessibility_label = Some(label);
        self.peak_retained_bytes = self.peak_retained_bytes.max(self.current_retained_bytes());
        Ok(())
    }

    fn current_retained_bytes(&self) -> usize {
        let state_bytes = match &self.state {
            PanelState::Closed | PanelState::Waiting(_) | PanelState::Preparing(_) => 0,
            PanelState::Rename(input) => input
                .text
                .capacity()
                .saturating_add(input.composition.as_ref().map_or(0, String::capacity)),
            PanelState::Preview { prepared, .. } => prepared.retained_bytes(),
        };
        state_bytes
            .saturating_add(
                self.lines
                    .iter()
                    .map(|line| mem::size_of::<Box<str>>().saturating_add(line.len()))
                    .sum::<usize>(),
            )
            .saturating_add(
                self.accessibility_label
                    .as_ref()
                    .map_or(0, |label| label.len()),
            )
    }

    fn release(&mut self) {
        self.state = PanelState::Closed;
        self.lines = Box::new([]);
        self.accessibility_label = None;
    }
}

fn checked_input_length(current: usize, added: usize) -> Result<usize, WorkspaceEditPanelError> {
    let length = current
        .checked_add(added)
        .ok_or(WorkspaceEditPanelError::InputTooLong)?;
    if length > MAX_RENAME_INPUT_BYTES {
        Err(WorkspaceEditPanelError::InputTooLong)
    } else {
        Ok(length)
    }
}

fn validate_composition(
    text: &str,
    selected_start_utf16: u32,
    selected_length_utf16: u32,
) -> Result<(), WorkspaceEditPanelError> {
    let selected_end = selected_start_utf16
        .checked_add(selected_length_utf16)
        .ok_or(WorkspaceEditPanelError::InvalidComposition)?;
    let units = u32::try_from(text.encode_utf16().count())
        .map_err(|_| WorkspaceEditPanelError::InvalidComposition)?;
    if selected_end > units {
        Err(WorkspaceEditPanelError::InvalidComposition)
    } else {
        Ok(())
    }
}

fn preview_lines(
    kind: WorkspaceEditKind,
    prepared: &PreparedWorkspaceEdit,
) -> Result<Vec<Box<str>>, WorkspaceEditPanelError> {
    let files = prepared.file_count();
    let edits = prepared.edit_count();
    let visible = files.min(MAX_WORKSPACE_PREVIEW_FILES);
    let mut lines = Vec::new();
    lines
        .try_reserve_exact(visible.saturating_add(2))
        .map_err(|_| WorkspaceEditPanelError::AllocationFailed)?;
    lines.push(
        format!("{} preview: {files} file(s), {edits} edit(s)", kind.label()).into_boxed_str(),
    );
    for file in prepared.files().iter().take(visible) {
        let path = file.path().to_string_lossy();
        let mut line = String::new();
        let admitted = path.len().min(MAX_PREVIEW_LINE_BYTES.saturating_sub(4));
        let admitted = floor_char_boundary(&path, admitted);
        line.try_reserve(admitted.saturating_add(4))
            .map_err(|_| WorkspaceEditPanelError::AllocationFailed)?;
        line.push_str("- ");
        line.push_str(&path[..admitted]);
        if admitted != path.len() {
            line.push_str("..");
        }
        lines.push(line.into_boxed_str());
    }
    lines.push(Box::from(
        "Preview only. Escape closes; publication awaits crash-recovery approval.",
    ));
    Ok(lines)
}

fn floor_char_boundary(value: &str, mut index: usize) -> usize {
    index = index.min(value.len());
    while !value.is_char_boundary(index) {
        index = index.saturating_sub(1);
    }
    index
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rename_input_is_ime_safe_bounded_and_releases_storage() -> Result<(), Box<dyn Error>> {
        let mut panel = WorkspaceEditPanel::default();
        assert!(panel.open_rename()?);
        assert!(!panel.open_rename()?);
        assert!(panel.commit_text("renamed")?);
        assert!(panel.begin_composition());
        assert!(panel.update_composition("_value", 0, 6)?);
        assert_eq!(panel.line_count(), 1);
        assert!(
            panel
                .line(0)
                .is_some_and(|line| line.contains("renamed_value"))
        );
        assert_eq!(
            panel.update_composition("x", 2, 0),
            Err(WorkspaceEditPanelError::InvalidComposition)
        );
        assert!(panel.cancel_composition());
        assert!(panel.delete_backward()?);
        assert_eq!(panel.take_rename_for_request()?.as_ref(), "rename");
        assert!(panel.is_open());
        assert!(!panel.is_rename_input());
        assert!(panel.retained_bytes() > 0);
        assert!(panel.cancel());
        assert_eq!(panel.retained_bytes(), 0);
        assert!(panel.peak_retained_bytes() > 0);
        Ok(())
    }

    #[test]
    fn rename_rejects_invalid_and_oversized_names_without_state_loss() -> Result<(), Box<dyn Error>>
    {
        let mut panel = WorkspaceEditPanel::default();
        panel.open_rename()?;
        assert_eq!(
            panel.commit_text("bad\nname"),
            Err(WorkspaceEditPanelError::InvalidName)
        );
        panel.commit_text(&"x".repeat(MAX_RENAME_INPUT_BYTES))?;
        assert_eq!(
            panel.commit_text("y"),
            Err(WorkspaceEditPanelError::InputTooLong)
        );
        assert_eq!(
            panel.line(0).map(str::len),
            Some("Rename Rust symbol: ".len() + MAX_RENAME_INPUT_BYTES + " | Enter submits".len())
        );
        Ok(())
    }

    #[test]
    fn preview_line_truncation_preserves_utf8_boundaries() {
        let value = format!("{}x", "🙂".repeat(2_000));
        let boundary = floor_char_boundary(&value, MAX_PREVIEW_LINE_BYTES - 4);
        assert!(value.is_char_boundary(boundary));
        assert!(boundary <= MAX_PREVIEW_LINE_BYTES - 4);
    }
}
