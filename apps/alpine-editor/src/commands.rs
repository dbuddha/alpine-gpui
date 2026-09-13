//! Private bounded command registry and palette state.

use std::{
    error::Error,
    fmt::{self, Write as _},
    mem,
};

pub(crate) const MAX_COMMANDS: usize = 32;
pub(crate) const MAX_QUERY_BYTES: usize = 256;
pub(crate) const MAX_VISIBLE_COMMANDS: usize = 12;
pub(crate) const MAX_VISIBLE_OVERSCAN: usize = 3;
pub(crate) const MAX_DIAGNOSTIC_BYTES: usize = 512;
const _: () = assert!(MAX_QUERY_BYTES + 48 <= MAX_DIAGNOSTIC_BYTES);

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
pub(crate) enum EditorCommand {
    SaveFile,
    CloseTab,
    NavigateBack,
    NavigateForward,
    OpenQuickOpen,
    OpenProjectSearch,
    OpenFind,
    OpenReplace,
    TriggerCompletion,
    ShowRustHover,
    GoToRustDefinition,
    FindRustReferences,
    ShowRustDocumentSymbols,
    ShowRustWorkspaceSymbols,
    ReloadSettings,
    PreviewRustRename,
    PreviewRustFormatting,
    ToggleFileTree,
    SplitRight,
    SplitDown,
    FocusNextPane,
    ClosePane,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent command predicates preserve explicit fail-closed availability"
)]
pub(crate) struct CommandContext {
    pub(crate) can_save: bool,
    pub(crate) can_close_tab: bool,
    pub(crate) can_navigate_back: bool,
    pub(crate) can_navigate_forward: bool,
    pub(crate) has_workspace: bool,
    pub(crate) can_split_right: bool,
    pub(crate) can_split_down: bool,
    pub(crate) can_close_pane: bool,
    pub(crate) can_complete: bool,
}

impl CommandContext {
    const fn available(self, command: EditorCommand) -> bool {
        match command {
            EditorCommand::SaveFile => self.can_save,
            EditorCommand::CloseTab => self.can_close_tab,
            EditorCommand::NavigateBack => self.can_navigate_back,
            EditorCommand::NavigateForward => self.can_navigate_forward,
            EditorCommand::OpenQuickOpen
            | EditorCommand::OpenProjectSearch
            | EditorCommand::ToggleFileTree => self.has_workspace,
            EditorCommand::OpenFind
            | EditorCommand::OpenReplace
            | EditorCommand::ReloadSettings => true,
            EditorCommand::TriggerCompletion
            | EditorCommand::ShowRustHover
            | EditorCommand::GoToRustDefinition
            | EditorCommand::FindRustReferences
            | EditorCommand::ShowRustDocumentSymbols
            | EditorCommand::ShowRustWorkspaceSymbols
            | EditorCommand::PreviewRustRename
            | EditorCommand::PreviewRustFormatting => self.can_complete,
            EditorCommand::SplitRight => self.can_split_right,
            EditorCommand::SplitDown => self.can_split_down,
            EditorCommand::FocusNextPane | EditorCommand::ClosePane => self.can_close_pane,
        }
    }
}

#[derive(Clone, Copy)]
struct CommandSpec {
    command: EditorCommand,
    title: &'static str,
    search_terms: &'static str,
}

