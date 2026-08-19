//! Bounded Rust completion result admission and checked text-edit mapping.

use std::{error::Error, fmt, mem::size_of, ops::Range};

use alpine_text::{BufferSnapshot, ByteOffset};
use serde_json::{Value, value::RawValue};

use crate::lsp_language::{LspPosition, LspRange, parse_range};

const MAX_COMPLETION_WIRE_BYTES: usize = 1_048_576;
pub(crate) const MAX_COMPLETION_ITEMS: usize = 64;
pub(crate) const MAX_VISIBLE_COMPLETION_ROWS: usize = 8;
const MAX_COMPLETION_LABEL_BYTES: usize = 256;
const MAX_COMPLETION_DOCUMENTATION_BYTES: usize = 4_096;
const MAX_COMPLETION_EDIT_BYTES: usize = 65_536;
pub(crate) const MAX_COMPLETION_RETAINED_BYTES: usize = 262_144;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CompletionError {
    WireTooLarge,
    Malformed,
    LabelTooLong,
    EditTooLong,
    UnsupportedAdditionalEdits,
    UnsupportedSnippet,
    RetentionExceeded,
    InvalidTextRange,
    AllocationFailed,
}

impl fmt::Display for CompletionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Rust completion rejected input: {self:?}")
    }
}

impl Error for CompletionError {}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CompletionEdit {
    range: Option<LspRange>,
    new_text: Box<str>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CompletionItem {
    label: Box<str>,
    documentation: Option<Box<str>>,
    edit: CompletionEdit,
}

impl CompletionItem {
    pub(crate) fn label(&self) -> &str {
        &self.label
    }

    #[cfg(test)]
    pub(crate) fn documentation(&self) -> Option<&str> {
        self.documentation.as_deref()
    }

