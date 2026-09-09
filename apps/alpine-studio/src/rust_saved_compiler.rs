//! Disk-based compiler reports, never current-overlay diagnostic authority.

use std::{mem::size_of, sync::Arc};

use serde_json::{Value, value::RawValue};

use crate::lsp_language::{DiagnosticBatch, LanguageProtocolError};

const MAX_WIRE_BYTES: usize = 1_048_576;
const MAX_URI_BYTES: usize = 4_096;
const MAX_ITEMS: usize = 256;
const MAX_CODE_BYTES: usize = 128;
const MAX_REPORT_BYTES: usize = 262_144;
pub(super) const MAX_WORKSPACE_BYTES: usize = 1_048_576;
pub(super) const MAX_WORKSPACE_ITEMS: usize = 1_024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct SavedCompilerReport {
    batch: DiagnosticBatch,
    // Publication metadata only: the compiler can have checked older disk text.
    reported_version: Option<i32>,
    codes: Box<[Option<Box<str>>]>,
    status: Option<Arc<str>>,
    retained_bytes: usize,
}

impl SavedCompilerReport {
    pub(super) fn parse(params: &RawValue) -> Result<Option<Self>, LanguageProtocolError> {
        if params.get().len() > MAX_WIRE_BYTES {
            return Err(LanguageProtocolError::DiagnosticWireTooLarge);
        }
        let mut value = crate::lsp_value::parse(params)
            .map_err(|_| LanguageProtocolError::MalformedDiagnostics)?;
        let object = value
            .as_object_mut()
            .ok_or(LanguageProtocolError::MalformedDiagnostics)?;
        let uri = object
            .get("uri")
            .and_then(Value::as_str)
            .ok_or(LanguageProtocolError::MalformedDiagnostics)?;
        if uri.len() > MAX_URI_BYTES {
            return Err(LanguageProtocolError::UriTooLong);
        }
        if !uri.starts_with("file:///") {
            return Err(LanguageProtocolError::DocumentMismatch);
        }
        let uri = Box::<str>::from(uri);
        let reported_version = match object.get("version") {
            None | Some(Value::Null) => None,
            Some(version) => Some(
                i32::try_from(
                    version
                        .as_i64()
                        .ok_or(LanguageProtocolError::InvalidVersion)?,
                )
                .map_err(|_| LanguageProtocolError::InvalidVersion)?,
            ),
        };
        let items = object
            .get_mut("diagnostics")
            .and_then(Value::as_array_mut)
            .ok_or(LanguageProtocolError::MalformedDiagnostics)?;
        if items.len() > MAX_ITEMS {
            return Err(LanguageProtocolError::TooManyDiagnostics);
        }
        let clear = items.is_empty();
        for item in items.iter() {
            let item = item
                .as_object()
                .ok_or(LanguageProtocolError::MalformedDiagnostics)?;
            if item.get("source").is_some_and(|source| !source.is_string()) {
                return Err(LanguageProtocolError::MalformedDiagnostics);
            }
        }
        items.retain(|item| item.get("source").and_then(Value::as_str) == Some("rustc"));
        // A non-compiler publication is not a compiler-clear notification.
        if !clear && items.is_empty() {
            return Ok(None);
        }
        let codes = diagnostic_codes(items)?;
        let batch = DiagnosticBatch::from_saved_items(&uri, items)?;
        let status = saved_status(&batch, &codes);
        let retained_bytes = size_of::<Self>()
            + batch.retained_bytes()
            + codes.len() * size_of::<Option<Box<str>>>()
            + codes.iter().flatten().map(|code| code.len()).sum::<usize>()
            + status
                .as_ref()
                .map_or(0, |status| status.len() + 2 * size_of::<usize>());
        if retained_bytes > MAX_REPORT_BYTES {
            return Err(LanguageProtocolError::DiagnosticRetentionExceeded);
        }
        Ok(Some(Self {
            batch,
            reported_version,
            codes,
            status,
            retained_bytes,
        }))
    }

    pub(super) fn uri(&self) -> &str {
        self.batch.uri()
    }

    pub(super) fn items(&self) -> usize {
        self.batch.diagnostics().len()
    }

    pub(super) const fn retained_bytes(&self) -> usize {
        self.retained_bytes
    }

    pub(super) fn status(&self) -> Option<Arc<str>> {
        self.status.clone()
    }
}

