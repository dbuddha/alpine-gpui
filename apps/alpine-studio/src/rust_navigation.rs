//! Bounded Rust hover and local source-location admission.

use std::{
    error::Error,
    fmt, fs,
    mem::size_of,
    ops::Range,
    path::{Component, Path, PathBuf},
};

use alpine_text::BufferSnapshot;
use serde_json::{Value, value::RawValue};

use crate::{
    lsp_language::{LspRange, parse_range},
    rust_completion::byte_range,
};

const MAX_NAVIGATION_WIRE_BYTES: usize = 1_048_576;
pub(crate) const MAX_HOVER_RETAINED_BYTES: usize = 32_768;
pub(crate) const MAX_HOVER_LINES: usize = 64;
pub(crate) const MAX_VISIBLE_HOVER_LINES: usize = 12;
pub(crate) const MAX_SOURCE_LOCATIONS: usize = 256;
pub(crate) const MAX_VISIBLE_SOURCE_LOCATIONS: usize = 12;
pub(crate) const MAX_LOCATION_URI_BYTES: usize = 4_096;
pub(crate) const MAX_LOCATION_RETAINED_BYTES: usize = 131_072;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NavigationError {
    WireTooLarge,
    Malformed,
    HoverTooLarge,
    TooManyHoverLines,
    UriTooLong,
    UnsupportedUri,
    InvalidPercentEncoding,
    InvalidUtf8,
    InvalidRange,
    RetentionExceeded,
    AllocationFailed,
    WorkspaceUnavailable,
    WorkspaceSymlink,
    OutsideWorkspace,
    TargetUnavailable,
    TargetSymlink,
    TargetNotFile,
}

impl fmt::Display for NavigationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Rust navigation rejected input: {self:?}")
    }
}

impl Error for NavigationError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HoverContent {
    text: Box<str>,
    retained_bytes: usize,
}

impl HoverContent {
    pub(crate) fn admit(result: &RawValue) -> Result<Option<Self>, NavigationError> {
        checked_wire(result)?;
        let value: Value =
            serde_json::from_str(result.get()).map_err(|_| NavigationError::Malformed)?;
        if value.is_null() {
            return Ok(None);
        }
        let contents = value
            .as_object()
            .and_then(|object| object.get("contents"))
            .ok_or(NavigationError::Malformed)?;
        let mut text = String::new();
        append_hover(contents, &mut text)?;
        if text.is_empty() {
            return Ok(None);
        }
        let lines = text.lines().count().max(1);
        if lines > MAX_HOVER_LINES {
            return Err(NavigationError::TooManyHoverLines);
        }
        let retained_bytes = size_of::<Self>()
            .checked_add(text.len())
            .ok_or(NavigationError::RetentionExceeded)?;
        Ok(Some(Self {
            text: text.into_boxed_str(),
            retained_bytes,
        }))
    }

    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    pub(crate) const fn retained_bytes(&self) -> usize {
        self.retained_bytes
    }

    pub(crate) fn visible_lines(&self) -> impl Iterator<Item = &str> {
        self.text.lines().take(MAX_VISIBLE_HOVER_LINES)
    }
}

fn append_hover(value: &Value, destination: &mut String) -> Result<(), NavigationError> {
    match value {
        Value::String(text) => append_hover_part(destination, text),
        Value::Array(parts) => {
            for part in parts {
                append_hover(part, destination)?;
            }
            Ok(())
        }
        Value::Object(object) => object
            .get("value")
            .and_then(Value::as_str)
            .ok_or(NavigationError::Malformed)
            .and_then(|text| append_hover_part(destination, text)),
        _ => Err(NavigationError::Malformed),
    }
}

