//! Compiled language table: path lookup, overlay disable, and server identity.

use std::{
    env, fmt, fs,
    path::{Path, PathBuf},
};

use crate::syntax::SyntaxLanguage;

const BUILTIN_TABLE: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/languages.toml"));
const MAX_TABLE_BYTES: usize = 16_384;
const MAX_LANGUAGES: usize = 16;
const MAX_EXTENSIONS: usize = 16;
const MAX_FILENAMES: usize = 8;
const MAX_BINARIES: usize = 8;
const MAX_ARGUMENTS: usize = 8;
const MAX_ROOT_MARKERS: usize = 8;
const MAX_FIELD_BYTES: usize = 64;
const MAX_ID_BYTES: usize = 32;
const MAX_DISABLED: usize = 16;
const OVERLAY_NAME: &str = "languages.overlay.toml";

/// One closed language row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LanguageSpec {
    id: Box<str>,
    highlighter: SyntaxLanguage,
    extensions: Box<[Box<str>]>,
    filenames: Box<[Box<str>]>,
    lsp_language_id: Option<Box<str>>,
    server_id: Option<Box<str>>,
    binaries: Box<[Box<str>]>,
    arguments: Box<[Box<str>]>,
    env_override: Option<Box<str>>,
    root_markers: Box<[Box<str>]>,
}

impl LanguageSpec {
    pub(crate) fn id(&self) -> &str {
        &self.id
    }

    pub(crate) fn highlighter(&self) -> SyntaxLanguage {
        self.highlighter
    }

    pub(crate) fn lsp_language_id(&self) -> Option<&str> {
        self.lsp_language_id.as_deref()
    }

    pub(crate) fn server_id(&self) -> Option<&str> {
        self.server_id.as_deref()
    }

    pub(crate) fn binaries(&self) -> &[Box<str>] {
        &self.binaries
    }

    pub(crate) fn arguments(&self) -> &[Box<str>] {
        &self.arguments
    }

    pub(crate) fn env_override(&self) -> Option<&str> {
        self.env_override.as_deref()
    }

    pub(crate) fn root_markers(&self) -> &[Box<str>] {
        &self.root_markers
    }
}

/// Path-to-language lookup loaded from the compiled table plus an optional overlay.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LanguageRegistry {
    languages: Box<[LanguageSpec]>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LanguageRegistryError {
    TableTooLarge,
    TooManyLanguages,
    MissingId,
    DuplicateId,
    InvalidField,
    AllocationFailed,
}

impl fmt::Display for LanguageRegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::TableTooLarge => "language table exceeds the retained-byte ceiling",
            Self::TooManyLanguages => "language table exceeds the language ceiling",
            Self::MissingId => "language row is missing id",
            Self::DuplicateId => "language table repeats an id",
            Self::InvalidField => "language table field is invalid",
            Self::AllocationFailed => "language table allocation failed",
        })
    }
}

impl std::error::Error for LanguageRegistryError {}

impl LanguageRegistry {
    pub(crate) fn compiled() -> Self {
        parse_language_table(BUILTIN_TABLE).unwrap_or_else(|_| Self {
            languages: Box::from([]),
        })
    }

    #[cfg(test)]
    pub(crate) fn parse(text: &str) -> Result<Self, LanguageRegistryError> {
        parse_language_table(text)
    }

    pub(crate) fn from_home(home: Option<&Path>) -> Self {
        let mut registry = Self::compiled();
        if let Some(home) = home {
            let overlay = overlay_path(home);
            if let Ok(text) = fs::read_to_string(&overlay) {
                let _ = registry.apply_overlay(&text);
            }
        }
        registry
    }

    pub(crate) fn from_process_home() -> Self {
        Self::from_home(env::var_os("HOME").map(PathBuf::from).as_deref())
    }

    pub(crate) fn apply_overlay(&mut self, text: &str) -> Result<(), LanguageRegistryError> {
        let disabled = parse_disabled(text)?;
        if disabled.is_empty() {
            return Ok(());
        }
        let mut kept = Vec::new();
        kept.try_reserve(self.languages.len())
            .map_err(|_| LanguageRegistryError::AllocationFailed)?;
        for language in &self.languages {
            if !disabled
                .iter()
                .any(|id| id.as_ref() == language.id.as_ref())
            {
                kept.push(language.clone());
            }
        }
        self.languages = kept.into();
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn without(&self, id: &str) -> Self {
        let languages = self
            .languages
            .iter()
            .filter(|language| language.id.as_ref() != id)
            .cloned()
            .collect::<Vec<_>>()
            .into();
        Self { languages }
    }

    #[cfg(test)]
    pub(crate) fn get(&self, id: &str) -> Option<&LanguageSpec> {
        self.languages
            .iter()
            .find(|language| language.id.as_ref() == id)
    }

    #[cfg(test)]
    pub(crate) fn iter(&self) -> impl Iterator<Item = &LanguageSpec> {
        self.languages.iter()
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.languages.len()
    }

    pub(crate) fn for_path(&self, path: Option<&Path>) -> Option<&LanguageSpec> {
        let path = path?;
        if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
            for language in &self.languages {
                if language
                    .filenames
                    .iter()
                    .any(|filename| filename.as_ref() == name)
                {
                    return Some(language);
                }
            }
        }
        let extension = path.extension().and_then(|extension| extension.to_str())?;
        self.languages.iter().find(|language| {
            language
                .extensions
                .iter()
                .any(|candidate| candidate.as_ref() == extension)
        })
    }