const REGISTRY: [CommandSpec; 22] = [
    CommandSpec {
        command: EditorCommand::SaveFile,
        title: "File: Save",
        search_terms: "write persist",
    },
    CommandSpec {
        command: EditorCommand::CloseTab,
        title: "File: Close Tab",
        search_terms: "document editor",
    },
    CommandSpec {
        command: EditorCommand::NavigateBack,
        title: "Navigation: Go Back",
        search_terms: "history previous",
    },
    CommandSpec {
        command: EditorCommand::NavigateForward,
        title: "Navigation: Go Forward",
        search_terms: "history next",
    },
    CommandSpec {
        command: EditorCommand::OpenQuickOpen,
        title: "Workspace: Quick Open",
        search_terms: "file fuzzy path",
    },
    CommandSpec {
        command: EditorCommand::OpenProjectSearch,
        title: "Workspace: Project Search",
        search_terms: "search content folder",
    },
    CommandSpec {
        command: EditorCommand::OpenFind,
        title: "Editor: Find",
        search_terms: "search document",
    },
    CommandSpec {
        command: EditorCommand::OpenReplace,
        title: "Editor: Find and Replace",
        search_terms: "search document change",
    },
    CommandSpec {
        command: EditorCommand::TriggerCompletion,
        title: "Editor: Trigger Rust Completion",
        search_terms: "language rust analyzer suggest",
    },
    CommandSpec {
        command: EditorCommand::ShowRustHover,
        title: "Navigation: Show Rust Hover",
        search_terms: "language rust analyzer documentation type",
    },
    CommandSpec {
        command: EditorCommand::GoToRustDefinition,
        title: "Navigation: Go to Rust Definition",
        search_terms: "language rust analyzer source jump",
    },
    CommandSpec {
        command: EditorCommand::FindRustReferences,
        title: "Navigation: Find Rust References",
        search_terms: "language rust analyzer usages source",
    },
    CommandSpec {
        command: EditorCommand::ShowRustDocumentSymbols,
        title: "Navigation: Rust Document Symbols",
        search_terms: "language rust analyzer outline functions types",
    },
    CommandSpec {
        command: EditorCommand::ShowRustWorkspaceSymbols,
        title: "Navigation: Rust Workspace Symbols",
        search_terms: "language rust analyzer project functions types",
    },
    CommandSpec {
        command: EditorCommand::ReloadSettings,
        title: "Preferences: Reload Settings",
        search_terms: "configuration theme keymap refresh",
    },
    CommandSpec {
        command: EditorCommand::PreviewRustRename,
        title: "Editor: Preview Rust Rename",
        search_terms: "language rust analyzer symbol refactor",
    },
    CommandSpec {
        command: EditorCommand::PreviewRustFormatting,
        title: "Editor: Preview Rust Formatting",
        search_terms: "language rust analyzer format document",
    },
    CommandSpec {
        command: EditorCommand::ToggleFileTree,
        title: "Workspace: Toggle File Tree",
        search_terms: "sidebar explorer files",
    },
    CommandSpec {
        command: EditorCommand::SplitRight,
        title: "Pane: Split Right",
        search_terms: "editor column view",
    },
    CommandSpec {
        command: EditorCommand::SplitDown,
        title: "Pane: Split Down",
        search_terms: "editor row view",
    },
    CommandSpec {
        command: EditorCommand::FocusNextPane,
        title: "Pane: Focus Next",
        search_terms: "editor navigate view",
    },
    CommandSpec {
        command: EditorCommand::ClosePane,
        title: "Pane: Close",
        search_terms: "editor remove view",
    },
];

const _: () = assert!(REGISTRY.len() <= MAX_COMMANDS);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CommandMatch {
    registry_index: u8,
    rank: u8,
    gaps: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CommandRow {
    pub(crate) command: EditorCommand,
    pub(crate) title: &'static str,
    pub(crate) selected: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[allow(
    dead_code,
    reason = "the opt-in local diagnostic overlay will consume this tested snapshot"
)]
pub(crate) struct CommandPaletteReport {
    pub(crate) query_bytes: usize,
    pub(crate) composition_bytes: usize,
    pub(crate) retained_matches: usize,
    pub(crate) retained_bytes: usize,
    pub(crate) peak_retained_bytes: usize,
    pub(crate) visible_rows: usize,
    pub(crate) executions: u64,
    pub(crate) cancellations: u64,
    pub(crate) truncations: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CommandPaletteError {
    QueryTooLong { actual: usize, limit: usize },
    AllocationFailed,
    MissingSelection,
    Unavailable(EditorCommand),
    InvalidComposition,
}

impl fmt::Display for CommandPaletteError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::QueryTooLong { actual, limit } => {
                write!(
                    formatter,
                    "command query is {actual} bytes; limit is {limit}"
                )
            }
            Self::AllocationFailed => formatter.write_str("command palette allocation failed"),
            Self::MissingSelection => formatter.write_str("command palette has no selection"),
            Self::Unavailable(command) => write!(formatter, "command is unavailable: {command:?}"),
            Self::InvalidComposition => {
                formatter.write_str("command palette composition range is invalid")
            }
        }
    }
}

impl Error for CommandPaletteError {}

