//! Bounded local settings file loading and current-generation admission.

use std::{
    borrow::Cow,
    ffi::OsString,
    fmt,
    fs::{self, File},
    io::{self, Read as _},
    path::{Path, PathBuf},
};

use alpine_core::LinearRgba;
use alpine_platform_macos::Modifiers;
use serde_json::{Map, Value};

use super::{
    EditorSettingsPatch, FONT_NAME, KeyAction, KeyBinding, Keymap, MAX_FONT_NAME_BYTES,
    MAX_KEY_BINDINGS, SettingsAdmission, SettingsEffect, SettingsFailure, SettingsLayer,
    SettingsSource, SettingsState, SettingsUpdate, StudioSettings, StudioTheme, SyntaxTheme, color,
};
use crate::commands::StudioCommand;

pub(crate) const SETTINGS_VERSION: u64 = 1;
pub(crate) const LEGACY_SETTINGS_VERSION: u64 = 0;
pub(crate) const MAX_SETTINGS_FILE_BYTES: usize = 64 * 1_024;
pub(crate) const MAX_SETTINGS_PATH_BYTES: usize = 4_096;
pub(crate) const MAX_SETTINGS_JSON_DEPTH: usize = 8;
pub(crate) const MAX_SETTINGS_JSON_VALUES: usize = 512;
pub(crate) const MAX_SETTINGS_STRING_BYTES: usize = 32 * 1_024;
const MAX_SETTINGS_READ_BYTES: usize = MAX_SETTINGS_FILE_BYTES + 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SettingsLoadError {
    PathTooLong,
    NotRegularFile,
    FileTooLarge,
    ConcurrentEdit,
    Io {
        operation: &'static str,
        kind: io::ErrorKind,
    },
    InvalidJson,
    JsonTooDeep,
    TooManyValues,
    TooManyStringBytes,
    MissingVersion,
    UnknownVersion(u64),
    UnknownField,
    MissingField(&'static str),
    InvalidValue(&'static str),
    AllocationFailed,
}

impl fmt::Display for SettingsLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PathTooLong => write!(
                formatter,
                "settings path exceeds its {MAX_SETTINGS_PATH_BYTES}-byte limit"
            ),
            Self::NotRegularFile => formatter.write_str("settings path is not a regular file"),
            Self::FileTooLarge => write!(
                formatter,
                "settings file exceeds its {MAX_SETTINGS_FILE_BYTES}-byte limit"
            ),
            Self::ConcurrentEdit => {
                formatter.write_str("settings file changed while it was being read")
            }
            Self::Io { operation, kind } => {
                write!(formatter, "settings {operation} failed with {kind:?}")
            }
            Self::InvalidJson => formatter.write_str("settings JSON is malformed"),
            Self::JsonTooDeep => write!(
                formatter,
                "settings JSON exceeds its {MAX_SETTINGS_JSON_DEPTH}-level depth limit"
            ),
            Self::TooManyValues => write!(
                formatter,
                "settings JSON exceeds its {MAX_SETTINGS_JSON_VALUES}-value limit"
            ),
            Self::TooManyStringBytes => write!(
                formatter,
                "settings JSON exceeds its {MAX_SETTINGS_STRING_BYTES}-byte string limit"
            ),
            Self::MissingVersion => formatter.write_str("settings version is missing"),
            Self::UnknownVersion(version) => {
                write!(formatter, "settings version {version} is unsupported")
            }
            Self::UnknownField => formatter.write_str("settings contain an unknown field"),
            Self::MissingField(field) => write!(formatter, "settings field {field} is missing"),
            Self::InvalidValue(field) => write!(formatter, "settings field {field} is invalid"),
            Self::AllocationFailed => formatter.write_str("settings allocation failed"),
        }
    }
}

impl std::error::Error for SettingsLoadError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SettingsLoadFailure {
    pub(crate) source: SettingsSource,
    pub(crate) error: SettingsLoadError,
}

impl fmt::Display for SettingsLoadFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?} settings: {}", self.source, self.error)
    }
}

impl std::error::Error for SettingsLoadFailure {}

#[derive(Clone, Debug, PartialEq)]
struct SettingsPaths {
    global: Result<Option<PathBuf>, SettingsLoadFailure>,
    project: Result<Option<PathBuf>, SettingsLoadFailure>,
}

impl SettingsPaths {
    fn new(home: Option<OsString>, workspace: Option<&Path>) -> Self {
        Self {
            global: global_path(home),
            project: project_path(workspace),
        }
    }

    #[cfg(test)]
    fn explicit(global: Option<PathBuf>, project: Option<PathBuf>) -> Self {
        Self {
            global: checked_path(global, SettingsSource::Global),
            project: checked_path(project, SettingsSource::Project),
        }
    }

    fn replace_project(&mut self, workspace: Option<&Path>) -> bool {
        let project = project_path(workspace);
        if self.project == project {
            false
        } else {
            self.project = project;
            true
        }
    }

    fn retained_bytes(&self) -> usize {
        [&self.global, &self.project]
            .into_iter()
            .filter_map(|path| path.as_ref().ok().and_then(Option::as_ref))
            .map(|path| path.as_os_str().as_encoded_bytes().len())
            .sum()
    }
}

fn global_path(home: Option<OsString>) -> Result<Option<PathBuf>, SettingsLoadFailure> {
    let path = home
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .map(|home| {
            home.join("Library")
                .join("Application Support")
                .join("Alpine Studio")
                .join("settings.json")
        });
    checked_path(path, SettingsSource::Global)
}

fn project_path(workspace: Option<&Path>) -> Result<Option<PathBuf>, SettingsLoadFailure> {
    checked_path(
        workspace.map(|root| root.join(".alpine").join("settings.json")),
        SettingsSource::Project,
    )
}

