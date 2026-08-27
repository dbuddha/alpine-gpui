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
pub(crate) enum StudioCommand {
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
    const fn available(self, command: StudioCommand) -> bool {
        match command {
            StudioCommand::SaveFile => self.can_save,
            StudioCommand::CloseTab => self.can_close_tab,
            StudioCommand::NavigateBack => self.can_navigate_back,
            StudioCommand::NavigateForward => self.can_navigate_forward,
            StudioCommand::OpenQuickOpen
            | StudioCommand::OpenProjectSearch
            | StudioCommand::ToggleFileTree => self.has_workspace,
            StudioCommand::OpenFind | StudioCommand::OpenReplace => true,
            StudioCommand::TriggerCompletion
            | StudioCommand::ShowRustHover
            | StudioCommand::GoToRustDefinition
            | StudioCommand::FindRustReferences
            | StudioCommand::ShowRustDocumentSymbols
            | StudioCommand::ShowRustWorkspaceSymbols
            | StudioCommand::PreviewRustRename
            | StudioCommand::PreviewRustFormatting => self.can_complete,
            StudioCommand::SplitRight => self.can_split_right,
            StudioCommand::SplitDown => self.can_split_down,
            StudioCommand::FocusNextPane | StudioCommand::ClosePane => self.can_close_pane,
        }
    }
}

#[derive(Clone, Copy)]
struct CommandSpec {
    command: StudioCommand,
    title: &'static str,
    search_terms: &'static str,
}