fn append_hover_part(destination: &mut String, part: &str) -> Result<(), NavigationError> {
    let separator = usize::from(!destination.is_empty());
    let next = destination
        .len()
        .checked_add(separator)
        .and_then(|bytes| bytes.checked_add(part.len()))
        .ok_or(NavigationError::RetentionExceeded)?;
    if size_of::<HoverContent>()
        .checked_add(next)
        .is_none_or(|bytes| bytes > MAX_HOVER_RETAINED_BYTES)
    {
        return Err(NavigationError::HoverTooLarge);
    }
    destination
        .try_reserve_exact(separator.saturating_add(part.len()))
        .map_err(|_| NavigationError::AllocationFailed)?;
    if separator != 0 {
        destination.push('\n');
    }
    destination.push_str(part);
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SourceLocation {
    uri: Box<str>,
    range: LspRange,
}

impl SourceLocation {
    pub(crate) fn resolve(
        &self,
        workspace_root: &Path,
    ) -> Result<ResolvedSourceLocation, NavigationError> {
        let path = resolve_local_file_uri(workspace_root, &self.uri)?;
        Ok(ResolvedSourceLocation {
            path,
            range: self.range,
        })
    }

    pub(crate) fn uri(&self) -> &str {
        &self.uri
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ResolvedSourceLocation {
    path: PathBuf,
    range: LspRange,
}

impl ResolvedSourceLocation {
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn byte_range(
        &self,
        snapshot: &BufferSnapshot,
    ) -> Result<Range<usize>, NavigationError> {
        byte_range(snapshot, self.range).map_err(|_| NavigationError::InvalidRange)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SourceLocations {
    locations: Box<[SourceLocation]>,
    retained_bytes: usize,
    omitted: usize,
}

impl SourceLocations {
    pub(crate) fn admit(result: &RawValue) -> Result<Self, NavigationError> {
        checked_wire(result)?;
        let value: Value =
            serde_json::from_str(result.get()).map_err(|_| NavigationError::Malformed)?;
        let source = match &value {
            Value::Null => &[][..],
            Value::Array(locations) => locations.as_slice(),
            Value::Object(_) => std::slice::from_ref(&value),
            _ => return Err(NavigationError::Malformed),
        };
        let admitted = source.len().min(MAX_SOURCE_LOCATIONS);
        let mut locations = Vec::new();
        locations
            .try_reserve_exact(admitted)
            .map_err(|_| NavigationError::AllocationFailed)?;
        let mut retained_bytes = 0_usize;
        for value in source.iter().take(admitted) {
            let location = parse_location(value)?;
            let next = retained_bytes
                .checked_add(size_of::<SourceLocation>())
                .and_then(|bytes| bytes.checked_add(location.uri.len()))
                .ok_or(NavigationError::RetentionExceeded)?;
            if next > MAX_LOCATION_RETAINED_BYTES {
                break;
            }
            retained_bytes = next;
            locations.push(location);
        }
        let omitted = source.len().saturating_sub(locations.len());
        Ok(Self {
            locations: locations.into_boxed_slice(),
            retained_bytes,
            omitted,
        })
    }

    pub(crate) fn locations(&self) -> &[SourceLocation] {
        &self.locations
    }

    pub(crate) const fn retained_bytes(&self) -> usize {
        self.retained_bytes
    }

    pub(crate) const fn omitted(&self) -> usize {
        self.omitted
    }

    pub(crate) fn visible_range(&self, first: usize) -> Range<usize> {
        let start = first.min(self.locations.len());
        let end = start
            .saturating_add(MAX_VISIBLE_SOURCE_LOCATIONS)
            .min(self.locations.len());
        start..end
    }
}

fn parse_location(value: &Value) -> Result<SourceLocation, NavigationError> {
    let object = value.as_object().ok_or(NavigationError::Malformed)?;
    let (uri, range) = if let Some(uri) = object.get("uri") {
        (
            uri.as_str().ok_or(NavigationError::Malformed)?,
            object.get("range").ok_or(NavigationError::Malformed)?,
        )
    } else {
        (
            object
                .get("targetUri")
                .and_then(Value::as_str)
                .ok_or(NavigationError::Malformed)?,
            object
                .get("targetSelectionRange")
                .or_else(|| object.get("targetRange"))
                .ok_or(NavigationError::Malformed)?,
        )
    };
    if uri.is_empty() || uri.len() > MAX_LOCATION_URI_BYTES {
        return Err(NavigationError::UriTooLong);
    }
    let range = parse_range(range).map_err(|_| NavigationError::InvalidRange)?;
    Ok(SourceLocation {
        uri: uri.into(),
        range,
    })
}

fn checked_wire(result: &RawValue) -> Result<(), NavigationError> {
    if result.get().len() > MAX_NAVIGATION_WIRE_BYTES {
        return Err(NavigationError::WireTooLarge);
    }
    Ok(())
}

fn decode_file_uri(uri: &str) -> Result<PathBuf, NavigationError> {
    let encoded = uri
        .strip_prefix("file://")
        .filter(|path| path.starts_with('/'))
        .ok_or(NavigationError::UnsupportedUri)?;
    if encoded.contains(['?', '#']) {
        return Err(NavigationError::UnsupportedUri);
    }
    let bytes = percent_decode(encoded.as_bytes())?;
    let decoded = String::from_utf8(bytes).map_err(|_| NavigationError::InvalidUtf8)?;
    if decoded.contains('\0') {
        return Err(NavigationError::InvalidUtf8);
    }
    let is_absolute_uri_path = decoded.starts_with('/');
    let path = PathBuf::from(decoded);
    if !is_absolute_uri_path
        || path
            .components()
            .any(|component| !matches!(component, Component::RootDir | Component::Normal(_)))
    {
        return Err(NavigationError::OutsideWorkspace);
    }
    Ok(path)
}

fn percent_decode(encoded: &[u8]) -> Result<Vec<u8>, NavigationError> {
    let mut decoded = Vec::new();
    decoded
        .try_reserve_exact(encoded.len())
        .map_err(|_| NavigationError::AllocationFailed)?;
    let mut index = 0;
    while index < encoded.len() {
        if encoded[index] == b'%' {
            let high = encoded
                .get(index + 1)
                .copied()
                .and_then(hex_value)
                .ok_or(NavigationError::InvalidPercentEncoding)?;
            let low = encoded
                .get(index + 2)
                .copied()
                .and_then(hex_value)
                .ok_or(NavigationError::InvalidPercentEncoding)?;
            decoded.push(high.saturating_mul(16).saturating_add(low));
            index = index.saturating_add(3);
        } else {
            decoded.push(encoded[index]);
            index = index.saturating_add(1);
        }
    }
    Ok(decoded)
}

const fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

pub(crate) fn resolve_local_file_uri(
    workspace_root: &Path,
    uri: &str,
) -> Result<PathBuf, NavigationError> {
    let path = decode_file_uri(uri)?;
    revalidate_local_path(workspace_root, &path)
}

pub(crate) fn revalidate_local_path(
    workspace_root: &Path,
    path: &Path,
) -> Result<PathBuf, NavigationError> {
    let root_metadata =
        fs::symlink_metadata(workspace_root).map_err(|_| NavigationError::WorkspaceUnavailable)?;
    if root_metadata.file_type().is_symlink() {
        return Err(NavigationError::WorkspaceSymlink);
    }
    let canonical_root =
        fs::canonicalize(workspace_root).map_err(|_| NavigationError::WorkspaceUnavailable)?;
    if !workspace_root.is_absolute() || !path.starts_with(workspace_root) {
        return Err(NavigationError::OutsideWorkspace);
    }
    let relative = path
        .strip_prefix(workspace_root)
        .map_err(|_| NavigationError::OutsideWorkspace)?;
    let mut current = canonical_root.clone();
    for component in relative.components() {
        let Component::Normal(component) = component else {
            return Err(NavigationError::OutsideWorkspace);
        };
        current.push(component);
        let metadata =
            fs::symlink_metadata(&current).map_err(|_| NavigationError::TargetUnavailable)?;
        if metadata.file_type().is_symlink() {
            return Err(NavigationError::TargetSymlink);
        }
    }
    let canonical_target =
        fs::canonicalize(path).map_err(|_| NavigationError::TargetUnavailable)?;
    ensure_within_workspace(&canonical_root, &canonical_target)?;
    if !fs::metadata(&canonical_target)
        .map_err(|_| NavigationError::TargetUnavailable)?
        .is_file()
    {
        return Err(NavigationError::TargetNotFile);
    }
    Ok(canonical_target)
}

#[cfg(test)]
fn validate_local_path(workspace_root: &Path, path: &Path) -> Result<(), NavigationError> {
    revalidate_local_path(workspace_root, path).map(|_| ())
}

fn ensure_within_workspace(root: &Path, target: &Path) -> Result<(), NavigationError> {
    if target.starts_with(root) {
        Ok(())
    } else {
        Err(NavigationError::OutsideWorkspace)
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use serde_json::value::RawValue;

    use super::*;

    fn raw(text: &str) -> Box<RawValue> {
        RawValue::from_string(text.to_owned()).unwrap_or_else(|_| unreachable!())
    }

    fn location(uri: &str) -> String {
        format!(
            r#"{{"uri":"{uri}","range":{{"start":{{"line":0,"character":0}},"end":{{"line":0,"character":1}}}}}}"#
        )
    }

    #[test]
    fn hover_shapes_are_bounded_and_visible_rows_are_capped() {
        let hover = HoverContent::admit(&raw(
            r#"{"contents":["first",{"language":"rust","value":"second"},{"kind":"markdown","value":"third"}]}"#,
        ))
        .unwrap_or_else(|_| unreachable!())
        .unwrap_or_else(|| unreachable!());
        assert_eq!(hover.text(), "first\nsecond\nthird");
        assert!(hover.retained_bytes() <= MAX_HOVER_RETAINED_BYTES);
        assert_eq!(hover.visible_lines().count(), 3);
        assert_eq!(HoverContent::admit(&raw("null")), Ok(None));
        assert_eq!(HoverContent::admit(&raw(r#"{"contents":[]}"#)), Ok(None));
        assert_eq!(
            HoverContent::admit(&raw(r#"{"contents":7}"#)),
            Err(NavigationError::Malformed)
        );
        assert_eq!(
            HoverContent::admit(&raw(r#"{"contents":{"kind":"markdown"}}"#)),
            Err(NavigationError::Malformed)
        );
        assert_eq!(
            NavigationError::Malformed.to_string(),
            "Rust navigation rejected input: Malformed"
        );

        let too_many = (0..=MAX_HOVER_LINES)
            .map(|_| "line")
            .collect::<Vec<_>>()
            .join("\\n");
        assert_eq!(
            HoverContent::admit(&raw(&format!(r#"{{"contents":"{too_many}"}}"#))),
            Err(NavigationError::TooManyHoverLines)
        );
    }

    #[test]
    fn hover_line_byte_and_wire_boundaries_are_exact() {
        let exact_lines = (0..MAX_HOVER_LINES)
            .map(|_| "line")
            .collect::<Vec<_>>()
            .join("\\n");
        assert!(HoverContent::admit(&raw(&format!(r#"{{"contents":"{exact_lines}"}}"#))).is_ok());

        let exact_text = "x".repeat(MAX_HOVER_RETAINED_BYTES - size_of::<HoverContent>());
        let exact = raw(&serde_json::json!({ "contents": exact_text }).to_string());
        let hover = HoverContent::admit(&exact)
            .unwrap_or_else(|_| unreachable!())
            .unwrap_or_else(|| unreachable!());
        assert_eq!(hover.retained_bytes(), MAX_HOVER_RETAINED_BYTES);

        let oversized_text = "x".repeat(MAX_HOVER_RETAINED_BYTES - size_of::<HoverContent>() + 1);
        let oversized = raw(&serde_json::json!({ "contents": oversized_text }).to_string());
        assert_eq!(
            HoverContent::admit(&oversized),
            Err(NavigationError::HoverTooLarge)
        );

        let exact_wire = raw(&format!(
            "\"{}\"",
            "x".repeat(MAX_NAVIGATION_WIRE_BYTES - 2)
        ));
        assert_eq!(exact_wire.get().len(), MAX_NAVIGATION_WIRE_BYTES);
        assert_eq!(checked_wire(&exact_wire), Ok(()));
        let oversized_wire = raw(&format!(
            "\"{}\"",
            "x".repeat(MAX_NAVIGATION_WIRE_BYTES - 1)
        ));
        assert_eq!(
            checked_wire(&oversized_wire),
            Err(NavigationError::WireTooLarge)
        );
    }

    #[test]
    fn location_and_link_shapes_retain_exact_bounded_values() {
        let direct = location("file:///tmp/work/main.rs");
        let link = r#"{"targetUri":"file:///tmp/work/lib.rs","targetRange":{"start":{"line":0,"character":0},"end":{"line":1,"character":0}},"targetSelectionRange":{"start":{"line":0,"character":2},"end":{"line":0,"character":4}}}"#;
        let batch = SourceLocations::admit(&raw(&format!("[{direct},{link}]")))
            .unwrap_or_else(|_| unreachable!());
        assert_eq!(batch.locations().len(), 2);
        assert_eq!(batch.locations()[1].uri(), "file:///tmp/work/lib.rs");
        assert!(batch.retained_bytes() <= MAX_LOCATION_RETAINED_BYTES);
        assert_eq!(batch.omitted(), 0);
        assert_eq!(batch.visible_range(0), 0..2);
        assert_eq!(
            SourceLocations::admit(&raw("null")),
            Ok(SourceLocations {
                locations: Box::new([]),
                retained_bytes: 0,
                omitted: 0,
            })
        );
    }

    fn locations_with_retained_bytes(target: usize) -> Vec<Value> {
        let item_bytes = size_of::<SourceLocation>();
        let mut remaining = target;
        let mut values = Vec::new();
        while remaining > 0 {
            assert!(remaining > item_bytes);
            let mut uri_bytes = (remaining - item_bytes).min(MAX_LOCATION_URI_BYTES);
            let leftover = remaining - item_bytes - uri_bytes;
            if leftover > 0 && leftover <= item_bytes {
                uri_bytes -= item_bytes + 1 - leftover;
            }
            assert!(uri_bytes > 0);
            values.push(serde_json::json!({
                "uri": "x".repeat(uri_bytes),
                "range": {
                    "start": { "line": 0, "character": 0 },
                    "end": { "line": 0, "character": 1 }
                }
            }));
            remaining -= item_bytes + uri_bytes;
        }
        values
    }

    #[test]
    fn location_uri_and_aggregate_retention_boundaries_are_exact() {
        let exact_uri = "x".repeat(MAX_LOCATION_URI_BYTES);
        let exact_uri_batch =
            SourceLocations::admit(&raw(&location(&exact_uri))).unwrap_or_else(|_| unreachable!());
        assert_eq!(exact_uri_batch.locations().len(), 1);
        assert_eq!(exact_uri_batch.locations()[0].uri(), exact_uri);

        assert_eq!(
            SourceLocations::admit(&raw(&location(""))),
            Err(NavigationError::UriTooLong)
        );
        let oversized_uri = "x".repeat(MAX_LOCATION_URI_BYTES + 1);
        assert_eq!(
            SourceLocations::admit(&raw(&location(&oversized_uri))),
            Err(NavigationError::UriTooLong)
        );

        let exact = raw(
            &Value::Array(locations_with_retained_bytes(MAX_LOCATION_RETAINED_BYTES)).to_string(),
        );
        let exact_batch = SourceLocations::admit(&exact).unwrap_or_else(|_| unreachable!());
        assert_eq!(exact_batch.retained_bytes(), MAX_LOCATION_RETAINED_BYTES);
        assert_eq!(exact_batch.omitted(), 0);

        let oversized = raw(&Value::Array(locations_with_retained_bytes(
            MAX_LOCATION_RETAINED_BYTES + 1,
        ))
        .to_string());
        let bounded = SourceLocations::admit(&oversized).unwrap_or_else(|_| unreachable!());
        assert!(bounded.retained_bytes() <= MAX_LOCATION_RETAINED_BYTES);
        assert!(bounded.omitted() > 0);
    }

    #[test]
    fn uri_decoding_rejects_remote_traversal_and_invalid_escapes() {
        assert_eq!(
            decode_file_uri(concat!("https:", "//example.com/main.rs")),
            Err(NavigationError::UnsupportedUri)
        );
        assert_eq!(
            decode_file_uri("file://host/tmp/main.rs"),
            Err(NavigationError::UnsupportedUri)
        );
        assert_eq!(
            decode_file_uri("file:///tmp/../etc/passwd"),
            Err(NavigationError::OutsideWorkspace)
        );
        assert_eq!(
            decode_file_uri("file:///tmp/%zz"),
            Err(NavigationError::InvalidPercentEncoding)
        );
        assert_eq!(
            decode_file_uri("file:///tmp/main.rs?query"),
            Err(NavigationError::UnsupportedUri)
        );
        assert_eq!(
            decode_file_uri("file:///tmp/main.rs#fragment"),
            Err(NavigationError::UnsupportedUri)
        );
        assert_eq!(
            decode_file_uri("file:///tmp/%00.rs"),
            Err(NavigationError::InvalidUtf8)
        );
        assert_eq!(
            decode_file_uri("file:///tmp/%ff.rs"),
            Err(NavigationError::InvalidUtf8)
        );
        assert_eq!(
            decode_file_uri("file:///tmp/%a"),
            Err(NavigationError::InvalidPercentEncoding)
        );
        assert_eq!(
            decode_file_uri("file:///tmp/a%20b.rs"),
            Ok(PathBuf::from("/tmp/a b.rs"))
        );
        assert_eq!(percent_decode(b"%aF"), Ok(vec![0xaf]));
        assert_eq!(percent_decode(b"%Fa"), Ok(vec![0xfa]));
        assert_eq!(hex_value(b'a'), Some(10));
        assert_eq!(hex_value(b'f'), Some(15));
        assert_eq!(hex_value(b'A'), Some(10));
        assert_eq!(hex_value(b'F'), Some(15));
        assert_eq!(hex_value(b'0'), Some(0));
        assert_eq!(hex_value(b'9'), Some(9));
        assert_eq!(hex_value(b'g'), None);
    }

    #[cfg(unix)]
    #[test]
    fn local_resolution_rejects_escape_symlink_directory_and_missing_file()
    -> Result<(), Box<dyn Error>> {
        use std::os::unix::fs::symlink;

        let root = std::env::temp_dir().join(format!(
            "alpine-navigation-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("src"))?;
        fs::write(root.join("src/main.rs"), "fn main() {}")?;
        let root = fs::canonicalize(root)?;
        let accepted = root.join("src/main.rs");
        assert_eq!(validate_local_path(&root, &accepted), Ok(()));
        let batch =
            SourceLocations::admit(&raw(&location(&format!("file://{}", accepted.display()))))?;
        let resolved = batch.locations()[0].resolve(&root)?;
        assert_eq!(resolved.path(), accepted);
        let snapshot = alpine_text::Buffer::new("fn main() {}").snapshot();
        assert_eq!(resolved.byte_range(&snapshot), Ok(0..1));
        assert_eq!(
            validate_local_path(&root, Path::new("/etc/passwd")),
            Err(NavigationError::OutsideWorkspace)
        );
        assert_eq!(
            validate_local_path(&root, &root.join("src/../src/main.rs")),
            Err(NavigationError::OutsideWorkspace)
        );
        assert_eq!(
            validate_local_path(&root, &root.join("missing.rs")),
            Err(NavigationError::TargetUnavailable)
        );
        assert_eq!(
            validate_local_path(&root, &root.join("src")),
            Err(NavigationError::TargetNotFile)
        );
        symlink(&accepted, root.join("linked.rs"))?;
        assert_eq!(
            validate_local_path(&root, &root.join("linked.rs")),
            Err(NavigationError::TargetSymlink)
        );
        let root_link = root.with_extension("workspace-link");
        let _ = fs::remove_file(&root_link);
        symlink(&root, &root_link)?;
        assert_eq!(
            validate_local_path(&root_link, &accepted),
            Err(NavigationError::WorkspaceSymlink)
        );
        fs::remove_file(root_link)?;
        assert_eq!(ensure_within_workspace(&root, &accepted), Ok(()));
        assert_eq!(
            ensure_within_workspace(&root, Path::new("/outside/main.rs")),
            Err(NavigationError::OutsideWorkspace)
        );
        let adjusted =
            locations_with_retained_bytes(MAX_LOCATION_URI_BYTES + size_of::<SourceLocation>() + 1);
        assert_eq!(adjusted.len(), 2);
        fs::remove_dir_all(&root)?;
        Ok(())
    }

    #[test]
    fn malformed_location_shapes_are_rejected() {
        for value in [
            "true",
            r#"{"uri":7,"range":{}}"#,
            r#"{"targetUri":7,"targetRange":{}}"#,
            r#"{"targetUri":"file:///tmp/main.rs"}"#,
        ] {
            assert_eq!(
                SourceLocations::admit(&raw(value)),
                Err(NavigationError::Malformed)
            );
        }
        assert_eq!(
            SourceLocations::admit(&raw(r#"{"uri":"file:///tmp/main.rs","range":7}"#)),
            Err(NavigationError::InvalidRange)
        );
    }
}
