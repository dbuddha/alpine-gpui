//! Bounded LSP document identity and diagnostic admission.

use std::{error::Error, fmt, mem::size_of, path::Path};

use serde_json::{Value, value::RawValue};

const MAX_URI_BYTES: usize = 4_096;
const MAX_LANGUAGE_ID_BYTES: usize = 64;
const MAX_DOCUMENT_TEXT_BYTES: usize = 8_388_608;
const MAX_DIAGNOSTIC_WIRE_BYTES: usize = 1_048_576;
const MAX_DIAGNOSTICS: usize = 256;
const MAX_DIAGNOSTIC_MESSAGE_BYTES: usize = 4_096;
const MAX_DIAGNOSTIC_RETAINED_BYTES: usize = 262_144;
const MAX_POSITION_LINE: u32 = 10_000_000;
const MAX_POSITION_UTF16: u32 = 1_000_000;
const PINNED_SERVER_VERSION: &str = "rust-analyzer 0.3.3016-standalone (bb3bbbd9e4 2026-08-16)";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LanguageProtocolError {
    InvalidPath,
    UriTooLong,
    LanguageIdTooLong,
    DocumentTooLarge,
    InvalidVersion,
    InvalidPosition,
    InvalidRange,
    MalformedDiagnostics,
    DiagnosticWireTooLarge,
    TooManyDiagnostics,
    DiagnosticMessageTooLong,
    DiagnosticRetentionExceeded,
    DocumentMismatch,
    StaleDiagnostics,
    AllocationFailed,
}

impl fmt::Display for LanguageProtocolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "local language protocol rejected input: {self:?}"
        )
    }
}

impl Error for LanguageProtocolError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LspPosition {
    line: u32,
    utf16_character: u32,
}

impl LspPosition {
    pub(crate) const fn new(
        line: u32,
        utf16_character: u32,
    ) -> Result<Self, LanguageProtocolError> {
        if line > MAX_POSITION_LINE || utf16_character > MAX_POSITION_UTF16 {
            return Err(LanguageProtocolError::InvalidPosition);
        }
        Ok(Self {
            line,
            utf16_character,
        })
    }
}

#[derive(Debug)]
pub(crate) struct LspDocument {
    uri: Box<str>,
    language_id: Box<str>,
    version: i32,
}

impl LspDocument {
    pub(crate) fn from_file_path(
        path: &Path,
        language_id: &str,
        version: i32,
    ) -> Result<Self, LanguageProtocolError> {
        let uri = file_uri(path)?;
        Self::new(&uri, language_id, version)
    }

    fn new(uri: &str, language_id: &str, version: i32) -> Result<Self, LanguageProtocolError> {
        if uri.is_empty() || uri.len() > MAX_URI_BYTES {
            return Err(LanguageProtocolError::UriTooLong);
        }
        if language_id.is_empty() || language_id.len() > MAX_LANGUAGE_ID_BYTES {
            return Err(LanguageProtocolError::LanguageIdTooLong);
        }
        if version < 0 {
            return Err(LanguageProtocolError::InvalidVersion);
        }
        Ok(Self {
            uri: uri.into(),
            language_id: language_id.into(),
            version,
        })
    }

    pub(crate) fn did_open_params(
        &self,
        text: &str,
    ) -> Result<Box<RawValue>, LanguageProtocolError> {
        if text.len() > MAX_DOCUMENT_TEXT_BYTES {
            return Err(LanguageProtocolError::DocumentTooLarge);
        }
        raw_value(&serde_json::json!({
            "textDocument": {
                "uri": self.uri,
                "languageId": self.language_id,
                "version": self.version,
                "text": text,
            }
        }))
    }

    pub(crate) fn position_params(
        &self,
        position: LspPosition,
    ) -> Result<Box<RawValue>, LanguageProtocolError> {
        raw_value(&serde_json::json!({
            "textDocument": { "uri": self.uri },
            "position": {
                "line": position.line,
                "character": position.utf16_character,
            }
        }))
    }