const REGISTRY: [CommandSpec; 21] = [
    CommandSpec {
        command: StudioCommand::SaveFile,
        title: "File: Save",
        search_terms: "write persist",
    },
    CommandSpec {
        command: StudioCommand::CloseTab,
        title: "File: Close Tab",
        search_terms: "document editor",
    },
    CommandSpec {
        command: StudioCommand::NavigateBack,
        title: "Navigation: Go Back",
        search_terms: "history previous",
    },
    CommandSpec {
        command: StudioCommand::NavigateForward,
        title: "Navigation: Go Forward",
        search_terms: "history next",
    },
    CommandSpec {
        command: StudioCommand::OpenQuickOpen,
        title: "Workspace: Quick Open",
        search_terms: "file fuzzy path",
    },
    CommandSpec {
        command: StudioCommand::OpenProjectSearch,
        title: "Workspace: Project Search",
        search_terms: "search content folder",
    },
    CommandSpec {
        command: StudioCommand::OpenFind,
        title: "Editor: Find",
        search_terms: "search document",
    },
    CommandSpec {
        command: StudioCommand::OpenReplace,
        title: "Editor: Find and Replace",
        search_terms: "search document change",
    },
    CommandSpec {
        command: StudioCommand::TriggerCompletion,
        title: "Editor: Trigger Rust Completion",
        search_terms: "language rust analyzer suggest",
    },
    CommandSpec {
        command: StudioCommand::ShowRustHover,
        title: "Navigation: Show Rust Hover",
        search_terms: "language rust analyzer documentation type",
    },
    CommandSpec {
        command: StudioCommand::GoToRustDefinition,
        title: "Navigation: Go to Rust Definition",
        search_terms: "language rust analyzer source jump",
    },
    CommandSpec {
        command: StudioCommand::FindRustReferences,
        title: "Navigation: Find Rust References",
        search_terms: "language rust analyzer usages source",
    },
    CommandSpec {
        command: StudioCommand::ShowRustDocumentSymbols,
        title: "Navigation: Rust Document Symbols",
        search_terms: "language rust analyzer outline functions types",
    },
    CommandSpec {
        command: StudioCommand::ShowRustWorkspaceSymbols,
        title: "Navigation: Rust Workspace Symbols",
        search_terms: "language rust analyzer project functions types",
    },
    CommandSpec {
        command: StudioCommand::PreviewRustRename,
        title: "Editor: Preview Rust Rename",
        search_terms: "language rust analyzer symbol refactor",
    },
    CommandSpec {
        command: StudioCommand::PreviewRustFormatting,
        title: "Editor: Preview Rust Formatting",
        search_terms: "language rust analyzer format document",
    },
    CommandSpec {
        command: StudioCommand::ToggleFileTree,
        title: "Workspace: Toggle File Tree",
        search_terms: "sidebar explorer files",
    },
    CommandSpec {
        command: StudioCommand::SplitRight,
        title: "Pane: Split Right",
        search_terms: "editor column view",
    },
    CommandSpec {
        command: StudioCommand::SplitDown,
        title: "Pane: Split Down",
        search_terms: "editor row view",
    },
    CommandSpec {
        command: StudioCommand::FocusNextPane,
        title: "Pane: Focus Next",
        search_terms: "editor navigate view",
    },
    CommandSpec {
        command: StudioCommand::ClosePane,
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
    pub(crate) command: StudioCommand,
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
    Unavailable(StudioCommand),
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
    composition: Option<String>,
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
        self.composition = None;
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

    pub(crate) fn begin_composition(&mut self) -> bool {
        if !self.open || self.composition.is_some() {
            false
        } else {
            self.composition = Some(String::new());
            true
        }
    }

    pub(crate) fn update_composition(
        &mut self,
        text: &str,
        selected_start_utf16: u32,
        selected_length_utf16: u32,
    ) -> Result<bool, CommandPaletteError> {
        let selected_end = selected_start_utf16
            .checked_add(selected_length_utf16)
            .ok_or(CommandPaletteError::InvalidComposition)?;
        let text_units = u32::try_from(text.encode_utf16().count())
            .map_err(|_| CommandPaletteError::InvalidComposition)?;
        if selected_end > text_units {
            return Err(CommandPaletteError::InvalidComposition);
        }
        Self::check_query_length(self.query.len().saturating_add(text.len()))?;
        let changed = self.composition.as_deref() != Some(text);
        if changed {
            let mut composition = String::new();
            composition
                .try_reserve_exact(text.len())
                .map_err(|_| CommandPaletteError::AllocationFailed)?;
            composition.push_str(text);
            self.composition = Some(composition);
            self.observe_peak();
        }
        Ok(changed)
    }

    pub(crate) fn cancel_composition(&mut self) -> bool {
        self.composition.take().is_some()
    }

    pub(crate) fn commit_text(
        &mut self,
        text: &str,
        context: CommandContext,
    ) -> Result<bool, CommandPaletteError> {
        if !self.open {
            return Ok(false);
        }
        let next_length =
            self.query
                .len()
                .checked_add(text.len())
                .ok_or(CommandPaletteError::QueryTooLong {
                    actual: usize::MAX,
                    limit: MAX_QUERY_BYTES,
                })?;
        Self::check_query_length(next_length)?;
        self.composition = None;
        if text.is_empty() {
            return Ok(false);
        }
        let mut query = String::new();
        query
            .try_reserve_exact(next_length)
            .map_err(|_| CommandPaletteError::AllocationFailed)?;
        query.push_str(&self.query);
        query.push_str(text);
        self.replace_query(query, context)
    }

    pub(crate) fn delete_backward(
        &mut self,
        context: CommandContext,
    ) -> Result<bool, CommandPaletteError> {
        self.composition = None;
        let mut query = String::new();
        query
            .try_reserve_exact(self.query.len())
            .map_err(|_| CommandPaletteError::AllocationFailed)?;
        query.push_str(&self.query);
        if query.pop().is_none() {
            return Ok(false);
        }
        self.replace_query(query, context)
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
    ) -> Result<StudioCommand, CommandPaletteError> {
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
        command: StudioCommand,
        context: CommandContext,
    ) -> Result<StudioCommand, CommandPaletteError> {
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
        let composition = self.composition.as_deref().unwrap_or_default();
        let mut display = String::new();
        let required = self
            .query
            .len()
            .saturating_add(composition.len())
            .saturating_add(48);
        display
            .try_reserve(required)
            .map_err(|_| CommandPaletteError::AllocationFailed)?;
        write!(
            display,
            "> {}{} | {} commands",
            self.query,
            composition,
            self.matches.len()
        )
        .map_err(|_| CommandPaletteError::AllocationFailed)?;
        Ok(display)
    }

    #[allow(
        dead_code,
        reason = "the opt-in local diagnostic overlay will consume this tested snapshot"
    )]
    pub(crate) fn report(&self) -> CommandPaletteReport {
        let composition_bytes = self.composition.as_deref().map_or(0, str::len);
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
        self.composition = None;
        self.matches = Vec::new();
        self.selected = 0;
        self.first_visible = 0;
    }

    fn check_query_length(actual: usize) -> Result<(), CommandPaletteError> {
        if actual > MAX_QUERY_BYTES {
            Err(CommandPaletteError::QueryTooLong {
                actual,
                limit: MAX_QUERY_BYTES,
            })
        } else {
            Ok(())
        }
    }

    fn retained_bytes(&self) -> usize {
        self.query
            .capacity()
            .saturating_add(self.composition.as_ref().map_or(0, String::capacity))
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
    fn locked_registry_query_and_memory_limits_are_exact() -> Result<(), Box<dyn Error>> {
        assert_eq!(REGISTRY.len(), 21);
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
        assert_eq!(file_rows[0].command, StudioCommand::SaveFile);
        assert_eq!(file_rows[1].command, StudioCommand::CloseTab);
        palette.cancel();

        palette.open(all_available())?;
        palette.commit_text("quick", all_available())?;
        assert_eq!(
            palette.visible_commands()?[0].command,
            StudioCommand::OpenQuickOpen
        );
        palette.cancel();

        palette.open(all_available())?;
        palette.commit_text("wqop", all_available())?;
        assert_eq!(
            palette.visible_commands()?[0].command,
            StudioCommand::OpenQuickOpen
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
            Some(StudioCommand::ClosePane)
        );
        assert!(palette.begin_composition());
        assert!(palette.update_composition("save", 0, 4)?);
        assert!(matches!(
            palette.update_composition("save", 5, 0),
            Err(CommandPaletteError::InvalidComposition)
        ));
        assert!(palette.cancel_composition());
        palette.cancel();

        let save_only = CommandContext {
            can_save: true,
            can_complete: false,
            ..CommandContext::default()
        };
        palette.open(save_only)?;
        assert_eq!(palette.visible_commands()?.len(), 3);
        let unavailable = CommandContext::default();
        assert!(matches!(
            palette.execute_selected(unavailable),
            Err(CommandPaletteError::Unavailable(StudioCommand::SaveFile))
        ));
        assert!(palette.is_open());
        assert_eq!(palette.report().retained_matches, 2);
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
            StudioCommand::OpenFind
        );
        assert_eq!(
            palette.execute_selected(all_available())?,
            StudioCommand::OpenFind
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
            CommandPaletteError::Unavailable(StudioCommand::SaveFile),
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