    pub(crate) fn replacement(
        &self,
        snapshot: &BufferSnapshot,
        fallback: Range<usize>,
    ) -> Result<(Range<usize>, Box<str>), CompletionError> {
        let range = self.edit.range.map_or_else(
            || validate_fallback(snapshot, fallback),
            |range| byte_range(snapshot, range),
        )?;
        Ok((range, self.edit.new_text.clone()))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CompletionBatch {
    items: Box<[CompletionItem]>,
    retained_bytes: usize,
    omitted_items: usize,
    truncated_documentation: usize,
}

impl CompletionBatch {
    pub(crate) fn admit(result: &RawValue) -> Result<Self, CompletionError> {
        if result.get().len() > MAX_COMPLETION_WIRE_BYTES {
            return Err(CompletionError::WireTooLarge);
        }
        let value: Value =
            serde_json::from_str(result.get()).map_err(|_| CompletionError::Malformed)?;
        let source = match &value {
            Value::Null => &[][..],
            Value::Array(items) => items.as_slice(),
            Value::Object(object) => object
                .get("items")
                .and_then(Value::as_array)
                .ok_or(CompletionError::Malformed)?
                .as_slice(),
            _ => return Err(CompletionError::Malformed),
        };
        let admitted = source.len().min(MAX_COMPLETION_ITEMS);
        let mut items = Vec::new();
        items
            .try_reserve_exact(admitted)
            .map_err(|_| CompletionError::AllocationFailed)?;
        let mut retained_bytes = 0usize;
        let mut truncated_documentation = 0usize;
        for value in source.iter().take(admitted) {
            let (item, documentation_was_truncated) = parse_item(value)?;
            let next_retained_bytes = retained_bytes
                .checked_add(size_of::<CompletionItem>())
                .and_then(|bytes| bytes.checked_add(item.label.len()))
                .and_then(|bytes| bytes.checked_add(item.edit.new_text.len()))
                .and_then(|bytes| {
                    bytes.checked_add(item.documentation.as_deref().map_or(0, str::len))
                })
                .ok_or(CompletionError::RetentionExceeded)?;
            if next_retained_bytes > MAX_COMPLETION_RETAINED_BYTES {
                break;
            }
            retained_bytes = next_retained_bytes;
            truncated_documentation =
                truncated_documentation.saturating_add(usize::from(documentation_was_truncated));
            items.push(item);
        }
        let omitted_items = source.len().saturating_sub(items.len());
        Ok(Self {
            items: items.into_boxed_slice(),
            retained_bytes,
            omitted_items,
            truncated_documentation,
        })
    }

    pub(crate) fn items(&self) -> &[CompletionItem] {
        &self.items
    }

    pub(crate) const fn retained_bytes(&self) -> usize {
        self.retained_bytes
    }

    #[cfg(test)]
    pub(crate) const fn omitted_items(&self) -> usize {
        self.omitted_items
    }

    pub(crate) const fn was_truncated(&self) -> bool {
        self.omitted_items > 0 || self.truncated_documentation > 0
    }
}

fn parse_item(value: &Value) -> Result<(CompletionItem, bool), CompletionError> {
    let object = value.as_object().ok_or(CompletionError::Malformed)?;
    let label = object
        .get("label")
        .and_then(Value::as_str)
        .ok_or(CompletionError::Malformed)?;
    if label.is_empty() || label.len() > MAX_COMPLETION_LABEL_BYTES {
        return Err(CompletionError::LabelTooLong);
    }
    if let Some(additional) = object.get("additionalTextEdits") {
        let edits = additional.as_array().ok_or(CompletionError::Malformed)?;
        if !edits.is_empty() {
            return Err(CompletionError::UnsupportedAdditionalEdits);
        }
    }
    if let Some(format) = object.get("insertTextFormat") {
        match format.as_u64() {
            Some(1) => {}
            Some(2) => return Err(CompletionError::UnsupportedSnippet),
            _ => return Err(CompletionError::Malformed),
        }
    }
    let (documentation, documentation_was_truncated) = match object.get("documentation") {
        Some(value) => {
            let (documentation, truncated) = parse_documentation(value)?;
            (Some(documentation), truncated)
        }
        None => (None, false),
    };
    let edit = if let Some(edit) = object.get("textEdit") {
        parse_text_edit(edit)?
    } else {
        let text = object
            .get("insertText")
            .and_then(Value::as_str)
            .unwrap_or(label);
        checked_edit(None, text)?
    };
    Ok((
        CompletionItem {
            label: label.into(),
            documentation,
            edit,
        },
        documentation_was_truncated,
    ))
}

fn parse_documentation(value: &Value) -> Result<(Box<str>, bool), CompletionError> {
    let text = value.as_str().or_else(|| {
        value
            .as_object()
            .and_then(|object| object.get("value"))
            .and_then(Value::as_str)
    });
    let text = text.ok_or(CompletionError::Malformed)?;
    let retained = (0..=text.len().min(MAX_COMPLETION_DOCUMENTATION_BYTES))
        .rev()
        .find(|candidate| text.is_char_boundary(*candidate))
        .ok_or(CompletionError::Malformed)?;
    Ok((text[..retained].into(), retained < text.len()))
}

fn parse_text_edit(value: &Value) -> Result<CompletionEdit, CompletionError> {
    let object = value.as_object().ok_or(CompletionError::Malformed)?;
    let text = object
        .get("newText")
        .and_then(Value::as_str)
        .ok_or(CompletionError::Malformed)?;
    let has_range = object.contains_key("range");
    let has_insert = object.contains_key("insert");
    let has_replace = object.contains_key("replace");
    if has_range {
        if has_insert || has_replace {
            return Err(CompletionError::InvalidTextRange);
        }
    } else if !has_insert || !has_replace {
        return Err(CompletionError::InvalidTextRange);
    }
    let range = if let Some(range) = object.get("range") {
        parse_range(range).map_err(|_| CompletionError::InvalidTextRange)?
    } else {
        let insert = parse_range(object.get("insert").ok_or(CompletionError::Malformed)?)
            .map_err(|_| CompletionError::InvalidTextRange)?;
        let replace = parse_range(object.get("replace").ok_or(CompletionError::Malformed)?)
            .map_err(|_| CompletionError::InvalidTextRange)?;
        let insert_end = (insert.end().line(), insert.end().utf16_character());
        let replace_end = (replace.end().line(), replace.end().utf16_character());
        if insert.start() != replace.start() || insert_end > replace_end {
            return Err(CompletionError::InvalidTextRange);
        }
        replace
    };
    checked_edit(Some(range), text)
}

fn checked_edit(range: Option<LspRange>, text: &str) -> Result<CompletionEdit, CompletionError> {
    if text.len() > MAX_COMPLETION_EDIT_BYTES {
        return Err(CompletionError::EditTooLong);
    }
    Ok(CompletionEdit {
        range,
        new_text: text.into(),
    })
}

fn validate_fallback(
    snapshot: &BufferSnapshot,
    fallback: Range<usize>,
) -> Result<Range<usize>, CompletionError> {
    snapshot
        .slice(fallback.clone())
        .map_err(|_| CompletionError::InvalidTextRange)?;
    Ok(fallback)
}

fn byte_range(snapshot: &BufferSnapshot, range: LspRange) -> Result<Range<usize>, CompletionError> {
    let start = byte_for_position(snapshot, range.start())?;
    let end = byte_for_position(snapshot, range.end())?;
    Ok(start..end)
}

pub(crate) fn position_for_byte(
    snapshot: &BufferSnapshot,
    offset: ByteOffset,
) -> Result<LspPosition, CompletionError> {
    if offset.get() > snapshot.len_bytes() {
        return Err(CompletionError::InvalidTextRange);
    }
    let mut lower = 0;
    let mut upper = snapshot.line_count();
    for _ in 0..usize::BITS {
        match lower.cmp(&upper) {
            std::cmp::Ordering::Less => {}
            std::cmp::Ordering::Equal => break,
            std::cmp::Ordering::Greater => return Err(CompletionError::InvalidTextRange),
        }
        let line = lower + (upper - lower) / 2;
        let range = snapshot
            .line_byte_range(line)
            .map_err(|_| CompletionError::InvalidTextRange)?;
        let after_line = usize::from(offset.get() >= range.end);
        lower = [lower, line.saturating_add(1)][after_line];
        upper = [line, upper][after_line];
    }
    match lower.cmp(&upper) {
        std::cmp::Ordering::Equal => {}
        std::cmp::Ordering::Less | std::cmp::Ordering::Greater => {
            return Err(CompletionError::InvalidTextRange);
        }
    }
    let selected_line = lower.min(snapshot.line_count().saturating_sub(1));
    let range = snapshot
        .line_byte_range(selected_line)
        .map_err(|_| CompletionError::InvalidTextRange)?;
    let prefix_end = offset.get().min(range.end);
    let prefix = snapshot
        .slice(range.start..prefix_end)
        .map_err(|_| CompletionError::InvalidTextRange)?;
    let utf16 = checked_u32(prefix.trim_end_matches(['\r', '\n']).encode_utf16().count())?;
    let line = checked_u32(selected_line)?;
    LspPosition::new(line, utf16).map_err(|_| CompletionError::InvalidTextRange)
}

fn checked_u32(value: usize) -> Result<u32, CompletionError> {
    u32::try_from(value).map_err(|_| CompletionError::InvalidTextRange)
}

fn byte_for_position(
    snapshot: &BufferSnapshot,
    position: LspPosition,
) -> Result<usize, CompletionError> {
    let line = usize::try_from(position.line()).map_err(|_| CompletionError::InvalidTextRange)?;
    let range = snapshot
        .line_byte_range(line)
        .map_err(|_| CompletionError::InvalidTextRange)?;
    let line_text = snapshot
        .slice(range.clone())
        .map_err(|_| CompletionError::InvalidTextRange)?;
    let content = line_text.trim_end_matches(['\r', '\n']);
    let target = position.utf16_character();
    let mut units = 0_u32;
    for (byte, character) in content.char_indices() {
        if units == target {
            return Ok(range.start + byte);
        }
        units = units
            .checked_add(
                u32::try_from(character.len_utf16())
                    .map_err(|_| CompletionError::InvalidTextRange)?,
            )
            .ok_or(CompletionError::InvalidTextRange)?;
        if units > target {
            return Err(CompletionError::InvalidTextRange);
        }
    }
    (units == target)
        .then_some(range.start + content.len())
        .ok_or(CompletionError::InvalidTextRange)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(text: &str) -> Box<RawValue> {
        RawValue::from_string(text.to_owned()).unwrap_or_else(|_| unreachable!())
    }

    #[test]
    fn array_and_completion_list_shapes_are_bounded() {
        let batch = CompletionBatch::admit(&raw(
            r#"{"isIncomplete":true,"items":[{"label":"alpha","documentation":{"kind":"markdown","value":"doc"},"textEdit":{"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":2}},"newText":"alphabet"}}]}"#,
        ))
        .unwrap_or_else(|_| unreachable!());
        assert_eq!(batch.items().len(), 1);
        assert_eq!(batch.items()[0].label(), "alpha");
        assert_eq!(batch.items()[0].documentation(), Some("doc"));
        assert!(batch.retained_bytes() > 0);
        assert_eq!(batch.omitted_items(), 0);
    }

    #[test]
    fn insert_replace_edit_uses_the_checked_replace_range() {
        let batch = CompletionBatch::admit(&raw(
            r#"[{"label":"value","textEdit":{"insert":{"start":{"line":0,"character":0},"end":{"line":0,"character":2}},"replace":{"start":{"line":0,"character":0},"end":{"line":0,"character":4}},"newText":"value"}}]"#,
        ))
        .unwrap_or_else(|_| unreachable!());
        let snapshot = alpine_text::Buffer::new("name🙂\n").snapshot();
        assert_eq!(
            batch.items()[0].replacement(&snapshot, 0..0),
            Ok((0..4, Box::<str>::from("value")))
        );
    }

    #[test]
    fn utf16_positions_reject_surrogate_and_line_overflow() {
        let snapshot = alpine_text::Buffer::new("a🙂b\nnext").snapshot();
        assert_eq!(
            position_for_byte(&snapshot, ByteOffset::new(5)),
            Ok(LspPosition::new(0, 3).unwrap_or_else(|_| unreachable!()))
        );
        assert_eq!(
            byte_for_position(
                &snapshot,
                LspPosition::new(0, 2).unwrap_or_else(|_| unreachable!())
            ),
            Err(CompletionError::InvalidTextRange)
        );
        assert_eq!(
            byte_for_position(
                &snapshot,
                LspPosition::new(9, 0).unwrap_or_else(|_| unreachable!())
            ),
            Err(CompletionError::InvalidTextRange)
        );
    }

    #[test]
    fn additional_edits_and_oversized_fields_fail_closed() {
        assert_eq!(
            CompletionBatch::admit(&raw(r#"[{"label":"x","additionalTextEdits":[{}]}]"#)),
            Err(CompletionError::UnsupportedAdditionalEdits)
        );
        assert!(
            CompletionBatch::admit(&raw(r#"[{"label":"x","additionalTextEdits":[]}]"#)).is_ok()
        );
        let label = "x".repeat(MAX_COMPLETION_LABEL_BYTES + 1);
        assert_eq!(
            CompletionBatch::admit(&raw(&format!(r#"[{{"label":"{label}"}}]"#))),
            Err(CompletionError::LabelTooLong)
        );
        assert_eq!(
            CompletionBatch::admit(&raw(r#"[{"label":"x","additionalTextEdits":{}}]"#)),
            Err(CompletionError::Malformed)
        );
        assert_eq!(
            CompletionBatch::admit(&raw(
                r#"[{"label":"x","insertText":"${0:x}","insertTextFormat":2}]"#
            )),
            Err(CompletionError::UnsupportedSnippet)
        );
        assert_eq!(
            CompletionBatch::admit(&raw(
                r#"[{"label":"x","textEdit":{"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":1}},"insert":{"start":{"line":0,"character":0},"end":{"line":0,"character":1}},"replace":{"start":{"line":0,"character":0},"end":{"line":0,"character":1}},"newText":"x"}}]"#
            )),
            Err(CompletionError::InvalidTextRange)
        );
        assert_eq!(
            CompletionBatch::admit(&raw(
                r#"[{"label":"x","textEdit":{"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":1}},"insert":{"start":{"line":0,"character":0},"end":{"line":0,"character":1}},"newText":"x"}}]"#
            )),
            Err(CompletionError::InvalidTextRange)
        );
        assert_eq!(
            CompletionBatch::admit(&raw(
                r#"[{"label":"x","textEdit":{"insert":{"start":{"line":0,"character":0},"end":{"line":0,"character":1}},"newText":"x"}}]"#
            )),
            Err(CompletionError::InvalidTextRange)
        );
    }

    #[test]
    fn item_truncation_and_retained_bytes_are_exactly_bounded() {
        let items = (0..=MAX_COMPLETION_ITEMS)
            .map(|index| format!(r#"{{"label":"item-{index}"}}"#))
            .collect::<Vec<_>>()
            .join(",");
        let batch =
            CompletionBatch::admit(&raw(&format!("[{items}]"))).unwrap_or_else(|_| unreachable!());
        assert_eq!(batch.items().len(), MAX_COMPLETION_ITEMS);
        assert_eq!(batch.omitted_items(), 1);
        assert!(batch.retained_bytes() <= MAX_COMPLETION_RETAINED_BYTES);
    }

    #[test]
    fn remaining_shapes_formats_and_limits_fail_closed() {
        assert_eq!(
            CompletionError::Malformed.to_string(),
            "Rust completion rejected input: Malformed"
        );
        assert!(
            CompletionBatch::admit(&raw("null"))
                .unwrap_or_else(|_| unreachable!())
                .items()
                .is_empty()
        );
        for malformed in ["true", "{}", "[1]", "[{}]"] {
            assert_eq!(
                CompletionBatch::admit(&raw(malformed)),
                Err(CompletionError::Malformed)
            );
        }
        assert!(
            CompletionBatch::admit(&raw(
                r#"[{"label":"plain","insertTextFormat":1,"documentation":"text"}]"#
            ))
            .is_ok()
        );
        assert_eq!(
            CompletionBatch::admit(&raw(r#"[{"label":"x","insertTextFormat":3}]"#)),
            Err(CompletionError::Malformed)
        );
        assert_eq!(
            CompletionBatch::admit(&raw(r#"[{"label":"x","documentation":1}]"#)),
            Err(CompletionError::Malformed)
        );

        let documentation = format!(
            "{}€",
            "d".repeat(MAX_COMPLETION_DOCUMENTATION_BYTES.saturating_sub(1))
        );
        let truncated = CompletionBatch::admit(&raw(&format!(
            r#"[{{"label":"x","documentation":"{documentation}"}}]"#
        )))
        .unwrap_or_else(|_| unreachable!());
        assert_eq!(
            truncated.items()[0]
                .documentation()
                .unwrap_or_else(|| unreachable!())
                .len(),
            MAX_COMPLETION_DOCUMENTATION_BYTES.saturating_sub(1)
        );
        assert!(truncated.was_truncated());
        assert_eq!(truncated.omitted_items(), 0);
        let edit = "e".repeat(MAX_COMPLETION_EDIT_BYTES + 1);
        assert_eq!(
            CompletionBatch::admit(&raw(&format!(r#"[{{"label":"x","insertText":"{edit}"}}]"#))),
            Err(CompletionError::EditTooLong)
        );

        let retained_edit = "e".repeat(MAX_COMPLETION_EDIT_BYTES);
        let retained_documentation = "d".repeat(MAX_COMPLETION_DOCUMENTATION_BYTES);
        let retained_item = format!(
            r#"{{"label":"x","insertText":"{retained_edit}","documentation":"{retained_documentation}"}}"#
        );
        let retained = CompletionBatch::admit(&raw(&format!(
            "[{retained_item},{retained_item},{retained_item},{retained_item}]"
        )))
        .unwrap_or_else(|_| unreachable!());
        assert_eq!(retained.items().len(), 3);
        assert_eq!(retained.omitted_items(), 1);
        assert!(retained.was_truncated());
        assert!(retained.retained_bytes() <= MAX_COMPLETION_RETAINED_BYTES);

        let oversized_wire = "w".repeat(MAX_COMPLETION_WIRE_BYTES);
        assert_eq!(
            CompletionBatch::admit(&raw(&format!(
                r#"[{{"label":"x","insertText":"{oversized_wire}"}}]"#
            ))),
            Err(CompletionError::WireTooLarge)
        );
    }

    #[test]
    fn every_text_edit_shape_and_range_is_checked() {
        for malformed in [
            r#"[{"label":"x","textEdit":1}]"#,
            r#"[{"label":"x","textEdit":{}}]"#,
            r#"[{"label":"x","textEdit":{"newText":"x","insert":{"start":{"line":0,"character":0},"end":{"line":0,"character":1}}}}]"#,
        ] {
            assert!(CompletionBatch::admit(&raw(malformed)).is_err());
        }
        assert_eq!(
            CompletionBatch::admit(&raw(
                r#"[{"label":"x","textEdit":{"insert":{"start":{"line":0,"character":1},"end":{"line":0,"character":2}},"replace":{"start":{"line":0,"character":0},"end":{"line":0,"character":2}},"newText":"x"}}]"#
            )),
            Err(CompletionError::InvalidTextRange)
        );
        assert_eq!(
            CompletionBatch::admit(&raw(
                r#"[{"label":"x","textEdit":{"insert":{"start":{"line":0,"character":0},"end":{"line":0,"character":3}},"replace":{"start":{"line":0,"character":0},"end":{"line":0,"character":2}},"newText":"x"}}]"#
            )),
            Err(CompletionError::InvalidTextRange)
        );

        let snapshot = alpine_text::Buffer::new("abcd\n").snapshot();
        let fallback =
            CompletionBatch::admit(&raw(r#"[{"label":"x"}]"#)).unwrap_or_else(|_| unreachable!());
        assert_eq!(
            fallback.items()[0].replacement(&snapshot, 1..3),
            Ok((1..3, Box::<str>::from("x")))
        );
        assert_eq!(
            fallback.items()[0].replacement(&snapshot, 1..usize::MAX),
            Err(CompletionError::InvalidTextRange)
        );

        assert_eq!(
            CompletionBatch::admit(&raw(
                r#"[{"label":"x","textEdit":{"range":{"start":{"line":0,"character":3},"end":{"line":0,"character":1}},"newText":"x"}}]"#,
            )),
            Err(CompletionError::InvalidTextRange)
        );
        assert_eq!(
            position_for_byte(&snapshot, ByteOffset::new(snapshot.len_bytes() + 1)),
            Err(CompletionError::InvalidTextRange)
        );
        assert_eq!(checked_u32(0), Ok(0));
        #[cfg(target_pointer_width = "64")]
        assert_eq!(
            checked_u32((u32::MAX as usize).saturating_add(1)),
            Err(CompletionError::InvalidTextRange)
        );
        assert_eq!(
            byte_for_position(
                &snapshot,
                LspPosition::new(0, 4).unwrap_or_else(|_| unreachable!())
            ),
            Ok(4)
        );
    }

    #[test]
    fn inclusive_wire_field_and_retention_ceilings_are_admitted() {
        let prefix = r#"[{"label":"x","padding":""#;
        let suffix = r#""}]"#;
        let padding = "w".repeat(MAX_COMPLETION_WIRE_BYTES - prefix.len() - suffix.len());
        let exact_wire = format!("{prefix}{padding}{suffix}");
        assert_eq!(exact_wire.len(), MAX_COMPLETION_WIRE_BYTES);
        let exact_wire = raw(&exact_wire);
        assert_eq!(exact_wire.get().len(), MAX_COMPLETION_WIRE_BYTES);
        assert!(CompletionBatch::admit(&exact_wire).is_ok());

        let exact_label = "l".repeat(MAX_COMPLETION_LABEL_BYTES);
        assert!(
            CompletionBatch::admit(&raw(&format!(
                r#"[{{"label":"{exact_label}","insertText":""}}]"#
            )))
            .is_ok()
        );

        let exact_documentation = "d".repeat(MAX_COMPLETION_DOCUMENTATION_BYTES);
        let exact_documentation = CompletionBatch::admit(&raw(&format!(
            r#"[{{"label":"x","documentation":"{exact_documentation}"}}]"#
        )))
        .unwrap_or_else(|_| unreachable!());
        assert_eq!(exact_documentation.truncated_documentation, 0);

        let item_count = 4;
        let fixed = item_count * size_of::<CompletionItem>() + item_count;
        let mut remaining = MAX_COMPLETION_RETAINED_BYTES - fixed;
        let mut values = Vec::with_capacity(item_count);
        for _ in 0..item_count {
            let edit_bytes = remaining.min(MAX_COMPLETION_EDIT_BYTES);
            remaining -= edit_bytes;
            let documentation_bytes = remaining.min(MAX_COMPLETION_DOCUMENTATION_BYTES);
            remaining -= documentation_bytes;
            values.push(serde_json::json!({
                "label": "x",
                "insertText": "e".repeat(edit_bytes),
                "documentation": "d".repeat(documentation_bytes),
            }));
        }
        assert_eq!(remaining, 0);
        let exact_retention = serde_json::to_string(&values).unwrap_or_else(|_| unreachable!());
        let batch =
            CompletionBatch::admit(&raw(&exact_retention)).unwrap_or_else(|_| unreachable!());
        assert_eq!(batch.retained_bytes(), MAX_COMPLETION_RETAINED_BYTES);
        assert_ne!(batch.retained_bytes(), 1);
    }

    #[test]
    fn text_edit_shape_guards_and_equal_insert_replace_ends_are_exact() {
        let range = r#"{"start":{"line":0,"character":0},"end":{"line":0,"character":1}}"#;
        assert_eq!(
            CompletionBatch::admit(&raw(&format!(
                r#"[{{"label":"x","textEdit":{{"insert":{range},"newText":"x"}}}}]"#
            ))),
            Err(CompletionError::InvalidTextRange)
        );
        assert_eq!(
            CompletionBatch::admit(&raw(&format!(
                r#"[{{"label":"x","textEdit":{{"range":{range},"insert":{range},"replace":{range},"newText":"x"}}}}]"#
            ))),
            Err(CompletionError::InvalidTextRange)
        );
        let equal = CompletionBatch::admit(&raw(&format!(
            r#"[{{"label":"x","textEdit":{{"insert":{range},"replace":{range},"newText":"x"}}}}]"#
        )))
        .unwrap_or_else(|_| unreachable!());
        let snapshot = alpine_text::Buffer::new("ab\n").snapshot();
        assert_eq!(
            equal.items()[0].replacement(&snapshot, 0..0),
            Ok((0..1, Box::<str>::from("x")))
        );
    }

    #[test]
    fn byte_to_lsp_binary_search_distinguishes_every_line_boundary() {
        let snapshot = alpine_text::Buffer::new("a\nbb\nccc").snapshot();
        let expected = [
            (0, 0, 0),
            (1, 0, 1),
            (2, 1, 0),
            (3, 1, 1),
            (4, 1, 2),
            (5, 2, 0),
            (7, 2, 2),
            (8, 2, 3),
        ];
        for (byte, line, utf16) in expected {
            assert_eq!(
                position_for_byte(&snapshot, ByteOffset::new(byte)),
                Ok(LspPosition::new(line, utf16).unwrap_or_else(|_| unreachable!())),
                "byte boundary {byte}"
            );
        }
        assert_eq!(
            position_for_byte(&snapshot, ByteOffset::new(snapshot.len_bytes())),
            Ok(LspPosition::new(2, 3).unwrap_or_else(|_| unreachable!()))
        );
    }
}
