//! Compiled, dependency-free syntax presentation for the v1 language cohort.

use std::{fmt, mem::size_of, ops::Range, path::Path, sync::Arc};

use alpine_text::{BufferSnapshot, TextError, TextFingerprint};

pub(crate) const DEFAULT_SYNTAX_BUDGET_BYTES: usize = 4 * 1024 * 1024;
pub(crate) const MAX_SYNTAX_LINE_BYTES: usize = 64 * 1024;
pub(crate) const MAX_SYNTAX_SPANS_PER_LINE: usize = 1_024;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum SyntaxLanguage {
    PlainText,
    Rust,
    Markdown,
    Toml,
    Json,
}

impl SyntaxLanguage {
    pub(crate) fn from_path(path: Option<&Path>) -> Self {
        let Some(path) = path else {
            return Self::PlainText;
        };
        if path.file_name().and_then(|name| name.to_str()) == Some("Cargo.lock") {
            return Self::Toml;
        }
        match path.extension().and_then(|extension| extension.to_str()) {
            Some("rs") => Self::Rust,
            Some("md" | "markdown") => Self::Markdown,
            Some("toml") => Self::Toml,
            Some("json") => Self::Json,
            _ => Self::PlainText,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SyntaxClass {
    Comment,
    Keyword,
    String,
    Number,
    Type,
    Property,
    Heading,
    Code,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SyntaxSpan {
    start_utf16: u32,
    end_utf16: u32,
    class: SyntaxClass,
}

impl SyntaxSpan {
    #[cfg(test)]
    pub(crate) const fn start_utf16(self) -> u32 {
        self.start_utf16
    }

    #[cfg(test)]
    pub(crate) const fn end_utf16(self) -> u32 {
        self.end_utf16
    }

    #[cfg(test)]
    pub(crate) const fn class(self) -> SyntaxClass {
        self.class
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SyntaxLine {
    spans: Arc<[SyntaxSpan]>,
    scanned_bytes: usize,
    omitted: bool,
}

impl SyntaxLine {
    fn plain(scanned_bytes: usize, omitted: bool) -> Self {
        Self {
            spans: Arc::from([]),
            scanned_bytes,
            omitted,
        }
    }

    #[cfg(test)]
    fn spans(&self) -> &[SyntaxSpan] {
        &self.spans
    }

    pub(crate) fn class_at(&self, source_utf16: u32) -> Option<SyntaxClass> {
        let index = self
            .spans
            .partition_point(|span| span.end_utf16 <= source_utf16);
        let span = self.spans.get(index)?;
        (span.start_utf16 <= source_utf16).then_some(span.class)
    }

    #[cfg(test)]
    const fn scanned_bytes(&self) -> usize {
        self.scanned_bytes
    }

    #[cfg(test)]
    const fn omitted(&self) -> bool {
        self.omitted
    }

    fn retained_bytes(&self) -> usize {
        self.spans.len().saturating_mul(size_of::<SyntaxSpan>())
    }
}

#[derive(Clone)]
struct CacheEntry {
    fingerprint: TextFingerprint,
    snapshot: BufferSnapshot,
    range: Range<usize>,
    language: SyntaxLanguage,
    line: Arc<SyntaxLine>,
    retained_bytes: usize,
}

impl CacheEntry {
    fn matches(
        &self,
        fingerprint: TextFingerprint,
        snapshot: &BufferSnapshot,
        range: Range<usize>,
        language: SyntaxLanguage,
    ) -> Result<bool, TextError> {
        if self.fingerprint != fingerprint || self.language != language {
            return Ok(false);
        }
        self.snapshot.range_eq(self.range.clone(), snapshot, range)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct SyntaxCacheSnapshot {
    current_bytes: usize,
    peak_bytes: usize,
    budget_bytes: usize,
    hits: u64,
    misses: u64,
    omitted_lines: u64,
}

impl SyntaxCacheSnapshot {
    pub(crate) const fn current_bytes(self) -> usize {
        self.current_bytes
    }

    pub(crate) const fn peak_bytes(self) -> usize {
        self.peak_bytes
    }

    pub(crate) const fn budget_bytes(self) -> usize {
        self.budget_bytes
    }

    pub(crate) const fn hits(self) -> u64 {
        self.hits
    }

    pub(crate) const fn misses(self) -> u64 {
        self.misses
    }

    pub(crate) const fn omitted_lines(self) -> u64 {
        self.omitted_lines
    }
}

pub(crate) struct SyntaxCache {
    current: Vec<CacheEntry>,
    previous: Vec<CacheEntry>,
    budget_bytes: usize,
    peak_bytes: usize,
    hits: u64,
    misses: u64,
    omitted_lines: u64,
}

impl SyntaxCache {
    pub(crate) fn new(budget_bytes: usize) -> Result<Self, SyntaxError> {
        if budget_bytes == 0 {
            return Err(SyntaxError::InvalidBudget);
        }
        Ok(Self {
            current: Vec::new(),
            previous: Vec::new(),
            budget_bytes,
            peak_bytes: 0,
            hits: 0,
            misses: 0,
            omitted_lines: 0,
        })
    }

    pub(crate) fn begin_frame(&mut self) {
        self.previous.clear();
        std::mem::swap(&mut self.current, &mut self.previous);
    }

    pub(crate) fn line(
        &mut self,
        snapshot: &BufferSnapshot,
        line: usize,
        language: SyntaxLanguage,
    ) -> Result<Arc<SyntaxLine>, SyntaxError> {
        let range = snapshot.line_byte_range(line)?;
        let fingerprint = snapshot.fingerprint(range.clone())?;
        if let Some(index) = find_match(
            &self.current,
            fingerprint,
            snapshot,
            range.clone(),
            language,
        )? {
            self.hits = self
                .hits
                .checked_add(1)
                .ok_or(SyntaxError::SequenceExhausted)?;
            return Ok(Arc::clone(&self.current[index].line));
        }
        if let Some(index) = find_match(
            &self.previous,
            fingerprint,
            snapshot,
            range.clone(),
            language,
        )? {
            let entry = self.previous.remove(index);
            let line = Arc::clone(&entry.line);
            self.current.push(entry);
            self.hits = self
                .hits
                .checked_add(1)
                .ok_or(SyntaxError::SequenceExhausted)?;
            return Ok(line);
        }
        self.misses = self
            .misses
            .checked_add(1)
            .ok_or(SyntaxError::SequenceExhausted)?;
        let bytes = range.end.saturating_sub(range.start);
        let line = if language == SyntaxLanguage::PlainText {
            Arc::new(SyntaxLine::plain(0, false))
        } else if bytes > MAX_SYNTAX_LINE_BYTES {
            self.omitted_lines = self
                .omitted_lines
                .checked_add(1)
                .ok_or(SyntaxError::SequenceExhausted)?;
            Arc::new(SyntaxLine::plain(0, true))
        } else {
            let mut text = snapshot.slice(range.clone())?;
            trim_line_ending(&mut text);
            Arc::new(highlight_line(language, &text)?)
        };
        let retained_bytes = size_of::<SyntaxLine>()
            .checked_add(line.retained_bytes())
            .ok_or(SyntaxError::AllocationFailed)?;
        self.current
            .try_reserve(1)
            .map_err(|_| SyntaxError::AllocationFailed)?;
        self.current.push(CacheEntry {
            fingerprint,
            snapshot: snapshot.clone(),
            range,
            language,
            line: Arc::clone(&line),
            retained_bytes,
        });
        self.enforce_budget();
        Ok(line)
    }

    pub(crate) fn snapshot(&self) -> SyntaxCacheSnapshot {
        SyntaxCacheSnapshot {
            current_bytes: self.current_bytes(),
            peak_bytes: self.peak_bytes,
            budget_bytes: self.budget_bytes,
            hits: self.hits,
            misses: self.misses,
            omitted_lines: self.omitted_lines,
        }
    }

    fn current_bytes(&self) -> usize {
        self.current
            .capacity()
            .saturating_add(self.previous.capacity())
            .saturating_mul(size_of::<CacheEntry>())
            .saturating_add(
                self.current
                    .iter()
                    .chain(&self.previous)
                    .map(|entry| entry.retained_bytes)
                    .sum(),
            )
    }

    fn enforce_budget(&mut self) {
        while self.current_bytes() > self.budget_bytes {
            if !self.previous.is_empty() {
                self.previous.remove(0);
            } else if !self.current.is_empty() {
                self.current.remove(0);
            } else {
                self.current.shrink_to_fit();
                self.previous.shrink_to_fit();
                break;
            }
            self.current.shrink_to_fit();
            self.previous.shrink_to_fit();
        }
        self.peak_bytes = self.peak_bytes.max(self.current_bytes());
    }
}

fn find_match(
    entries: &[CacheEntry],
    fingerprint: TextFingerprint,
    snapshot: &BufferSnapshot,
    range: Range<usize>,
    language: SyntaxLanguage,
) -> Result<Option<usize>, TextError> {
    for (index, entry) in entries.iter().enumerate() {
        if entry.matches(fingerprint, snapshot, range.clone(), language)? {
            return Ok(Some(index));
        }
    }
    Ok(None)
}

fn trim_line_ending(text: &mut String) {
    if text.ends_with('\n') {
        text.pop();
        if text.ends_with('\r') {
            text.pop();
        }
    } else if text.ends_with('\r') {
        text.pop();
    }
}

#[derive(Clone, Copy)]
struct ByteSpan {
    start: usize,
    end: usize,
    class: SyntaxClass,
}

struct Emitter<'a> {
    text: &'a str,
    spans: Vec<ByteSpan>,
    omitted: bool,
}

impl<'a> Emitter<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            text,
            spans: Vec::new(),
            omitted: false,
        }
    }

    fn push(&mut self, start: usize, end: usize, class: SyntaxClass) -> Result<(), SyntaxError> {
        if start >= end || end > self.text.len() {
            return Ok(());
        }
        if self.spans.len() == MAX_SYNTAX_SPANS_PER_LINE {
            self.omitted = true;
            return Ok(());
        }
        if self
            .spans
            .last()
            .is_some_and(|previous| previous.end > start)
        {
            return Err(SyntaxError::InvalidSpan);
        }
        reserve(&mut self.spans, 1)?;
        self.spans.push(ByteSpan { start, end, class });
        Ok(())
    }

    fn finish(self) -> Result<SyntaxLine, SyntaxError> {
        let mut spans = Vec::new();
        reserve_exact(&mut spans, self.spans.len())?;
        let mut byte = 0_usize;
        let mut utf16 = 0_u32;
        for span in self.spans {
            advance_utf16(self.text, &mut byte, &mut utf16, span.start)?;
            let start_utf16 = utf16;
            advance_utf16(self.text, &mut byte, &mut utf16, span.end)?;
            spans.push(SyntaxSpan {
                start_utf16,
                end_utf16: utf16,
                class: span.class,
            });
        }
        Ok(SyntaxLine {
            spans: spans.into(),
            scanned_bytes: self.text.len(),
            omitted: self.omitted,
        })
    }
}

fn advance_utf16(
    text: &str,
    byte: &mut usize,
    utf16: &mut u32,
    target: usize,
) -> Result<(), SyntaxError> {
    if target > text.len()
        || !text.is_char_boundary(target)
        || *byte > target
        || !text.is_char_boundary(*byte)
    {
        return Err(SyntaxError::InvalidSpan);
    }
    for character in text[*byte..target].chars() {
        let utf16_units = if character.len_utf16() == 1 { 1 } else { 2 };
        *utf16 = utf16
            .checked_add(utf16_units)
            .ok_or(SyntaxError::InvalidSpan)?;
    }
    *byte = target;
    Ok(())
}

fn reserve<T>(values: &mut Vec<T>, additional: usize) -> Result<(), SyntaxError> {
    values
        .try_reserve(additional)
        .map_err(|_| SyntaxError::AllocationFailed)
}

fn reserve_exact<T>(values: &mut Vec<T>, additional: usize) -> Result<(), SyntaxError> {
    values
        .try_reserve_exact(additional)
        .map_err(|_| SyntaxError::AllocationFailed)
}

fn highlight_line(language: SyntaxLanguage, text: &str) -> Result<SyntaxLine, SyntaxError> {
    if text.len() > MAX_SYNTAX_LINE_BYTES {
        return Ok(SyntaxLine::plain(0, true));
    }
    let mut emitter = Emitter::new(text);
    match language {
        SyntaxLanguage::PlainText => {}
        SyntaxLanguage::Rust => highlight_rust(text, &mut emitter)?,
        SyntaxLanguage::Markdown => highlight_markdown(text, &mut emitter)?,
        SyntaxLanguage::Toml => highlight_toml(text, &mut emitter)?,
        SyntaxLanguage::Json => highlight_json(text, &mut emitter)?,
    }
    emitter.finish()
}

fn highlight_rust(text: &str, emitter: &mut Emitter<'_>) -> Result<(), SyntaxError> {
    let bytes = text.as_bytes();
    let mut index = 0_usize;
    for _ in 0..bytes.len() {
        if index == bytes.len() {
            return Ok(());
        }
        if bytes[index..].starts_with(b"//") {
            emitter.push(index, bytes.len(), SyntaxClass::Comment)?;
            return Ok(());
        }
        let next = if bytes[index..].starts_with(b"/*") {
            let end = find_pair(bytes, index + 2, *b"*/").unwrap_or(bytes.len());
            emitter.push(index, end, SyntaxClass::Comment)?;
            end
        } else if matches!(bytes[index], b'"' | b'\'') {
            let end = quoted_end(bytes, index, bytes[index]);
            emitter.push(index, end, SyntaxClass::String)?;
            end
        } else if bytes[index].is_ascii_digit() {
            let end = take_while(bytes, next_index(index)?, |byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.')
            });
            emitter.push(index, end, SyntaxClass::Number)?;
            end
        } else if is_identifier_start(bytes[index]) {
            let end = take_while(bytes, next_index(index)?, is_identifier_continue);
            let word = &text[index..end];
            if rust_keyword(word) {
                emitter.push(index, end, SyntaxClass::Keyword)?;
            } else if bytes[index].is_ascii_uppercase() {
                emitter.push(index, end, SyntaxClass::Type)?;
            }
            end
        } else {
            next_boundary(text, index)
        };
        index = require_forward(index, next, bytes.len())?;
    }
    require_scan_complete(index, bytes.len())
}

fn highlight_markdown(text: &str, emitter: &mut Emitter<'_>) -> Result<(), SyntaxError> {
    let trimmed = text.trim_start();
    let leading = text.len() - trimmed.len();
    if trimmed.starts_with('#') {
        let hashes = trimmed.bytes().take_while(|byte| *byte == b'#').count();
        if (1..=6).contains(&hashes) && trimmed.as_bytes().get(hashes) == Some(&b' ') {
            emitter.push(leading, text.len(), SyntaxClass::Heading)?;
            return Ok(());
        }
    }
    let bytes = text.as_bytes();
    let mut index = 0_usize;
    for _ in 0..bytes.len() {
        if index == bytes.len() {
            return Ok(());
        }
        let next = if bytes[index] == b'`' {
            let end = bytes[index + 1..]
                .iter()
                .position(|byte| *byte == b'`')
                .map_or(bytes.len(), |offset| index + offset + 2);
            emitter.push(index, end, SyntaxClass::Code)?;
            end
        } else {
            next_boundary(text, index)
        };
        index = require_forward(index, next, bytes.len())?;
    }
    require_scan_complete(index, bytes.len())
}

fn highlight_toml(text: &str, emitter: &mut Emitter<'_>) -> Result<(), SyntaxError> {
    let trimmed = text.trim_start();
    let leading = text.len() - trimmed.len();
    if trimmed.starts_with('[') && trimmed.ends_with(']') {
        emitter.push(leading, text.len(), SyntaxClass::Heading)?;
        return Ok(());
    }
    let value_start = if let Some(equal) = find_unquoted(text.as_bytes(), b'=') {
        let key_end = text[..equal].trim_end().len();
        emitter.push(leading, key_end, SyntaxClass::Property)?;
        next_index(equal)?
    } else {
        0
    };
    highlight_data_values(text, emitter, true, value_start)
}

fn highlight_json(text: &str, emitter: &mut Emitter<'_>) -> Result<(), SyntaxError> {
    highlight_data_values(text, emitter, false, 0)
}

fn highlight_data_values(
    text: &str,
    emitter: &mut Emitter<'_>,
    comments: bool,
    start: usize,
) -> Result<(), SyntaxError> {
    let bytes = text.as_bytes();
    let mut index = start;
    for _ in 0..bytes.len() {
        if index == bytes.len() {
            return Ok(());
        }
        if comments && bytes[index] == b'#' {
            emitter.push(index, bytes.len(), SyntaxClass::Comment)?;
            return Ok(());
        }
        let next = if matches!(bytes[index], b'"' | b'\'') {
            let end = quoted_end(bytes, index, bytes[index]);
            let next_non_space = bytes.get(end..).map_or(bytes.len(), |remaining| {
                remaining
                    .iter()
                    .position(|byte| !byte.is_ascii_whitespace())
                    .map_or(bytes.len(), |offset| end + offset)
            });
            let class = if !comments && bytes.get(next_non_space) == Some(&b':') {
                SyntaxClass::Property
            } else {
                SyntaxClass::String
            };
            emitter.push(index, end, class)?;
            end
        } else if bytes[index].is_ascii_digit() || bytes[index] == b'-' {
            let end = take_while(bytes, next_index(index)?, |byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'+' | b'-' | b':')
            });
            emitter.push(index, end, SyntaxClass::Number)?;
            end
        } else if is_identifier_start(bytes[index]) {
            let end = take_while(bytes, next_index(index)?, is_identifier_continue);
            if matches!(&text[index..end], "true" | "false" | "null") {
                emitter.push(index, end, SyntaxClass::Keyword)?;
            }
            end
        } else {
            next_boundary(text, index)
        };
        index = require_forward(index, next, bytes.len())?;
    }
    require_scan_complete(index, bytes.len())
}

fn quoted_end(bytes: &[u8], start: usize, quote: u8) -> usize {
    let mut escaped = false;
    for (offset, byte) in bytes[start + 1..].iter().copied().enumerate() {
        if escaped {
            escaped = false;
        } else if byte == b'\\' && quote == b'"' {
            escaped = true;
        } else if byte == quote {
            return start + offset + 2;
        }
    }
    bytes.len()
}

fn find_pair(bytes: &[u8], start: usize, pair: [u8; 2]) -> Option<usize> {
    bytes[start..]
        .windows(2)
        .position(|window| window == pair)
        .map(|offset| start + offset + 2)
}

fn find_unquoted(bytes: &[u8], needle: u8) -> Option<usize> {
    let mut quote = None;
    let mut escaped = false;
    for (index, byte) in bytes.iter().copied().enumerate() {
        if escaped {
            escaped = false;
        } else if byte == b'\\' && quote == Some(b'"') {
            escaped = true;
        } else if quote == Some(byte) {
            quote = None;
        } else if quote.is_none() && matches!(byte, b'"' | b'\'') {
            quote = Some(byte);
        } else if quote.is_none() && byte == needle {
            return Some(index);
        }
    }
    None
}

fn take_while(bytes: &[u8], index: usize, predicate: impl Fn(u8) -> bool) -> usize {
    bytes.get(index..).map_or(bytes.len(), |remaining| {
        remaining
            .iter()
            .position(|byte| !predicate(*byte))
            .map_or(bytes.len(), |offset| index + offset)
    })
}

fn next_boundary(text: &str, index: usize) -> usize {
    text[index..]
        .chars()
        .next()
        .map_or(text.len(), |character| index + character.len_utf8())
}

fn next_index(index: usize) -> Result<usize, SyntaxError> {
    index.checked_add(1).ok_or(SyntaxError::InvalidSpan)
}

fn require_forward(current: usize, next: usize, len: usize) -> Result<usize, SyntaxError> {
    if next <= current || next > len {
        return Err(SyntaxError::InvalidSpan);
    }
    Ok(next)
}

fn require_scan_complete(index: usize, len: usize) -> Result<(), SyntaxError> {
    if index == len {
        Ok(())
    } else {
        Err(SyntaxError::InvalidSpan)
    }
}

const fn is_identifier_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

const fn is_identifier_continue(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn rust_keyword(word: &str) -> bool {
    matches!(
        word,
        "as" | "async"
            | "await"
            | "break"
            | "const"
            | "continue"
            | "crate"
            | "dyn"
            | "else"
            | "enum"
            | "extern"
            | "false"
            | "fn"
            | "for"
            | "if"
            | "impl"
            | "in"
            | "let"
            | "loop"
            | "match"
            | "mod"
            | "move"
            | "mut"
            | "pub"
            | "ref"
            | "return"
            | "self"
            | "Self"
            | "static"
            | "struct"
            | "super"
            | "trait"
            | "true"
            | "type"
            | "unsafe"
            | "use"
            | "where"
            | "while"
            | "yield"
    )
}

#[derive(Debug)]
pub(crate) enum SyntaxError {
    InvalidBudget,
    AllocationFailed,
    SequenceExhausted,
    InvalidSpan,
    Text(TextError),
}

impl From<TextError> for SyntaxError {
    fn from(error: TextError) -> Self {
        Self::Text(error)
    }
}

impl fmt::Display for SyntaxError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBudget => formatter.write_str("syntax cache budget must be nonzero"),
            Self::AllocationFailed => formatter.write_str("syntax allocation failed"),
            Self::SequenceExhausted => formatter.write_str("syntax evidence sequence exhausted"),
            Self::InvalidSpan => formatter.write_str("syntax span is invalid"),
            Self::Text(error) => write!(formatter, "syntax text access failed: {error}"),
        }
    }
}

