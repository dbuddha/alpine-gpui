//! Bounded go-to-line overlay. Accepts a 1-based `line` or `line:column`.

use std::{error::Error, fmt, fmt::Write as _};

use alpine_text::{BufferSnapshot, ByteOffset, Selection, TextError};

use crate::field_edit::{EditError, FieldEdit, Prepared};

pub(crate) const MAX_QUERY_BYTES: usize = 32;
pub(crate) const DISPLAY_PREFIX: &str = "Go to line: ";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Target {
    /// Zero-based logical line.
    pub(crate) line: usize,
    /// Zero-based Unicode scalar column, or `None` for the start of the line.
    pub(crate) column: Option<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum GoToLineError {
    Empty,
    InvalidNumber,
    ExtraInput,
    QueryTooLong { actual: usize, limit: usize },
    AllocationFailed,
    InvalidSelection,
    Text(TextError),
}

impl fmt::Display for GoToLineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("go to line needs a line number"),
            Self::InvalidNumber => {
                formatter.write_str("go to line expects a 1-based line or line:column")
            }
            Self::ExtraInput => formatter.write_str("go to line does not accept extra separators"),
            Self::QueryTooLong { actual, limit } => {
                write!(
                    formatter,
                    "go to line query is {actual} bytes; limit is {limit}"
                )
            }
            Self::AllocationFailed => formatter.write_str("go to line allocation failed"),
            Self::InvalidSelection => formatter.write_str("go to line selection is invalid"),
            Self::Text(error) => write!(formatter, "{error}"),
        }
    }
}

impl Error for GoToLineError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Text(error) => Some(error),
            Self::Empty
            | Self::InvalidNumber
            | Self::ExtraInput
            | Self::QueryTooLong { .. }
            | Self::AllocationFailed
            | Self::InvalidSelection => None,
        }
    }
}

impl From<EditError> for GoToLineError {
    fn from(error: EditError) -> Self {
        match error {
            EditError::InvalidSelection => Self::InvalidSelection,
            EditError::AllocationFailed => Self::AllocationFailed,
            EditError::TooLong { actual, limit } => Self::QueryTooLong { actual, limit },
        }
    }
}

#[derive(Default)]
pub(crate) struct GoToLineState {
    open: bool,
    query: String,
    pub(crate) edit: FieldEdit,
}

impl GoToLineState {
    pub(crate) const fn is_open(&self) -> bool {
        self.open
    }

    pub(crate) fn query(&self) -> &str {
        &self.query
    }

    pub(crate) fn edit(&self) -> &FieldEdit {
        &self.edit
    }

    pub(crate) fn edit_parts(&mut self) -> (&str, &mut FieldEdit) {
        (&self.query, &mut self.edit)
    }

    pub(crate) fn open(&mut self, line_one_based: usize) -> Result<bool, GoToLineError> {
        let mut query = String::new();
        query
            .try_reserve(MAX_QUERY_BYTES)
            .map_err(|_| GoToLineError::AllocationFailed)?;
        write!(&mut query, "{line_one_based}").map_err(|_| GoToLineError::AllocationFailed)?;
        if query.len() > MAX_QUERY_BYTES {
            return Err(GoToLineError::QueryTooLong {
                actual: query.len(),
                limit: MAX_QUERY_BYTES,
            });
        }
        let len = query.len();
        let changed = !self.open || self.query != query || self.edit.is_composing();
        self.open = true;
        self.query = query;
        self.edit = FieldEdit::default();
        self.edit.set_selection(
            &self.query,
            Selection::new(ByteOffset::new(0), ByteOffset::new(len)),
        )?;
        Ok(changed)
    }

    pub(crate) fn close(&mut self) -> bool {
        let changed = self.open;
        self.open = false;
        self.edit.reset_focus();
        changed
    }

    pub(crate) fn select_all(&mut self) -> bool {
        self.set_selection(Selection::new(
            ByteOffset::new(0),
            ByteOffset::new(self.query.len()),
        ))
        .unwrap_or(false)
    }

    pub(crate) fn set_selection(&mut self, selection: Selection) -> Result<bool, GoToLineError> {
        Ok(self.edit.set_selection(&self.query, selection)?)
    }

    pub(crate) fn begin_composition(&mut self) -> bool {
        self.open && self.edit.begin_composition()
    }

    pub(crate) fn update_composition(
        &mut self,
        text: &str,
        start: u32,
        length: u32,
    ) -> Result<bool, GoToLineError> {
        Ok(self
            .edit
            .update_composition(&self.query, text, start, length, MAX_QUERY_BYTES)?)
    }

    pub(crate) fn cancel_composition(&mut self) -> bool {
        self.edit.cancel_composition()
    }

