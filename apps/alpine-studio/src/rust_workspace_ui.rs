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
    Queued {
        identity: WorkspaceEditIdentity,
        prepared: PreparedWorkspaceEdit,
    },
    Publishing(WorkspaceEditIdentity),
}

#[derive(Default)]
pub(crate) struct WorkspaceEditPanel {
    state: PanelState,
    lines: Box<[Box<str>]>,
    accessibility_label: Option<Arc<str>>,
    peak_retained_bytes: usize,
    #[cfg(test)]
    force_replace_lines_failure: bool,
}

impl WorkspaceEditPanel {
    pub(crate) fn is_open(&self) -> bool {
        !matches!(self.state, PanelState::Closed)
    }

    pub(crate) fn is_rename_input(&self) -> bool {
        matches!(self.state, PanelState::Rename(_))
    }

    pub(crate) fn is_publication_pending(&self) -> bool {
        matches!(
            self.state,
            PanelState::Queued { .. } | PanelState::Publishing(_)
        )
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
        let lines = vec![format!("{} requested. Escape cancels.", kind.label()).into_boxed_str()];
        let label = Arc::from(format!("{} requested", kind.label()));
        self.replace_lines(lines, label)?;
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
        let lines =
            vec![format!("Preparing {} preview...", identity.kind().label()).into_boxed_str()];
        let label = Arc::from(format!("Preparing {} preview", identity.kind().label()));
        self.replace_lines(lines, label)?;
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
        if edits == 0 {
            self.release();
            return Ok(WorkspaceEditPanelOutcome::Empty(kind));
        }
        let lines = preview_lines(kind, &prepared)?;
        let label = Arc::from(format!(
            "{} preview: {files} file(s), {edits} edit(s). Enter applies atomically.",
            kind.label()
        ));
        self.state = PanelState::Preview {
            identity: output.identity,
            prepared,
        };
        self.replace_lines(lines, label)?;
        Ok(WorkspaceEditPanelOutcome::Preview { kind, files, edits })
    }

    pub(crate) fn queue_publication(
        &mut self,
        current: LanguageIdentity,
        language: &RustDiagnosticsSnapshot,
    ) -> Result<bool, WorkspaceEditPanelError> {
        let state = mem::take(&mut self.state);
        let PanelState::Preview { identity, prepared } = state else {
            self.state = state;
            return Ok(false);
        };
        if !identity.matches(current, language) {
            self.release();
            return Err(WorkspaceEditPanelError::StaleResponse);
        }
        let kind = identity.kind();
        self.state = PanelState::Queued { identity, prepared };
        let lines = vec![format!("{} publication queued...", kind.label()).into_boxed_str()];
        let label = Arc::from(format!("{} publication queued", kind.label()));
        self.replace_lines(lines, label)?;
        Ok(true)
    }

    pub(crate) fn take_queued_publication(
        &mut self,
    ) -> Option<(WorkspaceEditIdentity, PreparedWorkspaceEdit)> {
        let state = mem::take(&mut self.state);
        let PanelState::Queued { identity, prepared } = state else {
            self.state = state;
            return None;
        };
        self.state = PanelState::Publishing(identity);
        Some((identity, prepared))
    }

    pub(crate) fn publication_matches(
        &self,
        identity: WorkspaceEditIdentity,
        current: LanguageIdentity,
    ) -> bool {
        matches!(self.state, PanelState::Publishing(active) if active == identity)
            && identity.matches_document(current)
    }

    pub(crate) fn publication_failed(
        &mut self,
        identity: WorkspaceEditIdentity,
        prepared: PreparedWorkspaceEdit,
    ) -> Result<bool, WorkspaceEditPanelError> {
        if !matches!(self.state, PanelState::Publishing(active) if active == identity) {
            return Ok(false);
        }
        let kind = identity.kind();
        let lines = preview_lines(kind, &prepared)?;
        let files = prepared.file_count();
        let edits = prepared.edit_count();
        self.state = PanelState::Preview { identity, prepared };
        let label = Arc::from(format!(
            "{} publication failed; preview retained for {files} file(s), {edits} edit(s)",
            kind.label()
        ));
        self.replace_lines(lines, label)?;
        Ok(true)
    }