impl std::error::Error for SyntaxError {}

#[cfg(test)]
mod tests {
    use super::*;
    use alpine_text::Buffer;

    fn classes(language: SyntaxLanguage, text: &str) -> Result<Vec<SyntaxClass>, SyntaxError> {
        Ok(highlight_line(language, text)?
            .spans()
            .iter()
            .map(|span| span.class())
            .collect())
    }

    #[test]
    fn language_identity_is_compiled_and_path_bounded() {
        assert_eq!(DEFAULT_SYNTAX_BUDGET_BYTES, 4_194_304);
        assert_eq!(MAX_SYNTAX_LINE_BYTES, 65_536);
        assert_eq!(MAX_SYNTAX_SPANS_PER_LINE, 1_024);
        assert_eq!(
            SyntaxLanguage::from_path(Some(Path::new("main.rs"))),
            SyntaxLanguage::Rust
        );
        assert_eq!(
            SyntaxLanguage::from_path(Some(Path::new("README.md"))),
            SyntaxLanguage::Markdown
        );
        assert_eq!(
            SyntaxLanguage::from_path(Some(Path::new("Cargo.lock"))),
            SyntaxLanguage::Toml
        );
        assert_eq!(
            SyntaxLanguage::from_path(Some(Path::new("settings.toml"))),
            SyntaxLanguage::Toml
        );
        assert_eq!(
            SyntaxLanguage::from_path(Some(Path::new("data.json"))),
            SyntaxLanguage::Json
        );
        assert_eq!(
            SyntaxLanguage::from_path(Some(Path::new("image.png"))),
            SyntaxLanguage::PlainText
        );
        assert_eq!(SyntaxLanguage::from_path(None), SyntaxLanguage::PlainText);
    }