fn checked_path(
    path: Option<PathBuf>,
    source: SettingsSource,
) -> Result<Option<PathBuf>, SettingsLoadFailure> {
    if path
        .as_ref()
        .is_some_and(|path| path.as_os_str().as_encoded_bytes().len() > MAX_SETTINGS_PATH_BYTES)
    {
        Err(SettingsLoadFailure {
            source,
            error: SettingsLoadError::PathTooLong,
        })
    } else {
        Ok(path)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct DecodeReport {
    file_bytes: usize,
    parsed_values: usize,
    string_bytes: usize,
    migrations: u64,
}

impl DecodeReport {
    fn merge(self, other: Self) -> Result<Self, SettingsLoadFailure> {
        let overflow = || SettingsLoadFailure {
            source: SettingsSource::Runtime,
            error: SettingsLoadError::AllocationFailed,
        };
        Ok(Self {
            file_bytes: self
                .file_bytes
                .checked_add(other.file_bytes)
                .ok_or_else(overflow)?,
            parsed_values: self
                .parsed_values
                .checked_add(other.parsed_values)
                .ok_or_else(overflow)?,
            string_bytes: self
                .string_bytes
                .checked_add(other.string_bytes)
                .ok_or_else(overflow)?,
            migrations: self.migrations.saturating_add(other.migrations),
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
struct LoadedSettings {
    update: SettingsUpdate,
    report: DecodeReport,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SettingsLoadOutput {
    generation: u64,
    announce: bool,
    result: Result<LoadedSettings, SettingsLoadFailure>,
}

#[derive(Clone, Debug)]
pub(crate) struct SettingsLoadRequest {
    generation: u64,
    announce: bool,
    paths: SettingsPaths,
}

impl SettingsLoadRequest {
    pub(crate) const fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) const fn announce(&self) -> bool {
        self.announce
    }

    pub(crate) fn execute(self) -> SettingsLoadOutput {
        SettingsLoadOutput {
            generation: self.generation,
            announce: self.announce,
            result: load_settings(self.generation, &self.paths),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SettingsReloadError {
    GenerationExhausted,
    SubmissionFailed,
}

impl fmt::Display for SettingsReloadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GenerationExhausted => formatter.write_str("settings generation exhausted"),
            Self::SubmissionFailed => formatter.write_str("settings worker queue rejected reload"),
        }
    }
}

impl std::error::Error for SettingsReloadError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SettingsReloadAdmission {
    Applied {
        effect: SettingsEffect,
        revision: u64,
        migrations: u64,
        announce: bool,
    },
    Unchanged {
        revision: u64,
        migrations: u64,
        announce: bool,
    },
    Stale,
    Failed(SettingsLoadFailure),
    Rejected(SettingsFailure),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[allow(
    dead_code,
    reason = "the opt-in local diagnostic overlay will consume this bounded snapshot"
)]
pub(crate) struct SettingsReloadReport {
    pub(crate) requested_generation: u64,
    pub(crate) submitted_generation: u64,
    pub(crate) in_flight: bool,
    pub(crate) pending: bool,
    pub(crate) path_bytes: usize,
    pub(crate) current_file_bytes: usize,
    pub(crate) peak_file_bytes: usize,
    pub(crate) current_parsed_values: usize,
    pub(crate) peak_parsed_values: usize,
    pub(crate) current_string_bytes: usize,
    pub(crate) peak_string_bytes: usize,
    pub(crate) reloads: u64,
    pub(crate) submissions: u64,
    pub(crate) migrations: u64,
    pub(crate) stale_results: u64,
    pub(crate) failures: u64,
}

pub(crate) struct SettingsReload {
    paths: SettingsPaths,
    requested_generation: u64,
    submitted_generation: u64,
    in_flight: bool,
    pending: bool,
    pending_announcement: bool,
    report: SettingsReloadReport,
}

impl SettingsReload {
    pub(crate) fn new(home: Option<OsString>, workspace: Option<&Path>) -> Self {
        Self::from_paths(SettingsPaths::new(home, workspace))
    }

    #[cfg(test)]
    pub(crate) fn explicit(global: Option<PathBuf>, project: Option<PathBuf>) -> Self {
        Self::from_paths(SettingsPaths::explicit(global, project))
    }

    fn from_paths(paths: SettingsPaths) -> Self {
        let path_bytes = paths.retained_bytes();
        Self {
            paths,
            requested_generation: 1,
            submitted_generation: 0,
            in_flight: false,
            pending: true,
            pending_announcement: false,
            report: SettingsReloadReport {
                requested_generation: 1,
                pending: true,
                path_bytes,
                reloads: 1,
                ..SettingsReloadReport::default()
            },
        }
    }

    pub(crate) fn request(&mut self, announce: bool) -> Result<u64, SettingsReloadError> {
        let generation = self
            .requested_generation
            .checked_add(1)
            .ok_or(SettingsReloadError::GenerationExhausted)?;
        self.requested_generation = generation;
        self.pending = true;
        self.pending_announcement |= announce;
        self.report.requested_generation = generation;
        self.report.pending = true;
        self.report.reloads = self.report.reloads.saturating_add(1);
        Ok(generation)
    }

    pub(crate) fn replace_project(
        &mut self,
        workspace: Option<&Path>,
    ) -> Result<bool, SettingsReloadError> {
        if !self.paths.replace_project(workspace) {
            return Ok(false);
        }
        self.report.path_bytes = self.paths.retained_bytes();
        self.request(false).map(|_| true)
    }

    pub(crate) fn take_request(&mut self) -> Option<SettingsLoadRequest> {
        if !self.pending || self.in_flight {
            return None;
        }
        self.pending = false;
        self.in_flight = true;
        self.submitted_generation = self.requested_generation;
        let announce = std::mem::take(&mut self.pending_announcement);
        self.report.submitted_generation = self.submitted_generation;
        self.report.in_flight = true;
        self.report.pending = false;
        self.report.submissions = self.report.submissions.saturating_add(1);
        Some(SettingsLoadRequest {
            generation: self.submitted_generation,
            announce,
            paths: self.paths.clone(),
        })
    }

    pub(crate) fn reject_submission(
        &mut self,
        generation: u64,
        announce: bool,
    ) -> Result<(), SettingsReloadError> {
        if !self.in_flight || generation != self.submitted_generation {
            self.report.stale_results = self.report.stale_results.saturating_add(1);
            return Ok(());
        }
        self.in_flight = false;
        self.pending = true;
        self.pending_announcement |= announce;
        self.report.in_flight = false;
        self.report.pending = true;
        self.report.failures = self.report.failures.saturating_add(1);
        Err(SettingsReloadError::SubmissionFailed)
    }

    pub(crate) fn admit(
        &mut self,
        output: SettingsLoadOutput,
        state: &mut SettingsState,
    ) -> SettingsReloadAdmission {
        if !completion_is_current(
            self.requested_generation,
            self.submitted_generation,
            output.generation,
            self.in_flight,
        ) {
            if self.in_flight && output.generation == self.submitted_generation {
                self.in_flight = false;
                self.report.in_flight = false;
            }
            self.report.stale_results = self.report.stale_results.saturating_add(1);
            return SettingsReloadAdmission::Stale;
        }
        self.in_flight = false;
        self.report.in_flight = false;
        let loaded = match output.result {
            Ok(loaded) => loaded,
            Err(failure) => {
                self.report.failures = self.report.failures.saturating_add(1);
                return SettingsReloadAdmission::Failed(failure);
            }
        };
        self.observe_decode(loaded.report);
        match state.admit(&loaded.update) {
            SettingsAdmission::Applied { revision, effect } => SettingsReloadAdmission::Applied {
                effect,
                revision,
                migrations: loaded.report.migrations,
                announce: output.announce,
            },
            SettingsAdmission::Unchanged { revision } => SettingsReloadAdmission::Unchanged {
                revision,
                migrations: loaded.report.migrations,
                announce: output.announce,
            },
            SettingsAdmission::Stale { .. } => {
                self.report.stale_results = self.report.stale_results.saturating_add(1);
                SettingsReloadAdmission::Stale
            }
            SettingsAdmission::Rejected(failure) => {
                self.report.failures = self.report.failures.saturating_add(1);
                SettingsReloadAdmission::Rejected(failure)
            }
        }
    }

    #[allow(
        dead_code,
        reason = "the opt-in local diagnostic overlay will consume this bounded snapshot"
    )]
    pub(crate) const fn report(&self) -> SettingsReloadReport {
        self.report
    }

    fn observe_decode(&mut self, decoded: DecodeReport) {
        self.report.current_file_bytes = decoded.file_bytes;
        self.report.peak_file_bytes = self.report.peak_file_bytes.max(decoded.file_bytes);
        self.report.current_parsed_values = decoded.parsed_values;
        self.report.peak_parsed_values = self.report.peak_parsed_values.max(decoded.parsed_values);
        self.report.current_string_bytes = decoded.string_bytes;
        self.report.peak_string_bytes = self.report.peak_string_bytes.max(decoded.string_bytes);
        self.report.migrations = self.report.migrations.saturating_add(decoded.migrations);
    }
}

pub(crate) const fn completion_is_current(
    requested_generation: u64,
    submitted_generation: u64,
    completed_generation: u64,
    in_flight: bool,
) -> bool {
    in_flight
        && completed_generation == submitted_generation
        && completed_generation == requested_generation
}

fn load_settings(
    generation: u64,
    paths: &SettingsPaths,
) -> Result<LoadedSettings, SettingsLoadFailure> {
    let global_path = paths.global.clone()?;
    let project_path = paths.project.clone()?;
    let compiled = StudioSettings::compiled().map_err(|_| SettingsLoadFailure {
        source: SettingsSource::Compiled,
        error: SettingsLoadError::InvalidValue("compiled settings"),
    })?;
    let (global, global_report) = load_layer(
        global_path.as_deref(),
        SettingsSource::Global,
        &compiled.theme,
    )?;
    let project_base = global
        .as_ref()
        .and_then(|layer| layer.theme)
        .unwrap_or(compiled.theme);
    let (project, project_report) = load_layer(
        project_path.as_deref(),
        SettingsSource::Project,
        &project_base,
    )?;
    Ok(LoadedSettings {
        update: SettingsUpdate {
            generation,
            global,
            project,
        },
        report: global_report.merge(project_report)?,
    })
}

fn load_layer(
    path: Option<&Path>,
    source: SettingsSource,
    base_theme: &StudioTheme,
) -> Result<(Option<SettingsLayer>, DecodeReport), SettingsLoadFailure> {
    let Some(path) = path else {
        return Ok((None, DecodeReport::default()));
    };
    let Some(bytes) = read_file(path).map_err(|error| SettingsLoadFailure { source, error })?
    else {
        return Ok((None, DecodeReport::default()));
    };
    let file_bytes = bytes.len();
    let (layer, mut report) =
        decode_layer(&bytes, base_theme).map_err(|error| SettingsLoadFailure { source, error })?;
    report.file_bytes = file_bytes;
    Ok((Some(layer), report))
}

fn read_file(path: &Path) -> Result<Option<Vec<u8>>, SettingsLoadError> {
    read_file_with(path, || {})
}

fn read_file_with(
    path: &Path,
    after_read: impl FnOnce(),
) -> Result<Option<Vec<u8>>, SettingsLoadError> {
    if path.as_os_str().as_encoded_bytes().len() > MAX_SETTINGS_PATH_BYTES {
        return Err(SettingsLoadError::PathTooLong);
    }
    let before = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(map_io("metadata", &error)),
    };
    if before.file_type().is_symlink() || !before.is_file() {
        return Err(SettingsLoadError::NotRegularFile);
    }
    if before.len() > u64::try_from(MAX_SETTINGS_FILE_BYTES).unwrap_or(u64::MAX) {
        return Err(SettingsLoadError::FileTooLarge);
    }
    let mut file = File::open(path).map_err(|error| map_io("open", &error))?;
    let opened = file
        .metadata()
        .map_err(|error| map_io("metadata", &error))?;
    if !same_file(&before, &opened) {
        return Err(SettingsLoadError::ConcurrentEdit);
    }
    let mut bytes = Vec::new();
    let reserve = usize::try_from(before.len()).unwrap_or(MAX_SETTINGS_FILE_BYTES);
    bytes
        .try_reserve_exact(reserve.min(MAX_SETTINGS_FILE_BYTES))
        .map_err(|_| SettingsLoadError::AllocationFailed)?;
    {
        let mut limited = (&mut file).take(MAX_SETTINGS_READ_BYTES as u64);
        limited
            .read_to_end(&mut bytes)
            .map_err(|error| map_io("read", &error))?;
    }
    if bytes.len() > MAX_SETTINGS_FILE_BYTES {
        return Err(SettingsLoadError::FileTooLarge);
    }
    after_read();
    let opened_after = file
        .metadata()
        .map_err(|error| map_io("metadata", &error))?;
    let path_after = fs::symlink_metadata(path).map_err(|error| map_io("metadata", &error))?;
    if !same_file(&opened, &opened_after) || !same_file(&opened, &path_after) {
        return Err(SettingsLoadError::ConcurrentEdit);
    }
    Ok(Some(bytes))
}

fn map_io(operation: &'static str, error: &io::Error) -> SettingsLoadError {
    SettingsLoadError::Io {
        operation,
        kind: error.kind(),
    }
}

fn same_file(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    if left.len() != right.len() || left.modified().ok() != right.modified().ok() {
        return false;
    }
    same_platform_file(left, right)
}

#[cfg(unix)]
fn same_platform_file(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt as _;

    left.dev() == right.dev() && left.ino() == right.ino()
}

#[cfg(not(unix))]
const fn same_platform_file(_left: &fs::Metadata, _right: &fs::Metadata) -> bool {
    true
}

fn decode_layer(
    bytes: &[u8],
    base_theme: &StudioTheme,
) -> Result<(SettingsLayer, DecodeReport), SettingsLoadError> {
    let value: Value = serde_json::from_slice(bytes).map_err(|_| SettingsLoadError::InvalidJson)?;
    let mut report = DecodeReport::default();
    inspect_json(&value, 1, &mut report)?;
    let object = value
        .as_object()
        .ok_or(SettingsLoadError::InvalidValue("root"))?;
    let version = object
        .get("version")
        .ok_or(SettingsLoadError::MissingVersion)?
        .as_u64()
        .ok_or(SettingsLoadError::InvalidValue("version"))?;
    let layer = match version {
        LEGACY_SETTINGS_VERSION => {
            report.migrations = 1;
            decode_legacy(object)?
        }
        SETTINGS_VERSION => decode_current(object, base_theme)?,
        version => return Err(SettingsLoadError::UnknownVersion(version)),
    };
    Ok((layer, report))
}

fn inspect_json(
    value: &Value,
    depth: usize,
    report: &mut DecodeReport,
) -> Result<(), SettingsLoadError> {
    if depth > MAX_SETTINGS_JSON_DEPTH {
        return Err(SettingsLoadError::JsonTooDeep);
    }
    report.parsed_values = report
        .parsed_values
        .checked_add(1)
        .ok_or(SettingsLoadError::TooManyValues)?;
    if report.parsed_values > MAX_SETTINGS_JSON_VALUES {
        return Err(SettingsLoadError::TooManyValues);
    }
    match value {
        Value::String(value) => add_string_bytes(value.len(), report)?,
        Value::Array(values) => {
            for value in values {
                inspect_json(value, depth + 1, report)?;
            }
        }
        Value::Object(values) => {
            for (key, value) in values {
                add_string_bytes(key.len(), report)?;
                inspect_json(value, depth + 1, report)?;
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
    Ok(())
}

fn add_string_bytes(bytes: usize, report: &mut DecodeReport) -> Result<(), SettingsLoadError> {
    report.string_bytes = report
        .string_bytes
        .checked_add(bytes)
        .ok_or(SettingsLoadError::TooManyStringBytes)?;
    if report.string_bytes > MAX_SETTINGS_STRING_BYTES {
        Err(SettingsLoadError::TooManyStringBytes)
    } else {
        Ok(())
    }
}

fn decode_legacy(object: &Map<String, Value>) -> Result<SettingsLayer, SettingsLoadError> {
    reject_unknown(
        object,
        &[
            "version",
            "font_size",
            "font_scale",
            "line_height",
            "tab_columns",
        ],
    )?;
    Ok(SettingsLayer {
        editor: EditorSettingsPatch {
            font_size: optional_f32(object, "font_size")?,
            font_scale: optional_f32(object, "font_scale")?,
            line_height: optional_f32(object, "line_height")?,
            tab_columns: optional_u32(object, "tab_columns")?,
            ..EditorSettingsPatch::default()
        },
        ..SettingsLayer::default()
    })
}

fn decode_current(
    object: &Map<String, Value>,
    base_theme: &StudioTheme,
) -> Result<SettingsLayer, SettingsLoadError> {
    reject_unknown(object, &["version", "editor", "theme", "keymap"])?;
    Ok(SettingsLayer {
        editor: object
            .get("editor")
            .map(decode_editor)
            .transpose()?
            .unwrap_or_default(),
        theme: object
            .get("theme")
            .map(|theme| decode_theme(theme, base_theme))
            .transpose()?,
        keymap: object.get("keymap").map(decode_keymap).transpose()?,
    })
}

fn decode_editor(value: &Value) -> Result<EditorSettingsPatch, SettingsLoadError> {
    let object = value
        .as_object()
        .ok_or(SettingsLoadError::InvalidValue("editor"))?;
    reject_unknown(
        object,
        &[
            "font_name",
            "font_size",
            "font_scale",
            "line_height",
            "tab_columns",
        ],
    )?;
    let font_name = object
        .get("font_name")
        .map(|value| {
            let value = value
                .as_str()
                .ok_or(SettingsLoadError::InvalidValue("editor.font_name"))?;
            if value.len() > MAX_FONT_NAME_BYTES {
                return Err(SettingsLoadError::InvalidValue("editor.font_name"));
            }
            if value != FONT_NAME {
                return Err(SettingsLoadError::InvalidValue("editor.font_name"));
            }
            let mut owned = String::new();
            owned
                .try_reserve_exact(value.len())
                .map_err(|_| SettingsLoadError::AllocationFailed)?;
            owned.push_str(value);
            Ok(Cow::Owned(owned))
        })
        .transpose()?;
    Ok(EditorSettingsPatch {
        font_name,
        font_size: optional_f32(object, "font_size")?,
        font_scale: optional_f32(object, "font_scale")?,
        line_height: optional_f32(object, "line_height")?,
        tab_columns: optional_u32(object, "tab_columns")?,
        ..EditorSettingsPatch::default()
    })
}

fn decode_theme(value: &Value, base_theme: &StudioTheme) -> Result<StudioTheme, SettingsLoadError> {
    let object = value
        .as_object()
        .ok_or(SettingsLoadError::InvalidValue("theme"))?;
    validate_theme_fields(object)?;
    let mut theme = *base_theme;
    set_color(object, "clear", "clear", &mut theme.clear)?;
    set_color(object, "background", "background", &mut theme.background)?;
    set_color(
        object,
        "editor_background",
        "editor background",
        &mut theme.editor_background,
    )?;
    set_color(object, "selection", "selection", &mut theme.selection)?;
    set_color(object, "text", "text", &mut theme.text)?;
    set_color(object, "caret", "caret", &mut theme.caret)?;
    set_color(
        object,
        "status_background",
        "status background",
        &mut theme.status_background,
    )?;
    set_color(
        object,
        "sidebar_background",
        "sidebar background",
        &mut theme.sidebar_background,
    )?;
    set_color(object, "active_row", "active row", &mut theme.active_row)?;
    set_color(
        object,
        "tab_background",
        "tab background",
        &mut theme.tab_background,
    )?;
    set_color(object, "active_tab", "active tab", &mut theme.active_tab)?;
    set_color(object, "find_match", "find match", &mut theme.find_match)?;
    set_color(
        object,
        "find_background",
        "find background",
        &mut theme.find_background,
    )?;
    set_color(
        object,
        "quick_open_background",
        "quick open background",
        &mut theme.quick_open_background,
    )?;
    set_color(
        object,
        "quick_open_selected",
        "quick open selected",
        &mut theme.quick_open_selected,
    )?;
    set_color(
        object,
        "project_search_background",
        "project search background",
        &mut theme.project_search_background,
    )?;
    set_color(
        object,
        "project_search_selected",
        "project search selected",
        &mut theme.project_search_selected,
    )?;
    set_color(
        object,
        "command_palette_background",
        "command palette background",
        &mut theme.command_palette_background,
    )?;
    set_color(
        object,
        "command_palette_selected",
        "command palette selected",
        &mut theme.command_palette_selected,
    )?;
    if let Some(syntax) = object.get("syntax") {
        theme.syntax = decode_syntax(syntax, theme.syntax)?;
    }
    Ok(theme)
}

fn validate_theme_fields(object: &Map<String, Value>) -> Result<(), SettingsLoadError> {
    reject_unknown(
        object,
        &[
            "clear",
            "background",
            "editor_background",
            "selection",
            "text",
            "caret",
            "status_background",
            "sidebar_background",
            "active_row",
            "tab_background",
            "active_tab",
            "find_match",
            "find_background",
            "quick_open_background",
            "quick_open_selected",
            "project_search_background",
            "project_search_selected",
            "command_palette_background",
            "command_palette_selected",
            "syntax",
        ],
    )
}

fn decode_syntax(value: &Value, mut syntax: SyntaxTheme) -> Result<SyntaxTheme, SettingsLoadError> {
    let object = value
        .as_object()
        .ok_or(SettingsLoadError::InvalidValue("theme.syntax"))?;
    reject_unknown(
        object,
        &[
            "comment", "keyword", "string", "number", "type", "property", "heading", "code",
        ],
    )?;
    set_color(object, "comment", "syntax comment", &mut syntax.comment)?;
    set_color(object, "keyword", "syntax keyword", &mut syntax.keyword)?;
    set_color(object, "string", "syntax string", &mut syntax.string)?;
    set_color(object, "number", "syntax number", &mut syntax.number)?;
    set_color(object, "type", "syntax type", &mut syntax.type_name)?;
    set_color(object, "property", "syntax property", &mut syntax.property)?;
    set_color(object, "heading", "syntax heading", &mut syntax.heading)?;
    set_color(object, "code", "syntax code", &mut syntax.code)?;
    Ok(syntax)
}

fn set_color(
    object: &Map<String, Value>,
    field: &str,
    diagnostic: &'static str,
    target: &mut LinearRgba,
) -> Result<(), SettingsLoadError> {
    if let Some(value) = object.get(field) {
        *target = decode_color(value, diagnostic)?;
    }
    Ok(())
}

fn decode_color(value: &Value, diagnostic: &'static str) -> Result<LinearRgba, SettingsLoadError> {
    let channels = value
        .as_array()
        .ok_or(SettingsLoadError::InvalidValue("theme color"))?;
    if channels.len() != 4 {
        return Err(SettingsLoadError::InvalidValue("theme color"));
    }
    let channel = |index: usize| {
        checked_f32(&channels[index], "theme color")
            .ok()
            .filter(|value| value.is_finite() && (0.0..=1.0).contains(value))
            .ok_or(SettingsLoadError::InvalidValue("theme color"))
    };
    color(
        diagnostic,
        channel(0)?,
        channel(1)?,
        channel(2)?,
        channel(3)?,
    )
    .map_err(|_| SettingsLoadError::InvalidValue("theme color"))
}

fn decode_keymap(value: &Value) -> Result<Keymap, SettingsLoadError> {
    let object = value
        .as_object()
        .ok_or(SettingsLoadError::InvalidValue("keymap"))?;
    reject_unknown(object, &["bindings"])?;
    let values = object
        .get("bindings")
        .ok_or(SettingsLoadError::MissingField("keymap.bindings"))?
        .as_array()
        .ok_or(SettingsLoadError::InvalidValue("keymap.bindings"))?;
    if values.len() > MAX_KEY_BINDINGS {
        return Err(SettingsLoadError::InvalidValue("keymap.bindings"));
    }
    let mut bindings = Vec::new();
    bindings
        .try_reserve_exact(values.len())
        .map_err(|_| SettingsLoadError::AllocationFailed)?;
    for value in values {
        bindings.push(decode_binding(value)?);
    }
    Ok(Keymap {
        bindings: Cow::Owned(bindings),
    })
}

fn decode_binding(value: &Value) -> Result<KeyBinding, SettingsLoadError> {
    let object = value
        .as_object()
        .ok_or(SettingsLoadError::InvalidValue("key binding"))?;
    reject_unknown(object, &["physical_key", "modifiers", "action", "label"])?;
    let physical_key = object
        .get("physical_key")
        .and_then(Value::as_u64)
        .and_then(|value| u16::try_from(value).ok())
        .ok_or(SettingsLoadError::InvalidValue("key binding physical_key"))?;
    let modifiers = decode_modifiers(
        object
            .get("modifiers")
            .ok_or(SettingsLoadError::MissingField("key binding modifiers"))?,
    )?;
    let action = object
        .get("action")
        .and_then(Value::as_str)
        .and_then(decode_action)
        .ok_or(SettingsLoadError::InvalidValue("key binding action"))?;
    let label = object
        .get("label")
        .and_then(Value::as_str)
        .ok_or(SettingsLoadError::InvalidValue("key binding label"))?;
    let mut owned = String::new();
    owned
        .try_reserve_exact(label.len())
        .map_err(|_| SettingsLoadError::AllocationFailed)?;
    owned.push_str(label);
    Ok(KeyBinding {
        physical_key,
        required_modifiers: modifiers,
        action,
        label: Cow::Owned(owned),
    })
}

fn decode_modifiers(value: &Value) -> Result<u8, SettingsLoadError> {
    let modifiers = value
        .as_array()
        .ok_or(SettingsLoadError::InvalidValue("key binding modifiers"))?;
    if modifiers.len() > 4 {
        return Err(SettingsLoadError::InvalidValue("key binding modifiers"));
    }
    let mut bits = 0_u8;
    for modifier in modifiers {
        let bit = match modifier.as_str() {
            Some("command") => Modifiers::COMMAND,
            Some("shift") => Modifiers::SHIFT,
            Some("option") => Modifiers::OPTION,
            Some("control") => Modifiers::CONTROL,
            _ => return Err(SettingsLoadError::InvalidValue("key binding modifiers")),
        };
        if bits & bit != 0 {
            return Err(SettingsLoadError::InvalidValue("key binding modifiers"));
        }
        bits |= bit;
    }
    Ok(bits)
}

fn decode_action(value: &str) -> Option<KeyAction> {
    match value {
        "command_palette" => Some(KeyAction::CommandPalette),
        "select_all" => Some(KeyAction::SelectAll),
        "undo" => Some(KeyAction::Undo),
        "redo" => Some(KeyAction::Redo),
        value => decode_command(value).map(KeyAction::Command),
    }
}

fn decode_command(value: &str) -> Option<StudioCommand> {
    match value {
        "save_file" => Some(StudioCommand::SaveFile),
        "close_tab" => Some(StudioCommand::CloseTab),
        "navigate_back" => Some(StudioCommand::NavigateBack),
        "navigate_forward" => Some(StudioCommand::NavigateForward),
        "open_quick_open" => Some(StudioCommand::OpenQuickOpen),
        "open_project_search" => Some(StudioCommand::OpenProjectSearch),
        "open_find" => Some(StudioCommand::OpenFind),
        "open_replace" => Some(StudioCommand::OpenReplace),
        "trigger_completion" => Some(StudioCommand::TriggerCompletion),
        "show_rust_hover" => Some(StudioCommand::ShowRustHover),
        "go_to_rust_definition" => Some(StudioCommand::GoToRustDefinition),
        "find_rust_references" => Some(StudioCommand::FindRustReferences),
        "show_rust_document_symbols" => Some(StudioCommand::ShowRustDocumentSymbols),
        "show_rust_workspace_symbols" => Some(StudioCommand::ShowRustWorkspaceSymbols),
        "reload_settings" => Some(StudioCommand::ReloadSettings),
        "toggle_file_tree" => Some(StudioCommand::ToggleFileTree),
        "split_right" => Some(StudioCommand::SplitRight),
        "split_down" => Some(StudioCommand::SplitDown),
        "focus_next_pane" => Some(StudioCommand::FocusNextPane),
        "close_pane" => Some(StudioCommand::ClosePane),
        _ => None,
    }
}

fn optional_f32(
    object: &Map<String, Value>,
    field: &'static str,
) -> Result<Option<f32>, SettingsLoadError> {
    object
        .get(field)
        .map(|value| checked_f32(value, field))
        .transpose()
}

fn checked_f32(value: &Value, field: &'static str) -> Result<f32, SettingsLoadError> {
    let value = value
        .as_f64()
        .filter(|value| {
            value.is_finite() && *value >= f64::from(f32::MIN) && *value <= f64::from(f32::MAX)
        })
        .ok_or(SettingsLoadError::InvalidValue(field))?;
    #[allow(
        clippy::cast_possible_truncation,
        reason = "the finite value is range-checked against f32 before narrowing"
    )]
    let value = value as f32;
    Ok(value)
}

fn optional_u32(
    object: &Map<String, Value>,
    field: &'static str,
) -> Result<Option<u32>, SettingsLoadError> {
    object
        .get(field)
        .map(|value| {
            value
                .as_u64()
                .and_then(|value| u32::try_from(value).ok())
                .ok_or(SettingsLoadError::InvalidValue(field))
        })
        .transpose()
}

fn reject_unknown(object: &Map<String, Value>, allowed: &[&str]) -> Result<(), SettingsLoadError> {
    if object.keys().all(|key| allowed.contains(&key.as_str())) {
        Ok(())
    } else {
        Err(SettingsLoadError::UnknownField)
    }
}

#[cfg(kani)]
mod proofs {
    use super::*;

    #[cfg_attr(test, mutants::skip)]
    #[kani::proof]
    fn only_the_current_submitted_generation_can_publish() {
        let requested: u64 = kani::any();
        let submitted: u64 = kani::any();
        let completed: u64 = kani::any();
        let in_flight: bool = kani::any();
        let admitted = completion_is_current(requested, submitted, completed, in_flight);
        assert!(!admitted || (in_flight && completed == requested && completed == submitted));
    }
}

#[cfg(test)]
mod tests {
    use std::{
        error::Error,
        sync::atomic::{AtomicU64, Ordering},
    };

    use super::*;

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

    struct TestRoot(PathBuf);

    impl TestRoot {
        fn new() -> io::Result<Self> {
            let id = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("alpine-settings-{}-{id}", std::process::id()));
            fs::create_dir_all(&path)?;
            Ok(Self(path))
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn write(path: &Path, contents: &[u8]) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, contents)
    }

    #[test]
    fn constants_paths_and_missing_files_are_exact() -> Result<(), Box<dyn Error>> {
        assert_eq!(SETTINGS_VERSION, 1);
        assert_eq!(LEGACY_SETTINGS_VERSION, 0);
        assert_eq!(MAX_SETTINGS_FILE_BYTES, 65_536);
        assert_eq!(MAX_SETTINGS_READ_BYTES, 65_537);
        assert_eq!(MAX_SETTINGS_PATH_BYTES, 4_096);
        assert_eq!(MAX_SETTINGS_JSON_DEPTH, 8);
        assert_eq!(MAX_SETTINGS_JSON_VALUES, 512);
        assert_eq!(MAX_SETTINGS_STRING_BYTES, 32_768);
        let home = PathBuf::from("/tmp/alpine-home");
        let paths = SettingsPaths::new(Some(home.clone().into_os_string()), Some(&home));
        assert_eq!(
            paths.global?,
            Some(
                home.join("Library")
                    .join("Application Support")
                    .join("Alpine Studio")
                    .join("settings.json")
            )
        );
        assert_eq!(paths.project?, Some(home.join(".alpine/settings.json")));
        let loaded = load_settings(1, &SettingsPaths::explicit(None, None))?;
        assert_eq!(loaded.update.global, None);
        assert_eq!(loaded.update.project, None);
        assert_eq!(loaded.report, DecodeReport::default());
        Ok(())
    }

    #[test]
    fn global_then_project_and_migration_are_atomic() -> Result<(), Box<dyn Error>> {
        let root = TestRoot::new()?;
        let global = root.path().join("global.json");
        let project = root.path().join("project.json");
        write(&global, br#"{"version":0,"font_size":16,"tab_columns":2}"#)?;
        write(
            &project,
            br#"{"version":1,"editor":{"font_size":18},"theme":{"caret":[0.1,0.2,0.3,1.0]},"keymap":{"bindings":[{"physical_key":1,"modifiers":["command"],"action":"reload_settings","label":"Cmd+S"}]}}"#,
        )?;
        let mut reload = SettingsReload::explicit(Some(global), Some(project));
        let request = reload.take_request().ok_or("missing startup request")?;
        let mut state = SettingsState::compiled()?;
        let admission = reload.admit(request.execute(), &mut state);
        assert!(matches!(
            admission,
            SettingsReloadAdmission::Applied {
                effect: SettingsEffect {
                    typography: true,
                    theme: true,
                    keymap: true
                },
                migrations: 1,
                announce: false,
                ..
            }
        ));
        assert!((state.active().editor.font_size - 18.0).abs() < f32::EPSILON);
        assert_eq!(state.active().editor.tab_columns, 2);
        assert_eq!(
            state
                .active()
                .keymap
                .resolve(1, Modifiers::from_bits(Modifiers::COMMAND)),
            Some(KeyAction::Command(StudioCommand::ReloadSettings))
        );
        assert_eq!(reload.report().migrations, 1);
        assert!(reload.report().current_parsed_values > 0);
        Ok(())
    }

    #[test]
    fn stale_completion_and_rejected_candidate_preserve_state() -> Result<(), Box<dyn Error>> {
        let root = TestRoot::new()?;
        let global = root.path().join("settings.json");
        write(&global, br#"{"version":1,"editor":{"font_size":16}}"#)?;
        let mut reload = SettingsReload::explicit(Some(global.clone()), None);
        let stale_request = reload.take_request().ok_or("missing startup request")?;
        reload.request(true)?;
        let mut settings_state = SettingsState::compiled()?;
        let compiled = settings_state.active().clone();
        assert_eq!(
            reload.admit(stale_request.execute(), &mut settings_state),
            SettingsReloadAdmission::Stale
        );
        assert_eq!(settings_state.active(), &compiled);
        let current = reload.take_request().ok_or("missing current request")?;
        write(&global, br#"{"version":1,"editor":{"font_size":0}}"#)?;
        assert!(matches!(
            reload.admit(current.execute(), &mut settings_state),
            SettingsReloadAdmission::Rejected(SettingsFailure {
                source: SettingsSource::Global,
                ..
            })
        ));
        assert_eq!(settings_state.active(), &compiled);
        Ok(())
    }

    #[test]
    fn file_and_json_boundaries_fail_closed() -> Result<(), Box<dyn Error>> {
        let root = TestRoot::new()?;
        let path = root.path().join("settings.json");
        let mut exact = br#"{"version":1}"#.to_vec();
        exact.resize(MAX_SETTINGS_FILE_BYTES, b' ');
        write(&path, &exact)?;
        assert!(read_file(&path)?.is_some());
        let (_, report) = decode_layer(&exact, &StudioTheme::compiled()?)?;
        assert!(report.parsed_values <= MAX_SETTINGS_JSON_VALUES);
        exact.push(b' ');
        write(&path, &exact)?;
        assert_eq!(read_file(&path), Err(SettingsLoadError::FileTooLarge));
        assert_eq!(
            decode_layer(br#"{"version":2}"#, &StudioTheme::compiled()?),
            Err(SettingsLoadError::UnknownVersion(2))
        );
        assert_eq!(
            decode_layer(
                br#"{"version":1,"unknown":true}"#,
                &StudioTheme::compiled()?
            ),
            Err(SettingsLoadError::UnknownField)
        );
        assert_eq!(
            decode_layer(
                br#"{"version":1,"editor":{"font_name":"Unregistered-Mono"}}"#,
                &StudioTheme::compiled()?
            ),
            Err(SettingsLoadError::InvalidValue("editor.font_name"))
        );
        let deep = br#"{"version":1,"editor":{"x":{"x":{"x":{"x":{"x":{"x":{"x":0}}}}}}}}"#;
        assert!(matches!(
            decode_layer(deep, &StudioTheme::compiled()?),
            Err(SettingsLoadError::JsonTooDeep | SettingsLoadError::UnknownField)
        ));
        Ok(())
    }

    #[test]
    fn concurrent_edit_and_symlink_are_rejected() -> Result<(), Box<dyn Error>> {
        let root = TestRoot::new()?;
        let path = root.path().join("settings.json");
        write(&path, br#"{"version":1}"#)?;
        assert_eq!(
            read_file_with(&path, || {
                let _ = fs::write(&path, br#"{"version":1,"editor":{}}"#);
            }),
            Err(SettingsLoadError::ConcurrentEdit)
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let link = root.path().join("linked.json");
            symlink(&path, &link)?;
            assert_eq!(read_file(&link), Err(SettingsLoadError::NotRegularFile));
        }
        Ok(())
    }

    #[test]
    fn duplicate_bindings_and_submission_failure_remain_bounded() -> Result<(), Box<dyn Error>> {
        let root = TestRoot::new()?;
        let path = root.path().join("settings.json");
        write(
            &path,
            br#"{"version":1,"keymap":{"bindings":[{"physical_key":1,"modifiers":["command"],"action":"save_file","label":"Cmd+S"},{"physical_key":1,"modifiers":["command"],"action":"reload_settings","label":"Cmd+R"}]}}"#,
        )?;
        let mut reload = SettingsReload::explicit(Some(path), None);
        let request = reload.take_request().ok_or("missing request")?;
        let mut state = SettingsState::compiled()?;
        assert!(matches!(
            reload.admit(request.execute(), &mut state),
            SettingsReloadAdmission::Rejected(SettingsFailure {
                source: SettingsSource::Global,
                ..
            })
        ));
        reload.request(true)?;
        let request = reload.take_request().ok_or("missing retry")?;
        assert_eq!(
            reload.reject_submission(request.generation(), request.announce()),
            Err(SettingsReloadError::SubmissionFailed)
        );
        assert!(reload.take_request().is_some());
        assert!(reload.report().path_bytes <= MAX_SETTINGS_PATH_BYTES * 2);
        assert_eq!(reload.report().failures, 2);
        Ok(())
    }
}