fn diagnostic_codes(items: &[Value]) -> Result<Box<[Option<Box<str>>]>, LanguageProtocolError> {
    let mut codes = Vec::new();
    codes
        .try_reserve_exact(items.len())
        .map_err(|_| LanguageProtocolError::AllocationFailed)?;
    for item in items {
        if item.get("severity").is_some_and(|value| {
            !value
                .as_u64()
                .is_some_and(|severity| (1..=4).contains(&severity))
        }) {
            return Err(LanguageProtocolError::MalformedDiagnostics);
        }
        let code = match item.get("code") {
            None | Some(Value::Null) => None,
            Some(Value::String(code)) if code.len() <= MAX_CODE_BYTES => {
                Some(Box::from(code.as_str()))
            }
            Some(Value::Number(code)) => Some(
                i32::try_from(
                    code.as_i64()
                        .ok_or(LanguageProtocolError::MalformedDiagnostics)?,
                )
                .map_err(|_| LanguageProtocolError::MalformedDiagnostics)?
                .to_string()
                .into_boxed_str(),
            ),
            Some(_) => return Err(LanguageProtocolError::MalformedDiagnostics),
        };
        codes.push(code);
    }
    Ok(codes.into_boxed_slice())
}

fn saved_status(batch: &DiagnosticBatch, codes: &[Option<Box<str>>]) -> Option<Arc<str>> {
    let (index, diagnostic) = batch
        .diagnostics()
        .iter()
        .enumerate()
        .min_by_key(|(_, diagnostic)| diagnostic.severity().unwrap_or(1))?;
    let code = codes[index].as_deref().unwrap_or("diagnostic");
    Some(Arc::from(format!(
        "Saved compiler (not current buffer): {code} at {}:{}: {}",
        diagnostic.start().line() + 1,
        diagnostic.start().utf16_character() + 1,
        diagnostic.message(),
    )))
}

#[cfg(test)]
mod tests {
    use super::super::{
        LanguageIdentity, RustDiagnostics, RustDocumentInput, RustSession, workspace,
    };
    use super::*;
    use std::{
        error::Error,
        path::{Path, PathBuf},
    };

    fn item(source: &str) -> Value {
        serde_json::json!({"source":source,"code":"E0502","severity":1,
            "range":{"start":{"line":3,"character":4},"end":{"line":3,"character":18}},
            "message":"cannot borrow `values` as mutable"})
    }

    fn report(uri: &str, items: Vec<Value>) -> Result<SavedCompilerReport, Box<dyn Error>> {
        let raw = serde_json::value::to_raw_value(&serde_json::json!({
            "uri":uri,"version":2,"diagnostics":Value::Array(items)}))?;
        SavedCompilerReport::parse(&raw)?.ok_or_else(|| "missing compiler report".into())
    }

    fn component_ledger(report: &SavedCompilerReport) -> usize {
        // The batch has its own ledger. Independently sum this wrapper's
        // components without consulting its cached counter or claiming RSS.
        let code_text: usize = report.codes.iter().flatten().map(|code| code.len()).sum();
        let status = report.status.as_ref().map_or(0, |text| {
            [text.len(), size_of::<usize>(), size_of::<usize>()]
                .into_iter()
                .sum()
        });
        [
            std::mem::size_of_val(report),
            report.batch.retained_bytes(),
            std::mem::size_of_val(report.codes.as_ref()),
            code_text,
            status,
        ]
        .into_iter()
        .sum()
    }

    #[test]
    fn compiler_report_wire_admission_has_exact_boundaries() -> Result<(), Box<dyn Error>> {
        let body = r#"{"uri":"file:///tmp/compiler-1.rs","diagnostics":[]}"#;
        for (length, admitted) in [
            (MAX_WIRE_BYTES - 1, true),
            (MAX_WIRE_BYTES, true),
            (MAX_WIRE_BYTES + 1, false),
        ] {
            // Interior JSON whitespace changes wire size, not URI, string,
            // diagnostic count, nesting, or retained-report size.
            let raw = RawValue::from_string(format!(
                "{{{}{}",
                " ".repeat(length - body.len()),
                &body[1..]
            ))?;
            assert_eq!(raw.get().len(), length);
            let parsed = SavedCompilerReport::parse(&raw);
            if admitted {
                assert_eq!(parsed?.ok_or("missing empty report")?.items(), 0);
            } else {
                assert_eq!(
                    parsed.err(),
                    Some(LanguageProtocolError::DiagnosticWireTooLarge)
                );
            }
        }
        Ok(())
    }