    #[test]
    fn rust_json_toml_and_markdown_tokens_are_discriminating() -> Result<(), SyntaxError> {
        assert_eq!(
            classes(SyntaxLanguage::Rust, "pub fn main() { let n = 42; // note")?,
            [
                SyntaxClass::Keyword,
                SyntaxClass::Keyword,
                SyntaxClass::Keyword,
                SyntaxClass::Number,
                SyntaxClass::Comment
            ]
        );
        let json_source = r#"{"name": "alpine", "ok": true, "n": -2}"#;
        let json = classes(SyntaxLanguage::Json, json_source)?;
        assert_eq!(
            json,
            [
                SyntaxClass::Property,
                SyntaxClass::String,
                SyntaxClass::Property,
                SyntaxClass::Keyword,
                SyntaxClass::Property,
                SyntaxClass::Number
            ]
        );
        assert_eq!(
            classes(SyntaxLanguage::Toml, "name = \"alpine\" # local")?,
            [
                SyntaxClass::Property,
                SyntaxClass::String,
                SyntaxClass::Comment
            ]
        );
        assert_eq!(
            classes(SyntaxLanguage::Markdown, "## Alpine Studio")?,
            [SyntaxClass::Heading]
        );
        assert_eq!(
            highlight_line(SyntaxLanguage::Markdown, "## Alpine Studio")?.spans(),
            &[SyntaxSpan {
                start_utf16: 0,
                end_utf16: 16,
                class: SyntaxClass::Heading
            }]
        );
        assert_eq!(
            classes(SyntaxLanguage::Markdown, "Use `cargo run` now")?,
            [SyntaxClass::Code]
        );
        Ok(())
    }