#[derive(Default)]
pub(crate) struct CommandPalette {
    open: bool,
    query: String,
    pub(crate) edit: super::field_edit::FieldEdit,
    matches: Vec<CommandMatch>,
    selected: usize,
    first_visible: usize,
    peak_retained_bytes: usize,
    executions: u64,
    cancellations: u64,
    #[allow(
        dead_code,
        reason = "reserved for bounded diagnostic truncation accounting"
    )]
    truncations: u64,
    #[cfg(test)]
    fail_next_open: bool,
    #[cfg(test)]
    fail_next_query_update: bool,
}

impl From<super::field_edit::EditError> for CommandPaletteError {
    fn from(error: super::field_edit::EditError) -> Self {
        match error {
            super::field_edit::EditError::InvalidSelection => Self::InvalidComposition,
            super::field_edit::EditError::TooLong { actual, limit } => {
                Self::QueryTooLong { actual, limit }
            }
            super::field_edit::EditError::AllocationFailed => Self::AllocationFailed,
        }
    }
}

impl CommandPalette {
    pub(crate) const fn is_open(&self) -> bool {
        self.open
    }

    pub(crate) fn open(&mut self, context: CommandContext) -> Result<bool, CommandPaletteError> {
        #[cfg(test)]
        if mem::take(&mut self.fail_next_open) {
            return Err(CommandPaletteError::AllocationFailed);
        }
        if self.open {
            return Ok(false);
        }
        let matches = rank_matches("", context)?;
        self.open = true;
        self.query = String::new();
        self.edit = super::field_edit::FieldEdit::default();
        self.matches = matches;
        self.selected = 0;
        self.first_visible = 0;
        self.observe_peak();
        Ok(true)
    }

    pub(crate) fn cancel(&mut self) -> bool {
        if !self.open {
            return false;
        }
        self.cancellations = self.cancellations.saturating_add(1);
        self.release();
        true
    }

    pub(crate) fn query(&self) -> &str {
        &self.query
    }

    pub(crate) fn edit_parts(&mut self) -> (&str, &mut super::field_edit::FieldEdit) {
        (&self.query, &mut self.edit)
    }

    pub(crate) fn begin_composition(&mut self) -> bool {
        self.open && self.edit.begin_composition()
    }

    pub(crate) fn update_composition(
        &mut self,
        text: &str,
        start: u32,
        length: u32,
    ) -> Result<bool, CommandPaletteError> {
        let changed =
            self.edit
                .update_composition(&self.query, text, start, length, MAX_QUERY_BYTES)?;
        self.observe_peak();
        Ok(changed)
    }

    pub(crate) fn cancel_composition(&mut self) -> bool {
        self.edit.cancel_composition()
    }

    pub(crate) fn commit_text(
        &mut self,
        text: &str,
        context: CommandContext,
    ) -> Result<bool, CommandPaletteError> {
        self.commit_text_at(text, text.len(), context)
    }

    pub(crate) fn commit_text_at(
        &mut self,
        text: &str,
        caret: usize,
        context: CommandContext,
    ) -> Result<bool, CommandPaletteError> {
        if !self.open {
            return Ok(false);
        }
        let prepared = self
            .edit
            .prepare(&self.query, text, caret, MAX_QUERY_BYTES)?;
        self.apply_edit(prepared, context)
    }

    pub(crate) fn delete_backward(
        &mut self,
        context: CommandContext,
    ) -> Result<bool, CommandPaletteError> {
        self.delete(false, context)
    }

    pub(crate) fn delete(
        &mut self,
        forward: bool,
        context: CommandContext,
    ) -> Result<bool, CommandPaletteError> {
        let prepared = self
            .edit
            .prepare_delete(&self.query, forward, MAX_QUERY_BYTES)?;
        self.apply_edit(prepared, context)
    }

    pub(crate) fn apply_edit(
        &mut self,
        mut prepared: super::field_edit::Prepared,
        context: CommandContext,
    ) -> Result<bool, CommandPaletteError> {
        let changed = self.query != prepared.value;
        if changed {
            self.replace_query(std::mem::take(&mut prepared.value), context)?;
        }
        self.edit.accept(prepared, changed);
        self.observe_peak();
        Ok(changed)
    }

    pub(crate) fn refresh(&mut self, context: CommandContext) -> Result<bool, CommandPaletteError> {
        if !self.open {
            return Ok(false);
        }
        let matches = rank_matches(&self.query, context)?;
        let changed = matches != self.matches;
        self.matches = matches;
        self.selected = self.selected.min(self.matches.len().saturating_sub(1));
        self.first_visible = self.first_visible.min(self.selected);
        self.observe_peak();
        Ok(changed)
    }