    pub(crate) fn publication_succeeded(&mut self, identity: WorkspaceEditIdentity) -> bool {
        if !matches!(self.state, PanelState::Publishing(active) if active == identity) {
            return false;
        }
        self.release();
        true
    }

    pub(crate) fn cancel(&mut self) -> bool {
        if !self.is_open() || self.is_publication_pending() {
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

    #[cfg(test)]
    pub(crate) fn install_preview_for_test(
        &mut self,
        identity: WorkspaceEditIdentity,
        prepared: PreparedWorkspaceEdit,
    ) -> Result<(), WorkspaceEditPanelError> {
        let kind = identity.kind();
        let files = prepared.file_count();
        let edits = prepared.edit_count();
        let lines = preview_lines(kind, &prepared)?;
        self.state = PanelState::Preview { identity, prepared };
        self.replace_lines(
            lines,
            Arc::from(format!(
                "{} preview: {files} file(s), {edits} edit(s). Enter applies atomically.",
                kind.label()
            )),
        )
    }

    #[cfg(test)]
    pub(crate) fn force_replace_lines_failure_once(&mut self) {
        self.force_replace_lines_failure = true;
    }

    fn rebuild_rename_lines(&mut self) -> Result<(), WorkspaceEditPanelError> {
        let PanelState::Rename(input) = &self.state else {
            return Ok(());
        };
        let composition = input.composition.as_deref().unwrap_or_default();
        let mut line = String::new();
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
        #[cfg(test)]
        if mem::take(&mut self.force_replace_lines_failure) {
            return Err(WorkspaceEditPanelError::AllocationFailed);
        }
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
            PanelState::Closed
            | PanelState::Waiting(_)
            | PanelState::Preparing(_)
            | PanelState::Publishing(_) => 0,
            PanelState::Rename(input) => input
                .text
                .capacity()
                .saturating_add(input.composition.as_ref().map_or(0, String::capacity)),
            PanelState::Preview { prepared, .. } | PanelState::Queued { prepared, .. } => {
                prepared.retained_bytes()
            }
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
        lines.push(preview_path_line(&path)?);
    }
    lines.push(Box::from("Enter applies atomically; Escape closes."));
    Ok(lines)
}

fn preview_path_line(path: &str) -> Result<Box<str>, WorkspaceEditPanelError> {
    let mut line = String::new();
    let admitted = path.len().min(MAX_PREVIEW_LINE_BYTES.saturating_sub(4));
    let admitted = floor_char_boundary(path, admitted);
    line.try_reserve(admitted.saturating_add(4))
        .map_err(|_| WorkspaceEditPanelError::AllocationFailed)?;
    line.push_str("- ");
    line.push_str(&path[..admitted]);
    if admitted != path.len() {
        line.push_str("..");
    }
    Ok(line.into_boxed_str())
}

fn floor_char_boundary(value: &str, index: usize) -> usize {
    (0..=index.min(value.len()))
        .rev()
        .find(|index| value.is_char_boundary(*index))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::value::RawValue;

    use super::*;
    use crate::{
        rust_diagnostics::{
            RustDiagnostics, RustDocumentInput,
            tests::{diagnostics, fixture, mock_executable},
        },
        rust_navigation::local_file_uri,
        rust_workspace_edit::WorkspaceEditProposal,
    };

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

    #[test]
    fn publication_queue_is_identity_bound_non_cancellable_and_retryable()
    -> Result<(), Box<dyn Error>> {
        let (root, path, snapshot, language_identity) = fixture();
        let mut diagnostics_model = RustDiagnostics::default();
        diagnostics_model
            .install_for_test(
                RustDocumentInput::new(&path, &root, language_identity, snapshot),
                &diagnostics(&path, 1),
                mock_executable(),
            )
            .unwrap_or_else(|_| unreachable!());
        let language = diagnostics_model.snapshot();
        let identity = WorkspaceEditIdentity::for_test(
            language_identity,
            language.process_epoch,
            language.lsp_version,
            7,
            WorkspaceEditKind::Rename,
        );
        let uri = local_file_uri(&path);
        let raw = RawValue::from_string(
            serde_json::json!({
                "changes": {
                    (uri): [{
                        "range": {
                            "start": {"line": 0, "character": 0},
                            "end": {"line": 0, "character": 0}
                        },
                        "newText": "renamed_"
                    }]
                }
            })
            .to_string(),
        )
        .unwrap_or_else(|_| unreachable!());
        let prepared = WorkspaceEditProposal::admit_rename(&raw, &root)?.prepare()?;
        let mut panel = WorkspaceEditPanel {
            state: PanelState::Preview { identity, prepared },
            ..WorkspaceEditPanel::default()
        };
        assert!(panel.queue_publication(language_identity, &language)?);
        assert!(panel.is_publication_pending());
        assert!(!panel.cancel());
        let (queued_identity, prepared) = panel.take_queued_publication().ok_or("queued")?;
        assert_eq!(queued_identity, identity);
        assert!(panel.publication_matches(identity, language_identity));
        assert!(!panel.cancel());
        assert!(panel.publication_failed(identity, prepared)?);
        assert!(!panel.is_publication_pending());
        assert!(panel.preview().is_some());
        assert!(panel.queue_publication(language_identity, &language)?);
        let (_, prepared) = panel.take_queued_publication().ok_or("queued retry")?;
        drop(prepared);
        assert!(panel.publication_succeeded(identity));
        assert!(!panel.is_open());
        assert_eq!(panel.retained_bytes(), 0);
        assert!(!diagnostics_model.shutdown().active);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn invalid_panel_transitions_are_atomic_and_bounded() -> Result<(), Box<dyn Error>> {
        assert_eq!(
            WorkspaceEditPanelError::InvalidName.to_string(),
            "Rust workspace edit unavailable: InvalidName"
        );
        assert_eq!(WorkspaceEditKind::Rename.label(), "Rust rename");
        assert_eq!(WorkspaceEditKind::Formatting.label(), "Rust formatting");
        let mut panel = WorkspaceEditPanel::default();
        assert!(!panel.is_open());
        assert!(!panel.begin_composition());
        assert!(!panel.cancel_composition());
        assert!(!panel.delete_backward()?);
        assert!(!panel.commit_text("ignored")?);
        assert!(!panel.update_composition("ignored", 0, 0)?);
        assert_eq!(
            panel.take_rename_for_request(),
            Err(WorkspaceEditPanelError::InvalidName)
        );
        assert!(!panel.cancel());
        assert!(panel.line(0).is_none());
        assert!(panel.accessibility_label().is_none());
        assert_eq!(
            checked_input_length(usize::MAX, 1),
            Err(WorkspaceEditPanelError::InputTooLong)
        );
        assert_eq!(
            validate_composition("x", u32::MAX, 1),
            Err(WorkspaceEditPanelError::InvalidComposition)
        );
        assert_eq!(
            validate_composition("x", 0, 2),
            Err(WorkspaceEditPanelError::InvalidComposition)
        );

        assert!(panel.open_rename()?);
        assert_eq!(
            panel.take_rename_for_request(),
            Err(WorkspaceEditPanelError::InvalidName)
        );
        assert!(!panel.delete_backward()?);
        assert!(!panel.cancel_composition());
        assert!(!panel.commit_text("")?);
        assert!(panel.begin_composition());
        assert!(!panel.begin_composition());
        assert!(panel.update_composition("name", 1, 2)?);
        assert!(!panel.update_composition("name", 1, 2)?);
        assert_eq!(
            panel.update_composition("bad\nname", 0, 0),
            Err(WorkspaceEditPanelError::InvalidName)
        );
        assert_eq!(
            panel.take_rename_for_request(),
            Err(WorkspaceEditPanelError::InvalidName)
        );
        assert!(panel.cancel_composition());
        assert!(panel.commit_text("name")?);
        assert!(panel.accessibility_label().is_some());
        assert!(panel.cancel());
        assert!(!panel.is_open());

        let short_lines =
            std::iter::repeat_n(Box::<str>::from("x"), MAX_PREVIEW_LINES).collect::<Vec<_>>();
        assert!(panel.replace_lines(short_lines, Arc::from("exact")).is_ok());
        assert!(
            panel
                .replace_lines(
                    vec!["x".repeat(MAX_PREVIEW_LINE_BYTES).into_boxed_str()],
                    Arc::from("exact line"),
                )
                .is_ok()
        );
        let too_many =
            std::iter::repeat_n(Box::<str>::from("x"), MAX_PREVIEW_LINES + 1).collect::<Vec<_>>();
        assert_eq!(
            panel.replace_lines(too_many, Arc::from("too many")),
            Err(WorkspaceEditPanelError::AllocationFailed)
        );
        assert_eq!(
            panel.replace_lines(
                vec!["x".repeat(MAX_PREVIEW_LINE_BYTES + 1).into_boxed_str()],
                Arc::from("too long"),
            ),
            Err(WorkspaceEditPanelError::AllocationFailed)
        );
        assert_eq!(floor_char_boundary("aé", 3), 3);
        assert_eq!(floor_char_boundary("aé", 2), 1);
        assert_eq!(floor_char_boundary("🙂", 1), 0);
        assert_eq!(floor_char_boundary("🙂", 2), 0);
        assert_eq!(floor_char_boundary("🙂", 3), 0);
        panel.force_replace_lines_failure_once();
        assert_eq!(
            panel.replace_lines(vec![Box::from("retry")], Arc::from("retry")),
            Err(WorkspaceEditPanelError::AllocationFailed)
        );
        assert!(
            panel
                .replace_lines(vec![Box::from("retry")], Arc::from("retry"))
                .is_ok()
        );
        Ok(())
    }

    #[test]
    fn preview_path_uses_ellipsis_only_when_bytes_are_omitted() -> Result<(), Box<dyn Error>> {
        let (root, path, _, _) = fixture();
        let uri = local_file_uri(&path);
        let raw = RawValue::from_string(
            serde_json::json!({
                "changes": {
                    (uri): [{
                        "range": {
                            "start": {"line": 0, "character": 0},
                            "end": {"line": 0, "character": 0}
                        },
                        "newText": "preview_"
                    }]
                }
            })
            .to_string(),
        )
        .unwrap_or_else(|_| unreachable!());
        let prepared = WorkspaceEditProposal::admit_rename(&raw, &root)?.prepare()?;
        let lines = preview_lines(WorkspaceEditKind::Rename, &prepared)?;
        assert_eq!(lines[1].as_ref(), format!("- {}", path.to_string_lossy()));
        let long = format!("{}tail", "🙂".repeat(MAX_PREVIEW_LINE_BYTES));
        let shortened = preview_path_line(&long)?;
        assert!(shortened.ends_with(".."));
        assert!(shortened.len() <= MAX_PREVIEW_LINE_BYTES);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "one ordered panel state-machine journey keeps each authority transition visible"
    )]
    fn preparation_empty_stale_and_terminal_states_fail_closed() -> Result<(), Box<dyn Error>> {
        let (root, path, snapshot, language_identity) = fixture();
        let mut diagnostics_model = RustDiagnostics::default();
        diagnostics_model
            .install_for_test(
                RustDocumentInput::new(&path, &root, language_identity, snapshot),
                &diagnostics(&path, 1),
                mock_executable(),
            )
            .unwrap_or_else(|_| unreachable!());
        let language = diagnostics_model.snapshot();
        let identity = WorkspaceEditIdentity::for_test(
            language_identity,
            language.process_epoch,
            language.lsp_version,
            17,
            WorkspaceEditKind::Formatting,
        );
        let other = WorkspaceEditIdentity::for_test(
            language_identity,
            language.process_epoch,
            language.lsp_version,
            18,
            WorkspaceEditKind::Rename,
        );
        let uri = local_file_uri(&path);
        let raw = RawValue::from_string(
            serde_json::json!({
                "changes": {
                    (uri): [{
                        "range": {
                            "start": {"line": 0, "character": 0},
                            "end": {"line": 0, "character": 0}
                        },
                        "newText": "formatted_"
                    }]
                }
            })
            .to_string(),
        )
        .unwrap_or_else(|_| unreachable!());
        let prepared = WorkspaceEditProposal::admit_rename(&raw, &root)?.prepare()?;
        let empty = prepared.publication_fixture_for_test(0, true);
        let mut panel = WorkspaceEditPanel::default();
        assert!(panel.rebuild_rename_lines().is_ok());

        assert!(!panel.preparation_started(identity)?);
        assert!(!panel.preparation_failed(identity));
        assert!(!panel.queue_publication(language_identity, &language)?);
        assert!(panel.take_queued_publication().is_none());
        assert!(!panel.publication_matches(identity, language_identity));
        assert!(!panel.publication_failed(identity, prepared.clone())?);
        assert!(!panel.publication_succeeded(identity));

        panel.wait(WorkspaceEditKind::Formatting)?;
        panel.preparation_started(identity)?;
        let mut stale_identity = language_identity;
        stale_identity.selection_revision += 1;
        assert_eq!(
            panel.complete(
                WorkspaceEditPreparationOutput {
                    identity,
                    wire_bytes: 0,
                    result: Ok(prepared.clone()),
                },
                stale_identity,
                &language,
            ),
            Err(WorkspaceEditPanelError::StaleResponse)
        );

        panel.wait(WorkspaceEditKind::Formatting)?;
        assert!(!panel.preparation_started(other)?);
        assert!(panel.preparation_started(identity)?);
        let ignored = panel
            .complete(
                WorkspaceEditPreparationOutput {
                    identity: other,
                    wire_bytes: 0,
                    result: Ok(prepared.clone()),
                },
                language_identity,
                &language,
            )
            .unwrap_or_else(|_| unreachable!());
        assert_eq!(ignored, WorkspaceEditPanelOutcome::Ignored);
        assert!(panel.preparation_failed(identity));

        panel.wait(WorkspaceEditKind::Formatting)?;
        panel.preparation_started(identity)?;
        assert_eq!(
            panel.complete(
                WorkspaceEditPreparationOutput {
                    identity,
                    wire_bytes: 0,
                    result: Err(WorkspaceEditError::Malformed),
                },
                language_identity,
                &language,
            ),
            Err(WorkspaceEditPanelError::Preparation(
                WorkspaceEditError::Malformed
            ))
        );
        assert!(!panel.is_open());

        panel.wait(WorkspaceEditKind::Formatting)?;
        panel.preparation_started(identity)?;
        let empty_outcome = panel
            .complete(
                WorkspaceEditPreparationOutput {
                    identity,
                    wire_bytes: 0,
                    result: Ok(empty),
                },
                language_identity,
                &language,
            )
            .unwrap_or_else(|_| unreachable!());
        assert_eq!(
            empty_outcome,
            WorkspaceEditPanelOutcome::Empty(WorkspaceEditKind::Formatting)
        );

        panel.wait(WorkspaceEditKind::Formatting)?;
        panel.preparation_started(identity)?;
        let preview_outcome = panel
            .complete(
                WorkspaceEditPreparationOutput {
                    identity,
                    wire_bytes: 23,
                    result: Ok(prepared.clone()),
                },
                language_identity,
                &language,
            )
            .unwrap_or_else(|_| unreachable!());
        assert_eq!(
            preview_outcome,
            WorkspaceEditPanelOutcome::Preview {
                kind: WorkspaceEditKind::Formatting,
                files: 1,
                edits: 1,
            }
        );
        assert_eq!(panel.line_count(), 3);
        assert!(panel.line(0).is_some_and(|line| line.contains("preview")));
        let stopped = diagnostics_model.shutdown();
        assert_eq!(
            panel.queue_publication(language_identity, &stopped),
            Err(WorkspaceEditPanelError::StaleResponse)
        );
        assert!(!panel.is_open());
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