    pub(crate) fn text_document_params(&self) -> Result<Box<RawValue>, LanguageProtocolError> {
        raw_value(&serde_json::json!({ "textDocument": { "uri": self.uri } }))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DiagnosticRange {
    start: LspPosition,
    end: LspPosition,
}

#[derive(Debug)]
struct Diagnostic {
    range: DiagnosticRange,
    severity: Option<u8>,
    message: Box<str>,
}

#[derive(Debug)]
pub(crate) struct DiagnosticBatch {
    uri: Box<str>,
    document_version: Option<i32>,
    diagnostics: Box<[Diagnostic]>,
    retained_bytes: usize,
}

impl DiagnosticBatch {
    pub(crate) fn admit(
        params: &RawValue,
        expected: &LspDocument,
    ) -> Result<Self, LanguageProtocolError> {
        if params.get().len() > MAX_DIAGNOSTIC_WIRE_BYTES {
            return Err(LanguageProtocolError::DiagnosticWireTooLarge);
        }
        let value: Value = serde_json::from_str(params.get())
            .map_err(|_| LanguageProtocolError::MalformedDiagnostics)?;
        let object = value
            .as_object()
            .ok_or(LanguageProtocolError::MalformedDiagnostics)?;
        let uri = object
            .get("uri")
            .and_then(Value::as_str)
            .ok_or(LanguageProtocolError::MalformedDiagnostics)?;
        if uri != expected.uri.as_ref() {
            return Err(LanguageProtocolError::DocumentMismatch);
        }
        let document_version = match object.get("version") {
            None | Some(Value::Null) => None,
            Some(value) => Some(
                i32::try_from(
                    value
                        .as_i64()
                        .ok_or(LanguageProtocolError::InvalidVersion)?,
                )
                .map_err(|_| LanguageProtocolError::InvalidVersion)?,
            ),
        };
        if document_version.is_some_and(|version| version != expected.version) {
            return Err(LanguageProtocolError::StaleDiagnostics);
        }
        let items = object
            .get("diagnostics")
            .and_then(Value::as_array)
            .ok_or(LanguageProtocolError::MalformedDiagnostics)?;
        if items.len() > MAX_DIAGNOSTICS {
            return Err(LanguageProtocolError::TooManyDiagnostics);
        }
        let mut diagnostics = Vec::new();
        diagnostics
            .try_reserve_exact(items.len())
            .map_err(|_| LanguageProtocolError::AllocationFailed)?;
        let mut retained_bytes = uri
            .len()
            .checked_add(items.len() * size_of::<Diagnostic>())
            .ok_or(LanguageProtocolError::DiagnosticRetentionExceeded)?;
        for item in items {
            let item = item
                .as_object()
                .ok_or(LanguageProtocolError::MalformedDiagnostics)?;
            let range = parse_range(
                item.get("range")
                    .ok_or(LanguageProtocolError::MalformedDiagnostics)?,
            )?;
            let message = item
                .get("message")
                .and_then(Value::as_str)
                .ok_or(LanguageProtocolError::MalformedDiagnostics)?;
            if message.len() > MAX_DIAGNOSTIC_MESSAGE_BYTES {
                return Err(LanguageProtocolError::DiagnosticMessageTooLong);
            }
            retained_bytes = retained_bytes
                .checked_add(message.len())
                .ok_or(LanguageProtocolError::DiagnosticRetentionExceeded)?;
            if retained_bytes > MAX_DIAGNOSTIC_RETAINED_BYTES {
                return Err(LanguageProtocolError::DiagnosticRetentionExceeded);
            }
            let severity = item
                .get("severity")
                .map(|value| {
                    u8::try_from(
                        value
                            .as_u64()
                            .ok_or(LanguageProtocolError::MalformedDiagnostics)?,
                    )
                    .map_err(|_| LanguageProtocolError::MalformedDiagnostics)
                })
                .transpose()?;
            diagnostics.push(Diagnostic {
                range,
                severity,
                message: message.into(),
            });
        }
        Ok(Self {
            uri: uri.into(),
            document_version,
            diagnostics: diagnostics.into_boxed_slice(),
            retained_bytes,
        })
    }

    pub(crate) const fn document_version(&self) -> Option<i32> {
        self.document_version
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.diagnostics.is_empty()
    }

    pub(crate) const fn retained_bytes(&self) -> usize {
        self.retained_bytes
    }
}

pub(crate) fn initialize_params(workspace: &Path) -> Result<Box<RawValue>, LanguageProtocolError> {
    let uri = file_uri(workspace)?;
    let name = workspace
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(LanguageProtocolError::InvalidPath)?;
    raw_value(&serde_json::json!({
        "processId": null,
        "clientInfo": { "name": "Alpine Studio", "version": "0.0.0" },
        "rootUri": uri,
        "capabilities": {
            "general": { "positionEncodings": ["utf-16"] },
            "textDocument": { "publishDiagnostics": { "versionSupport": true } }
        },
        "workspaceFolders": [{ "uri": uri, "name": name }]
    }))
}

pub(crate) const fn pinned_server_version() -> &'static str {
    PINNED_SERVER_VERSION
}

fn parse_range(value: &Value) -> Result<DiagnosticRange, LanguageProtocolError> {
    let object = value
        .as_object()
        .ok_or(LanguageProtocolError::MalformedDiagnostics)?;
    let start_value = object
        .get("start")
        .ok_or(LanguageProtocolError::MalformedDiagnostics)?;
    let start = parse_position(start_value)?;
    let end_value = object
        .get("end")
        .ok_or(LanguageProtocolError::MalformedDiagnostics)?;
    let end = parse_position(end_value)?;
    if (end.line, end.utf16_character) < (start.line, start.utf16_character) {
        return Err(LanguageProtocolError::InvalidRange);
    }
    Ok(DiagnosticRange { start, end })
}

fn parse_position(value: &Value) -> Result<LspPosition, LanguageProtocolError> {
    let object = value
        .as_object()
        .ok_or(LanguageProtocolError::MalformedDiagnostics)?;
    let line = u32::try_from(
        object
            .get("line")
            .and_then(Value::as_u64)
            .ok_or(LanguageProtocolError::InvalidPosition)?,
    )
    .map_err(|_| LanguageProtocolError::InvalidPosition)?;
    let character = u32::try_from(
        object
            .get("character")
            .and_then(Value::as_u64)
            .ok_or(LanguageProtocolError::InvalidPosition)?,
    )
    .map_err(|_| LanguageProtocolError::InvalidPosition)?;
    LspPosition::new(line, character)
}

fn raw_value(value: &Value) -> Result<Box<RawValue>, LanguageProtocolError> {
    RawValue::from_string(value.to_string()).map_err(|_| LanguageProtocolError::AllocationFailed)
}

fn file_uri(path: &Path) -> Result<String, LanguageProtocolError> {
    if !path.is_absolute() {
        return Err(LanguageProtocolError::InvalidPath);
    }
    let path = path.to_str().ok_or(LanguageProtocolError::InvalidPath)?;
    let mut uri = String::from("file://");
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'/' | b':') {
            uri.push(char::from(byte));
        } else {
            const HEX: &[u8; 16] = b"0123456789ABCDEF";
            uri.push('%');
            uri.push(char::from(HEX[usize::from(byte >> 4)]));
            uri.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
    }
    if uri.len() > MAX_URI_BYTES {
        return Err(LanguageProtocolError::UriTooLong);
    }
    Ok(uri)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(text: &str) -> Box<RawValue> {
        RawValue::from_string(text.to_owned()).unwrap_or_else(|_| unreachable!())
    }