    pub(crate) fn navigate(&mut self, forward: bool) -> bool {
        if self.matches.is_empty() {
            return false;
        }
        let previous = self.selected;
        self.selected = if forward {
            (self.selected + 1) % self.matches.len()
        } else if self.selected == 0 {
            self.matches.len() - 1
        } else {
            self.selected - 1
        };
        if self.selected >= self.first_visible.saturating_add(MAX_VISIBLE_COMMANDS) {
            self.first_visible = self
                .selected
                .saturating_add(1)
                .saturating_sub(MAX_VISIBLE_COMMANDS);
        }
        self.first_visible = self.first_visible.min(self.selected);
        self.selected != previous
    }

    pub(crate) fn execute_selected(
        &mut self,
        context: CommandContext,
    ) -> Result<EditorCommand, CommandPaletteError> {
        let selected = *self
            .matches
            .get(self.selected)
            .ok_or(CommandPaletteError::MissingSelection)?;
        let spec = REGISTRY
            .get(usize::from(selected.registry_index))
            .ok_or(CommandPaletteError::MissingSelection)?;
        self.execute(spec.command, context)
    }

    pub(crate) fn execute(
        &mut self,
        command: EditorCommand,
        context: CommandContext,
    ) -> Result<EditorCommand, CommandPaletteError> {
        if !context.available(command) {
            let _ = self.refresh(context)?;
            return Err(CommandPaletteError::Unavailable(command));
        }
        self.executions = self.executions.saturating_add(1);
        self.release();
        Ok(command)
    }

    pub(crate) fn visible_commands(&self) -> Result<Vec<CommandRow>, CommandPaletteError> {
        let start = self.first_visible.saturating_sub(MAX_VISIBLE_OVERSCAN);
        let end = self
            .first_visible
            .saturating_add(MAX_VISIBLE_COMMANDS)
            .saturating_add(MAX_VISIBLE_OVERSCAN)
            .min(self.matches.len())
            .min(start.saturating_add(
                MAX_VISIBLE_COMMANDS.saturating_add(MAX_VISIBLE_OVERSCAN.saturating_mul(2)),
            ));
        let mut rows = Vec::new();
        rows.try_reserve(end.saturating_sub(start))
            .map_err(|_| CommandPaletteError::AllocationFailed)?;
        for (offset, matched) in self.matches[start..end].iter().enumerate() {
            let spec = REGISTRY
                .get(usize::from(matched.registry_index))
                .ok_or(CommandPaletteError::MissingSelection)?;
            rows.push(CommandRow {
                command: spec.command,
                title: spec.title,
                selected: start.saturating_add(offset) == self.selected,
            });
        }
        Ok(rows)
    }

    pub(crate) fn display_text(&self) -> Result<String, CommandPaletteError> {
        let projected = self.edit.projected_value(&self.query)?;
        let mut display = String::new();
        let required = self
            .query
            .len()
            .saturating_add(projected.len())
            .saturating_add(48);
        display
            .try_reserve(required)
            .map_err(|_| CommandPaletteError::AllocationFailed)?;
        write!(display, "> {} | {} commands", projected, self.matches.len())
            .map_err(|_| CommandPaletteError::AllocationFailed)?;
        Ok(display)
    }