    #[test]
    fn compiler_report_uri_admission_has_exact_boundaries() -> Result<(), Box<dyn Error>> {
        for (length, admitted) in [
            (MAX_URI_BYTES - 1, true),
            (MAX_URI_BYTES, true),
            (MAX_URI_BYTES + 1, false),
        ] {
            let uri = format!("file:///{}", "x".repeat(length - "file:///".len()));
            let raw = serde_json::value::to_raw_value(&serde_json::json!({
                "uri":uri,"diagnostics":[]}))?;
            let parsed = SavedCompilerReport::parse(&raw);
            if admitted {
                let parsed = parsed?.ok_or("missing empty report")?;
                assert_eq!(parsed.uri(), uri);
                assert_eq!(parsed.items(), 0);
            } else {
                assert_eq!(parsed.err(), Some(LanguageProtocolError::UriTooLong));
            }
        }
        Ok(())
    }

    #[test]
    fn compiler_report_retention_matches_component_ledger() -> Result<(), Box<dyn Error>> {
        let mut no_code = item("rustc");
        no_code["code"] = Value::Null;
        let mut numeric_code = item("rustc");
        numeric_code["code"] = serde_json::json!(-123);
        let mut unicode_code = item("rustc");
        unicode_code["code"] = Value::String("\u{e9}".repeat(MAX_CODE_BYTES / 2));
        for diagnostics in [
            vec![],
            vec![item("rustc")],
            vec![no_code, numeric_code, unicode_code],
        ] {
            let parsed = report("file:///tmp/compiler-1.rs", diagnostics)?;
            assert_eq!(parsed.retained_bytes(), component_ledger(&parsed));
        }
        Ok(())
    }

    #[test]
    fn compiler_report_retention_limit_has_exact_boundaries() -> Result<(), Box<dyn Error>> {
        let uri = "file:///tmp/compiler-1.rs";
        let mut diagnostic = item("rustc");
        diagnostic["message"] = Value::String("x".repeat(3_930));
        let diagnostics = vec![diagnostic; 64];
        let baseline = report(uri, diagnostics.clone())?;
        let padding = MAX_REPORT_BYTES
            .checked_sub(component_ledger(&baseline))
            .ok_or("baseline exceeds retained-report limit")?;
        assert!(padding > 0);
        assert!(uri.len() + padding < MAX_URI_BYTES);
        for (extra, admitted) in [(padding - 1, true), (padding, true), (padding + 1, false)] {
            let padded_uri = format!("{uri}{}", "x".repeat(extra));
            let raw = serde_json::value::to_raw_value(&serde_json::json!({
                "uri":padded_uri,"version":2,"diagnostics":diagnostics}))?;
            assert!(raw.get().len() < MAX_WIRE_BYTES);
            let parsed = SavedCompilerReport::parse(&raw);
            if admitted {
                let parsed = parsed?.ok_or("missing compiler report")?;
                let expected = component_ledger(&baseline) + extra;
                assert_eq!(component_ledger(&parsed), expected);
                assert_eq!(parsed.retained_bytes(), expected);
            } else {
                assert_eq!(
                    parsed.err(),
                    Some(LanguageProtocolError::DiagnosticRetentionExceeded)
                );
            }
        }
        Ok(())
    }

    fn report_at_limit(uri: &str, marker: char) -> Result<SavedCompilerReport, Box<dyn Error>> {
        let mut diagnostic = item("rustc");
        diagnostic["severity"] = serde_json::json!(2);
        diagnostic["message"] = Value::String("x".repeat(3_930));
        let mut diagnostics = vec![diagnostic; 64];
        diagnostics[0]["severity"] = serde_json::json!(1);
        diagnostics[0]["message"] = Value::String(format!("{marker}{}", "x".repeat(3_929)));
        let baseline = report(uri, diagnostics.clone())?;
        let mut remaining = MAX_REPORT_BYTES
            .checked_sub(component_ledger(&baseline))
            .ok_or("baseline exceeds report budget")?;
        // Keep URI and the primary status unchanged. Only non-primary message
        // payloads grow, each within the existing 4096-byte diagnostic limit.
        for diagnostic in diagnostics.iter_mut().skip(1) {
            let added = remaining.min(4_096 - 3_930);
            diagnostic["message"] = Value::String("x".repeat(3_930 + added));
            remaining -= added;
        }
        assert_eq!(remaining, 0);
        let report = report(uri, diagnostics)?;
        assert_eq!(component_ledger(&report), MAX_REPORT_BYTES);
        assert_eq!(report.retained_bytes(), MAX_REPORT_BYTES);
        assert_eq!(report.items(), 64);
        Ok(report)
    }

