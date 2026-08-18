use std::path::{Path, PathBuf};

use super::*;

fn raw(text: &str) -> Box<RawValue> {
    RawValue::from_string(text.to_owned()).unwrap_or_else(|_| unreachable!())
}

fn absolute_path(name: &str) -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(format!(r"C:\tmp\{name}"))
    } else {
        PathBuf::from(format!("/tmp/{name}"))
    }
}

fn root_path() -> &'static Path {
    if cfg!(windows) {
        Path::new(r"C:\")
    } else {
        Path::new("/")
    }
}

fn diagnostic_params(range: &str, suffix: &str) -> Box<RawValue> {
    raw(&format!(
        r#"{{"uri":"file:///tmp/main.rs","diagnostics":[{{"range":{range},"message":"x"{suffix}}}]}}"#
    ))
}

#[test]
fn errors_documents_and_initialization_preserve_exact_contracts()
-> Result<(), LanguageProtocolError> {
    for error in [
        LanguageProtocolError::InvalidPath,
        LanguageProtocolError::UriTooLong,
        LanguageProtocolError::LanguageIdTooLong,
        LanguageProtocolError::DocumentTooLarge,
        LanguageProtocolError::InvalidVersion,
        LanguageProtocolError::InvalidPosition,
        LanguageProtocolError::InvalidRange,
        LanguageProtocolError::MalformedDiagnostics,
        LanguageProtocolError::DiagnosticWireTooLarge,
        LanguageProtocolError::TooManyDiagnostics,
        LanguageProtocolError::DiagnosticMessageTooLong,
        LanguageProtocolError::DiagnosticRetentionExceeded,
        LanguageProtocolError::DocumentMismatch,
        LanguageProtocolError::StaleDiagnostics,
        LanguageProtocolError::AllocationFailed,
    ] {
        assert!(error.to_string().contains(&format!("{error:?}")));
        assert!(error.source().is_none());
    }

    let path = absolute_path("a b.rs");
    let document = LspDocument::from_file_path(&path, "rust", 9)?;
    assert!(document.uri.contains("a%20b.rs"));
    assert!(
        document
            .did_open_params(&"x".repeat(MAX_DOCUMENT_TEXT_BYTES))?
            .get()
            .contains(r#""version":9"#)
    );
    assert_eq!(
        document.text_document_params()?.get(),
        format!(r#"{{"textDocument":{{"uri":"{}"}}}}"#, document.uri)
    );

    let workspace = absolute_path("alpine workspace");
    let initialize = initialize_params(&workspace)?;
    assert!(
        initialize
            .get()
            .contains(r#""positionEncodings":["utf-16"]"#)
    );
    assert!(initialize.get().contains(r#""name":"alpine workspace""#));
    assert!(initialize.get().contains("alpine%20workspace"));
    assert_eq!(
        initialize_params(root_path()).err(),
        Some(LanguageProtocolError::InvalidPath)
    );

    let empty = raw(&format!(
        r#"{{"uri":"{}","version":null,"diagnostics":[]}}"#,
        document.uri
    ));
    let batch = DiagnosticBatch::admit(&empty, &document)?;
    assert!(batch.is_empty());
    assert_eq!(batch.document_version(), None);
    assert_eq!(batch.retained_bytes(), document.uri.len());
    assert_eq!(
        pinned_server_version(),
        "rust-analyzer 0.3.3016-standalone (bb3bbbd9e4 2026-08-16)"
    );
    Ok(())
}

#[test]
fn diagnostic_shape_position_and_retention_rejections_are_independent() {
    let document =
        LspDocument::new("file:///tmp/main.rs", "rust", 1).unwrap_or_else(|_| unreachable!());
    for (params, expected) in [
        (
            diagnostic_params(r"{}", ""),
            LanguageProtocolError::MalformedDiagnostics,
        ),
        (
            diagnostic_params(r#"{"start":{"line":0,"character":0}}"#, ""),
            LanguageProtocolError::MalformedDiagnostics,
        ),
        (
            diagnostic_params(r#"{"start":{"line":0,"character":0},"end":{"line":0}}"#, ""),
            LanguageProtocolError::InvalidPosition,
        ),
        (
            diagnostic_params(
                r#"{"start":{"line":0,"character":0},"end":{"line":0,"character":0}}"#,
                r#","severity":256"#,
            ),
            LanguageProtocolError::MalformedDiagnostics,
        ),
    ] {
        assert_eq!(
            DiagnosticBatch::admit(&params, &document).err(),
            Some(expected)
        );
    }

    let message = "x".repeat(1_024);
    let item = format!(
        r#"{{"range":{{"start":{{"line":0,"character":0}},"end":{{"line":0,"character":1}}}},"message":"{message}"}}"#
    );
    let retained = raw(&format!(
        r#"{{"uri":"file:///tmp/main.rs","diagnostics":[{}]}}"#,
        std::iter::repeat_n(item.as_str(), MAX_DIAGNOSTICS)
            .collect::<Vec<_>>()
            .join(",")
    ));
    assert_eq!(
        DiagnosticBatch::admit(&retained, &document).err(),
        Some(LanguageProtocolError::DiagnosticRetentionExceeded)
    );
}

#[test]
fn diagnostic_wire_message_and_retention_limits_accept_exact_boundaries()
-> Result<(), LanguageProtocolError> {
    let document = LspDocument::new("file:///tmp/main.rs", "rust", 1)?;
    let wire_prefix = r#"{"uri":"file:///tmp/main.rs","diagnostics":[],"padding":""#;
    let wire_suffix = r#""}"#;
    let wire_padding = MAX_DIAGNOSTIC_WIRE_BYTES - wire_prefix.len() - wire_suffix.len();
    let wire = raw(&format!(
        "{wire_prefix}{}{wire_suffix}",
        "x".repeat(wire_padding)
    ));
    assert_eq!(wire.get().len(), MAX_DIAGNOSTIC_WIRE_BYTES);
    assert!(DiagnosticBatch::admit(&wire, &document)?.is_empty());

    let exact_message = "x".repeat(MAX_DIAGNOSTIC_MESSAGE_BYTES);
    let message = raw(&format!(
        r#"{{"uri":"file:///tmp/main.rs","diagnostics":[{{"range":{{"start":{{"line":0,"character":0}},"end":{{"line":0,"character":1}}}},"message":"{exact_message}"}}]}}"#
    ));
    let message_batch = DiagnosticBatch::admit(&message, &document)?;
    assert!(!message_batch.is_empty());
    assert_eq!(
        message_batch.diagnostics[0].message.len(),
        MAX_DIAGNOSTIC_MESSAGE_BYTES
    );

    let fixed_bytes = document.uri.len() + MAX_DIAGNOSTICS * size_of::<Diagnostic>();
    let message_bytes = MAX_DIAGNOSTIC_RETAINED_BYTES - fixed_bytes;
    let base_message_bytes = message_bytes / MAX_DIAGNOSTICS;
    let remainder = message_bytes % MAX_DIAGNOSTICS;
    let items = (0..MAX_DIAGNOSTICS)
        .map(|index| {
            let length = base_message_bytes + usize::from(index < remainder);
            let text = "x".repeat(length);
            format!(
                r#"{{"range":{{"start":{{"line":0,"character":0}},"end":{{"line":0,"character":1}}}},"message":"{text}"}}"#
            )
        })
        .collect::<Vec<_>>();
    let retained = raw(&format!(
        r#"{{"uri":"file:///tmp/main.rs","diagnostics":[{}]}}"#,
        items.join(",")
    ));
    let retained_batch = DiagnosticBatch::admit(&retained, &document)?;
    assert!(!retained_batch.is_empty());
    assert_eq!(
        retained_batch.retained_bytes(),
        MAX_DIAGNOSTIC_RETAINED_BYTES
    );
    Ok(())
}

#[test]
fn file_uri_encoding_and_length_are_bounded() {
    let encoded = file_uri(&absolute_path("a b#é.rs")).unwrap_or_else(|_| unreachable!());
    assert!(encoded.contains("a%20b%23%C3%A9.rs"));

    let exact_prefix = if cfg!(windows) { "C:/" } else { "/" };
    let exact = format!(
        "{exact_prefix}{}",
        "x".repeat(MAX_URI_BYTES - "file://".len() - exact_prefix.len())
    );
    let exact_uri = file_uri(Path::new(&exact)).unwrap_or_else(|_| unreachable!());
    assert_eq!(exact_uri.len(), MAX_URI_BYTES);

    let mut oversized = if cfg!(windows) {
        String::from(r"C:\")
    } else {
        String::from("/")
    };
    oversized.push_str(&"x".repeat(MAX_URI_BYTES));
    assert_eq!(
        file_uri(Path::new(&oversized)).err(),
        Some(LanguageProtocolError::UriTooLong)
    );
}

#[cfg(unix)]
#[test]
fn non_utf8_file_path_is_rejected_before_uri_allocation() {
    use std::{ffi::OsStr, os::unix::ffi::OsStrExt};

    let path = Path::new(OsStr::from_bytes(b"/tmp/\xff.rs"));
    assert_eq!(
        file_uri(path).err(),
        Some(LanguageProtocolError::InvalidPath)
    );
}