    #[allow(
        dead_code,
        reason = "the opt-in local diagnostic overlay will consume this tested snapshot"
    )]
    pub(crate) fn report(&self) -> CommandPaletteReport {
        let composition_bytes = self.edit.composition().map_or(0, str::len);
        CommandPaletteReport {
            query_bytes: self.query.len(),
            composition_bytes,
            retained_matches: self.matches.len(),
            retained_bytes: self.retained_bytes(),
            peak_retained_bytes: self.peak_retained_bytes,
            visible_rows: self
                .matches
                .len()
                .min(MAX_VISIBLE_COMMANDS + MAX_VISIBLE_OVERSCAN * 2),
            executions: self.executions,
            cancellations: self.cancellations,
            truncations: self.truncations,
        }
    }

    #[cfg(test)]
    pub(crate) fn fail_next_open(&mut self) {
        self.fail_next_open = true;
    }

    #[cfg(test)]
    pub(crate) fn fail_next_query_update(&mut self) {
        self.fail_next_query_update = true;
    }

    fn replace_query(
        &mut self,
        query: String,
        context: CommandContext,
    ) -> Result<bool, CommandPaletteError> {
        #[cfg(test)]
        if mem::take(&mut self.fail_next_query_update) {
            return Err(CommandPaletteError::AllocationFailed);
        }
        let matches = rank_matches(&query, context)?;
        let changed = query != self.query;
        self.query = query;
        self.matches = matches;
        self.selected = 0;
        self.first_visible = 0;
        self.observe_peak();
        Ok(changed)
    }

    fn release(&mut self) {
        self.open = false;
        self.query = String::new();
        self.edit = super::field_edit::FieldEdit::default();
        self.matches = Vec::new();
        self.selected = 0;
        self.first_visible = 0;
    }

    fn retained_bytes(&self) -> usize {
        self.query
            .capacity()
            .saturating_add(self.edit.retained_bytes())
            .saturating_add(
                self.matches
                    .capacity()
                    .saturating_mul(mem::size_of::<CommandMatch>()),
            )
    }

    fn observe_peak(&mut self) {
        self.peak_retained_bytes = self.peak_retained_bytes.max(self.retained_bytes());
    }
}

fn rank_matches(
    query: &str,
    context: CommandContext,
) -> Result<Vec<CommandMatch>, CommandPaletteError> {
    let query = query.trim();
    let mut matches = Vec::new();
    matches
        .try_reserve_exact(REGISTRY.len())
        .map_err(|_| CommandPaletteError::AllocationFailed)?;
    for (index, spec) in REGISTRY.iter().enumerate() {
        if !context.available(spec.command) {
            continue;
        }
        let Some((rank, gaps)) = match_score(spec, query) else {
            continue;
        };
        matches.push(CommandMatch {
            registry_index: u8::try_from(index)
                .map_err(|_| CommandPaletteError::AllocationFailed)?,
            rank,
            gaps,
        });
    }
    matches.sort_unstable_by_key(|matched| (matched.rank, matched.gaps, matched.registry_index));
    Ok(matches)
}

fn match_score(spec: &CommandSpec, query: &str) -> Option<(u8, u16)> {
    if query.is_empty() {
        return Some((0, 0));
    }
    if ascii_prefix(spec.title, query) || ascii_prefix(spec.search_terms, query) {
        return Some((0, 0));
    }
    if spec
        .title
        .split(|character: char| character.is_ascii_whitespace() || character == ':')
        .chain(spec.search_terms.split_ascii_whitespace())
        .any(|token| ascii_prefix(token, query))
    {
        return Some((1, 0));
    }
    subsequence_gaps(spec.title, query)
        .or_else(|| subsequence_gaps(spec.search_terms, query))
        .map(|gaps| (2, gaps))
}

fn ascii_prefix(value: &str, query: &str) -> bool {
    value.len() >= query.len()
        && value
            .bytes()
            .zip(query.bytes())
            .all(|(left, right)| left.eq_ignore_ascii_case(&right))
}