    fn assert_compiler_owner(
        session: &mut RustSession,
        id: u64,
        marker: char,
    ) -> Result<(), Box<dyn Error>> {
        session.park_and_activate(input(id), true)?;
        let report = session
            .saved_compiler
            .as_ref()
            .ok_or("missing owned report")?;
        assert_eq!(report.uri(), document_uri(id)?);
        assert!(
            report
                .batch
                .primary_message()
                .ok_or("missing primary diagnostic")?
                .starts_with(marker)
        );
        assert_eq!(report.items(), 64);
        assert_eq!(report.retained_bytes(), MAX_REPORT_BYTES);
        Ok(())
    }

    #[test]
    fn workspace_byte_pressure_replaces_and_clears_by_owner() -> Result<(), Box<dyn Error>> {
        let mut model = model()?;
        let session = model.session.as_mut().ok_or("missing session")?;
        for id in 1..=4 {
            if id != 1 {
                session.park_and_activate(input(id), true)?;
            }
            let report = report_at_limit(&document_uri(id)?, 'a')?;
            assert!(route(session, report)?);
        }
        let full = (256, MAX_WORKSPACE_BYTES);
        assert_eq!(
            workspace::saved_compiler_counts(session.saved_compiler.as_ref(), &session.parked),
            full
        );
        let active = report_at_limit(&document_uri(4)?, 'b')?;
        assert!(route(session, active)?);
        let inactive = report_at_limit(&document_uri(2)?, 'c')?;
        assert!(!route(session, inactive)?);
        for (id, marker) in [(1, 'a'), (2, 'c'), (3, 'a'), (4, 'b')] {
            assert_compiler_owner(session, id, marker)?;
        }
        session.park_and_activate(input(5), true)?;
        assert_eq!(
            route(session, report(&document_uri(5)?, vec![item("rustc")])?).err(),
            Some(LanguageProtocolError::DiagnosticRetentionExceeded)
        );
        assert!(session.saved_compiler.is_none());
        for (id, marker) in [(1, 'a'), (2, 'c'), (3, 'a'), (4, 'b')] {
            assert_compiler_owner(session, id, marker)?;
        }
        session.park_and_activate(input(5), true)?;
        assert!(!route(session, report(&document_uri(5)?, vec![])?)?);
        assert_eq!(
            workspace::saved_compiler_counts(session.saved_compiler.as_ref(), &session.parked),
            full
        );
        assert!(!route(session, report(&document_uri(2)?, vec![])?)?);
        let after_clear = (192, MAX_WORKSPACE_BYTES - MAX_REPORT_BYTES);
        assert_eq!(
            workspace::saved_compiler_counts(session.saved_compiler.as_ref(), &session.parked),
            after_clear
        );
        assert!(!route(session, report(&document_uri(2)?, vec![])?)?);
        session.park_and_activate(input(2), true)?;
        assert!(session.saved_compiler.is_none());
        session.park_and_activate(input(5), true)?;
        let replacement = report_at_limit(&document_uri(2)?, 'd')?;
        assert!(!route(session, replacement)?);
        for (id, marker) in [(1, 'a'), (2, 'd'), (3, 'a'), (4, 'b')] {
            assert_compiler_owner(session, id, marker)?;
        }
        assert_eq!(
            workspace::saved_compiler_counts(session.saved_compiler.as_ref(), &session.parked),
            full
        );
        Ok(())
    }

    fn input(id: u64) -> RustDocumentInput {
        let snapshot = alpine_text::Buffer::new("fn main() {}\n").snapshot();
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("saved-compiler-fixture");
        RustDocumentInput::new(
            &root.join(format!("compiler-{id}.rs")),
            &root,
            LanguageIdentity {
                workspace_id: 1,
                workspace_revision: 1,
                document_id: id,
                document_revision: 1,
                buffer_revision: snapshot.revision().get(),
                selection_revision: 1,
            },
            snapshot,
        )
    }

