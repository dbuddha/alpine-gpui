//! Bounded local Rust formatting and rename edit admission.

use std::{
    error::Error,
    fmt,
    mem::size_of,
    path::{Path, PathBuf},
};

use alpine_text::{Buffer, BufferSnapshot, Transaction};
use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value, value::RawValue};

use crate::{
    lsp_language::{LspRange, parse_range},
    rust_completion::byte_range,
    rust_navigation::{NavigationError, resolve_local_file_uri, revalidate_local_path},
};

const MAX_WORKSPACE_EDIT_WIRE_BYTES: usize = 2_097_152;
const MAX_WORKSPACE_EDIT_FILES: usize = 32;
const MAX_WORKSPACE_EDIT_EDITS: usize = 4_096;
const MAX_FILE_EDITS: usize = 1_024;
const MAX_URI_BYTES: usize = 4_096;
const MAX_EDIT_TEXT_BYTES: usize = 1_048_576;
const MAX_INSERTED_TEXT_BYTES: usize = 8_388_608;
const MAX_FILE_TEXT_BYTES: usize = 33_554_432;
const MAX_PREPARED_TEXT_BYTES: usize = 67_108_864;
const MAX_RETAINED_BYTES: usize = 67_371_008;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WorkspaceEditError {
    WireTooLarge,
    Malformed,
    UnsupportedShape,
    UnsupportedAnnotation,
    UnsupportedResourceOperation,
    TooManyFiles,
    TooManyEdits,
    UriTooLong,
    DuplicatePath,
    InvalidRange,
    OverlappingEdits,
    EditTextTooLong,
    InsertedTextTooLarge,
    RetentionExceeded,
    FileTooLarge,
    FileUnavailable,
    InvalidUtf8,
    StaleFile,
    AllocationFailed,
    LocalPath(NavigationError),
}

impl fmt::Display for WorkspaceEditError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Rust workspace edit rejected input: {self:?}")
    }
}

impl Error for WorkspaceEditError {}