fn subsequence_gaps(value: &str, query: &str) -> Option<u16> {
    let mut query = query.bytes().filter(|byte| !byte.is_ascii_whitespace());
    let mut expected = query.next()?;
    let mut seen = false;
    let mut gaps = 0_u16;
    for byte in value.bytes() {
        if byte.eq_ignore_ascii_case(&expected) {
            seen = true;
            if let Some(next) = query.next() {
                expected = next;
            } else {
                return Some(gaps);
            }
        } else if seen {
            gaps = gaps.saturating_add(1);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_available() -> CommandContext {
        CommandContext {
            can_save: true,
            can_close_tab: true,
            can_navigate_back: true,
            can_navigate_forward: true,
            has_workspace: true,
            can_split_right: true,
            can_split_down: true,
            can_close_pane: true,
            can_complete: true,
        }
    }

    #[test]
    fn no_op_delete_preserves_highlight_and_history_peaks_are_reported()
    -> Result<(), Box<dyn Error>> {
        let mut palette = CommandPalette::default();
        palette.open(all_available())?;
        palette.navigate(true);
        let selected = palette.selected;
        assert!(!palette.delete_backward(all_available())?);
        assert_eq!(palette.selected, selected);
        palette.begin_composition();
        palette.update_composition("save", 4, 0)?;
        assert!(palette.report().peak_retained_bytes >= palette.report().retained_bytes);
        palette.commit_text("save", all_available())?;
        assert!(palette.report().peak_retained_bytes >= palette.report().retained_bytes);
        Ok(())
    }

    #[test]
    fn locked_registry_query_and_memory_limits_are_exact() -> Result<(), Box<dyn Error>> {
        assert_eq!(REGISTRY.len(), 22);
        assert!(REGISTRY.len() <= MAX_COMMANDS);
        let mut palette = CommandPalette::default();
        assert!(palette.open(all_available())?);
        assert!(!palette.open(all_available())?);
        assert_eq!(palette.report().retained_matches, REGISTRY.len());
        let accepted = "a".repeat(MAX_QUERY_BYTES);
        assert!(palette.commit_text(&accepted, all_available())?);
        assert_eq!(palette.report().query_bytes, MAX_QUERY_BYTES);
        assert!(matches!(
            palette.commit_text("b", all_available()),
            Err(CommandPaletteError::QueryTooLong { actual, limit })
                if actual == MAX_QUERY_BYTES + 1 && limit == MAX_QUERY_BYTES
        ));
        assert_eq!(palette.report().query_bytes, MAX_QUERY_BYTES);
        assert!(palette.cancel());
        let report = palette.report();
        assert_eq!(report.query_bytes, 0);
        assert_eq!(report.composition_bytes, 0);
        assert_eq!(report.retained_matches, 0);
        assert_eq!(report.retained_bytes, 0);
        assert!(report.peak_retained_bytes > 0);
        assert_eq!(report.cancellations, 1);
        Ok(())
    }

    #[test]
    fn exact_token_and_subsequence_ranking_are_stable() -> Result<(), Box<dyn Error>> {
        let mut palette = CommandPalette::default();
        palette.open(all_available())?;
        palette.commit_text("file", all_available())?;
        let file_rows = palette.visible_commands()?;
        assert_eq!(file_rows[0].command, EditorCommand::SaveFile);
        assert_eq!(file_rows[1].command, EditorCommand::CloseTab);
        palette.cancel();

        palette.open(all_available())?;
        palette.commit_text("quick", all_available())?;
        assert_eq!(
            palette.visible_commands()?[0].command,
            EditorCommand::OpenQuickOpen
        );
        palette.cancel();

        palette.open(all_available())?;
        palette.commit_text("wqop", all_available())?;
        assert_eq!(
            palette.visible_commands()?[0].command,
            EditorCommand::OpenQuickOpen
        );
        Ok(())
    }

    #[test]
    fn navigation_composition_and_stale_availability_fail_closed() -> Result<(), Box<dyn Error>> {
        let mut palette = CommandPalette::default();
        palette.open(all_available())?;
        assert!(palette.navigate(false));
        assert_eq!(
            palette
                .visible_commands()?
                .iter()
                .find(|row| row.selected)
                .map(|row| row.command),
            Some(EditorCommand::ClosePane)
        );
        assert!(palette.begin_composition());
        assert!(palette.update_composition("save", 0, 4)?);
        assert!(matches!(
            palette.update_composition("save", 5, 0),
            Err(CommandPaletteError::InvalidComposition)
        ));
        assert!(!palette.cancel_composition());
        palette.cancel();

        let save_only = CommandContext {
            can_save: true,
            can_complete: false,
            ..CommandContext::default()
        };
        palette.open(save_only)?;
        assert_eq!(palette.visible_commands()?.len(), 4);
        let unavailable = CommandContext::default();
        assert!(matches!(
            palette.execute_selected(unavailable),
            Err(CommandPaletteError::Unavailable(EditorCommand::SaveFile))
        ));
        assert!(palette.is_open());
        assert_eq!(palette.report().retained_matches, 3);
        Ok(())
    }

    #[test]
    fn unicode_delete_display_and_execution_accounting_are_bounded() -> Result<(), Box<dyn Error>> {
        let mut palette = CommandPalette::default();
        palette.open(all_available())?;
        palette.commit_text("findé", all_available())?;
        assert!(palette.delete_backward(all_available())?);
        assert!(palette.display_text()?.starts_with("> find"));
        assert_eq!(
            palette.visible_commands()?[0].command,
            EditorCommand::OpenFind
        );
        assert_eq!(
            palette.execute_selected(all_available())?,
            EditorCommand::OpenFind
        );
        let report = palette.report();
        assert_eq!(report.executions, 1);
        assert_eq!(report.retained_bytes, 0);
        assert!(!palette.cancel());
        assert!(
            CommandPaletteError::MissingSelection
                .to_string()
                .contains("no selection")
        );
        Ok(())
    }

    #[test]
    fn randomized_query_navigation_sequences_preserve_every_bound() -> Result<(), Box<dyn Error>> {
        let mut palette = CommandPalette::default();
        palette.open(all_available())?;
        let mut state = 0x6a09_e667_f3bc_c909_u64;
        let steps = if cfg!(miri) { 256 } else { 4_096 };
        for _ in 0..steps {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            match state % 5 {
                0 => {
                    let _ = palette.commit_text("a", all_available());
                }
                1 => {
                    let _ = palette.delete_backward(all_available())?;
                }
                2 => {
                    let _ = palette.navigate(true);
                }
                3 => {
                    let _ = palette.navigate(false);
                }
                _ => {
                    let _ = palette.refresh(all_available())?;
                }
            }
            let report = palette.report();
            assert!(report.query_bytes <= MAX_QUERY_BYTES);
            assert!(report.retained_matches <= MAX_COMMANDS);
            assert!(report.visible_rows <= MAX_VISIBLE_COMMANDS + MAX_VISIBLE_OVERSCAN * 2);
            assert!(
                palette.visible_commands()?.len()
                    <= MAX_VISIBLE_COMMANDS + MAX_VISIBLE_OVERSCAN * 2
            );
        }
        palette.cancel();
        assert_eq!(palette.report().retained_bytes, 0);
        Ok(())
    }

    #[test]
    fn defensive_noops_errors_and_scroll_window_are_discriminating() -> Result<(), Box<dyn Error>> {
        let errors = [
            CommandPaletteError::QueryTooLong {
                actual: MAX_QUERY_BYTES + 1,
                limit: MAX_QUERY_BYTES,
            },
            CommandPaletteError::AllocationFailed,
            CommandPaletteError::MissingSelection,
            CommandPaletteError::Unavailable(EditorCommand::SaveFile),
            CommandPaletteError::InvalidComposition,
        ];
        for error in errors {
            assert!(!error.to_string().is_empty());
            assert!(Error::source(&error).is_none());
        }

        let mut palette = CommandPalette::default();
        assert!(!palette.begin_composition());
        assert!(!palette.commit_text("ignored", all_available())?);
        assert!(!palette.refresh(all_available())?);
        palette.fail_next_open();
        assert!(matches!(
            palette.open(all_available()),
            Err(CommandPaletteError::AllocationFailed)
        ));
        assert!(palette.open(all_available())?);
        assert!(palette.begin_composition());
        assert!(!palette.begin_composition());
        assert!(palette.update_composition("same", 0, 4)?);
        assert!(!palette.update_composition("same", 0, 4)?);
        assert!(palette.cancel_composition());
        assert!(!palette.cancel_composition());
        assert!(!palette.commit_text("", all_available())?);

        assert!(palette.refresh(CommandContext::default())?);
        assert!(!palette.refresh(CommandContext::default())?);

        palette.matches = vec![
            CommandMatch {
                registry_index: 0,
                rank: 0,
                gaps: 0,
            };
            14
        ];
        palette.selected = 11;
        palette.first_visible = 0;
        assert!(palette.navigate(true));
        assert_eq!(palette.selected, 12);
        assert_eq!(palette.first_visible, 1);
        palette.matches = vec![
            CommandMatch {
                registry_index: 0,
                rank: 0,
                gaps: 0,
            };
            40
        ];
        palette.selected = 2;
        palette.first_visible = 0;
        assert!(palette.navigate(false));
        assert_eq!(palette.selected, 1);
        assert_eq!(
            palette.report().visible_rows,
            MAX_VISIBLE_COMMANDS + MAX_VISIBLE_OVERSCAN * 2
        );
        palette.matches.clear();
        assert!(!palette.navigate(true));
        Ok(())
    }

    #[test]
    fn ranking_predicates_preserve_prefix_token_and_subsequence_classes() {
        assert!(ascii_prefix("Workspace", "work"));
        assert!(!ascii_prefix("Work", "workspace"));
        assert_eq!(match_score(&REGISTRY[0], "write"), Some((0, 0)));
        assert_eq!(match_score(&REGISTRY[4], "quick"), Some((1, 0)));
        assert_eq!(match_score(&REGISTRY[4], "wqop"), Some((2, 15)));
    }
}