    fn document_uri(id: u64) -> Result<String, Box<dyn Error>> {
        Ok(
            crate::lsp_language::LspDocument::from_file_path(&input(id).path, "rust", 1)?
                .uri()
                .to_owned(),
        )
    }

    #[test]
    fn compiler_fixture_paths_are_absolute_and_document_uris_are_distinct()
    -> Result<(), Box<dyn Error>> {
        assert!(input(1).path.is_absolute());
        assert!(input(2).path.is_absolute());
        assert_ne!(document_uri(1)?, document_uri(2)?);
        Ok(())
    }

    fn model() -> Result<RustDiagnostics, Box<dyn Error>> {
        let input = input(1);
        let document = crate::lsp_language::LspDocument::from_file_path(&input.path, "rust", 1)?;
        let native = DiagnosticBatch::from_saved_items(document.uri(), &[])?;
        Ok(RustDiagnostics {
            session: Some(super::super::test_session(
                input,
                document,
                native,
                Path::new("/inert"),
            )),
            ..RustDiagnostics::default()
        })
    }

    fn route(
        session: &mut RustSession,
        report: SavedCompilerReport,
    ) -> Result<bool, LanguageProtocolError> {
        workspace::route_saved_compiler(
            &session.document,
            session.active_view,
            &mut session.saved_compiler,
            &mut session.parked,
            report,
        )
    }

    #[test]
    fn saved_versions_never_become_current_overlay_authority() -> Result<(), Box<dyn Error>> {
        let report = report(
            "file:///tmp/compiler-1.rs",
            vec![item("rustc"), item("rust-analyzer")],
        )?;
        assert_eq!(report.reported_version, Some(2));
        assert_eq!(report.batch.document_version(), None);
        assert_eq!(report.items(), 1);
        let first = report.status().ok_or("missing status")?;
        assert!(first.contains("not current buffer"));
        assert!(first.contains("E0502 at 4:5"));
        assert!(Arc::ptr_eq(
            &first,
            &report.status().ok_or("missing cached status")?
        ));
        let raw = serde_json::value::to_raw_value(&serde_json::json!({
            "uri":report.uri(),"version":2,"diagnostics":[item("rust-analyzer")]}))?;
        assert!(SavedCompilerReport::parse(&raw)?.is_none());
        Ok(())
    }