#[derive(Clone, Debug, Eq, PartialEq)]
struct WorkspaceTextEdit {
    range: LspRange,
    new_text: Box<str>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct WorkspaceFileEdit {
    path: PathBuf,
    lsp_version: Option<i32>,
    edits: Box<[WorkspaceTextEdit]>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WorkspaceEditProposal {
    workspace_root: PathBuf,
    files: Box<[WorkspaceFileEdit]>,
    retained_bytes: usize,
}

impl WorkspaceEditProposal {
    pub(crate) fn admit_formatting(
        result: &RawValue,
        workspace_root: &Path,
        document_uri: &str,
        lsp_version: i32,
    ) -> Result<Self, WorkspaceEditError> {
        checked_wire(result)?;
        let value = strict_value(result)?;
        if value.is_null() {
            return Self::empty(workspace_root);
        }
        let edits = value.as_array().ok_or(WorkspaceEditError::Malformed)?;
        let file = parse_file_edit(workspace_root, document_uri, Some(lsp_version), edits)?;
        Self::finish(workspace_root, vec![file])
    }

    pub(crate) fn admit_rename(
        result: &RawValue,
        workspace_root: &Path,
    ) -> Result<Self, WorkspaceEditError> {
        checked_wire(result)?;
        let value = strict_value(result)?;
        if value.is_null() {
            return Self::empty(workspace_root);
        }
        let object = value.as_object().ok_or(WorkspaceEditError::Malformed)?;
        if object.contains_key("changeAnnotations") {
            return Err(WorkspaceEditError::UnsupportedAnnotation);
        }
        if object
            .keys()
            .any(|key| !matches!(key.as_str(), "changes" | "documentChanges"))
        {
            return Err(WorkspaceEditError::UnsupportedShape);
        }
        let files = match (object.get("changes"), object.get("documentChanges")) {
            (Some(changes), None) => parse_changes(workspace_root, changes)?,
            (None, Some(changes)) => parse_document_changes(workspace_root, changes)?,
            _ => return Err(WorkspaceEditError::UnsupportedShape),
        };
        Self::finish(workspace_root, files)
    }

    fn empty(workspace_root: &Path) -> Result<Self, WorkspaceEditError> {
        let root = canonical_workspace_root(workspace_root)?;
        let retained_bytes = root.as_os_str().len();
        Ok(Self {
            workspace_root: root,
            files: Box::new([]),
            retained_bytes,
        })
    }

    fn finish(
        workspace_root: &Path,
        mut files: Vec<WorkspaceFileEdit>,
    ) -> Result<Self, WorkspaceEditError> {
        if files.len() > MAX_WORKSPACE_EDIT_FILES {
            return Err(WorkspaceEditError::TooManyFiles);
        }
        files.sort_unstable_by(|left, right| left.path.cmp(&right.path));
        if files.windows(2).any(|pair| pair[0].path == pair[1].path) {
            return Err(WorkspaceEditError::DuplicatePath);
        }
        let root = canonical_workspace_root(workspace_root)?;
        let mut retained_bytes = root.as_os_str().len();
        let mut edit_count = 0_usize;
        let mut inserted_bytes = 0_usize;
        for file in &files {
            edit_count = edit_count
                .checked_add(file.edits.len())
                .ok_or(WorkspaceEditError::TooManyEdits)?;
            if edit_count > MAX_WORKSPACE_EDIT_EDITS {
                return Err(WorkspaceEditError::TooManyEdits);
            }
            retained_bytes = retained_bytes
                .checked_add(size_of::<WorkspaceFileEdit>())
                .and_then(|bytes| bytes.checked_add(file.path.as_os_str().len()))
                .ok_or(WorkspaceEditError::RetentionExceeded)?;
            for edit in &file.edits {
                inserted_bytes = inserted_bytes
                    .checked_add(edit.new_text.len())
                    .ok_or(WorkspaceEditError::InsertedTextTooLarge)?;
                retained_bytes = retained_bytes
                    .checked_add(size_of::<WorkspaceTextEdit>())
                    .and_then(|bytes| bytes.checked_add(edit.new_text.len()))
                    .ok_or(WorkspaceEditError::RetentionExceeded)?;
            }
        }
        if inserted_bytes > MAX_INSERTED_TEXT_BYTES {
            return Err(WorkspaceEditError::InsertedTextTooLarge);
        }
        if retained_bytes > MAX_RETAINED_BYTES {
            return Err(WorkspaceEditError::RetentionExceeded);
        }
        Ok(Self {
            workspace_root: root,
            files: files.into_boxed_slice(),
            retained_bytes,
        })
    }

    pub(crate) fn file_count(&self) -> usize {
        self.files.len()
    }

    pub(crate) fn edit_count(&self) -> usize {
        self.files.iter().map(|file| file.edits.len()).sum()
    }

    pub(crate) const fn retained_bytes(&self) -> usize {
        self.retained_bytes
    }

    pub(crate) fn prepare(&self) -> Result<PreparedWorkspaceEdit, WorkspaceEditError> {
        let mut files = Vec::new();
        files
            .try_reserve_exact(self.files.len())
            .map_err(|_| WorkspaceEditError::AllocationFailed)?;
        let mut retained_bytes = 0_usize;
        let mut prepared_text_bytes = 0_usize;
        for file in &self.files {
            let current = revalidate_local_path(&self.workspace_root, &file.path)
                .map_err(WorkspaceEditError::LocalPath)?;
            if current != file.path {
                return Err(WorkspaceEditError::StaleFile);
            }
            let metadata =
                std::fs::metadata(&current).map_err(|_| WorkspaceEditError::FileUnavailable)?;
            if metadata.len() > MAX_FILE_TEXT_BYTES as u64 {
                return Err(WorkspaceEditError::FileTooLarge);
            }
            let bytes = std::fs::read(&current).map_err(|_| WorkspaceEditError::FileUnavailable)?;
            if bytes.len() > MAX_FILE_TEXT_BYTES {
                return Err(WorkspaceEditError::FileTooLarge);
            }
            let original = String::from_utf8(bytes).map_err(|_| WorkspaceEditError::InvalidUtf8)?;
            let prepared = prepare_file(file, original)?;
            prepared_text_bytes = prepared_text_bytes
                .checked_add(prepared.original.len())
                .and_then(|bytes| bytes.checked_add(prepared.replacement.len()))
                .ok_or(WorkspaceEditError::RetentionExceeded)?;
            if prepared_text_bytes > MAX_PREPARED_TEXT_BYTES {
                return Err(WorkspaceEditError::RetentionExceeded);
            }
            retained_bytes = retained_bytes
                .checked_add(prepared.retained_bytes())
                .ok_or(WorkspaceEditError::RetentionExceeded)?;
            files.push(prepared);
        }
        if retained_bytes > MAX_RETAINED_BYTES {
            return Err(WorkspaceEditError::RetentionExceeded);
        }
        Ok(PreparedWorkspaceEdit {
            files: files.into_boxed_slice(),
            retained_bytes,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PreparedTextEdit {
    range: std::ops::Range<usize>,
    new_text: Box<str>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PreparedFileEdit {
    path: PathBuf,
    lsp_version: Option<i32>,
    original: Box<str>,
    replacement: Box<str>,
    edits: Box<[PreparedTextEdit]>,
}

impl PreparedFileEdit {
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) const fn lsp_version(&self) -> Option<i32> {
        self.lsp_version
    }

    pub(crate) fn original(&self) -> &str {
        &self.original
    }

    pub(crate) fn replacement(&self) -> &str {
        &self.replacement
    }

    pub(crate) fn transaction_for(
        &self,
        snapshot: &BufferSnapshot,
    ) -> Result<Transaction, WorkspaceEditError> {
        if snapshot.text() != self.original.as_ref() {
            return Err(WorkspaceEditError::StaleFile);
        }
        let mut transaction = Transaction::new(snapshot.revision());
        for edit in &self.edits {
            transaction
                .replace(edit.range.clone(), edit.new_text.as_ref())
                .map_err(|_| WorkspaceEditError::InvalidRange)?;
        }
        Ok(transaction)
    }

    fn retained_bytes(&self) -> usize {
        size_of::<Self>()
            .saturating_add(self.path.as_os_str().len())
            .saturating_add(self.original.len())
            .saturating_add(self.replacement.len())
            .saturating_add(
                self.edits
                    .iter()
                    .map(|edit| size_of::<PreparedTextEdit>().saturating_add(edit.new_text.len()))
                    .sum::<usize>(),
            )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PreparedWorkspaceEdit {
    files: Box<[PreparedFileEdit]>,
    retained_bytes: usize,
}

impl PreparedWorkspaceEdit {
    pub(crate) fn files(&self) -> &[PreparedFileEdit] {
        &self.files
    }

    pub(crate) const fn retained_bytes(&self) -> usize {
        self.retained_bytes
    }
}

fn prepare_file(
    file: &WorkspaceFileEdit,
    original: String,
) -> Result<PreparedFileEdit, WorkspaceEditError> {
    let buffer = Buffer::new(&original);
    let snapshot = buffer.snapshot();
    let mut edits = Vec::new();
    edits
        .try_reserve_exact(file.edits.len())
        .map_err(|_| WorkspaceEditError::AllocationFailed)?;
    for edit in &file.edits {
        let range =
            byte_range(&snapshot, edit.range).map_err(|_| WorkspaceEditError::InvalidRange)?;
        edits.push(PreparedTextEdit {
            range,
            new_text: edit.new_text.clone(),
        });
    }
    edits.sort_unstable_by(|left, right| {
        left.range
            .start
            .cmp(&right.range.start)
            .then(left.range.end.cmp(&right.range.end))
    });
    if edits
        .windows(2)
        .any(|pair| byte_ranges_conflict(&pair[0].range, &pair[1].range))
    {
        return Err(WorkspaceEditError::OverlappingEdits);
    }
    let removed_bytes = edits
        .iter()
        .try_fold(0_usize, |bytes, edit| bytes.checked_add(edit.range.len()))
        .ok_or(WorkspaceEditError::RetentionExceeded)?;
    let inserted_bytes = edits
        .iter()
        .try_fold(0_usize, |bytes, edit| {
            bytes.checked_add(edit.new_text.len())
        })
        .ok_or(WorkspaceEditError::RetentionExceeded)?;
    let output_length = original
        .len()
        .checked_sub(removed_bytes)
        .and_then(|bytes| bytes.checked_add(inserted_bytes))
        .ok_or(WorkspaceEditError::RetentionExceeded)?;
    if output_length > MAX_FILE_TEXT_BYTES {
        return Err(WorkspaceEditError::FileTooLarge);
    }
    let mut replacement = String::new();
    replacement
        .try_reserve_exact(output_length)
        .map_err(|_| WorkspaceEditError::AllocationFailed)?;
    let mut cursor = 0_usize;
    for edit in &edits {
        replacement.push_str(
            original
                .get(cursor..edit.range.start)
                .ok_or(WorkspaceEditError::InvalidRange)?,
        );
        replacement.push_str(&edit.new_text);
        cursor = edit.range.end;
    }
    replacement.push_str(
        original
            .get(cursor..)
            .ok_or(WorkspaceEditError::InvalidRange)?,
    );
    Ok(PreparedFileEdit {
        path: file.path.clone(),
        lsp_version: file.lsp_version,
        original: original.into_boxed_str(),
        replacement: replacement.into_boxed_str(),
        edits: edits.into_boxed_slice(),
    })
}

fn checked_wire(result: &RawValue) -> Result<(), WorkspaceEditError> {
    if result.get().len() > MAX_WORKSPACE_EDIT_WIRE_BYTES {
        Err(WorkspaceEditError::WireTooLarge)
    } else {
        Ok(())
    }
}

fn strict_value(result: &RawValue) -> Result<Value, WorkspaceEditError> {
    serde_json::from_str::<StrictValue>(result.get())
        .map(|value| value.0)
        .map_err(|_| WorkspaceEditError::Malformed)
}

fn parse_changes(
    workspace_root: &Path,
    value: &Value,
) -> Result<Vec<WorkspaceFileEdit>, WorkspaceEditError> {
    let object = value.as_object().ok_or(WorkspaceEditError::Malformed)?;
    if object.len() > MAX_WORKSPACE_EDIT_FILES {
        return Err(WorkspaceEditError::TooManyFiles);
    }
    let mut files = Vec::new();
    files
        .try_reserve_exact(object.len())
        .map_err(|_| WorkspaceEditError::AllocationFailed)?;
    for (uri, edits) in object {
        let edits = edits.as_array().ok_or(WorkspaceEditError::Malformed)?;
        files.push(parse_file_edit(workspace_root, uri, None, edits)?);
    }
    Ok(files)
}

fn parse_document_changes(
    workspace_root: &Path,
    value: &Value,
) -> Result<Vec<WorkspaceFileEdit>, WorkspaceEditError> {
    let changes = value.as_array().ok_or(WorkspaceEditError::Malformed)?;
    if changes.len() > MAX_WORKSPACE_EDIT_FILES {
        return Err(WorkspaceEditError::TooManyFiles);
    }
    let mut files = Vec::new();
    files
        .try_reserve_exact(changes.len())
        .map_err(|_| WorkspaceEditError::AllocationFailed)?;
    for change in changes {
        let object = change
            .as_object()
            .ok_or(WorkspaceEditError::UnsupportedResourceOperation)?;
        if object.len() != 2
            || !object.contains_key("textDocument")
            || !object.contains_key("edits")
        {
            return Err(WorkspaceEditError::UnsupportedResourceOperation);
        }
        let document = object["textDocument"]
            .as_object()
            .ok_or(WorkspaceEditError::Malformed)?;
        if document.len() != 2 || !document.contains_key("uri") || !document.contains_key("version")
        {
            return Err(WorkspaceEditError::Malformed);
        }
        let uri = document["uri"]
            .as_str()
            .ok_or(WorkspaceEditError::Malformed)?;
        let version = match &document["version"] {
            Value::Null => None,
            Value::Number(number) => Some(
                i32::try_from(number.as_i64().ok_or(WorkspaceEditError::Malformed)?)
                    .map_err(|_| WorkspaceEditError::Malformed)?,
            ),
            _ => return Err(WorkspaceEditError::Malformed),
        };
        if version.is_some_and(|version| version < 0) {
            return Err(WorkspaceEditError::Malformed);
        }
        let edits = object["edits"]
            .as_array()
            .ok_or(WorkspaceEditError::Malformed)?;
        files.push(parse_file_edit(workspace_root, uri, version, edits)?);
    }
    Ok(files)
}

fn parse_file_edit(
    workspace_root: &Path,
    uri: &str,
    lsp_version: Option<i32>,
    values: &[Value],
) -> Result<WorkspaceFileEdit, WorkspaceEditError> {
    if uri.is_empty() || uri.len() > MAX_URI_BYTES {
        return Err(WorkspaceEditError::UriTooLong);
    }
    if values.len() > MAX_FILE_EDITS {
        return Err(WorkspaceEditError::TooManyEdits);
    }
    let path =
        resolve_local_file_uri(workspace_root, uri).map_err(WorkspaceEditError::LocalPath)?;
    let mut edits = Vec::new();
    edits
        .try_reserve_exact(values.len())
        .map_err(|_| WorkspaceEditError::AllocationFailed)?;
    for value in values {
        let object = value.as_object().ok_or(WorkspaceEditError::Malformed)?;
        if object.contains_key("annotationId") {
            return Err(WorkspaceEditError::UnsupportedAnnotation);
        }
        if object.len() != 2 || !object.contains_key("range") || !object.contains_key("newText") {
            return Err(WorkspaceEditError::Malformed);
        }
        let range = parse_range(&object["range"]).map_err(|_| WorkspaceEditError::InvalidRange)?;
        let new_text = object["newText"]
            .as_str()
            .ok_or(WorkspaceEditError::Malformed)?;
        if new_text.len() > MAX_EDIT_TEXT_BYTES {
            return Err(WorkspaceEditError::EditTextTooLong);
        }
        edits.push(WorkspaceTextEdit {
            range,
            new_text: new_text.into(),
        });
    }
    edits.sort_unstable_by_key(|edit| range_key(edit.range));
    if edits
        .windows(2)
        .any(|pair| lsp_ranges_conflict(pair[0].range, pair[1].range))
    {
        return Err(WorkspaceEditError::OverlappingEdits);
    }
    Ok(WorkspaceFileEdit {
        path,
        lsp_version,
        edits: edits.into_boxed_slice(),
    })
}

fn canonical_workspace_root(workspace_root: &Path) -> Result<PathBuf, WorkspaceEditError> {
    let metadata = std::fs::symlink_metadata(workspace_root)
        .map_err(|_| WorkspaceEditError::LocalPath(NavigationError::WorkspaceUnavailable))?;
    if metadata.file_type().is_symlink() {
        return Err(WorkspaceEditError::LocalPath(
            NavigationError::WorkspaceSymlink,
        ));
    }
    let root = std::fs::canonicalize(workspace_root)
        .map_err(|_| WorkspaceEditError::LocalPath(NavigationError::WorkspaceUnavailable))?;
    if !std::fs::metadata(&root)
        .map_err(|_| WorkspaceEditError::LocalPath(NavigationError::WorkspaceUnavailable))?
        .is_dir()
    {
        return Err(WorkspaceEditError::LocalPath(
            NavigationError::WorkspaceUnavailable,
        ));
    }
    Ok(root)
}

const fn range_key(range: LspRange) -> (u32, u32, u32, u32) {
    (
        range.start().line(),
        range.start().utf16_character(),
        range.end().line(),
        range.end().utf16_character(),
    )
}

fn lsp_ranges_conflict(left: LspRange, right: LspRange) -> bool {
    let left_start = (left.start().line(), left.start().utf16_character());
    let left_end = (left.end().line(), left.end().utf16_character());
    let right_start = (right.start().line(), right.start().utf16_character());
    right_start < left_end || right_start == left_start
}

#[cfg(kani)]
const fn bounded_ranges_conflict(left_start: u8, left_end: u8, right_start: u8) -> bool {
    right_start < left_end || right_start == left_start
}

fn byte_ranges_conflict(left: &std::ops::Range<usize>, right: &std::ops::Range<usize>) -> bool {
    right.start < left.end || right.start == left.start
}

struct StrictValue(Value);

impl<'de> Deserialize<'de> for StrictValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(StrictValueVisitor)
    }
}

struct StrictValueVisitor;

impl<'de> Visitor<'de> for StrictValueVisitor {
    type Value = StrictValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("strict JSON without duplicate object keys")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::Bool(value)))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::Number(Number::from(value))))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::Number(Number::from(value))))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Number::from_f64(value)
            .map(Value::Number)
            .map(StrictValue)
            .ok_or_else(|| E::custom("non-finite JSON number"))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::String(value.to_owned())))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::String(value)))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::Null))
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::Null))
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        StrictValue::deserialize(deserializer)
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element::<StrictValue>()? {
            values.push(value.0);
        }
        Ok(StrictValue(Value::Array(values)))
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut values = Map::new();
        while let Some(key) = map.next_key::<String>()? {
            if values.contains_key(&key) {
                return Err(de::Error::custom("duplicate JSON object key"));
            }
            values.insert(key, map.next_value::<StrictValue>()?.0);
        }
        Ok(StrictValue(Value::Object(values)))
    }
}