    pub(crate) fn highlighter_for_path(&self, path: Option<&Path>) -> SyntaxLanguage {
        self.for_path(path)
            .map_or(SyntaxLanguage::PlainText, LanguageSpec::highlighter)
    }

    pub(crate) fn workspace_root<'a>(&self, path: &'a Path) -> &'a Path {
        let markers = self.for_path(Some(path)).map(LanguageSpec::root_markers);
        let Some(markers) = markers else {
            return path.parent().unwrap_or(path);
        };
        if markers.is_empty() {
            return path.parent().unwrap_or(path);
        }
        let mut current = if path.is_file() {
            path.parent().unwrap_or(path)
        } else {
            path
        };
        loop {
            if markers
                .iter()
                .any(|marker| current.join(marker.as_ref()).exists())
            {
                return current;
            }
            match current.parent() {
                Some(parent) if parent != current => current = parent,
                _ => return path.parent().unwrap_or(path),
            }
        }
    }
}

pub(crate) fn overlay_path(home: &Path) -> PathBuf {
    home.join("Library")
        .join("Application Support")
        .join("Alpine Editor")
        .join(OVERLAY_NAME)
}

fn parse_language_table(text: &str) -> Result<LanguageRegistry, LanguageRegistryError> {
    if text.len() > MAX_TABLE_BYTES {
        return Err(LanguageRegistryError::TableTooLarge);
    }
    let mut languages = Vec::new();
    languages
        .try_reserve(MAX_LANGUAGES)
        .map_err(|_| LanguageRegistryError::AllocationFailed)?;
    let mut current: Option<TableBuilder> = None;
    for raw in text.lines() {
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        if line == "[[language]]" {
            if let Some(builder) = current.take() {
                push_language(&mut languages, builder)?;
            }
            current = Some(TableBuilder::default());
            continue;
        }
        let builder = current
            .as_mut()
            .ok_or(LanguageRegistryError::InvalidField)?;
        let (key, value) = split_assignment(line)?;
        match key {
            "id" => builder.id = Some(parse_string(value)?),
            "highlighter" => builder.highlighter = Some(parse_string(value)?),
            "lsp_language_id" => builder.lsp_language_id = Some(parse_string(value)?),
            "server_id" => builder.server_id = Some(parse_string(value)?),
            "env_override" => builder.env_override = Some(parse_string(value)?),
            "extensions" => builder.extensions = parse_string_array(value)?,
            "filenames" => builder.filenames = parse_string_array(value)?,
            "binaries" => builder.binaries = parse_string_array(value)?,
            "arguments" => builder.arguments = parse_string_array(value)?,
            "root_markers" => builder.root_markers = parse_string_array(value)?,
            _ => return Err(LanguageRegistryError::InvalidField),
        }
    }
    if let Some(builder) = current.take() {
        push_language(&mut languages, builder)?;
    }
    Ok(LanguageRegistry {
        languages: languages.into(),
    })
}

fn push_language(
    languages: &mut Vec<LanguageSpec>,
    builder: TableBuilder,
) -> Result<(), LanguageRegistryError> {
    if languages.len() == MAX_LANGUAGES {
        return Err(LanguageRegistryError::TooManyLanguages);
    }
    let spec = builder.finish()?;
    if languages.iter().any(|language| language.id == spec.id) {
        return Err(LanguageRegistryError::DuplicateId);
    }
    languages.push(spec);
    Ok(())
}

fn parse_disabled(text: &str) -> Result<Vec<Box<str>>, LanguageRegistryError> {
    if text.len() > MAX_TABLE_BYTES {
        return Err(LanguageRegistryError::TableTooLarge);
    }
    let mut disabled = Vec::new();
    for raw in text.lines() {
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        let (key, value) = split_assignment(line)?;
        if key != "disabled" {
            return Err(LanguageRegistryError::InvalidField);
        }
        disabled = parse_string_array(value)?;
        if disabled.len() > MAX_DISABLED {
            return Err(LanguageRegistryError::TooManyLanguages);
        }
    }
    Ok(disabled)
}

#[derive(Default)]
struct TableBuilder {
    id: Option<Box<str>>,
    highlighter: Option<Box<str>>,
    extensions: Vec<Box<str>>,
    filenames: Vec<Box<str>>,
    lsp_language_id: Option<Box<str>>,
    server_id: Option<Box<str>>,
    binaries: Vec<Box<str>>,
    arguments: Vec<Box<str>>,
    env_override: Option<Box<str>>,
    root_markers: Vec<Box<str>>,
}