    #[test]
    fn malformed_and_oversized_compiler_reports_fail_closed() -> Result<(), Box<dyn Error>> {
        let mut invalid = Vec::new();
        for (key, value) in [
            ("source", Value::Bool(true)),
            ("code", Value::Bool(false)),
            ("code", Value::String("x".repeat(MAX_CODE_BYTES + 1))),
            ("severity", serde_json::json!(0)),
            ("severity", serde_json::json!(5)),
            (
                "range",
                serde_json::json!({"start":{"line":4,"character":0},"end":{"line":3,"character":0}}),
            ),
            ("message", Value::String("x".repeat(4097))),
        ] {
            let mut diagnostic = item("rustc");
            diagnostic[key] = value;
            invalid.push(vec![diagnostic]);
        }
        invalid.push(vec![item("rustc"); MAX_ITEMS + 1]);
        let mut large = item("rustc");
        large["message"] = Value::String("x".repeat(4096));
        invalid.push(vec![large; 64]);
        for diagnostics in invalid {
            assert!(report("file:///tmp/compiler-1.rs", diagnostics).is_err());
        }
        let duplicate = RawValue::from_string(String::from(
            r#"{"uri":"file:///tmp/compiler-1.rs","diagnostics":[],"diagnost\u0069cs":[]}"#,
        ))?;
        assert_eq!(
            SavedCompilerReport::parse(&duplicate).err(),
            Some(LanguageProtocolError::MalformedDiagnostics)
        );
        let large_wire =
            RawValue::from_string(format!(r#"{{"padding":"{}"}}"#, "x".repeat(MAX_WIRE_BYTES)))?;
        assert_eq!(
            SavedCompilerReport::parse(&large_wire).err(),
            Some(LanguageProtocolError::DiagnosticWireTooLarge)
        );
        Ok(())
    }

    #[test]
    fn native_and_saved_channels_clear_independently() -> Result<(), Box<dyn Error>> {
        let mut model = model()?;
        let uri = document_uri(1)?;
        let native = DiagnosticBatch::from_saved_items(&uri, &[item("rust-analyzer")])?;
        assert!(model.admit(Ok(native)));
        let session = model.session.as_mut().ok_or("missing session")?;
        let saved = report(&uri, vec![item("rustc")])?;
        assert!(route(session, saved.clone())?);
        assert!(!route(session, saved)?);
        assert_eq!(
            session
                .diagnostics
                .as_ref()
                .ok_or("missing native")?
                .batch
                .diagnostics()
                .len(),
            1
        );
        assert!(
            model
                .status_message()
                .ok_or("missing composed status")?
                .contains(" | Saved compiler")
        );
        assert!(model.admit(Ok(DiagnosticBatch::from_saved_items(&uri, &[])?)));
        assert_eq!(model.snapshot().diagnostic_items, 0);
        assert_eq!(model.snapshot().saved_compiler_items, 1);
        assert!(
            model
                .status_message()
                .ok_or("missing saved status")?
                .contains("E0502")
        );
        assert!(model.admit(Ok(DiagnosticBatch::from_saved_items(
            &uri,
            &[item("rust-analyzer")]
        )?)));
        assert!(route(
            model.session.as_mut().ok_or("missing session")?,
            report(&uri, vec![])?
        )?);
        assert_eq!(model.snapshot().diagnostic_items, 1);
        assert_eq!(model.snapshot().saved_compiler_items, 0);
        assert_eq!(model.snapshot().saved_compiler_bytes, 0);
        let closed = model.shutdown();
        assert_eq!(closed.saved_compiler_bytes, 0);
        Ok(())
    }

    #[test]
    fn compiler_reports_follow_documents_and_drain_on_close_or_restart()
    -> Result<(), Box<dyn Error>> {
        let mut model = model()?;
        let session = model.session.as_mut().ok_or("missing session")?;
        assert!(route(
            session,
            report(&document_uri(1)?, vec![item("rustc")])?
        )?);
        session.park_and_activate(input(2), true)?;
        assert!(session.saved_compiler.is_none());
        assert_eq!(workspace::saved_compiler_counts(None, &session.parked).0, 1);
        session.park_and_activate(input(1), true)?;
        assert!(session.saved_compiler.is_some());
        session.park_and_activate(input(2), false)?;
        assert_eq!(
            workspace::saved_compiler_counts(session.saved_compiler.as_ref(), &session.parked),
            (0, 0)
        );
        assert_eq!(
            route(session, report(&document_uri(1)?, vec![item("rustc")])?).err(),
            Some(LanguageProtocolError::DocumentMismatch)
        );
        assert!(route(
            session,
            report(&document_uri(2)?, vec![item("rustc")])?
        )?);
        session.reset_overlay_transport();
        assert_eq!(
            workspace::saved_compiler_counts(session.saved_compiler.as_ref(), &session.parked),
            (0, 0)
        );
        Ok(())
    }

    #[test]
    fn workspace_item_pressure_rejects_without_evicting_accepted_reports()
    -> Result<(), Box<dyn Error>> {
        let mut model = model()?;
        let session = model.session.as_mut().ok_or("missing session")?;
        for id in 1..=4 {
            if id != 1 {
                session.park_and_activate(input(id), true)?;
            }
            assert!(route(
                session,
                report(&document_uri(id)?, vec![item("rustc"); MAX_ITEMS])?
            )?);
        }
        let mut changed = item("rustc");
        changed["message"] = Value::String("replacement at the item limit".into());
        assert!(route(
            session,
            report(&document_uri(4)?, vec![changed; MAX_ITEMS])?
        )?);
        session.park_and_activate(input(5), true)?;
        let before =
            workspace::saved_compiler_counts(session.saved_compiler.as_ref(), &session.parked);
        assert_eq!(before.0, MAX_WORKSPACE_ITEMS);
        assert_eq!(
            route(session, report(&document_uri(5)?, vec![item("rustc")])?).err(),
            Some(LanguageProtocolError::DiagnosticRetentionExceeded)
        );
        assert_eq!(
            workspace::saved_compiler_counts(session.saved_compiler.as_ref(), &session.parked),
            before
        );
        Ok(())
    }
}