#[cfg(kani)]
mod proofs {
    use super::bounded_ranges_conflict;

    /// Task #220: sorted admitted ranges are strictly disjoint.
    #[kani::proof]
    fn accepted_sorted_ranges_are_strictly_disjoint() {
        let left_start = kani::any::<u8>();
        let left_end = kani::any::<u8>();
        let right_start = kani::any::<u8>();
        kani::assume(left_start <= left_end);
        kani::assume(left_start <= right_start);
        let conflicts = bounded_ranges_conflict(left_start, left_end, right_start);
        kani::cover!(right_start == left_end && left_start < left_end, "adjacent");
        kani::cover!(right_start > left_end, "separated");
        kani::cover!(
            right_start < left_end || right_start == left_start,
            "rejected"
        );
        if !conflicts {
            assert!(right_start >= left_end);
            assert!(right_start != left_start);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
    };

    use serde_json::value::RawValue;

    use super::*;

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

    struct Fixture {
        root: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "alpine-workspace-edit-{}-{sequence}",
                std::process::id()
            ));
            assert!(fs::create_dir(&root).is_ok());
            Self { root }
        }

        fn write(&self, name: &str, text: &str) -> PathBuf {
            let path = self.root.join(name);
            assert!(fs::write(&path, text).is_ok());
            path
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn raw(text: &str) -> Box<RawValue> {
        RawValue::from_string(text.to_owned()).unwrap_or_else(|_| unreachable!())
    }

    fn uri(path: &Path) -> String {
        format!("file://{}", path.display())
    }

    fn range(start: usize, end: usize, replacement: &str) -> String {
        format!(
            r#"{{"range":{{"start":{{"line":0,"character":{start}}},"end":{{"line":0,"character":{end}}}}},"newText":{}}}"#,
            serde_json::to_string(replacement).unwrap_or_else(|_| unreachable!())
        )
    }

    #[test]
    fn formatting_preparation_matches_the_independent_string_oracle() {
        let fixture = Fixture::new();
        let path = fixture.write("main.rs", "fn  main() {🙂}\n");
        let result = raw(&format!("[{},{}]", range(2, 4, " "), range(12, 14, "")));
        let proposal =
            WorkspaceEditProposal::admit_formatting(&result, &fixture.root, &uri(&path), 7)
                .unwrap_or_else(|_| unreachable!());
        assert_eq!(proposal.file_count(), 1);
        assert_eq!(proposal.edit_count(), 2);
        assert!(proposal.retained_bytes() <= MAX_RETAINED_BYTES);
        let prepared = proposal.prepare().unwrap_or_else(|_| unreachable!());
        assert_eq!(prepared.files().len(), 1);
        assert_eq!(
            prepared.files()[0].path(),
            fs::canonicalize(&path).unwrap_or_else(|_| unreachable!())
        );
        assert_eq!(prepared.files()[0].lsp_version(), Some(7));
        assert_eq!(prepared.files()[0].replacement(), "fn main() {}\n");
        let mut buffer = Buffer::new(prepared.files()[0].original());
        let transaction = prepared.files()[0]
            .transaction_for(&buffer.snapshot())
            .unwrap_or_else(|_| unreachable!());
        assert!(buffer.apply(transaction).is_ok());
        assert_eq!(buffer.snapshot().text(), prepared.files()[0].replacement());
        assert!(prepared.retained_bytes() <= MAX_RETAINED_BYTES);
    }

    #[test]
    fn rename_accepts_both_standard_shapes_and_orders_paths() {
        let fixture = Fixture::new();
        let first = fixture.write("a.rs", "let old = 1;\n");
        let second = fixture.write("b.rs", "old();\n");
        let changes = raw(&format!(
            r#"{{"changes":{{"{}":[{}],"{}":[{}]}}}}"#,
            uri(&second),
            range(0, 3, "new"),
            uri(&first),
            range(4, 7, "new")
        ));
        let proposal = WorkspaceEditProposal::admit_rename(&changes, &fixture.root)
            .unwrap_or_else(|_| unreachable!());
        let prepared = proposal.prepare().unwrap_or_else(|_| unreachable!());
        assert_eq!(
            prepared.files()[0].path(),
            fs::canonicalize(&first).unwrap_or_else(|_| unreachable!())
        );
        assert_eq!(
            prepared.files()[1].path(),
            fs::canonicalize(&second).unwrap_or_else(|_| unreachable!())
        );
        assert_eq!(prepared.files()[0].replacement(), "let new = 1;\n");
        assert_eq!(prepared.files()[1].replacement(), "new();\n");

        let document_changes = raw(&format!(
            r#"{{"documentChanges":[{{"textDocument":{{"uri":"{}","version":9}},"edits":[{}]}}]}}"#,
            uri(&first),
            range(4, 7, "next")
        ));
        let proposal = WorkspaceEditProposal::admit_rename(&document_changes, &fixture.root)
            .unwrap_or_else(|_| unreachable!());
        assert_eq!(
            proposal
                .prepare()
                .unwrap_or_else(|_| unreachable!())
                .files()[0]
                .lsp_version(),
            Some(9)
        );
    }

    #[test]
    fn malformed_duplicate_remote_and_resource_operations_fail_closed() {
        let fixture = Fixture::new();
        let path = fixture.write("main.rs", "old\n");
        let file_uri = uri(&path);
        let duplicate_key = raw(&format!(
            r#"{{"changes":{{"{file_uri}":[{}],"{file_uri}":[{}]}}}}"#,
            range(0, 1, "a"),
            range(1, 2, "b")
        ));
        assert_eq!(
            WorkspaceEditProposal::admit_rename(&duplicate_key, &fixture.root),
            Err(WorkspaceEditError::Malformed)
        );
        let duplicate_document = raw(&format!(
            r#"{{"documentChanges":[{{"textDocument":{{"uri":"{file_uri}","version":1}},"edits":[]}},{{"textDocument":{{"uri":"{file_uri}","version":1}},"edits":[]}}]}}"#
        ));
        assert_eq!(
            WorkspaceEditProposal::admit_rename(&duplicate_document, &fixture.root),
            Err(WorkspaceEditError::DuplicatePath)
        );
        let resource = raw(r#"{"documentChanges":[{"kind":"create","uri":"file:///tmp/new.rs"}]}"#);
        assert_eq!(
            WorkspaceEditProposal::admit_rename(&resource, &fixture.root),
            Err(WorkspaceEditError::UnsupportedResourceOperation)
        );
        let remote_uri = concat!("https:", "//example.com/main.rs");
        let remote = raw(&format!(
            r#"{{"changes":{{"{remote_uri}":[{}]}}}}"#,
            range(0, 1, "x")
        ));
        assert!(matches!(
            WorkspaceEditProposal::admit_rename(&remote, &fixture.root),
            Err(WorkspaceEditError::LocalPath(
                NavigationError::UnsupportedUri
            ))
        ));
        let annotated = raw(&format!(
            r#"{{"changes":{{"{file_uri}":[{{"range":{{"start":{{"line":0,"character":0}},"end":{{"line":0,"character":1}}}},"newText":"x","annotationId":"a"}}]}}}}"#
        ));
        assert_eq!(
            WorkspaceEditProposal::admit_rename(&annotated, &fixture.root),
            Err(WorkspaceEditError::UnsupportedAnnotation)
        );
    }

    #[test]
    fn overlap_staleness_and_utf16_surrogates_are_atomic_rejections() {
        let fixture = Fixture::new();
        let path = fixture.write("main.rs", "a🙂b\n");
        let file_uri = uri(&path);
        let overlap = raw(&format!(
            r#"{{"changes":{{"{file_uri}":[{},{}]}}}}"#,
            range(0, 2, "x"),
            range(1, 3, "y")
        ));
        assert_eq!(
            WorkspaceEditProposal::admit_rename(&overlap, &fixture.root),
            Err(WorkspaceEditError::OverlappingEdits)
        );
        let surrogate = raw(&format!(
            r#"{{"changes":{{"{file_uri}":[{}]}}}}"#,
            range(2, 3, "x")
        ));
        let proposal = WorkspaceEditProposal::admit_rename(&surrogate, &fixture.root)
            .unwrap_or_else(|_| unreachable!());
        assert_eq!(proposal.prepare(), Err(WorkspaceEditError::InvalidRange));

        let valid = raw(&format!(
            r#"{{"changes":{{"{file_uri}":[{}]}}}}"#,
            range(0, 1, "x")
        ));
        let proposal = WorkspaceEditProposal::admit_rename(&valid, &fixture.root)
            .unwrap_or_else(|_| unreachable!());
        let prepared = proposal.prepare().unwrap_or_else(|_| unreachable!());
        let stale = Buffer::new("changed").snapshot();
        assert_eq!(
            prepared.files()[0].transaction_for(&stale),
            Err(WorkspaceEditError::StaleFile)
        );
        assert_eq!(fs::read_to_string(path).ok().as_deref(), Some("a🙂b\n"));
    }

    #[test]
    fn exact_wire_and_collection_ceilings_are_enforced() {
        let fixture = Fixture::new();
        let path = fixture.write("main.rs", "x\n");
        let file_uri = uri(&path);
        let oversized_text = "x".repeat(MAX_EDIT_TEXT_BYTES + 1);
        let oversized_edit = raw(&format!(
            r#"{{"changes":{{"{file_uri}":[{}]}}}}"#,
            range(0, 1, &oversized_text)
        ));
        assert_eq!(
            WorkspaceEditProposal::admit_rename(&oversized_edit, &fixture.root),
            Err(WorkspaceEditError::EditTextTooLong)
        );
        let edits = std::iter::repeat_with(|| range(0, 0, ""))
            .take(MAX_FILE_EDITS + 1)
            .collect::<Vec<_>>()
            .join(",");
        let too_many = raw(&format!(r#"{{"changes":{{"{file_uri}":[{edits}]}}}}"#));
        assert_eq!(
            WorkspaceEditProposal::admit_rename(&too_many, &fixture.root),
            Err(WorkspaceEditError::TooManyEdits)
        );
        let oversized_wire = raw(&format!(
            "\"{}\"",
            "x".repeat(MAX_WORKSPACE_EDIT_WIRE_BYTES)
        ));
        assert_eq!(
            WorkspaceEditProposal::admit_rename(&oversized_wire, &fixture.root),
            Err(WorkspaceEditError::WireTooLarge)
        );
        assert_eq!(
            WorkspaceEditError::Malformed.to_string(),
            "Rust workspace edit rejected input: Malformed"
        );
    }

    #[test]
    fn deterministic_non_overlapping_edits_match_string_replacement() {
        let fixture = Fixture::new();
        let path = fixture.write("main.rs", "0123456789\n");
        for seed in 0..64_usize {
            let first = seed % 4;
            let second = 6 + seed % 3;
            let edits = raw(&format!(
                "[{},{}]",
                range(first, first + 1, "A"),
                range(second, second + 1, "B")
            ));
            let proposal =
                WorkspaceEditProposal::admit_formatting(&edits, &fixture.root, &uri(&path), 1)
                    .unwrap_or_else(|_| unreachable!());
            let prepared = proposal.prepare().unwrap_or_else(|_| unreachable!());
            let mut expected = String::from("0123456789\n");
            expected.replace_range(second..=second, "B");
            expected.replace_range(first..=first, "A");
            assert_eq!(prepared.files()[0].replacement(), expected);
        }
    }
}