impl TableBuilder {
    fn finish(self) -> Result<LanguageSpec, LanguageRegistryError> {
        let id = self.id.ok_or(LanguageRegistryError::MissingId)?;
        if id.len() > MAX_ID_BYTES {
            return Err(LanguageRegistryError::InvalidField);
        }
        let highlighter = self.highlighter.as_deref().map_or(
            SyntaxLanguage::PlainText,
            SyntaxLanguage::from_registry_name,
        );
        if self.extensions.len() > MAX_EXTENSIONS
            || self.filenames.len() > MAX_FILENAMES
            || self.binaries.len() > MAX_BINARIES
            || self.arguments.len() > MAX_ARGUMENTS
            || self.root_markers.len() > MAX_ROOT_MARKERS
        {
            return Err(LanguageRegistryError::InvalidField);
        }
        Ok(LanguageSpec {
            id,
            highlighter,
            extensions: self.extensions.into(),
            filenames: self.filenames.into(),
            lsp_language_id: self.lsp_language_id,
            server_id: self.server_id,
            binaries: self.binaries.into(),
            arguments: self.arguments.into(),
            env_override: self.env_override,
            root_markers: self.root_markers.into(),
        })
    }
}

fn strip_comment(line: &str) -> &str {
    let mut in_string = false;
    for (index, character) in line.char_indices() {
        if character == '"' {
            in_string = !in_string;
        } else if character == '#' && !in_string {
            return &line[..index];
        }
    }
    line
}

fn split_assignment(line: &str) -> Result<(&str, &str), LanguageRegistryError> {
    let equal = line.find('=').ok_or(LanguageRegistryError::InvalidField)?;
    let key = line[..equal].trim();
    let value = line[equal + 1..].trim();
    if key.is_empty() || key.len() > MAX_FIELD_BYTES {
        return Err(LanguageRegistryError::InvalidField);
    }
    Ok((key, value))
}

fn parse_string(value: &str) -> Result<Box<str>, LanguageRegistryError> {
    let value = value.trim();
    let inner = value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .ok_or(LanguageRegistryError::InvalidField)?;
    if inner.len() > MAX_FIELD_BYTES || inner.contains('"') {
        return Err(LanguageRegistryError::InvalidField);
    }
    Ok(Box::from(inner))
}

fn parse_string_array(value: &str) -> Result<Vec<Box<str>>, LanguageRegistryError> {
    let value = value.trim();
    let inner = value
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .ok_or(LanguageRegistryError::InvalidField)?;
    if inner.trim().is_empty() {
        return Ok(Vec::new());
    }
    let mut values = Vec::new();
    for part in inner.split(',') {
        values.push(parse_string(part.trim())?);
    }
    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiled_table_covers_the_phase_two_cohort() {
        let registry = LanguageRegistry::compiled();
        assert_eq!(registry.len(), 9);
        assert!(registry.iter().any(|language| language.id() == "rust"));
        assert!(registry.get("rust").is_some());
        assert!(registry.get("python").is_some());
        assert!(registry.get("cpp").is_some());
        assert!(registry.get("java").is_some());
        assert!(registry.get("typescript").is_some());
        assert!(registry.get("javascript").is_some());
        assert_eq!(
            registry.get("javascript").and_then(LanguageSpec::server_id),
            Some("typescript")
        );
        assert_eq!(
            registry.highlighter_for_path(Some(Path::new("main.py"))),
            SyntaxLanguage::Python
        );
        assert_eq!(
            registry.highlighter_for_path(Some(Path::new("Cargo.lock"))),
            SyntaxLanguage::Toml
        );
        assert_eq!(
            registry.highlighter_for_path(Some(Path::new("App.tsx"))),
            SyntaxLanguage::TypeScript
        );
        assert_eq!(
            registry.highlighter_for_path(Some(Path::new("lib.hpp"))),
            SyntaxLanguage::Cpp
        );
    }

    #[test]
    fn deleting_a_row_removes_lookup_without_code() -> Result<(), LanguageRegistryError> {
        let registry = LanguageRegistry::compiled().without("java");
        assert!(registry.get("java").is_none());
        assert_eq!(
            registry.highlighter_for_path(Some(Path::new("Main.java"))),
            SyntaxLanguage::PlainText
        );
        assert_eq!(
            registry.highlighter_for_path(Some(Path::new("lib.rs"))),
            SyntaxLanguage::Rust
        );
        let mut overlay = LanguageRegistry::compiled();
        overlay.apply_overlay(r#"disabled = ["java"]"#)?;
        assert!(overlay.get("java").is_none());
        assert!(overlay.get("python").is_some());
        Ok(())
    }

    #[test]
    fn unknown_keys_and_duplicate_ids_are_rejected() {
        assert_eq!(
            LanguageRegistry::parse("[[language]]\nid = \"a\"\nunknown = \"x\"\n").err(),
            Some(LanguageRegistryError::InvalidField)
        );
        assert_eq!(
            LanguageRegistry::parse("[[language]]\nid = \"a\"\n[[language]]\nid = \"a\"\n").err(),
            Some(LanguageRegistryError::DuplicateId)
        );
    }
}