    #[test]
    fn unicode_offsets_and_span_boundaries_are_exact() -> Result<(), SyntaxError> {
        let line = highlight_line(SyntaxLanguage::Rust, "é let value = \"雪\"")?;
        assert_eq!(line.spans()[0].start_utf16(), 2);
        assert_eq!(line.spans()[0].end_utf16(), 5);
        assert_eq!(line.spans().last().map(|span| span.end_utf16()), Some(17));
        for pair in line.spans().windows(2) {
            assert!(pair[0].end_utf16() <= pair[1].start_utf16());
        }
        Ok(())
    }

    #[test]
    fn class_lookup_is_half_open_and_gap_aware() -> Result<(), SyntaxError> {
        let line = highlight_line(SyntaxLanguage::Rust, "x let gap Value")?;
        assert_eq!(line.class_at(0), None);
        assert_eq!(line.class_at(2), Some(SyntaxClass::Keyword));
        assert_eq!(line.class_at(4), Some(SyntaxClass::Keyword));
        assert_eq!(line.class_at(5), None);
        assert_eq!(line.class_at(9), None);
        assert_eq!(line.class_at(10), Some(SyntaxClass::Type));
        assert_eq!(line.class_at(14), Some(SyntaxClass::Type));
        assert_eq!(line.class_at(15), None);
        Ok(())
    }