    pub(crate) fn apply_edit(&mut self, mut prepared: Prepared) -> bool {
        let changed = self.query != prepared.value;
        self.query = std::mem::take(&mut prepared.value);
        self.edit.accept(prepared, changed);
        changed
    }

    pub(crate) fn projected_value(&self) -> Result<String, GoToLineError> {
        Ok(self.edit.projected_value(&self.query)?)
    }

    pub(crate) fn display_text(&self) -> Result<String, GoToLineError> {
        let projected = self.projected_value()?;
        let mut display = String::new();
        display
            .try_reserve_exact(DISPLAY_PREFIX.len().saturating_add(projected.len()))
            .map_err(|_| GoToLineError::AllocationFailed)?;
        display.push_str(DISPLAY_PREFIX);
        display.push_str(&projected);
        Ok(display)
    }
}

pub(crate) fn parse_target(query: &str) -> Result<Target, GoToLineError> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Err(GoToLineError::Empty);
    }
    if trimmed.bytes().filter(|byte| *byte == b':').count() > 1 {
        return Err(GoToLineError::ExtraInput);
    }
    let (line_part, column_part) = match trimmed.split_once(':') {
        Some((line, column)) => (line, Some(column)),
        None => (trimmed, None),
    };
    let line = parse_one_based(line_part.trim())?;
    let column = match column_part {
        None => None,
        Some(part) => {
            let part = part.trim();
            if part.is_empty() {
                Some(0)
            } else {
                Some(parse_one_based(part)?.saturating_sub(1))
            }
        }
    };
    Ok(Target {
        line: line.saturating_sub(1),
        column,
    })
}

pub(crate) fn offset_in(
    snapshot: &BufferSnapshot,
    query: &str,
) -> Result<ByteOffset, GoToLineError> {
    let target = parse_target(query)?;
    let last = snapshot.line_count().saturating_sub(1);
    let line = target.line.min(last);
    let range = snapshot
        .line_byte_range(line)
        .map_err(GoToLineError::Text)?;
    let text = snapshot.slice(range.clone()).map_err(GoToLineError::Text)?;
    let content = text.trim_end_matches(['\r', '\n']);
    let column_bytes = match target.column {
        None => 0,
        Some(column) => {
            let mut bytes = 0_usize;
            for (index, ch) in content.chars().enumerate() {
                if index == column {
                    break;
                }
                bytes = bytes.saturating_add(ch.len_utf8());
            }
            bytes.min(content.len())
        }
    };
    Ok(ByteOffset::new(range.start.saturating_add(column_bytes)))
}

fn parse_one_based(part: &str) -> Result<usize, GoToLineError> {
    if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(GoToLineError::InvalidNumber);
    }
    let value = part
        .parse::<u32>()
        .map_err(|_| GoToLineError::InvalidNumber)?;
    if value == 0 {
        return Err(GoToLineError::InvalidNumber);
    }
    usize::try_from(value).map_err(|_| GoToLineError::InvalidNumber)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alpine_text::Buffer;

    #[test]
    fn parse_target_accepts_line_and_optional_column() -> Result<(), GoToLineError> {
        assert_eq!(
            parse_target("12")?,
            Target {
                line: 11,
                column: None
            }
        );
        assert_eq!(
            parse_target(" 3:4 ")?,
            Target {
                line: 2,
                column: Some(3)
            }
        );
        assert_eq!(
            parse_target("8:")?,
            Target {
                line: 7,
                column: Some(0)
            }
        );
        assert_eq!(parse_target(""), Err(GoToLineError::Empty));
        assert_eq!(parse_target("0"), Err(GoToLineError::InvalidNumber));
        assert_eq!(parse_target("1:0"), Err(GoToLineError::InvalidNumber));
        assert_eq!(parse_target("1:2:3"), Err(GoToLineError::ExtraInput));
        assert_eq!(parse_target("ab"), Err(GoToLineError::InvalidNumber));
        Ok(())
    }

    #[test]
    fn offset_in_clamps_and_counts_unicode_columns() -> Result<(), Box<dyn Error>> {
        let snapshot = Buffer::new("a\nbb\náx").snapshot();
        assert_eq!(offset_in(&snapshot, "1")?, ByteOffset::new(0));
        assert_eq!(offset_in(&snapshot, "2")?, ByteOffset::new(2));
        assert_eq!(offset_in(&snapshot, "2:2")?, ByteOffset::new(3));
        assert_eq!(offset_in(&snapshot, "3:2")?, ByteOffset::new(7));
        assert_eq!(offset_in(&snapshot, "99")?, ByteOffset::new(5));
        assert_eq!(offset_in(&snapshot, "3:99")?, ByteOffset::new(8));
        Ok(())
    }
}