    #[test]
    fn document_identity_serializes_utf16_lsp_contracts() -> Result<(), LanguageProtocolError> {
        let document = LspDocument::new("file:///tmp/a%20b.rs", "rust", 7)?;
        let open = document.did_open_params("fn main() {}")?;
        assert!(open.get().contains(r#""languageId":"rust""#));
        assert!(open.get().contains(r#""version":7"#));
        let positioned = document.position_params(LspPosition::new(3, 11)?)?;
        assert!(positioned.get().contains(r#""character":11"#));
        assert_eq!(
            LspPosition::new(MAX_POSITION_LINE + 1, 0),
            Err(LanguageProtocolError::InvalidPosition)
        );
        assert_eq!(
            LspDocument::new("", "rust", 0).err(),
            Some(LanguageProtocolError::UriTooLong)
        );
        assert_eq!(
            LspDocument::new("file:///a", "", 0).err(),
            Some(LanguageProtocolError::LanguageIdTooLong)
        );
        assert_eq!(
            LspDocument::new("file:///a", "rust", -1).err(),
            Some(LanguageProtocolError::InvalidVersion)
        );
        assert_eq!(
            document
                .did_open_params(&"x".repeat(MAX_DOCUMENT_TEXT_BYTES + 1))
                .err(),
            Some(LanguageProtocolError::DocumentTooLarge)
        );
        Ok(())
    }

    #[test]
    fn diagnostics_are_revision_checked_and_byte_accounted() -> Result<(), LanguageProtocolError> {
        let document = LspDocument::new("file:///tmp/main.rs", "rust", 3)?;
        let params = raw(
            r#"{"uri":"file:///tmp/main.rs","version":3,"diagnostics":[{"range":{"start":{"line":1,"character":2},"end":{"line":1,"character":4}},"severity":1,"message":"broken"}]}"#,
        );
        let batch = DiagnosticBatch::admit(&params, &document)?;
        assert_eq!(batch.uri.as_ref(), "file:///tmp/main.rs");
        assert_eq!(batch.document_version(), Some(3));
        assert_eq!(batch.diagnostics.len(), 1);
        assert_eq!(batch.diagnostics[0].severity, Some(1));
        assert_eq!(batch.diagnostics[0].message.as_ref(), "broken");
        assert_eq!(batch.diagnostics[0].range.start, LspPosition::new(1, 2)?);
        assert_eq!(
            batch.retained_bytes(),
            "file:///tmp/main.rs".len() + size_of::<Diagnostic>() + 6
        );
        let stale = raw(r#"{"uri":"file:///tmp/main.rs","version":2,"diagnostics":[]}"#);
        assert_eq!(
            DiagnosticBatch::admit(&stale, &document).err(),
            Some(LanguageProtocolError::StaleDiagnostics)
        );
        let wrong = raw(r#"{"uri":"file:///tmp/other.rs","diagnostics":[]}"#);
        assert_eq!(
            DiagnosticBatch::admit(&wrong, &document).err(),
            Some(LanguageProtocolError::DocumentMismatch)
        );
        Ok(())
    }

    #[test]
    fn diagnostics_fail_closed_at_every_independent_bound() {
        let document =
            LspDocument::new("file:///tmp/main.rs", "rust", 1).unwrap_or_else(|_| unreachable!());
        let malformed =
            raw(r#"{"uri":"file:///tmp/main.rs","diagnostics":[{"message":"missing range"}]}"#);
        assert_eq!(
            DiagnosticBatch::admit(&malformed, &document).err(),
            Some(LanguageProtocolError::MalformedDiagnostics)
        );
        let reversed = raw(
            r#"{"uri":"file:///tmp/main.rs","diagnostics":[{"range":{"start":{"line":2,"character":0},"end":{"line":1,"character":0}},"message":"bad"}]}"#,
        );
        assert_eq!(
            DiagnosticBatch::admit(&reversed, &document).err(),
            Some(LanguageProtocolError::InvalidRange)
        );
        let long_message = "x".repeat(MAX_DIAGNOSTIC_MESSAGE_BYTES + 1);
        let long = raw(&format!(
            r#"{{"uri":"file:///tmp/main.rs","diagnostics":[{{"range":{{"start":{{"line":0,"character":0}},"end":{{"line":0,"character":1}}}},"message":"{long_message}"}}]}}"#
        ));
        assert_eq!(
            DiagnosticBatch::admit(&long, &document).err(),
            Some(LanguageProtocolError::DiagnosticMessageTooLong)
        );
        let item = r#"{"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":1}},"message":"x"}"#;
        let too_many = raw(&format!(
            r#"{{"uri":"file:///tmp/main.rs","diagnostics":[{}]}}"#,
            std::iter::repeat_n(item, MAX_DIAGNOSTICS + 1)
                .collect::<Vec<_>>()
                .join(",")
        ));
        assert_eq!(
            DiagnosticBatch::admit(&too_many, &document).err(),
            Some(LanguageProtocolError::TooManyDiagnostics)
        );
        let too_large = raw(&format!(
            r#"{{"padding":"{}"}}"#,
            "x".repeat(MAX_DIAGNOSTIC_WIRE_BYTES)
        ));
        assert_eq!(
            DiagnosticBatch::admit(&too_large, &document).err(),
            Some(LanguageProtocolError::DiagnosticWireTooLarge)
        );
        assert_eq!(
            file_uri(Path::new("relative.rs")).err(),
            Some(LanguageProtocolError::InvalidPath)
        );
        assert!(!pinned_server_version().is_empty());
    }
}

#[cfg(test)]
#[path = "lsp_language_coverage_tests.rs"]
mod coverage_tests;