    #[test]
    fn cache_reuses_exact_content_and_bounds_oversized_lines() -> Result<(), SyntaxError> {
        let first = Buffer::new("let value = 1;\n").snapshot();
        let equal = Buffer::new("let value = 1;\n").snapshot();
        let changed = Buffer::new("let value = 2;\n").snapshot();
        let mut cache = SyntaxCache::new(DEFAULT_SYNTAX_BUDGET_BYTES)?;
        let initial = cache.line(&first, 0, SyntaxLanguage::Rust)?;
        assert!(!initial.spans().is_empty());
        cache.begin_frame();
        let reused = cache.line(&equal, 0, SyntaxLanguage::Rust)?;
        assert!(Arc::ptr_eq(&initial, &reused));
        let changed_line = cache.line(&changed, 0, SyntaxLanguage::Rust)?;
        assert!(!Arc::ptr_eq(&initial, &changed_line));
        let oversized = Buffer::new(&"a".repeat(MAX_SYNTAX_LINE_BYTES + 1)).snapshot();
        let omitted = cache.line(&oversized, 0, SyntaxLanguage::Rust)?;
        assert!(omitted.omitted());
        assert_eq!(omitted.scanned_bytes(), 0);
        let snapshot = cache.snapshot();
        assert_eq!(snapshot.hits(), 1);
        assert_eq!(snapshot.misses(), 3);
        assert_eq!(snapshot.omitted_lines(), 1);
        assert!(snapshot.current_bytes() <= snapshot.budget_bytes());
        assert!(snapshot.peak_bytes() <= snapshot.budget_bytes());
        Ok(())
    }

    #[test]
    fn span_ceiling_degrades_without_overlap_or_growth() -> Result<(), SyntaxError> {
        let text = "1 ".repeat(MAX_SYNTAX_SPANS_PER_LINE + 8);
        let line = highlight_line(SyntaxLanguage::Json, &text)?;
        assert_eq!(line.spans().len(), MAX_SYNTAX_SPANS_PER_LINE);
        assert!(line.omitted());
        assert!(
            line.spans()
                .windows(2)
                .all(|pair| pair[0].end_utf16() <= pair[1].start_utf16())
        );
        Ok(())
    }

    #[test]
    fn quoted_toml_keys_do_not_overlap_value_tokens() -> Result<(), SyntaxError> {
        let line = highlight_line(SyntaxLanguage::Toml, r#""display name" = "Alpine""#)?;
        assert_eq!(
            line.spans()
                .iter()
                .map(|span| span.class())
                .collect::<Vec<_>>(),
            [SyntaxClass::Property, SyntaxClass::String]
        );
        assert!(
            line.spans()
                .windows(2)
                .all(|pair| pair[0].end_utf16() <= pair[1].start_utf16())
        );
        Ok(())
    }

    #[test]
    fn tiny_cache_budget_evicts_storage_instead_of_exceeding_ceiling() -> Result<(), SyntaxError> {
        let snapshot = Buffer::new("pub fn main() {}\n").snapshot();
        let mut cache = SyntaxCache::new(1)?;
        let line = cache.line(&snapshot, 0, SyntaxLanguage::Rust)?;
        assert!(!line.spans().is_empty());
        let evidence = cache.snapshot();
        assert_eq!(evidence.current_bytes(), 0);
        assert!(evidence.current_bytes() <= evidence.budget_bytes());
        assert!(evidence.peak_bytes() <= evidence.budget_bytes());
        Ok(())
    }

    #[test]
    fn cache_language_line_and_budget_boundaries_are_exact() -> Result<(), SyntaxError> {
        let snapshot = Buffer::new("true\n").snapshot();
        let mut cache = SyntaxCache::new(DEFAULT_SYNTAX_BUDGET_BYTES)?;
        let rust = cache.line(&snapshot, 0, SyntaxLanguage::Rust)?;
        let json = cache.line(&snapshot, 0, SyntaxLanguage::Json)?;
        assert!(!Arc::ptr_eq(&rust, &json));
        assert_eq!(cache.snapshot().misses(), 2);

        let exact = Buffer::new(&"A".repeat(MAX_SYNTAX_LINE_BYTES)).snapshot();
        let exact_line = cache.line(&exact, 0, SyntaxLanguage::Rust)?;
        assert!(!exact_line.omitted());
        assert_eq!(exact_line.scanned_bytes(), MAX_SYNTAX_LINE_BYTES);
        assert_eq!(
            exact_line.retained_bytes(),
            std::mem::size_of_val(exact_line.spans())
        );

        let retained_entries = cache.current.len();
        cache.budget_bytes = cache.current_bytes();
        cache.enforce_budget();
        assert_eq!(cache.current.len(), retained_entries);
        Ok(())
    }

    #[test]
    fn defensive_errors_and_sequence_limits_are_structured() -> Result<(), SyntaxError> {
        assert!(matches!(
            SyntaxCache::new(0),
            Err(SyntaxError::InvalidBudget)
        ));

        let snapshot = Buffer::new("let value = 1;\n").snapshot();
        let mut current_hit = SyntaxCache::new(DEFAULT_SYNTAX_BUDGET_BYTES)?;
        let initial = current_hit.line(&snapshot, 0, SyntaxLanguage::Rust)?;
        let repeated = current_hit.line(&snapshot, 0, SyntaxLanguage::Rust)?;
        assert!(Arc::ptr_eq(&initial, &repeated));
        current_hit.hits = u64::MAX;
        assert!(matches!(
            current_hit.line(&snapshot, 0, SyntaxLanguage::Rust),
            Err(SyntaxError::SequenceExhausted)
        ));

        let mut previous_hit = SyntaxCache::new(DEFAULT_SYNTAX_BUDGET_BYTES)?;
        let _ = previous_hit.line(&snapshot, 0, SyntaxLanguage::Rust)?;
        previous_hit.begin_frame();
        previous_hit.hits = u64::MAX;
        assert!(matches!(
            previous_hit.line(&snapshot, 0, SyntaxLanguage::Rust),
            Err(SyntaxError::SequenceExhausted)
        ));

        let mut miss = SyntaxCache::new(DEFAULT_SYNTAX_BUDGET_BYTES)?;
        miss.misses = u64::MAX;
        assert!(matches!(
            miss.line(&snapshot, 0, SyntaxLanguage::Rust),
            Err(SyntaxError::SequenceExhausted)
        ));

        let oversized = Buffer::new(&"x".repeat(MAX_SYNTAX_LINE_BYTES + 1)).snapshot();
        let mut omission = SyntaxCache::new(DEFAULT_SYNTAX_BUDGET_BYTES)?;
        omission.omitted_lines = u64::MAX;
        assert!(matches!(
            omission.line(&oversized, 0, SyntaxLanguage::Rust),
            Err(SyntaxError::SequenceExhausted)
        ));

        let mut text_error = SyntaxCache::new(DEFAULT_SYNTAX_BUDGET_BYTES)?;
        assert!(matches!(
            text_error.line(&snapshot, 2, SyntaxLanguage::Rust),
            Err(error)
                if matches!(&error, SyntaxError::Text(_))
                    && error.to_string().starts_with("syntax text access failed:")
        ));

        let mut current_comparison = SyntaxCache::new(DEFAULT_SYNTAX_BUDGET_BYTES)?;
        let _ = current_comparison.line(&snapshot, 0, SyntaxLanguage::Rust)?;
        current_comparison.current[0].range = 0..usize::MAX;
        assert!(matches!(
            current_comparison.line(&snapshot, 0, SyntaxLanguage::Rust),
            Err(SyntaxError::Text(_))
        ));

        let mut previous_comparison = SyntaxCache::new(DEFAULT_SYNTAX_BUDGET_BYTES)?;
        let _ = previous_comparison.line(&snapshot, 0, SyntaxLanguage::Rust)?;
        previous_comparison.begin_frame();
        previous_comparison.previous[0].range = 0..usize::MAX;
        assert!(matches!(
            previous_comparison.line(&snapshot, 0, SyntaxLanguage::Rust),
            Err(SyntaxError::Text(_))
        ));
        Ok(())
    }

    #[test]
    fn allocation_span_and_utf16_failures_are_discriminating() -> Result<(), SyntaxError> {
        let mut values = Vec::<u8>::new();
        assert!(matches!(
            reserve(&mut values, usize::MAX),
            Err(SyntaxError::AllocationFailed)
        ));
        assert!(matches!(
            reserve_exact(&mut values, usize::MAX),
            Err(SyntaxError::AllocationFailed)
        ));

        let mut emitter = Emitter::new("abc");
        emitter.push(0, 0, SyntaxClass::Code)?;
        emitter.push(0, 4, SyntaxClass::Code)?;
        emitter.push(0, 2, SyntaxClass::Code)?;
        assert!(matches!(
            emitter.push(1, 3, SyntaxClass::String),
            Err(SyntaxError::InvalidSpan)
        ));

        let mut touching = Emitter::new("ab");
        touching.push(0, 1, SyntaxClass::Code)?;
        touching.push(1, 2, SyntaxClass::String)?;
        assert_eq!(touching.finish()?.spans().len(), 2);

        let mut byte = 0;
        let mut utf16 = 0;
        assert!(matches!(
            advance_utf16("é", &mut byte, &mut utf16, 1),
            Err(SyntaxError::InvalidSpan)
        ));
        byte = 1;
        assert!(matches!(
            advance_utf16("é", &mut byte, &mut utf16, 2),
            Err(SyntaxError::InvalidSpan)
        ));
        byte = 0;
        utf16 = u32::MAX;
        assert!(matches!(
            advance_utf16("a", &mut byte, &mut utf16, 1),
            Err(SyntaxError::InvalidSpan)
        ));
        Ok(())
    }

    #[test]
    fn scanner_progress_is_strict_bounded_and_complete() -> Result<(), SyntaxError> {
        assert!(classes(SyntaxLanguage::Markdown, "plain")?.is_empty());
        assert_eq!(require_forward(0, 1, 2)?, 1);
        assert!(matches!(
            require_forward(1, 1, 2),
            Err(SyntaxError::InvalidSpan)
        ));
        assert!(matches!(
            require_forward(1, 0, 2),
            Err(SyntaxError::InvalidSpan)
        ));
        assert!(matches!(
            require_forward(1, 3, 2),
            Err(SyntaxError::InvalidSpan)
        ));
        require_scan_complete(2, 2)?;
        assert!(matches!(
            require_scan_complete(1, 2),
            Err(SyntaxError::InvalidSpan)
        ));
        Ok(())
    }

    #[test]
    fn every_compiled_lexer_fallback_is_explicit() -> Result<(), SyntaxError> {
        let mut crlf = String::from("value\r\n");
        trim_line_ending(&mut crlf);
        assert_eq!(crlf, "value");
        let mut cr = String::from("value\r");
        trim_line_ending(&mut cr);
        assert_eq!(cr, "value");

        assert!(classes(SyntaxLanguage::PlainText, "plain")?.is_empty());
        assert!(
            highlight_line(SyntaxLanguage::Rust, &"x".repeat(MAX_SYNTAX_LINE_BYTES + 1))?.omitted()
        );
        assert_eq!(
            classes(SyntaxLanguage::Rust, "/* closed */ /* open")?,
            [SyntaxClass::Comment, SyntaxClass::Comment]
        );
        assert_eq!(
            classes(SyntaxLanguage::Toml, "[package]")?,
            [SyntaxClass::Heading]
        );
        assert!(classes(SyntaxLanguage::Toml, "[package")?.is_empty());
        assert!(classes(SyntaxLanguage::Toml, "package]")?.is_empty());
        assert!(classes(SyntaxLanguage::Toml, "bare value")?.is_empty());
        assert_eq!(
            classes(SyntaxLanguage::Toml, " = 1")?,
            [SyntaxClass::Number]
        );
        let mut overlapping_toml = Emitter::new("name = 1");
        overlapping_toml.push(0, 1, SyntaxClass::Code)?;
        assert!(matches!(
            highlight_toml("name = 1", &mut overlapping_toml),
            Err(SyntaxError::InvalidSpan)
        ));
        assert_eq!(
            classes(SyntaxLanguage::Markdown, "####### not-a-heading `open")?,
            [SyntaxClass::Code]
        );
        let escaped_source = r#"{"escaped": "a\\\"b", "false": false, "none": null}"#;
        let escaped_json = classes(SyntaxLanguage::Json, escaped_source)?;
        assert_eq!(
            escaped_json,
            [
                SyntaxClass::Property,
                SyntaxClass::String,
                SyntaxClass::Property,
                SyntaxClass::Keyword,
                SyntaxClass::Property,
                SyntaxClass::Keyword
            ]
        );
        assert_eq!(
            classes(SyntaxLanguage::Toml, r#""a\\\"=b" = "unterminated"#)?,
            [SyntaxClass::Property, SyntaxClass::String]
        );
        assert_eq!(
            classes(SyntaxLanguage::Json, r#"{"name"  : "alpine"}"#)?,
            [SyntaxClass::Property, SyntaxClass::String]
        );
        assert_eq!(find_pair(b"ab*/cd", 1, *b"*/"), Some(4));
        assert_eq!(find_pair(b"abcdef", 1, *b"*/"), None);
        assert_eq!(find_unquoted(br#""a\"=b" = 1"#, b'='), Some(8));
        Ok(())
    }

    #[test]
    fn previous_and_empty_capacity_eviction_release_every_accounted_byte() -> Result<(), SyntaxError>
    {
        let snapshot = Buffer::new("pub fn main() {}\n").snapshot();
        let mut previous = SyntaxCache::new(DEFAULT_SYNTAX_BUDGET_BYTES)?;
        let _ = previous.line(&snapshot, 0, SyntaxLanguage::Rust)?;
        previous.begin_frame();
        previous.budget_bytes = 1;
        previous.enforce_budget();
        assert!(previous.previous.is_empty());
        assert!(previous.current_bytes() <= previous.budget_bytes);

        let mut empty_capacity = SyntaxCache::new(1)?;
        empty_capacity
            .current
            .try_reserve(8)
            .map_err(|_| SyntaxError::AllocationFailed)?;
        empty_capacity.enforce_budget();
        assert!(empty_capacity.current.is_empty());
        assert!(empty_capacity.current_bytes() <= empty_capacity.budget_bytes);
        Ok(())
    }

    #[test]
    fn syntax_error_messages_are_stable() {
        assert_eq!(
            SyntaxError::InvalidBudget.to_string(),
            "syntax cache budget must be nonzero"
        );
        assert_eq!(
            SyntaxError::AllocationFailed.to_string(),
            "syntax allocation failed"
        );
        assert_eq!(
            SyntaxError::SequenceExhausted.to_string(),
            "syntax evidence sequence exhausted"
        );
        assert_eq!(
            SyntaxError::InvalidSpan.to_string(),
            "syntax span is invalid"
        );
    }
}
