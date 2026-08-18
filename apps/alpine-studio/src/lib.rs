#![cfg_attr(
    not(any(test, all(target_os = "macos", target_arch = "aarch64"))),
    expect(dead_code)
)]

//! Local-only Alpine Studio editor boundary.

mod commands;
mod documents;
mod file_tree;
mod find;
mod panes;
mod project_search;
mod quick_open;
mod recovery;
mod session;

use std::path::PathBuf;
mod workspace;

pub use workspace::WorkspaceError;

use std::{
    error::Error,
    fmt,
    num::{NonZeroU32, NonZeroUsize},
    ops::Range,
    path::Path,
    sync::Arc,
};

use alpine_core::{LinearRgba, Point, Rect, Size};
use alpine_platform_macos::{
    ClipboardError, ClipboardEvent, ClipboardOperation, ClipboardText, ClipboardWrite, ImeEvent,
    KeyState, Modifiers, PointerAction, PointerButton, SurfaceError, SurfaceEvent,
};
use alpine_runtime::{AppContext, AppDelegate, DocumentRevision, RuntimeError, WindowContext};
use alpine_scene::{
    AtlasBounds, Clip, Glyph, GlyphAtlasImage, Primitive, Quad, Scene, SceneBuilder, SceneError,
    SceneRevision,
};
use alpine_text::{
    Buffer, BufferSnapshot, ByteOffset, Editor, ExternalChange, FileError, SaveReport, Selection,
    SelectionSet, TextError, Transaction,
};
use alpine_text_layout::{
    DEFAULT_ATLAS_BUDGET_BYTES, DEFAULT_LAYOUT_BUDGET_BYTES, DEFAULT_OVERSCAN_LINES, FontKey,
    GlyphAtlas, GlyphKey, GlyphRasterizer, LayoutError, LineLayout, LineLayoutCache,
    PositiveFinite, TextShaper, VisibleLines,
};
use commands::{CommandContext, CommandPalette, CommandPaletteError, StudioCommand};
use documents::RestoredDocumentTab;
use documents::{DocumentTabError, DocumentTabLimits, DocumentTabs, DocumentViewState};
use file_tree::{
    FileTreeAction, FileTreeAdmission, FileTreeError, FileTreeRequest, FileTreeState,
    FileTreeWorkerOutput,
};
use find::{
    FindAdmission, FindError, FindNavigation, FindRequest, FindState, FindWorkerOutput,
    MAX_REPLACEMENT_TRANSACTION_BYTES,
};
use panes::{MAX_PANES, PaneError, PaneGrid, SplitAxis};
use project_search::{
    ProjectSearchAdmission, ProjectSearchError, ProjectSearchRequest, ProjectSearchState,
    ProjectSearchWorkerOutput, SelectedProjectMatch,
};
use quick_open::{
    QuickOpenAdmission, QuickOpenError, QuickOpenRequest, QuickOpenState, QuickOpenWorkerOutput,
};
use workspace::Workspace;

#[cfg(test)]
use workspace::WorkspaceLimits;

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
use alpine_platform_macos::SurfaceDescriptor;
#[cfg(any(test, all(target_os = "macos", target_arch = "aarch64")))]
use alpine_runtime::{Application, WorkerConfig};

const WINDOW_WIDTH: f32 = 960.0;
const WINDOW_HEIGHT: f32 = 540.0;
const CONTENT_INSET: f32 = 24.0;
const LINE_HEIGHT: f32 = 22.0;
const FONT_SIZE: f32 = 15.0;
const DEFAULT_SCALE: f32 = 2.0;
const FONT_FAMILY: u64 = 1;
const CARET_WIDTH: f32 = 1.5;
const SELECTION_ALPHA: f32 = 0.42;
const SIDEBAR_WIDTH: f32 = 236.0;
const TREE_ROW_HEIGHT: f32 = 22.0;
const TREE_OVERSCAN_ROWS: usize = 3;
const TAB_BAR_HEIGHT: f32 = 24.0;
const TAB_WIDTH: f32 = 160.0;
const TAB_OVERSCAN: usize = 2;
const FIND_BAR_WIDTH: f32 = 420.0;
const FIND_BAR_HEIGHT: f32 = 30.0;
const FIND_BAR_INSET: f32 = 8.0;
const QUICK_OPEN_WIDTH: f32 = 620.0;
const QUICK_OPEN_QUERY_HEIGHT: f32 = 34.0;
const QUICK_OPEN_ROW_HEIGHT: f32 = 24.0;
const QUICK_OPEN_VISIBLE_ROWS: usize = 12;
const QUICK_OPEN_OVERSCAN_ROWS: usize = 3;
const PROJECT_SEARCH_WIDTH: f32 = 760.0;
const PROJECT_SEARCH_QUERY_HEIGHT: f32 = 34.0;
const PROJECT_SEARCH_ROW_HEIGHT: f32 = 24.0;
const PROJECT_SEARCH_VISIBLE_ROWS: usize = 12;
const PROJECT_SEARCH_OVERSCAN_ROWS: usize = 3;
const COMMAND_PALETTE_WIDTH: f32 = 620.0;
const COMMAND_PALETTE_QUERY_HEIGHT: f32 = 34.0;
const COMMAND_PALETTE_ROW_HEIGHT: f32 = 24.0;
const INITIAL_TEXT: &str = "fn main() {\n    println!(\"Alpine Studio\");\n}\n\n// Local, direct, and deliberately small.\n";

const KEY_A: u16 = 0;
const KEY_S: u16 = 1;
const KEY_E: u16 = 14;
const KEY_F: u16 = 3;
const KEY_P: u16 = 35;
const KEY_Z: u16 = 6;
const KEY_W: u16 = 13;
const KEY_RIGHT_BRACKET: u16 = 30;
const KEY_LEFT_BRACKET: u16 = 33;
const KEY_RETURN: u16 = 36;
const KEY_TAB: u16 = 48;
const KEY_DELETE_BACKWARD: u16 = 51;
const KEY_ESCAPE: u16 = 53;
const KEY_HOME: u16 = 115;
const KEY_DELETE_FORWARD: u16 = 117;
const KEY_END: u16 = 119;
const KEY_LEFT: u16 = 123;
const KEY_RIGHT: u16 = 124;
const KEY_DOWN: u16 = 125;
const KEY_UP: u16 = 126;

/// A structured Alpine Studio launch failure.
#[derive(Debug)]
pub enum StudioError {
    /// More than one positional file path was supplied.
    Usage,
    /// Opening or saving the selected local file failed.
    File(FileError),
    /// Opening or enumerating the selected local folder failed.
    Workspace(WorkspaceError),
    /// Native application construction or execution failed.
    Runtime(RuntimeError),
}

impl fmt::Display for StudioError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage => formatter.write_str("usage: alpine-studio [path]"),
            Self::File(error) => write!(formatter, "Studio file failed: {error}"),
            Self::Workspace(error) => write!(formatter, "Studio workspace failed: {error}"),
            Self::Runtime(error) => write!(formatter, "Studio runtime failed: {error}"),
        }
    }
}

impl Error for StudioError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Usage => None,
            Self::File(error) => Some(error),
            Self::Workspace(error) => Some(error),
            Self::Runtime(error) => Some(error),
        }
    }
}

impl From<FileError> for StudioError {
    fn from(error: FileError) -> Self {
        Self::File(error)
    }
}

impl From<WorkspaceError> for StudioError {
    fn from(error: WorkspaceError) -> Self {
        Self::Workspace(error)
    }
}

impl From<RuntimeError> for StudioError {
    fn from(error: RuntimeError) -> Self {
        Self::Runtime(error)
    }
}

impl From<SurfaceError> for StudioError {
    fn from(error: SurfaceError) -> Self {
        Self::Runtime(RuntimeError::Surface(error))
    }
}

/// Builds the first immutable native Studio editor scene.
///
/// # Errors
///
/// Returns a structured unsupported or native construction failure.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
pub fn initial_scene() -> Result<Scene, SurfaceError> {
    let viewport = Size::new(WINDOW_WIDTH, WINDOW_HEIGHT).ok_or(SurfaceError::DriverUnavailable)?;
    let mut app = native_app()?;
    Ok(app.scene(SceneRevision::new(1), viewport))
}

/// Rejects native scene construction on unsupported hosts.
///
/// # Errors
///
/// Always returns [`SurfaceError::UnsupportedPlatform`].
#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
pub fn initial_scene() -> Result<Scene, SurfaceError> {
    Err(SurfaceError::UnsupportedPlatform)
}

/// Opens one native Studio window, requests one frame, and runs until close.
///
/// # Errors
///
/// Returns the structured surface error from scene construction, native
/// initialization, frame admission, or the application run loop.
#[cfg_attr(test, mutants::skip)] // Entering AppKit is qualified by native process E2E.
pub fn run() -> Result<(), RuntimeError> {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        run_native(native_restored_app()?)
    }

    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    {
        Err(RuntimeError::Surface(SurfaceError::UnsupportedPlatform))
    }
}

/// Opens one existing UTF-8 file before starting the native Studio window.
///
/// # Errors
///
/// Returns a structured file error before native construction, or the
/// structured runtime error from native construction and execution.
#[cfg_attr(test, mutants::skip)] // Entering AppKit is qualified by native process E2E.
pub fn run_file(path: impl AsRef<Path>) -> Result<(), StudioError> {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        run_native(with_default_session(native_file_app(path.as_ref())?)?)
            .map_err(StudioError::from)
    }

    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    {
        let _ = path;
        Err(SurfaceError::UnsupportedPlatform.into())
    }
}

#[cfg(all(test, not(all(target_os = "macos", target_arch = "aarch64"))))]
mod entry_point_contract_tests {
    use super::{run, run_file};
    use std::path::Path;

    #[test]
    fn unsupported_platform_entry_points_return_structured_errors() {
        assert!(run().is_err());
        assert!(run_file(Path::new("alpine-entry-point-probe")).is_err());
    }
}

/// Opens one existing regular file or one bounded local folder.
///
/// # Errors
///
/// Returns a structured path, file, workspace, or runtime failure.
pub fn run_path(path: impl AsRef<Path>) -> Result<(), StudioError> {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        let path = path.as_ref();
        let metadata = std::fs::metadata(path)
            .map_err(|source| WorkspaceError::io("read launch metadata", path, source))?;
        if metadata.is_file() {
            run_native(with_default_session(native_file_app(path)?)?).map_err(StudioError::from)
        } else if metadata.is_dir() {
            run_native(with_default_session(native_workspace_app(path)?)?)
                .map_err(StudioError::from)
        } else {
            Err(WorkspaceError::UnsupportedTarget(path.to_path_buf()).into())
        }
    }

    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    {
        let _ = path;
        Err(SurfaceError::UnsupportedPlatform.into())
    }
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn run_native(app: StudioApp) -> Result<(), RuntimeError> {
    let clear = LinearRgba::new(0.02, 0.02, 0.02, 1.0).ok_or(SurfaceError::DriverUnavailable)?;
    let descriptor = SurfaceDescriptor::new(
        "Alpine Studio",
        f64::from(WINDOW_WIDTH),
        f64::from(WINDOW_HEIGHT),
        2.0,
    )?;
    let viewport = Size::new(WINDOW_WIDTH, WINDOW_HEIGHT).ok_or(SurfaceError::DriverUnavailable)?;
    Application::new(app, viewport, clear, WorkerConfig::default())?.run(&descriptor)
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn native_app() -> Result<StudioApp, SurfaceError> {
    let mut text_system = alpine_text_layout::CoreTextSystem::new();
    text_system
        .register_font(FONT_FAMILY, "Menlo-Regular")
        .map_err(|_| SurfaceError::DriverUnavailable)?;
    StudioApp::new(text_system)
}

#[cfg(test)]
fn with_session_path(
    mut app: StudioApp,
    session_path: Result<PathBuf, session::SessionError>,
) -> StudioApp {
    app.session_path = session_path.ok();
    app
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[cfg_attr(test, mutants::skip)] // Linux cannot type-check this Apple-only composition boundary.
fn native_restored_app() -> Result<StudioApp, SurfaceError> {
    let Ok(path) = session::default_path() else {
        return native_app();
    };
    let recovery_warning = match recovery::load(&recovery::path_for_session(&path)) {
        Ok(state) => {
            let availability = RestoreAvailability::for_recovery(state.documents.len());
            let mut text_system = alpine_text_layout::CoreTextSystem::new();
            text_system
                .register_font(FONT_FAMILY, "Menlo-Regular")
                .map_err(|_| SurfaceError::DriverUnavailable)?;
            match StudioApp::from_recovery(text_system, state) {
                Ok(mut app) => {
                    app.configure_persistence(path)?;
                    return Ok(app);
                }
                Err(error) => match availability {
                    RestoreAvailability::AllowPlaceholder => {
                        return Err(SurfaceError::DriverUnavailable);
                    }
                    RestoreAvailability::Strict => {
                        Some(format!("Recovery restore skipped: {error}"))
                    }
                },
            }
        }
        Err(recovery::RecoveryError::Io {
            kind: std::io::ErrorKind::NotFound,
            ..
        }) => None,
        Err(error) => Some(format!("Recovery restore skipped: {error}")),
    };
    let mut app = match session::load(&path) {
        Ok(state) => {
            let mut text_system = alpine_text_layout::CoreTextSystem::new();
            text_system
                .register_font(FONT_FAMILY, "Menlo-Regular")
                .map_err(|_| SurfaceError::DriverUnavailable)?;
            match StudioApp::from_session(text_system, state) {
                Ok(app) => app,
                Err(error) => session_fallback(&error.to_string())?,
            }
        }
        Err(session::SessionError::Io {
            kind: std::io::ErrorKind::NotFound,
            ..
        }) => native_app()?,
        Err(error) => session_fallback(&error.to_string())?,
    };
    if let Some(warning) = recovery_warning {
        app.local_status = Some(LocalStatus::Workspace(Arc::from(warning)));
    }
    app.configure_persistence(path)?;
    Ok(app)
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[cfg_attr(test, mutants::skip)] // Cross-platform fallback state is qualified below this adapter.
fn session_fallback(detail: &str) -> Result<StudioApp, SurfaceError> {
    let mut app = native_app()?;
    app.local_status = Some(LocalStatus::Workspace(Arc::from(format!(
        "Session restore skipped: {detail}"
    ))));
    Ok(app)
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[cfg_attr(test, mutants::skip)] // Cross-platform persistence behavior is qualified independently.
fn with_default_session(mut app: StudioApp) -> Result<StudioApp, SurfaceError> {
    if let Ok(path) = session::default_path() {
        recovery::ensure_replaceable(&recovery::path_for_session(&path))
            .map_err(|_| SurfaceError::DriverUnavailable)?;
        app.configure_persistence(path)?;
    }
    Ok(app)
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn native_file_app(path: &Path) -> Result<StudioApp, StudioError> {
    let document = StudioDocument::open(path)?;
    let mut text_system = alpine_text_layout::CoreTextSystem::new();
    text_system
        .register_font(FONT_FAMILY, "Menlo-Regular")
        .map_err(|_| SurfaceError::DriverUnavailable)?;
    StudioApp::from_document(text_system, document, Some(path)).map_err(StudioError::from)
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn native_workspace_app(path: &Path) -> Result<StudioApp, StudioError> {
    let workspace = Workspace::open_root(path)?;
    let mut text_system = alpine_text_layout::CoreTextSystem::new();
    text_system
        .register_font(FONT_FAMILY, "Menlo-Regular")
        .map_err(|_| SurfaceError::DriverUnavailable)?;
    let mut app = StudioApp::from_workspace(text_system, workspace)?;
    app.file_tree
        .activate(1)
        .map_err(|_| SurfaceError::DriverUnavailable)?;
    app.file_tree.unfocus();
    Ok(app)
}

trait StudioTextSystem: TextShaper + GlyphRasterizer {}

impl<T: TextShaper + GlyphRasterizer> StudioTextSystem for T {}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Composition {
    replacement: Range<usize>,
    text: Box<str>,
    selected_start_utf16: u32,
    selected_length_utf16: u32,
}

#[derive(Clone)]
struct RenderedLine {
    line: usize,
    top: f32,
    baseline: f32,
    layout: Arc<LineLayout>,
}

struct PendingGlyph {
    bounds: Rect,
    atlas_bounds: AtlasBounds,
    clip: alpine_scene::ClipId,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct EventEffect {
    visual_changed: bool,
    document_changed: bool,
    document_identity_advanced: bool,
}

impl EventEffect {
    const fn visual() -> Self {
        Self {
            visual_changed: true,
            document_changed: false,
            document_identity_advanced: false,
        }
    }

    const fn document() -> Self {
        Self {
            visual_changed: true,
            document_changed: true,
            document_identity_advanced: false,
        }
    }

    const fn document_replacement() -> Self {
        Self {
            visual_changed: true,
            document_changed: true,
            document_identity_advanced: true,
        }
    }

    const fn merge(self, other: Self) -> Self {
        Self {
            visual_changed: self.visual_changed || other.visual_changed,
            document_changed: self.document_changed || other.document_changed,
            document_identity_advanced: self.document_identity_advanced
                || other.document_identity_advanced,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PendingCut {
    revision: u64,
    selection: Selection,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum LocalStatus {
    Clipboard(Arc<str>),
    CloseBlocked,
    Command(Arc<str>),
    Workspace(Arc<str>),
}

impl LocalStatus {
    fn message(&self) -> &str {
        match self {
            Self::Clipboard(message) | Self::Command(message) | Self::Workspace(message) => message,
            Self::CloseBlocked => "Save changes before closing.",
        }
    }
}

#[derive(Default)]
struct StudioTransition {
    effect: EventEffect,
    clipboard_write: Option<ClipboardWrite>,
    cancel_close: bool,
}

impl StudioTransition {
    const fn effect(effect: EventEffect) -> Self {
        Self {
            effect,
            clipboard_write: None,
            cancel_close: false,
        }
    }
}

#[derive(Debug)]
enum StudioRenderError {
    CommandPalette(CommandPaletteError),
    Domain,
    FileTree(FileTreeError),
    Find(FindError),
    QuickOpen(QuickOpenError),
    ProjectSearch(ProjectSearchError),
    Text(TextError),
    Layout(LayoutError),
    Scene(SceneError),
}

impl From<CommandPaletteError> for StudioRenderError {
    fn from(error: CommandPaletteError) -> Self {
        Self::CommandPalette(error)
    }
}

impl From<TextError> for StudioRenderError {
    fn from(error: TextError) -> Self {
        Self::Text(error)
    }
}

impl From<FileTreeError> for StudioRenderError {
    fn from(error: FileTreeError) -> Self {
        Self::FileTree(error)
    }
}

impl From<FindError> for StudioRenderError {
    fn from(error: FindError) -> Self {
        Self::Find(error)
    }
}

impl From<QuickOpenError> for StudioRenderError {
    fn from(error: QuickOpenError) -> Self {
        Self::QuickOpen(error)
    }
}

impl From<ProjectSearchError> for StudioRenderError {
    fn from(error: ProjectSearchError) -> Self {
        Self::ProjectSearch(error)
    }
}

impl From<LayoutError> for StudioRenderError {
    fn from(error: LayoutError) -> Self {
        Self::Layout(error)
    }
}

impl From<SceneError> for StudioRenderError {
    fn from(error: SceneError) -> Self {
        Self::Scene(error)
    }
}

impl fmt::Display for StudioRenderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CommandPalette(error) => {
                write!(formatter, "command-palette rendering failed: {error}")
            }
            Self::Domain => formatter.write_str("invalid Studio render domain value"),
            Self::FileTree(error) => write!(formatter, "file-tree rendering failed: {error}"),
            Self::Find(error) => write!(formatter, "find rendering failed: {error}"),
            Self::QuickOpen(error) => write!(formatter, "quick-open rendering failed: {error}"),
            Self::ProjectSearch(error) => {
                write!(formatter, "project-search rendering failed: {error}")
            }
            Self::Text(error) => write!(formatter, "text layout input failed: {error}"),
            Self::Layout(error) => write!(formatter, "visible layout failed: {error}"),
            Self::Scene(error) => write!(formatter, "scene construction failed: {error}"),
        }
    }
}

impl Error for StudioRenderError {}

#[derive(Debug)]
enum WorkspaceSelectionError {
    NoWorkspace,
    DirtyDocument,
    RevisionExhausted,
    Tabs(DocumentTabError),
    Workspace(WorkspaceError),
    File(FileError),
    QuickOpen(QuickOpenError),
    ProjectSearch(ProjectSearchError),
}

impl fmt::Display for WorkspaceSelectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoWorkspace => formatter.write_str("no local workspace is open"),
            Self::DirtyDocument => formatter.write_str("save changes before switching files"),
            Self::RevisionExhausted => formatter.write_str("document identity is exhausted"),
            Self::Tabs(error) => write!(formatter, "document tabs failed: {error}"),
            Self::Workspace(error) => write!(formatter, "workspace selection failed: {error}"),
            Self::File(error) => write!(formatter, "workspace file failed: {error}"),
            Self::QuickOpen(error) => write!(formatter, "quick open failed: {error}"),
            Self::ProjectSearch(error) => write!(formatter, "project search failed: {error}"),
        }
    }
}

impl Error for WorkspaceSelectionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Workspace(error) => Some(error),
            Self::File(error) => Some(error),
            Self::Tabs(error) => Some(error),
            Self::QuickOpen(error) => Some(error),
            Self::ProjectSearch(error) => Some(error),
            Self::NoWorkspace | Self::DirtyDocument | Self::RevisionExhausted => None,
        }
    }
}

enum StudioDocument {
    Scratch {
        buffer: Buffer,
        clean_revision: u64,
        recovery_base: BufferSnapshot,
    },
    File {
        editor: Editor,
        recovery_base: BufferSnapshot,
    },
    Recovered {
        buffer: Buffer,
        recovery_base: BufferSnapshot,
        conflict: ExternalChange,
    },
    Unavailable {
        buffer: Buffer,
        clean_revision: u64,
        recovery_base: BufferSnapshot,
        conflict: ExternalChange,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RestoreAvailability {
    Strict,
    AllowPlaceholder,
}

impl RestoreAvailability {
    const fn for_recovery(recovered_count: usize) -> Self {
        if recovered_count > 0 {
            Self::AllowPlaceholder
        } else {
            Self::Strict
        }
    }

    const fn allows_placeholder(self) -> bool {
        matches!(self, Self::AllowPlaceholder)
    }
}

impl StudioDocument {
    fn scratch(text: &str) -> Self {
        let buffer = Buffer::new(text);
        let clean_revision = buffer.revision().get();
        let recovery_base = buffer.snapshot();
        Self::Scratch {
            buffer,
            clean_revision,
            recovery_base,
        }
    }

    fn open(path: impl AsRef<Path>) -> Result<Self, FileError> {
        Editor::open(path).map(|editor| {
            let recovery_base = editor.buffer().snapshot();
            Self::File {
                editor,
                recovery_base,
            }
        })
    }

    fn open_for_restore(path: &Path, availability: RestoreAvailability) -> Result<Self, FileError> {
        match Self::open(path) {
            Ok(document) => Ok(document),
            Err(_error) if availability.allows_placeholder() => {
                let buffer = Buffer::new("");
                let clean_revision = buffer.revision().get();
                let recovery_base = buffer.snapshot();
                Ok(Self::Unavailable {
                    buffer,
                    clean_revision,
                    recovery_base,
                    conflict: if path.exists() {
                        ExternalChange::Modified
                    } else {
                        ExternalChange::Deleted
                    },
                })
            }
            Err(error) => Err(error),
        }
    }

    fn recover(
        path: Option<&Path>,
        recovered: &recovery::RecoveredDocument,
    ) -> Result<Self, TextError> {
        let recovery_base = Buffer::new(&recovered.base).snapshot();
        let recovered_buffer = || Buffer::new(&recovered.local);
        let Some(path) = path else {
            let mut buffer = Buffer::new(&recovered.base);
            let mut transaction = Transaction::new(buffer.revision());
            transaction.replace(0..buffer.snapshot().len_bytes(), recovered.local.as_ref())?;
            buffer.apply(transaction)?;
            return Ok(Self::Scratch {
                buffer,
                clean_revision: 0,
                recovery_base,
            });
        };
        let Ok(mut editor) = Editor::open(path) else {
            return Ok(Self::Recovered {
                buffer: recovered_buffer(),
                recovery_base,
                conflict: if path.exists() {
                    ExternalChange::Modified
                } else {
                    ExternalChange::Deleted
                },
            });
        };
        if editor.buffer().snapshot().text() != recovered.base.as_ref() {
            return Ok(Self::Recovered {
                buffer: recovered_buffer(),
                recovery_base,
                conflict: ExternalChange::Modified,
            });
        }
        let length = editor.buffer().snapshot().len_bytes();
        let mut transaction = Transaction::new(editor.buffer().revision());
        transaction.replace(0..length, recovered.local.as_ref())?;
        editor.buffer_mut().apply(transaction)?;
        Ok(Self::File {
            editor,
            recovery_base,
        })
    }

    const fn buffer(&self) -> &Buffer {
        match self {
            Self::Scratch { buffer, .. }
            | Self::Recovered { buffer, .. }
            | Self::Unavailable { buffer, .. } => buffer,
            Self::File { editor, .. } => editor.buffer(),
        }
    }

    const fn buffer_mut(&mut self) -> &mut Buffer {
        match self {
            Self::Scratch { buffer, .. }
            | Self::Recovered { buffer, .. }
            | Self::Unavailable { buffer, .. } => buffer,
            Self::File { editor, .. } => editor.buffer_mut(),
        }
    }

    fn save(&mut self) -> Result<Option<SaveReport>, FileError> {
        match self {
            Self::Scratch { .. } => Ok(None),
            Self::File {
                editor,
                recovery_base,
            } => {
                let report = editor.save()?;
                *recovery_base = editor.buffer().snapshot();
                Ok(Some(report))
            }
            Self::Recovered { conflict, .. } | Self::Unavailable { conflict, .. } => {
                Err(FileError::Conflict(*conflict))
            }
        }
    }

    fn is_dirty(&self) -> bool {
        match self {
            Self::Scratch {
                buffer,
                clean_revision,
                ..
            }
            | Self::Unavailable {
                buffer,
                clean_revision,
                ..
            } => buffer.revision().get() != *clean_revision,
            Self::File { editor, .. } => editor.is_dirty(),
            Self::Recovered { .. } => true,
        }
    }

    const fn is_file(&self) -> bool {
        matches!(
            self,
            Self::File { .. } | Self::Recovered { .. } | Self::Unavailable { .. }
        )
    }

    fn recovery_base(&self) -> BufferSnapshot {
        match self {
            Self::Scratch { recovery_base, .. }
            | Self::File { recovery_base, .. }
            | Self::Recovered { recovery_base, .. }
            | Self::Unavailable { recovery_base, .. } => recovery_base.clone(),
        }
    }

    const fn has_recovery_conflict(&self) -> bool {
        matches!(self, Self::Recovered { .. })
    }

    const fn is_unavailable(&self) -> bool {
        matches!(self, Self::Unavailable { .. })
    }
}

#[cfg(test)]
macro_rules! force_quick_open_submission_failure {
    ($app:expr) => {
        $app.force_quick_open_submission_failure.is_some()
    };
}

#[cfg(test)]
macro_rules! force_project_search_submission_failure {
    ($app:expr) => {
        $app.force_project_search_submission_failure.is_some()
    };
}

#[cfg(test)]
macro_rules! force_file_tree_submission_failure {
    ($app:expr) => {
        $app.force_file_tree_submission_failure.is_some()
    };
}

#[cfg(not(test))]
macro_rules! force_file_tree_submission_failure {
    ($app:expr) => {
        false
    };
}

#[cfg(not(test))]
macro_rules! force_quick_open_submission_failure {
    ($app:expr) => {
        false
    };
}

#[cfg(not(test))]
macro_rules! force_project_search_submission_failure {
    ($app:expr) => {
        false
    };
}

struct StudioApp {
    document: StudioDocument,
    tabs: DocumentTabs<StudioDocument>,
    workspace: Option<Workspace>,
    active_workspace_entry: Option<usize>,
    workspace_scroll_y: f32,
    tab_scroll_x: f32,
    last_pointer_position: Option<Point>,
    runtime_document_revision: u64,
    selection: Selection,
    composition: Option<Composition>,
    scroll_y: f32,
    focused: bool,
    pointer_selecting: bool,
    last_viewport: Size,
    rendered_lines: Vec<RenderedLine>,
    layout_cache: LineLayoutCache,
    glyph_atlas: GlyphAtlas,
    published_atlas: Option<GlyphAtlasImage>,
    atlas_revision: u64,
    text_system: Box<dyn StudioTextSystem>,
    input_failures: u64,
    render_failures: u64,
    save_failures: u64,
    clipboard_failures: u64,
    last_save: Option<SaveReport>,
    last_file_error: Option<FileError>,
    last_clipboard_error: Option<ClipboardError>,
    workspace_failures: u64,
    last_workspace_error: Option<Arc<str>>,
    pending_cut: Option<PendingCut>,
    local_status: Option<LocalStatus>,
    find: FindState,
    find_needs_search: bool,
    quick_open: QuickOpenState,
    project_search: ProjectSearchState,
    file_tree: FileTreeState,
    command_palette: CommandPalette,
    panes: PaneGrid,
    session_path: Option<PathBuf>,
    recovery: Option<recovery::RecoveryCoordinator>,
    last_recovery_error: Option<recovery::RecoveryError>,
    pending_recovery: Vec<Option<recovery::RecoveredDocument>>,
    restore_availability: RestoreAvailability,
    #[cfg(test)]
    force_quick_open_submission_failure: Option<()>,
    #[cfg(test)]
    force_project_search_submission_failure: Option<()>,
    #[cfg(test)]
    force_project_search_clip_failure: Option<()>,
    #[cfg(test)]
    force_file_tree_submission_failure: Option<()>,
    #[cfg(test)]
    force_command_clip_failure: Option<()>,
    #[cfg(test)]
    force_empty_navigation_result: Option<()>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum SessionRestoreError {
    Invalid,
    Workspace,
    File,
    Surface,
    Tabs,
    Panes,
    FileTree,
    Allocation,
}

impl fmt::Display for SessionRestoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Invalid => "session contract is invalid",
            Self::Workspace => "session workspace is unavailable",
            Self::File => "session document is unavailable",
            Self::Surface => "session application construction failed",
            Self::Tabs => "session tab state is inconsistent",
            Self::Panes => "session pane state is inconsistent",
            Self::FileTree => "session file-tree state is inconsistent",
            Self::Allocation => "session restoration allocation failed",
        };
        formatter.write_str(message)
    }
}

fn classify_session_document_error(error: &WorkspaceSelectionError) -> SessionRestoreError {
    match error {
        WorkspaceSelectionError::File(_) => SessionRestoreError::File,
        WorkspaceSelectionError::Tabs(_) => SessionRestoreError::Tabs,
        WorkspaceSelectionError::NoWorkspace
        | WorkspaceSelectionError::DirtyDocument
        | WorkspaceSelectionError::RevisionExhausted
        | WorkspaceSelectionError::Workspace(_)
        | WorkspaceSelectionError::QuickOpen(_)
        | WorkspaceSelectionError::ProjectSearch(_) => SessionRestoreError::Invalid,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SessionCaptureError {
    DirtyDocument,
    Tabs,
    Panes,
    FileTree,
    Allocation,
    Invalid,
}

impl Drop for StudioApp {
    fn drop(&mut self) {
        if self.recovery.is_some()
            && let Ok(request) = self.capture_recovery_request()
            && let Some(recovery) = self.recovery.as_ref()
        {
            let _ = recovery.publish(request);
        }
        if let Some(mut recovery) = self.recovery.take() {
            let _ = recovery.shutdown();
        }
        let Some(path) = self.session_path.clone() else {
            return;
        };
        let Ok(state) = self.capture_session() else {
            return;
        };
        let _ = session::save(&path, &state);
    }
}

impl StudioApp {
    fn new(text_system: impl StudioTextSystem + 'static) -> Result<Self, SurfaceError> {
        Self::from_document(text_system, StudioDocument::scratch(INITIAL_TEXT), None)
    }

    #[cfg(test)]
    fn open_file(
        text_system: impl StudioTextSystem + 'static,
        path: impl AsRef<Path>,
    ) -> Result<Self, StudioError> {
        let path = path.as_ref();
        let document = StudioDocument::open(path)?;
        Self::from_document(text_system, document, Some(path)).map_err(StudioError::from)
    }

    #[cfg(test)]
    fn open_workspace(
        text_system: impl StudioTextSystem + 'static,
        path: impl AsRef<Path>,
    ) -> Result<Self, StudioError> {
        let workspace = Workspace::open(path.as_ref(), WorkspaceLimits::default())?;
        let root = workspace.root().to_path_buf();
        let mut app = Self::from_workspace(text_system, workspace).map_err(StudioError::from)?;
        app.file_tree
            .activate(1)
            .map_err(|_| SurfaceError::DriverUnavailable)?;
        let request = app.file_tree.take_request(&root);
        let admission = request.map(|request| app.file_tree.admit(request.execute()));
        assert_eq!(
            admission,
            Some(FileTreeAdmission::Directory),
            "test workspace root must be readable"
        );
        app.file_tree.unfocus();
        Ok(app)
    }

    #[cfg(test)]
    fn open_workspace_lazy(
        text_system: impl StudioTextSystem + 'static,
        path: impl AsRef<Path>,
    ) -> Result<Self, StudioError> {
        let workspace = Workspace::open_root(path.as_ref())?;
        Self::from_workspace(text_system, workspace).map_err(StudioError::from)
    }

    fn from_document(
        text_system: impl StudioTextSystem + 'static,
        document: StudioDocument,
        path: Option<&Path>,
    ) -> Result<Self, SurfaceError> {
        Self::from_parts(text_system, document, path, None)
    }

    fn from_workspace(
        text_system: impl StudioTextSystem + 'static,
        workspace: Workspace,
    ) -> Result<Self, SurfaceError> {
        #[cfg(test)]
        let omitted_entries = workspace.snapshot().omitted_entries;
        let document = StudioDocument::scratch(INITIAL_TEXT);
        let app = Self::from_parts(text_system, document, None, Some(workspace))?;
        #[cfg(test)]
        let mut app = app;
        #[cfg(test)]
        if omitted_entries > 0 {
            app.local_status = Some(LocalStatus::Workspace(Arc::from(format!(
                "Workspace tree truncated: {omitted_entries} entries omitted."
            ))));
        }
        Ok(app)
    }

    fn from_parts(
        text_system: impl StudioTextSystem + 'static,
        document: StudioDocument,
        path: Option<&Path>,
        workspace: Option<Workspace>,
    ) -> Result<Self, SurfaceError> {
        let last_viewport =
            Size::new(WINDOW_WIDTH, WINDOW_HEIGHT).ok_or(SurfaceError::DriverUnavailable)?;
        let layout_budget = NonZeroUsize::new(DEFAULT_LAYOUT_BUDGET_BYTES)
            .ok_or(SurfaceError::DriverUnavailable)?;
        let atlas_budget =
            NonZeroUsize::new(DEFAULT_ATLAS_BUDGET_BYTES).ok_or(SurfaceError::DriverUnavailable)?;
        let runtime_document_revision = document.buffer().revision().get();
        let tabs = DocumentTabs::new(path, None, DocumentTabLimits::default())
            .map_err(|_| SurfaceError::DriverUnavailable)?;
        let active_tab = tabs
            .active_id()
            .map_err(|_| SurfaceError::DriverUnavailable)?;
        let panes = PaneGrid::new(active_tab, DocumentViewState::default());
        Ok(Self {
            document,
            tabs,
            workspace,
            active_workspace_entry: None,
            workspace_scroll_y: 0.0,
            tab_scroll_x: 0.0,
            last_pointer_position: None,
            runtime_document_revision,
            selection: Selection::caret(ByteOffset::new(0)),
            composition: None,
            scroll_y: 0.0,
            focused: true,
            pointer_selecting: false,
            last_viewport,
            rendered_lines: Vec::new(),
            layout_cache: LineLayoutCache::new(layout_budget),
            glyph_atlas: GlyphAtlas::new(atlas_budget),
            published_atlas: None,
            atlas_revision: 0,
            text_system: Box::new(text_system),
            input_failures: 0,
            render_failures: 0,
            save_failures: 0,
            clipboard_failures: 0,
            last_save: None,
            last_file_error: None,
            last_clipboard_error: None,
            workspace_failures: 0,
            last_workspace_error: None,
            pending_cut: None,
            local_status: None,
            find: FindState::default(),
            find_needs_search: false,
            quick_open: QuickOpenState::default(),
            project_search: ProjectSearchState::default(),
            file_tree: FileTreeState::default(),
            command_palette: CommandPalette::default(),
            panes,
            session_path: None,
            recovery: None,
            last_recovery_error: None,
            pending_recovery: Vec::new(),
            restore_availability: RestoreAvailability::Strict,
            #[cfg(test)]
            force_quick_open_submission_failure: None,
            #[cfg(test)]
            force_project_search_submission_failure: None,
            #[cfg(test)]
            force_project_search_clip_failure: None,
            #[cfg(test)]
            force_file_tree_submission_failure: None,
            #[cfg(test)]
            force_command_clip_failure: None,
            #[cfg(test)]
            force_empty_navigation_result: None,
        })
    }

    fn from_session(
        text_system: impl StudioTextSystem + 'static,
        state: session::SessionState,
    ) -> Result<Self, SessionRestoreError> {
        Self::from_session_with_recovery(text_system, state, Vec::new())
    }

    fn from_recovery(
        text_system: impl StudioTextSystem + 'static,
        state: recovery::RecoveryState,
    ) -> Result<Self, SessionRestoreError> {
        Self::from_session_with_recovery(text_system, state.session, state.documents)
    }

    fn index_recoveries(
        tab_count: usize,
        recovered: Vec<recovery::RecoveredDocument>,
    ) -> Result<Vec<Option<recovery::RecoveredDocument>>, SessionRestoreError> {
        let mut recoveries: Vec<Option<recovery::RecoveredDocument>> =
            std::iter::repeat_with(|| None).take(tab_count).collect();
        for document in recovered {
            let index = usize::from(document.tab);
            let slot = recoveries
                .get_mut(index)
                .ok_or(SessionRestoreError::Invalid)?;
            if slot.replace(document).is_some() {
                return Err(SessionRestoreError::Invalid);
            }
        }
        Ok(recoveries)
    }

    fn restore_workspace(
        path: Option<&Path>,
        availability: RestoreAvailability,
    ) -> Result<(Option<Workspace>, Option<PathBuf>), SessionRestoreError> {
        match path.map(Workspace::open_root).transpose() {
            Ok(workspace) => Ok((workspace, None)),
            Err(_error) if availability.allows_placeholder() => {
                Ok((None, path.map(Path::to_path_buf)))
            }
            Err(_error) => Err(SessionRestoreError::Workspace),
        }
    }

    fn from_session_with_recovery(
        text_system: impl StudioTextSystem + 'static,
        state: session::SessionState,
        recovered: Vec<recovery::RecoveredDocument>,
    ) -> Result<Self, SessionRestoreError> {
        session::validate(&state).map_err(|_| SessionRestoreError::Invalid)?;
        let recovered_count = recovered.len();
        let availability = RestoreAvailability::for_recovery(recovered_count);
        let (workspace, unavailable_workspace) =
            Self::restore_workspace(state.workspace.as_deref(), availability)?;
        let session::SessionState {
            tabs,
            active_tab,
            mut panes,
            file_tree,
            ..
        } = state;
        let active_index = usize::from(active_tab);
        let active = tabs.get(active_index).ok_or(SessionRestoreError::Invalid)?;
        let mut recoveries = Self::index_recoveries(tabs.len(), recovered)?;
        let active_document = if let Some(recovered) = recoveries[active_index].as_ref() {
            let document = StudioDocument::recover(active.path.as_deref(), recovered)
                .map_err(|_| SessionRestoreError::Invalid)?;
            recoveries[active_index] = None;
            document
        } else {
            active
                .path
                .as_deref()
                .map(|path| StudioDocument::open_for_restore(path, availability))
                .transpose()
                .map_err(|_| SessionRestoreError::File)?
                .unwrap_or_else(|| StudioDocument::scratch(INITIAL_TEXT))
        };
        let mut app = Self::from_parts(
            text_system,
            active_document,
            active.path.as_deref(),
            workspace,
        )
        .map_err(|_| SessionRestoreError::Surface)?;
        if app.workspace.is_some() {
            app.file_tree
                .restore_session(1, &file_tree)
                .map_err(|_| SessionRestoreError::FileTree)?;
        }
        let restored_tabs = tabs
            .into_iter()
            .map(|tab| RestoredDocumentTab {
                path: tab.path,
                view: tab.view,
            })
            .collect();
        app.tabs =
            DocumentTabs::from_restored(restored_tabs, active_index, DocumentTabLimits::default())
                .map_err(|_| SessionRestoreError::Tabs)?;
        app.pending_recovery = recoveries;
        app.restore_availability = availability;
        for index in 0..app.pending_recovery.len() {
            if app.pending_recovery[index].is_some() {
                app.ensure_document_tab_loaded(index)
                    .map_err(|_| SessionRestoreError::Invalid)?;
            }
        }
        for pane in panes.panes.iter_mut().flatten() {
            let index = usize::from(pane.tab);
            app.ensure_document_tab_loaded(index)
                .map_err(|error| classify_session_document_error(&error))?;
            pane.view = app
                .clamp_tab_view(index, pane.view)
                .map_err(|_| SessionRestoreError::Tabs)?;
        }
        app.restore_availability = RestoreAvailability::Strict;
        let mut tab_ids = Vec::new();
        tab_ids
            .try_reserve_exact(app.tabs.len())
            .map_err(|_| SessionRestoreError::Allocation)?;
        for index in 0..app.tabs.len() {
            tab_ids.push(app.tabs.id_at(index).ok_or(SessionRestoreError::Tabs)?);
        }
        app.panes =
            PaneGrid::from_session(&panes, &tab_ids).map_err(|_| SessionRestoreError::Panes)?;
        let (_, view) = app
            .panes
            .active_document()
            .map_err(|_| SessionRestoreError::Panes)?;
        let view = app.clamp_document_view(view);
        app.apply_document_view(view);
        app.active_workspace_entry = app.tabs.active_workspace_entry();
        app.record_recovered_status(recovered_count, unavailable_workspace.as_deref())?;
        Ok(app)
    }

    fn record_recovered_status(
        &mut self,
        recovered_count: usize,
        unavailable_workspace: Option<&Path>,
    ) -> Result<(), SessionRestoreError> {
        if recovered_count > 0 {
            let mut conflicted_count = 0_usize;
            let mut unavailable_count = 0_usize;
            for index in 0..self.tabs.len() {
                if self
                    .tabs
                    .is_deferred(index)
                    .map_err(|_| SessionRestoreError::Tabs)?
                {
                    continue;
                }
                let document = self
                    .tabs
                    .document_at(index, &self.document)
                    .map_err(|_| SessionRestoreError::Tabs)?;
                conflicted_count += usize::from(document.has_recovery_conflict());
                unavailable_count += usize::from(document.is_unavailable());
            }
            let workspace_status = unavailable_workspace.map_or(
                "",
                |_| " The prior workspace is unavailable; document recovery remains active.",
            );
            self.local_status = Some(LocalStatus::Workspace(Arc::from(format!(
                "Recovered {recovered_count} dirty buffer(s); {conflicted_count} external conflict(s) and {unavailable_count} unavailable clean file(s) remain save-blocked.{workspace_status}"
            ))));
        }
        Ok(())
    }

    fn ensure_document_tab_loaded(&mut self, index: usize) -> Result<(), WorkspaceSelectionError> {
        if !self
            .tabs
            .is_deferred(index)
            .map_err(WorkspaceSelectionError::Tabs)?
        {
            return Ok(());
        }
        let document =
            if let Some(recovered) = self.pending_recovery.get(index).and_then(Option::as_ref) {
                let document = StudioDocument::recover(self.tabs.path_at(index), recovered)
                    .map_err(|_| WorkspaceSelectionError::RevisionExhausted)?;
                self.pending_recovery[index] = None;
                document
            } else {
                self.tabs.path_at(index).map_or_else(
                    || Ok(StudioDocument::scratch(INITIAL_TEXT)),
                    |path| {
                        StudioDocument::open_for_restore(path, self.restore_availability)
                            .map_err(WorkspaceSelectionError::File)
                    },
                )?
            };
        self.tabs
            .materialize(index, document)
            .map_err(WorkspaceSelectionError::Tabs)
    }

    fn clamp_tab_view(
        &self,
        index: usize,
        view: DocumentViewState,
    ) -> Result<DocumentViewState, DocumentTabError> {
        let document = self.tabs.document_at(index, &self.document)?;
        Ok(Self::clamp_view_to_document(document, view))
    }

    fn clamp_view_to_document(
        document: &StudioDocument,
        view: DocumentViewState,
    ) -> DocumentViewState {
        let snapshot = document.buffer().snapshot();
        let length = snapshot.len_bytes();
        let clamp = |offset: ByteOffset| {
            let mut value = offset.get().min(length);
            for _ in 0..3 {
                if snapshot.slice(value..value).is_ok() {
                    return ByteOffset::new(value);
                }
                value = value.saturating_sub(1);
            }
            debug_assert!(snapshot.slice(value..value).is_ok());
            ByteOffset::new(value)
        };
        DocumentViewState {
            selection: Selection::new(clamp(view.selection.anchor()), clamp(view.selection.head())),
            scroll_y: view
                .scroll_y
                .min(usize_as_f32(snapshot.line_count()) * LINE_HEIGHT),
        }
    }

    fn clamp_document_view(&self, view: DocumentViewState) -> DocumentViewState {
        let mut view = Self::clamp_view_to_document(&self.document, view);
        view.scroll_y = view.scroll_y.min(self.maximum_scroll());
        view
    }

    fn capture_session(&mut self) -> Result<session::SessionState, SessionCaptureError> {
        self.capture_session_state(true)
    }

    fn capture_recovery_request(
        &mut self,
    ) -> Result<recovery::RecoveryRequest, SessionCaptureError> {
        let session = self.capture_session_state(false)?;
        let mut documents = Vec::new();
        documents
            .try_reserve_exact(self.tabs.len())
            .map_err(|_| SessionCaptureError::Allocation)?;
        for index in 0..self.tabs.len() {
            if self
                .tabs
                .is_deferred(index)
                .map_err(|_| SessionCaptureError::Tabs)?
            {
                continue;
            }
            let document = self
                .tabs
                .document_at(index, &self.document)
                .map_err(|_| SessionCaptureError::Tabs)?;
            if document.is_dirty() {
                documents.push(recovery::RecoverySnapshot {
                    tab: u8::try_from(index).map_err(|_| SessionCaptureError::Tabs)?,
                    base: document.recovery_base(),
                    local: document.buffer().snapshot(),
                });
            }
        }
        Ok(recovery::RecoveryRequest {
            session,
            documents,
            authority_revision: self.runtime_document_revision,
        })
    }

    fn capture_session_state(
        &mut self,
        reject_dirty: bool,
    ) -> Result<session::SessionState, SessionCaptureError> {
        self.sync_active_pane_document()
            .map_err(|_| SessionCaptureError::Panes)?;
        let active_view = self.active_document_view();
        let mut tabs = Vec::new();
        tabs.try_reserve_exact(self.tabs.len())
            .map_err(|_| SessionCaptureError::Allocation)?;
        for index in 0..self.tabs.len() {
            if !self
                .tabs
                .is_deferred(index)
                .map_err(|_| SessionCaptureError::Tabs)?
            {
                let document = self
                    .tabs
                    .document_at(index, &self.document)
                    .map_err(|_| SessionCaptureError::Tabs)?;
                if reject_dirty && document.is_dirty() {
                    return Err(SessionCaptureError::DirtyDocument);
                }
            }
            tabs.push(session::SessionTab {
                path: self.tabs.path_at(index).map(Path::to_path_buf),
                view: self
                    .tabs
                    .view_at(index, active_view)
                    .map_err(|_| SessionCaptureError::Tabs)?,
            });
        }
        let panes = self
            .panes
            .session_state(|id| self.tabs.index_for_id(id))
            .map_err(|_| SessionCaptureError::Panes)?;
        let active_tab =
            u8::try_from(self.tabs.active_index()).map_err(|_| SessionCaptureError::Tabs)?;
        let file_tree = self
            .file_tree
            .session_state()
            .map_err(|_| SessionCaptureError::FileTree)?;
        let state = session::SessionState {
            workspace: self
                .workspace
                .as_ref()
                .map(|workspace| workspace.root().to_path_buf()),
            tabs,
            active_tab,
            panes,
            file_tree,
        };
        session::validate(&state).map_err(|_| SessionCaptureError::Invalid)?;
        Ok(state)
    }

    fn configure_persistence(&mut self, path: PathBuf) -> Result<(), SurfaceError> {
        let recovery_path = recovery::path_for_session(&path);
        self.session_path = Some(path);
        self.recovery = Some(
            recovery::RecoveryCoordinator::new(recovery_path)
                .map_err(|_| SurfaceError::DriverUnavailable)?,
        );
        self.publish_recovery();
        Ok(())
    }

    fn publish_recovery(&mut self) {
        if self.recovery.is_none() {
            return;
        }
        let request = match self.capture_recovery_request() {
            Ok(request) => request,
            Err(error) => {
                self.local_status = Some(LocalStatus::Workspace(Arc::from(format!(
                    "Recovery capture failed: {error:?}"
                ))));
                return;
            }
        };
        let result = self
            .recovery
            .as_ref()
            .ok_or(recovery::RecoveryError::Disconnected)
            .and_then(|coordinator| coordinator.publish(request));
        let error = result.err().or_else(|| {
            self.recovery
                .as_ref()
                .and_then(|coordinator| coordinator.status().last_error)
        });
        if error != self.last_recovery_error {
            self.last_recovery_error = error;
            if let Some(error) = error {
                self.local_status = Some(LocalStatus::Workspace(Arc::from(format!(
                    "Dirty-buffer recovery degraded: {error}"
                ))));
            }
        }
    }

    const fn buffer(&self) -> &Buffer {
        self.document.buffer()
    }

    const fn buffer_mut(&mut self) -> &mut Buffer {
        self.document.buffer_mut()
    }

    fn font() -> Result<FontKey, StudioRenderError> {
        let size = PositiveFinite::new(FONT_SIZE).ok_or(StudioRenderError::Domain)?;
        let scale = PositiveFinite::new(DEFAULT_SCALE).ok_or(StudioRenderError::Domain)?;
        let tabs = NonZeroU32::new(4).ok_or(StudioRenderError::Domain)?;
        Ok(FontKey::new(FONT_FAMILY, size, scale, tabs))
    }

    fn scene(&mut self, revision: SceneRevision, viewport: Size) -> Scene {
        match self.try_scene(revision, viewport) {
            Ok(scene) => scene,
            Err(error) => {
                let _error_message = error.to_string();
                self.render_failures = self.render_failures.saturating_add(1);
                self.rendered_lines.clear();
                Self::fallback_scene(revision, viewport)
            }
        }
    }

    fn fallback_scene(revision: SceneRevision, viewport: Size) -> Scene {
        let mut builder = SceneBuilder::new(revision, viewport);
        if let (Some(origin), Some(color)) = (
            Point::new(0.0, 0.0),
            LinearRgba::new(0.035, 0.04, 0.045, 1.0),
        ) {
            builder.push(Primitive::Quad {
                bounds: Rect::new(origin, viewport),
                color,
            });
        }
        builder.finish()
    }

    #[allow(
        clippy::too_many_lines,
        reason = "the frame builder keeps one explicit painter-order transaction"
    )]
    fn try_scene(
        &mut self,
        revision: SceneRevision,
        viewport: Size,
    ) -> Result<Scene, StudioRenderError> {
        self.last_viewport = viewport;
        self.clamp_scroll();
        self.layout_cache.begin_frame()?;

        let origin = Point::new(0.0, 0.0).ok_or(StudioRenderError::Domain)?;
        let sidebar_width = self.sidebar_width(viewport);
        let content_bounds = self
            .editor_region(viewport)
            .map_err(|_| StudioRenderError::Domain)?;
        let active_tab = self
            .tabs
            .active_id()
            .map_err(|_| StudioRenderError::Domain)?;
        let active_view = self.active_document_view();
        self.panes
            .sync_active_document(active_tab, active_view)
            .map_err(|_| StudioRenderError::Domain)?;
        let pane_layout = self
            .panes
            .layout(content_bounds)
            .map_err(|_| StudioRenderError::Domain)?;
        let active_pane = pane_layout.active().ok_or(StudioRenderError::Domain)?;
        let editor_origin_x = active_pane.bounds.origin().x();
        let content_size = active_pane.bounds.size();
        let line_height = PositiveFinite::new(LINE_HEIGHT).ok_or(StudioRenderError::Domain)?;
        let font = Self::font()?;
        let snapshot = self.buffer().snapshot();
        let background =
            LinearRgba::new(0.035, 0.04, 0.045, 1.0).ok_or(StudioRenderError::Domain)?;
        let editor_background =
            LinearRgba::new(0.055, 0.06, 0.067, 1.0).ok_or(StudioRenderError::Domain)?;
        let selection_color =
            LinearRgba::new(0.18, 0.48, 0.72, SELECTION_ALPHA).ok_or(StudioRenderError::Domain)?;
        let text_color = LinearRgba::new(0.86, 0.88, 0.9, 1.0).ok_or(StudioRenderError::Domain)?;
        let caret_color =
            LinearRgba::new(0.94, 0.72, 0.25, 1.0).ok_or(StudioRenderError::Domain)?;
        let status_background_color =
            LinearRgba::new(0.34, 0.075, 0.065, 0.96).ok_or(StudioRenderError::Domain)?;
        let sidebar_background =
            LinearRgba::new(0.027, 0.031, 0.035, 1.0).ok_or(StudioRenderError::Domain)?;
        let active_row_color =
            LinearRgba::new(0.12, 0.16, 0.19, 1.0).ok_or(StudioRenderError::Domain)?;
        let tab_background =
            LinearRgba::new(0.025, 0.028, 0.032, 1.0).ok_or(StudioRenderError::Domain)?;
        let active_tab_color =
            LinearRgba::new(0.095, 0.105, 0.115, 1.0).ok_or(StudioRenderError::Domain)?;
        let find_match_color =
            LinearRgba::new(0.62, 0.45, 0.08, 0.38).ok_or(StudioRenderError::Domain)?;
        let find_background_color =
            LinearRgba::new(0.08, 0.09, 0.10, 0.98).ok_or(StudioRenderError::Domain)?;
        let quick_open_background =
            LinearRgba::new(0.045, 0.052, 0.058, 0.99).ok_or(StudioRenderError::Domain)?;
        let quick_open_selected =
            LinearRgba::new(0.12, 0.25, 0.31, 1.0).ok_or(StudioRenderError::Domain)?;
        let project_search_background =
            LinearRgba::new(0.04, 0.06, 0.055, 0.995).ok_or(StudioRenderError::Domain)?;
        let project_search_selected =
            LinearRgba::new(0.10, 0.30, 0.22, 1.0).ok_or(StudioRenderError::Domain)?;
        let command_palette_background =
            LinearRgba::new(0.055, 0.062, 0.067, 0.995).ok_or(StudioRenderError::Domain)?;
        let command_palette_selected =
            LinearRgba::new(0.34, 0.22, 0.075, 1.0).ok_or(StudioRenderError::Domain)?;

        let mut builder = SceneBuilder::new(revision, viewport);
        builder.push_quad(Quad::new(Rect::new(origin, viewport), background))?;
        let mut pane_clips = [None; MAX_PANES];
        for (index, pane) in pane_layout.iter().enumerate() {
            let pane_clip = builder.push_clip(Clip::new(pane.bounds));
            pane_clips[index] = Some(pane_clip);
            builder.push_quad(Quad::new(pane.bounds, editor_background))?;
        }
        let active_clip = pane_layout
            .iter()
            .enumerate()
            .find(|(_, pane)| pane.active)
            .and_then(|(index, _)| pane_clips[index])
            .ok_or(StudioRenderError::Domain)?;

        let mut rendered_lines = Vec::new();
        let mut pending_glyphs = Vec::new();
        if self.workspace.is_some() && self.file_tree.is_visible() {
            let sidebar_size = Size::new(sidebar_width.max(1.0), viewport.height())
                .ok_or(StudioRenderError::Domain)?;
            let sidebar_bounds = Rect::new(origin, sidebar_size);
            let sidebar_clip = builder.push_clip(Clip::new(sidebar_bounds));
            builder.push_quad(Quad::new(sidebar_bounds, sidebar_background))?;
            let first_visible =
                floor_f32_to_usize(self.workspace_scroll_y / TREE_ROW_HEIGHT).unwrap_or(0);
            let visible_rows = floor_f32_to_usize(viewport.height() / TREE_ROW_HEIGHT)
                .unwrap_or(0)
                .saturating_add(1);
            let rows =
                self.file_tree
                    .visible_rows(first_visible, visible_rows, TREE_OVERSCAN_ROWS)?;
            for row in rows {
                let index = row.index;
                let top =
                    CONTENT_INSET + usize_as_f32(index) * TREE_ROW_HEIGHT - self.workspace_scroll_y;
                if row.selected {
                    let row_origin = Point::new(0.0, top).ok_or(StudioRenderError::Domain)?;
                    let row_size = Size::new(sidebar_width.max(1.0), TREE_ROW_HEIGHT)
                        .ok_or(StudioRenderError::Domain)?;
                    let row = Quad::new(Rect::new(row_origin, row_size), active_row_color)
                        .clipped(sidebar_clip);
                    builder.push_quad(row)?;
                }
                let layout = self.text_system.shape(row.label(), font)?;
                let baseline = top + layout.ascent();
                let indent = usize_as_f32(row.depth).mul_add(12.0, CONTENT_INSET);
                let glyphs = self.collect_glyphs(&layout, font, indent, baseline, sidebar_clip)?;
                pending_glyphs.extend(glyphs);
            }
        }
        let tab_origin = Point::new(sidebar_width, 0.0).ok_or(StudioRenderError::Domain)?;
        let tab_size = Size::new((viewport.width() - sidebar_width).max(1.0), TAB_BAR_HEIGHT)
            .ok_or(StudioRenderError::Domain)?;
        let tab_bounds = Rect::new(tab_origin, tab_size);
        let tab_clip = builder.push_clip(Clip::new(tab_bounds));
        let first_visible = floor_f32_to_usize(self.tab_scroll_x / TAB_WIDTH).unwrap_or(0);
        let visible_tabs = floor_f32_to_usize(tab_size.width() / TAB_WIDTH)
            .unwrap_or(0)
            .saturating_add(1);
        let tab_range = self
            .tabs
            .visible_range(first_visible, visible_tabs, TAB_OVERSCAN);
        let tab_labels: Vec<(usize, Arc<str>)> = tab_range
            .filter_map(|index| self.tabs.label(index).map(|label| (index, label)))
            .collect();
        for (index, label) in &tab_labels {
            let left = sidebar_width + usize_as_f32(*index) * TAB_WIDTH - self.tab_scroll_x;
            let layout = self.text_system.shape(label, font)?;
            pending_glyphs.extend(self.collect_glyphs(
                &layout,
                font,
                left + 8.0,
                layout.ascent() + 4.0,
                tab_clip,
            )?);
        }
        for (pane_index, pane) in pane_layout.iter().enumerate() {
            let pane_clip = pane_clips[pane_index].ok_or(StudioRenderError::Domain)?;
            let (pane_tab, pane_view) = self
                .panes
                .document_for(pane.id, active_tab, active_view)
                .map_err(|_| StudioRenderError::Domain)?;
            let pane_snapshot = self
                .tabs
                .document_for_id(pane_tab, &self.document)
                .map_err(|_| StudioRenderError::Domain)?
                .buffer()
                .snapshot();
            let pane_scroll = pane_view.scroll_y;
            let viewport_height = PositiveFinite::new(pane.bounds.size().height())
                .ok_or(StudioRenderError::Domain)?;
            let wrap_width =
                PositiveFinite::new(pane.bounds.size().width()).ok_or(StudioRenderError::Domain)?;
            let visible = VisibleLines::new(
                pane_snapshot.line_count(),
                pane_scroll,
                viewport_height,
                line_height,
                DEFAULT_OVERSCAN_LINES,
            )?;
            let pane_origin_x = pane.bounds.origin().x();
            let pane_selection = pane_view.selection.range();
            for line in visible.laid_out() {
                let layout = self.layout_cache.layout_line(
                    &pane_snapshot,
                    line,
                    font,
                    wrap_width,
                    &mut *self.text_system,
                )?;
                let top = pane.bounds.origin().y() + usize_as_f32(line) * LINE_HEIGHT - pane_scroll;
                let baseline = top + layout.ascent();
                let line_range = pane_snapshot.line_byte_range(line)?;
                if pane.active {
                    for found in self.find.visible_ranges(
                        self.runtime_document_revision,
                        pane_snapshot.revision().get(),
                        line_range,
                    ) {
                        Self::paint_selection(
                            &mut builder,
                            pane_clip,
                            &pane_snapshot,
                            line,
                            top,
                            &layout,
                            found.clone(),
                            find_match_color,
                            pane_origin_x,
                        )?;
                    }
                }
                if !pane_selection.is_empty() {
                    let selection_result = Self::paint_selection(
                        &mut builder,
                        pane_clip,
                        &pane_snapshot,
                        line,
                        top,
                        &layout,
                        pane_selection.clone(),
                        selection_color,
                        pane_origin_x,
                    );
                    selection_result?;
                }
                pending_glyphs.extend(self.collect_glyphs(
                    &layout,
                    font,
                    pane_origin_x,
                    baseline,
                    pane_clip,
                )?);
                if pane.active {
                    rendered_lines.push(RenderedLine {
                        line,
                        top,
                        baseline,
                        layout,
                    });
                }
            }
        }

        let mut composition_underline = None;
        if let Some(composition) = self.composition.clone()
            && let Some(line) = Self::line_for_offset(&snapshot, composition.replacement.start)?
            && let Some(rendered) = rendered_lines.iter().find(|rendered| rendered.line == line)
        {
            let source = snapshot.line_byte_range(line)?;
            let prefix_end = composition.replacement.start.min(source.end);
            let prefix = snapshot.slice(source.start..prefix_end)?;
            let prefix_utf16 = u32::try_from(prefix.encode_utf16().count())
                .map_err(|_| StudioRenderError::Domain)?;
            let start_x = editor_origin_x + x_for_utf16(&rendered.layout, prefix_utf16);
            let composition_layout = self.text_system.shape(&composition.text, font)?;
            let composition_glyphs = self.collect_glyphs(
                &composition_layout,
                font,
                start_x,
                rendered.baseline,
                active_clip,
            );
            pending_glyphs.extend(composition_glyphs?);
            let underline_origin = Point::new(
                start_x,
                rendered.baseline + composition_layout.descent() + 1.0,
            )
            .ok_or(StudioRenderError::Domain)?;
            let underline_size = Size::new(composition_layout.width().max(1.0), 1.0)
                .ok_or(StudioRenderError::Domain)?;
            composition_underline = Some(Rect::new(underline_origin, underline_size));
        }

        let status_background = if let Some(status) = self.local_status.clone() {
            let layout = self.text_system.shape(status.message(), font)?;
            let top = (active_pane.bounds.origin().y() + content_size.height() - LINE_HEIGHT)
                .max(active_pane.bounds.origin().y());
            let baseline = top + layout.ascent();
            let status_glyphs =
                self.collect_glyphs(&layout, font, editor_origin_x + 6.0, baseline, active_clip);
            pending_glyphs.extend(status_glyphs?);
            let origin = Point::new(editor_origin_x, top).ok_or(StudioRenderError::Domain)?;
            let size =
                Size::new(content_size.width(), LINE_HEIGHT).ok_or(StudioRenderError::Domain)?;
            Some(Rect::new(origin, size))
        } else {
            None
        };
        if let Some(bounds) = status_background {
            builder.push_quad(Quad::new(bounds, status_background_color).clipped(active_clip))?;
        }
        if self.find.is_open() {
            let width = FIND_BAR_WIDTH.min(content_size.width());
            let left = (active_pane.bounds.origin().x() + content_size.width() - width)
                .max(active_pane.bounds.origin().x());
            let overlay_origin = Point::new(left, TAB_BAR_HEIGHT + FIND_BAR_INSET)
                .ok_or(StudioRenderError::Domain)?;
            let overlay_size =
                Size::new(width.max(1.0), FIND_BAR_HEIGHT).ok_or(StudioRenderError::Domain)?;
            let overlay_bounds = Rect::new(overlay_origin, overlay_size);
            let overlay_clip = builder.push_clip(Clip::new(overlay_bounds));
            builder.push_quad(Quad::new(overlay_bounds, find_background_color))?;
            let display = self.find.display_text()?;
            let layout = self.text_system.shape(&display, font)?;
            let origin_x = left + FIND_BAR_INSET;
            let baseline = overlay_origin.y() + layout.ascent() + 6.0;
            let overlay_glyphs =
                self.collect_glyphs(&layout, font, origin_x, baseline, overlay_clip)?;
            pending_glyphs.extend(overlay_glyphs);
        }
        if self.quick_open.is_open() {
            let rows = self
                .quick_open
                .visible_results(QUICK_OPEN_VISIBLE_ROWS, QUICK_OPEN_OVERSCAN_ROWS);
            let width = QUICK_OPEN_WIDTH.min((viewport.width() - CONTENT_INSET * 2.0).max(1.0));
            let left = ((viewport.width() - width) * 0.5).max(0.0);
            let top = TAB_BAR_HEIGHT + CONTENT_INSET;
            let height = QUICK_OPEN_QUERY_HEIGHT + usize_as_f32(rows.len()) * QUICK_OPEN_ROW_HEIGHT;
            let overlay_origin = Point::new(left, top).ok_or(StudioRenderError::Domain)?;
            let overlay_size =
                Size::new(width, height.max(1.0)).ok_or(StudioRenderError::Domain)?;
            let overlay_bounds = Rect::new(overlay_origin, overlay_size);
            let overlay_clip = builder.push_clip(Clip::new(overlay_bounds));
            builder.push_quad(Quad::new(overlay_bounds, quick_open_background))?;
            let display = self.quick_open.display_text()?;
            let query_layout = self.text_system.shape(&display, font)?;
            pending_glyphs.extend(self.collect_glyphs(
                &query_layout,
                font,
                left + FIND_BAR_INSET,
                top + query_layout.ascent() + 7.0,
                overlay_clip,
            )?);
            for (row, (path, selected)) in rows.iter().enumerate() {
                let row_top =
                    top + QUICK_OPEN_QUERY_HEIGHT + usize_as_f32(row) * QUICK_OPEN_ROW_HEIGHT;
                if *selected {
                    let row_origin = Point::new(left, row_top).ok_or(StudioRenderError::Domain)?;
                    let row_size =
                        Size::new(width, QUICK_OPEN_ROW_HEIGHT).ok_or(StudioRenderError::Domain)?;
                    let selected_quad =
                        Quad::new(Rect::new(row_origin, row_size), quick_open_selected)
                            .clipped(overlay_clip);
                    builder.push_quad(selected_quad)?;
                }
                let layout = self.text_system.shape(path, font)?;
                let origin_x = left + FIND_BAR_INSET;
                let baseline = row_top + layout.ascent() + 4.0;
                let row_glyphs =
                    self.collect_glyphs(&layout, font, origin_x, baseline, overlay_clip)?;
                pending_glyphs.extend(row_glyphs);
            }
        }
        if self.project_search.is_open() {
            let rows = self
                .project_search
                .visible_results(PROJECT_SEARCH_VISIBLE_ROWS, PROJECT_SEARCH_OVERSCAN_ROWS)?;
            let width = PROJECT_SEARCH_WIDTH.min((viewport.width() - CONTENT_INSET * 2.0).max(1.0));
            let left = ((viewport.width() - width) * 0.5).max(0.0);
            let top = TAB_BAR_HEIGHT + CONTENT_INSET;
            let height =
                PROJECT_SEARCH_QUERY_HEIGHT + usize_as_f32(rows.len()) * PROJECT_SEARCH_ROW_HEIGHT;
            let overlay_origin = Point::new(left, top).ok_or(StudioRenderError::Domain)?;
            let overlay_size =
                Size::new(width, height.max(1.0)).ok_or(StudioRenderError::Domain)?;
            let overlay_bounds = Rect::new(overlay_origin, overlay_size);
            let overlay_clip = builder.push_clip(Clip::new(overlay_bounds));
            let project_selection_clip = overlay_clip;
            #[cfg(test)]
            let project_selection_clip = if self.force_project_search_clip_failure.take().is_some()
            {
                let mut foreign = SceneBuilder::new(revision, viewport);
                let mut invalid = foreign.push_clip(Clip::new(overlay_bounds));
                for _ in 0..128 {
                    invalid = foreign.push_clip(Clip::new(overlay_bounds));
                }
                invalid
            } else {
                project_selection_clip
            };
            builder.push_quad(Quad::new(overlay_bounds, project_search_background))?;
            let display = self.project_search.display_text()?;
            let query_layout = self.text_system.shape(&display, font)?;
            pending_glyphs.extend(self.collect_glyphs(
                &query_layout,
                font,
                left + FIND_BAR_INSET,
                top + query_layout.ascent() + 7.0,
                overlay_clip,
            )?);
            for (row_index, row) in rows.iter().enumerate() {
                let row_top = top
                    + PROJECT_SEARCH_QUERY_HEIGHT
                    + usize_as_f32(row_index) * PROJECT_SEARCH_ROW_HEIGHT;
                if row.selected {
                    let row_origin = Point::new(left, row_top).ok_or(StudioRenderError::Domain)?;
                    let row_size = Size::new(width, PROJECT_SEARCH_ROW_HEIGHT)
                        .ok_or(StudioRenderError::Domain)?;
                    builder.push_quad(
                        Quad::new(Rect::new(row_origin, row_size), project_search_selected)
                            .clipped(project_selection_clip),
                    )?;
                }
                let layout = self.text_system.shape(&row.label, font)?;
                let baseline = row_top + layout.ascent() + 4.0;
                #[allow(
                    clippy::question_mark,
                    reason = "explicit propagation keeps both renderer failure paths observable"
                )]
                let row_glyphs = match self.collect_glyphs(
                    &layout,
                    font,
                    left + FIND_BAR_INSET,
                    baseline,
                    overlay_clip,
                ) {
                    Ok(glyphs) => glyphs,
                    Err(error) => return Err(error),
                };
                pending_glyphs.extend(row_glyphs);
            }
        }
        if self.command_palette.is_open() {
            let rows = self.command_palette.visible_commands()?;
            let width =
                COMMAND_PALETTE_WIDTH.min((viewport.width() - CONTENT_INSET * 2.0).max(1.0));
            let left = ((viewport.width() - width) * 0.5).max(0.0);
            let top = TAB_BAR_HEIGHT + CONTENT_INSET;
            let height = COMMAND_PALETTE_QUERY_HEIGHT
                + usize_as_f32(rows.len()) * COMMAND_PALETTE_ROW_HEIGHT;
            let overlay_origin = Point::new(left, top).ok_or(StudioRenderError::Domain)?;
            let overlay_size =
                Size::new(width, height.max(1.0)).ok_or(StudioRenderError::Domain)?;
            let overlay_bounds = Rect::new(overlay_origin, overlay_size);
            let overlay_clip = builder.push_clip(Clip::new(overlay_bounds));
            let command_selection_clip = overlay_clip;
            #[cfg(test)]
            let command_selection_clip = if self.force_command_clip_failure.take().is_some() {
                let mut foreign = SceneBuilder::new(revision, viewport);
                let mut invalid = foreign.push_clip(Clip::new(overlay_bounds));
                for _ in 0..128 {
                    invalid = foreign.push_clip(Clip::new(overlay_bounds));
                }
                invalid
            } else {
                command_selection_clip
            };
            builder.push_quad(Quad::new(overlay_bounds, command_palette_background))?;
            let display = self.command_palette.display_text()?;
            let query_layout = self.text_system.shape(&display, font)?;
            pending_glyphs.extend(self.collect_glyphs(
                &query_layout,
                font,
                left + FIND_BAR_INSET,
                top + query_layout.ascent() + 7.0,
                overlay_clip,
            )?);
            for (row_index, row) in rows.iter().enumerate() {
                let row_top = top
                    + COMMAND_PALETTE_QUERY_HEIGHT
                    + usize_as_f32(row_index) * COMMAND_PALETTE_ROW_HEIGHT;
                if row.selected {
                    let row_origin = Point::new(left, row_top).ok_or(StudioRenderError::Domain)?;
                    let row_size = Size::new(width, COMMAND_PALETTE_ROW_HEIGHT)
                        .ok_or(StudioRenderError::Domain)?;
                    builder.push_quad(
                        Quad::new(Rect::new(row_origin, row_size), command_palette_selected)
                            .clipped(command_selection_clip),
                    )?;
                }
                let layout = self.text_system.shape(row.title, font)?;
                let baseline = row_top + layout.ascent() + 4.0;
                pending_glyphs.extend(self.collect_glyphs(
                    &layout,
                    font,
                    left + FIND_BAR_INSET,
                    baseline,
                    overlay_clip,
                )?);
            }
        }
        builder.push_quad(Quad::new(tab_bounds, tab_background).clipped(tab_clip))?;
        if tab_labels
            .iter()
            .any(|(index, _)| *index == self.tabs.active_index())
        {
            let active_left = sidebar_width + usize_as_f32(self.tabs.active_index()) * TAB_WIDTH
                - self.tab_scroll_x;
            let active_origin = Point::new(active_left, 0.0).ok_or(StudioRenderError::Domain)?;
            let active_size =
                Size::new(TAB_WIDTH, TAB_BAR_HEIGHT).ok_or(StudioRenderError::Domain)?;
            let active_quad = Quad::new(Rect::new(active_origin, active_size), active_tab_color)
                .clipped(tab_clip);
            builder.push_quad(active_quad)?;
        }

        self.publish_atlas_if_needed(&pending_glyphs)?;
        if !pending_glyphs.is_empty() {
            let atlas = self
                .published_atlas
                .clone()
                .ok_or(StudioRenderError::Domain)?;
            builder.set_glyph_atlas(atlas)?;
            for pending in pending_glyphs {
                let glyph = Glyph::new(pending.bounds, pending.atlas_bounds, text_color)
                    .clipped(pending.clip);
                builder.push_glyph(glyph)?;
            }
        }
        if let Some(bounds) = composition_underline {
            builder.push_quad(Quad::new(bounds, caret_color).clipped(active_clip))?;
        }
        if self.focused
            && !self.find.is_open()
            && !self.quick_open.is_open()
            && !self.project_search.is_open()
            && !self.file_tree.is_focused()
            && let Some(caret) = self.caret_bounds(&snapshot, &rendered_lines, editor_origin_x)?
        {
            builder.push_quad(Quad::new(caret, caret_color).clipped(active_clip))?;
        }

        self.rendered_lines = rendered_lines;
        Ok(builder.finish())
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "selection painting keeps all scene-local values explicit"
    )]
    fn paint_selection(
        builder: &mut SceneBuilder,
        clip: alpine_scene::ClipId,
        snapshot: &BufferSnapshot,
        line: usize,
        top: f32,
        layout: &LineLayout,
        selection: Range<usize>,
        color: LinearRgba,
        origin_x: f32,
    ) -> Result<(), StudioRenderError> {
        let line_range = snapshot.line_byte_range(line)?;
        if selection.end <= line_range.start || selection.start >= line_range.end {
            return Ok(());
        }
        let text = snapshot.slice(line_range.clone())?;
        let content = text.trim_end_matches(['\r', '\n']);
        let content_end = line_range.start + content.len();
        let start = selection.start.max(line_range.start).min(content_end);
        let end = selection.end.min(content_end);
        let start_utf16 = local_utf16(snapshot, line_range.start, start)?;
        let end_utf16 = if selection.end >= line_range.end && line_range.end < snapshot.len_bytes()
        {
            u32::try_from(content.encode_utf16().count()).map_err(|_| StudioRenderError::Domain)?
        } else {
            local_utf16(snapshot, line_range.start, end)?
        };
        let start_x = origin_x + x_for_utf16(layout, start_utf16);
        let end_x = origin_x + x_for_utf16(layout, end_utf16);
        let width = (end_x - start_x).max(if selection.end > content_end {
            6.0
        } else {
            1.0
        });
        let origin = Point::new(start_x, top).ok_or(StudioRenderError::Domain)?;
        let size = Size::new(width, LINE_HEIGHT).ok_or(StudioRenderError::Domain)?;
        builder.push_quad(Quad::new(Rect::new(origin, size), color).clipped(clip))?;
        Ok(())
    }

    fn collect_glyphs(
        &mut self,
        layout: &LineLayout,
        requested_font: FontKey,
        origin_x: f32,
        baseline: f32,
        clip: alpine_scene::ClipId,
    ) -> Result<Vec<PendingGlyph>, StudioRenderError> {
        let mut pending = Vec::new();
        pending
            .try_reserve(layout.glyphs().len())
            .map_err(|_| LayoutError::AllocationFailed)?;
        for glyph in layout.glyphs() {
            let family = if glyph.resolved_family() == 0 {
                requested_font.family()
            } else {
                glyph.resolved_family()
            };
            let font = FontKey::new(
                family,
                PositiveFinite::new(requested_font.size()).ok_or(StudioRenderError::Domain)?,
                PositiveFinite::new(requested_font.scale()).ok_or(StudioRenderError::Domain)?,
                requested_font.tab_columns(),
            );
            let rasterized = self.text_system.rasterize(font, glyph.glyph_id(), 0)?;
            let Some(bitmap) = rasterized.bitmap() else {
                continue;
            };
            let rect = self
                .glyph_atlas
                .insert(GlyphKey::new(font, glyph.glyph_id(), 0), bitmap)?;
            let width = u32_as_f32(rect.width().get()) / font.scale();
            let height = u32_as_f32(rect.height().get()) / font.scale();
            let origin = Point::new(
                origin_x + glyph.x() + rasterized.left(),
                baseline - rasterized.top() + glyph.y(),
            )
            .ok_or(StudioRenderError::Domain)?;
            let size = Size::new(width, height).ok_or(StudioRenderError::Domain)?;
            pending.push(PendingGlyph {
                bounds: Rect::new(origin, size),
                atlas_bounds: AtlasBounds::new(rect.x(), rect.y(), rect.width(), rect.height()),
                clip,
            });
        }
        Ok(pending)
    }

    fn publish_atlas_if_needed(
        &mut self,
        pending: &[PendingGlyph],
    ) -> Result<(), StudioRenderError> {
        if pending.is_empty() {
            return Ok(());
        }
        let snapshot = self.glyph_atlas.snapshot();
        let dimension = NonZeroU32::new(snapshot.dimension()).ok_or(StudioRenderError::Domain)?;
        let must_publish = self.published_atlas.as_ref().is_none_or(|atlas| {
            atlas.width() != dimension || atlas.pixels() != self.glyph_atlas.pixels()
        });
        if must_publish {
            self.atlas_revision = self
                .atlas_revision
                .checked_add(1)
                .ok_or(LayoutError::SequenceExhausted)?;
            let pixels: Arc<[u8]> = self.glyph_atlas.pixels().to_vec().into();
            let image = GlyphAtlasImage::new(self.atlas_revision, dimension, dimension, pixels);
            self.published_atlas = Some(image?);
        }
        Ok(())
    }

    fn caret_bounds(
        &self,
        snapshot: &BufferSnapshot,
        rendered_lines: &[RenderedLine],
        origin_x: f32,
    ) -> Result<Option<Rect>, StudioRenderError> {
        let offset = self.selection.head();
        let Some(line) = Self::line_for_offset(snapshot, offset.get())? else {
            return Ok(None);
        };
        let Some(rendered) = rendered_lines.iter().find(|rendered| rendered.line == line) else {
            return Ok(None);
        };
        let line_range = snapshot.line_byte_range(line)?;
        let utf16 = local_utf16(snapshot, line_range.start, offset.get())?;
        let x = origin_x + x_for_utf16(&rendered.layout, utf16);
        let origin = Point::new(x, rendered.top).ok_or(StudioRenderError::Domain)?;
        let size = Size::new(CARET_WIDTH, LINE_HEIGHT).ok_or(StudioRenderError::Domain)?;
        Ok(Some(Rect::new(origin, size)))
    }

    fn handle_event_with_response(&mut self, event: &SurfaceEvent) -> StudioTransition {
        if (self.find.is_open()
            || self.quick_open.is_open()
            || self.project_search.is_open()
            || self.command_palette.is_open()
            || self.file_tree.is_focused())
            && studio_clipboard_shortcut(event).is_some()
        {
            return StudioTransition::default();
        }
        if let Some(operation) = studio_clipboard_shortcut(event) {
            if operation == ClipboardOperation::Paste {
                return StudioTransition::default();
            }
            return self.begin_clipboard_operation(operation);
        }
        let effect = match event {
            SurfaceEvent::Keyboard {
                state: KeyState::Down,
                physical_key,
                modifiers,
                ..
            } => self.handle_key(*physical_key, *modifiers),
            SurfaceEvent::Pointer {
                action,
                position,
                button,
                modifiers,
                ..
            } => self.handle_pointer(*action, *position, *button, *modifiers),
            SurfaceEvent::Scroll { delta_y, .. } => {
                let over_workspace = self.last_pointer_position.is_some_and(|position| {
                    self.workspace.is_some()
                        && self.file_tree.is_visible()
                        && position.x() < self.sidebar_width(self.last_viewport)
                });
                let changed = if over_workspace {
                    let before = self.workspace_scroll_y;
                    self.workspace_scroll_y = (self.workspace_scroll_y - *delta_y)
                        .clamp(0.0, self.maximum_workspace_scroll());
                    self.workspace_scroll_y.to_bits() != before.to_bits()
                } else {
                    let before = self.scroll_y;
                    self.scroll_y = (self.scroll_y - *delta_y).clamp(0.0, self.maximum_scroll());
                    self.scroll_y.to_bits() != before.to_bits()
                };
                changed.then(EventEffect::visual).unwrap_or_default()
            }
            SurfaceEvent::Focus { focused, .. } => {
                let changed = self.focused != *focused;
                self.focused = *focused;
                changed.then(EventEffect::visual).unwrap_or_default()
            }
            SurfaceEvent::Ime { event, .. } => self.handle_ime(event),
            SurfaceEvent::Clipboard { event, .. } => {
                return self.handle_clipboard_completion(event);
            }
            SurfaceEvent::CloseRequested { .. } => return self.handle_close_request(),
            SurfaceEvent::Keyboard { .. }
            | SurfaceEvent::Resize { .. }
            | SurfaceEvent::Wake { .. } => EventEffect::default(),
        };
        StudioTransition::effect(effect)
    }

    #[cfg(test)]
    fn handle_event(&mut self, event: &SurfaceEvent) -> EventEffect {
        self.handle_event_with_response(event).effect
    }

    fn begin_clipboard_operation(&mut self, operation: ClipboardOperation) -> StudioTransition {
        self.pending_cut = None;
        let mut effect = self.clear_clipboard_status();
        let selection = self.selection;
        if selection.range().is_empty() {
            return StudioTransition::effect(effect);
        }
        let Ok(text) = self.buffer().snapshot().slice(selection.range()) else {
            self.input_failures = self.input_failures.saturating_add(1);
            effect = effect.merge(
                self.record_clipboard_protocol_failure("Clipboard selection is no longer valid."),
            );
            return StudioTransition::effect(effect);
        };
        let text = match ClipboardText::new(text) {
            Ok(text) => text,
            Err(error) => {
                effect = effect.merge(self.record_clipboard_error(error));
                return StudioTransition::effect(effect);
            }
        };
        let write = match ClipboardWrite::new(operation, text) {
            Ok(write) => write,
            Err(error) => {
                effect = effect.merge(self.record_clipboard_error(error));
                return StudioTransition::effect(effect);
            }
        };
        if operation == ClipboardOperation::Cut {
            self.pending_cut = Some(PendingCut {
                revision: self.buffer().revision().get(),
                selection,
            });
        }
        StudioTransition {
            effect,
            clipboard_write: Some(write),
            cancel_close: false,
        }
    }

    fn handle_clipboard_completion(&mut self, event: &ClipboardEvent) -> StudioTransition {
        let effect = match event {
            ClipboardEvent::CopyCompleted(Ok(())) => self.clear_clipboard_status(),
            ClipboardEvent::CopyCompleted(Err(error))
            | ClipboardEvent::PasteCompleted(Err(error)) => self.record_clipboard_error(*error),
            ClipboardEvent::CutCompleted(Ok(())) => self.complete_cut(),
            ClipboardEvent::CutCompleted(Err(error)) => {
                self.pending_cut = None;
                self.record_clipboard_error(*error)
            }
            ClipboardEvent::PasteCompleted(Ok(text)) => self
                .clear_clipboard_status()
                .merge(self.replace_selection(text.as_str())),
        };
        StudioTransition::effect(effect)
    }

    fn complete_cut(&mut self) -> EventEffect {
        let Some(pending) = self.pending_cut.take() else {
            return self.record_clipboard_protocol_failure(
                "Cut completion did not match an active selection.",
            );
        };
        if self.buffer().revision().get() != pending.revision || self.selection != pending.selection
        {
            return self.record_clipboard_protocol_failure(
                "Cut was not applied because the document changed.",
            );
        }
        let cleared = self.clear_clipboard_status();
        let edited = self.replace_range(pending.selection.range(), "");
        if edited.document_changed {
            cleared.merge(edited)
        } else {
            cleared.merge(
                self.record_clipboard_protocol_failure("Cut could not be applied atomically."),
            )
        }
    }

    fn handle_close_request(&mut self) -> StudioTransition {
        if self.document.is_dirty()
            || self.tabs.inactive_documents().any(StudioDocument::is_dirty)
            || self.last_file_error.is_some()
        {
            let effect = self.set_local_status(LocalStatus::CloseBlocked);
            StudioTransition {
                effect,
                clipboard_write: None,
                cancel_close: true,
            }
        } else {
            StudioTransition::default()
        }
    }

    fn record_clipboard_error(&mut self, error: ClipboardError) -> EventEffect {
        let message: Arc<str> = format!("Clipboard failed: {error}").into();
        self.last_clipboard_error = Some(error);
        self.clipboard_failures = self.clipboard_failures.saturating_add(1);
        self.set_local_status(LocalStatus::Clipboard(message))
    }

    fn record_clipboard_protocol_failure(&mut self, message: &'static str) -> EventEffect {
        self.last_clipboard_error = None;
        self.clipboard_failures = self.clipboard_failures.saturating_add(1);
        self.set_local_status(LocalStatus::Clipboard(Arc::from(message)))
    }

    fn reject_clipboard_response(&mut self, operation: ClipboardOperation) -> EventEffect {
        if operation == ClipboardOperation::Cut {
            self.pending_cut = None;
        }
        self.record_clipboard_protocol_failure("Clipboard response was not admitted.")
    }

    fn resolve_clipboard_admission(
        &mut self,
        effect: EventEffect,
        operation: ClipboardOperation,
        admitted: bool,
    ) -> EventEffect {
        if admitted {
            effect
        } else {
            effect.merge(self.reject_clipboard_response(operation))
        }
    }

    fn resolve_close_admission(&mut self, requested: bool, admitted: bool) {
        if requested && !admitted {
            self.input_failures = self.input_failures.saturating_add(1);
        }
    }

    fn set_local_status(&mut self, status: LocalStatus) -> EventEffect {
        let changed = self.local_status.as_ref() != Some(&status);
        self.local_status = Some(status);
        changed.then(EventEffect::visual).unwrap_or_default()
    }

    fn clear_clipboard_status(&mut self) -> EventEffect {
        self.last_clipboard_error = None;
        if matches!(self.local_status, Some(LocalStatus::Clipboard(_))) {
            self.local_status = None;
            EventEffect::visual()
        } else {
            EventEffect::default()
        }
    }

    fn clear_close_status(&mut self) -> EventEffect {
        if self.local_status == Some(LocalStatus::CloseBlocked) {
            self.local_status = None;
            EventEffect::visual()
        } else {
            EventEffect::default()
        }
    }

    fn handle_key(&mut self, physical_key: u16, modifiers: Modifiers) -> EventEffect {
        let command = modifiers.contains(Modifiers::COMMAND);
        let shift = modifiers.contains(Modifiers::SHIFT);
        let option = modifiers.contains(Modifiers::OPTION);
        if command && shift && physical_key == KEY_P {
            return self.open_command_palette();
        }
        if self.command_palette.is_open() {
            return self.handle_command_palette_key(physical_key, command);
        }
        if command && shift && physical_key == KEY_F {
            return self.open_project_search();
        }
        if self.project_search.is_open() {
            return self.handle_project_search_key(physical_key, command);
        }
        if command && shift && physical_key == KEY_E {
            self.find.close();
            self.find_needs_search = false;
            self.quick_open.close();
            self.project_search.close();
            if self.workspace.is_none() {
                return self.record_file_tree_error(&FileTreeError::NoWorkspace);
            }
            if self.file_tree.is_visible() && self.file_tree.is_focused() {
                return self
                    .file_tree
                    .hide()
                    .then(EventEffect::visual)
                    .unwrap_or_default();
            }
            return match self.file_tree.activate(1) {
                Ok(changed) => changed.then(EventEffect::visual).unwrap_or_default(),
                Err(error) => self.record_file_tree_error(&error),
            };
        }
        if command && physical_key == KEY_P {
            if self.workspace.is_none() {
                return self.record_quick_open_error(&QuickOpenError::NoWorkspace);
            }
            self.find.close();
            self.find_needs_search = false;
            self.project_search.close();
            return match self.quick_open.open(1) {
                Ok(changed) => changed.then(EventEffect::visual).unwrap_or_default(),
                Err(error) => self.record_quick_open_error(&error),
            };
        }
        if self.quick_open.is_open() {
            return self.handle_quick_open_key(physical_key, command);
        }
        if command && physical_key == KEY_F {
            let changed = self.find.open(option);
            self.find_needs_search |= !self.find.query().is_empty();
            return changed.then(EventEffect::visual).unwrap_or_default();
        }
        if self.find.is_open() {
            return self.handle_find_key(physical_key, command, option, shift);
        }
        if self.file_tree.is_focused() {
            return self.handle_file_tree_key(physical_key, command);
        }
        if command && physical_key == KEY_A {
            return self.set_selection(Selection::new(
                ByteOffset::new(0),
                ByteOffset::new(self.buffer().snapshot().len_bytes()),
            ));
        }
        if command && physical_key == KEY_S {
            return self.save_document();
        }
        if command && physical_key == KEY_Z {
            return if shift { self.redo() } else { self.undo() };
        }
        if command && physical_key == KEY_W {
            return self.close_active_tab_or_record();
        }
        if command && physical_key == KEY_LEFT_BRACKET {
            return self.navigate_document_history(false);
        }
        if command && physical_key == KEY_RIGHT_BRACKET {
            return self.navigate_document_history(true);
        }
        match physical_key {
            KEY_DELETE_BACKWARD => self.delete_backward(),
            KEY_DELETE_FORWARD => self.delete_forward(),
            KEY_RETURN if !command => self.replace_selection("\n"),
            KEY_TAB if !command => self.replace_selection("\t"),
            KEY_ESCAPE => self.cancel_composition(),
            KEY_LEFT => self.move_horizontal(false, shift),
            KEY_RIGHT => self.move_horizontal(true, shift),
            KEY_UP => self.move_vertical(-1, shift),
            KEY_DOWN => self.move_vertical(1, shift),
            KEY_HOME => self.move_to_line_edge(false, shift),
            KEY_END => self.move_to_line_edge(true, shift),
            _ => EventEffect::default(),
        }
    }

    fn handle_ime(&mut self, event: &ImeEvent) -> EventEffect {
        if self.command_palette.is_open() {
            return self.handle_command_palette_ime(event);
        }
        if self.project_search.is_open() {
            return self.handle_project_search_ime(event);
        }
        if self.quick_open.is_open() {
            return self.handle_quick_open_ime(event);
        }
        if self.find.is_open() {
            return self.handle_find_ime(event);
        }
        if self.file_tree.is_focused() {
            return EventEffect::default();
        }
        match event {
            ImeEvent::Started => {
                self.composition = Some(Composition {
                    replacement: self.selection.range(),
                    text: Box::default(),
                    selected_start_utf16: 0,
                    selected_length_utf16: 0,
                });
                EventEffect::visual()
            }
            ImeEvent::Updated {
                text,
                selected_start_utf16,
                selected_length_utf16,
            } => {
                let selected_end = selected_start_utf16.checked_add(*selected_length_utf16);
                let units = u32::try_from(text.encode_utf16().count()).ok();
                if selected_end.is_none_or(|end| units.is_none_or(|units| end > units)) {
                    self.input_failures = self.input_failures.saturating_add(1);
                    return EventEffect::default();
                }
                let replacement = self
                    .composition
                    .as_ref()
                    .map_or_else(|| self.selection.range(), |value| value.replacement.clone());
                self.composition = Some(Composition {
                    replacement,
                    text: text.clone(),
                    selected_start_utf16: *selected_start_utf16,
                    selected_length_utf16: *selected_length_utf16,
                });
                EventEffect::visual()
            }
            ImeEvent::Committed(text) => {
                let replacement = self
                    .composition
                    .take()
                    .map_or_else(|| self.selection.range(), |value| value.replacement);
                self.replace_range(replacement, text)
            }
            ImeEvent::Cancelled => self.cancel_composition(),
        }
    }

    fn open_command_palette(&mut self) -> EventEffect {
        self.find.close();
        self.find_needs_search = false;
        self.quick_open.close();
        self.project_search.close();
        self.file_tree.unfocus();
        let context = self.command_context();
        match self.command_palette.open(context) {
            Ok(changed) => changed.then(EventEffect::visual).unwrap_or_default(),
            Err(error) => self.record_command_palette_error(&error),
        }
    }

    fn handle_command_palette_key(&mut self, physical_key: u16, command: bool) -> EventEffect {
        match physical_key {
            KEY_ESCAPE => self
                .command_palette
                .cancel()
                .then(EventEffect::visual)
                .unwrap_or_default(),
            KEY_DELETE_BACKWARD if !command => {
                let context = self.command_context();
                match self.command_palette.delete_backward(context) {
                    Ok(changed) => changed.then(EventEffect::visual).unwrap_or_default(),
                    Err(error) => self.record_command_palette_error(&error),
                }
            }
            KEY_UP if !command => self
                .command_palette
                .navigate(false)
                .then(EventEffect::visual)
                .unwrap_or_default(),
            KEY_DOWN if !command => self
                .command_palette
                .navigate(true)
                .then(EventEffect::visual)
                .unwrap_or_default(),
            KEY_RETURN if !command => {
                let context = self.command_context();
                match self.command_palette.execute_selected(context) {
                    Ok(command) => EventEffect::visual().merge(self.dispatch_command(command)),
                    Err(error) => self.record_command_palette_error(&error),
                }
            }
            _ => EventEffect::default(),
        }
    }

    fn handle_command_palette_ime(&mut self, event: &ImeEvent) -> EventEffect {
        let context = self.command_context();
        let result = match event {
            ImeEvent::Started => {
                return self
                    .command_palette
                    .begin_composition()
                    .then(EventEffect::visual)
                    .unwrap_or_default();
            }
            ImeEvent::Updated {
                text,
                selected_start_utf16,
                selected_length_utf16,
            } => self.command_palette.update_composition(
                text,
                *selected_start_utf16,
                *selected_length_utf16,
            ),
            ImeEvent::Committed(text) => self.command_palette.commit_text(text, context),
            ImeEvent::Cancelled => {
                return self
                    .command_palette
                    .cancel_composition()
                    .then(EventEffect::visual)
                    .unwrap_or_default();
            }
        };
        match result {
            Ok(changed) => changed.then(EventEffect::visual).unwrap_or_default(),
            Err(error) => self.record_command_palette_error(&error),
        }
    }

    fn open_project_search(&mut self) -> EventEffect {
        if self.workspace.is_none() {
            return self.record_project_search_error(&ProjectSearchError::NoWorkspace);
        }
        self.find.close();
        self.find_needs_search = false;
        self.quick_open.close();
        self.command_palette.cancel();
        self.file_tree.unfocus();
        match self.project_search.open(1) {
            Ok(changed) => changed.then(EventEffect::visual).unwrap_or_default(),
            Err(error) => self.record_project_search_error(&error),
        }
    }

    fn handle_project_search_key(&mut self, physical_key: u16, command: bool) -> EventEffect {
        match physical_key {
            KEY_ESCAPE => self
                .project_search
                .close()
                .then(EventEffect::visual)
                .unwrap_or_default(),
            KEY_DELETE_BACKWARD if !command => match self.project_search.delete_backward() {
                Ok(changed) => changed.then(EventEffect::visual).unwrap_or_default(),
                Err(error) => self.record_project_search_error(&error),
            },
            KEY_UP if !command => self
                .project_search
                .navigate(false, PROJECT_SEARCH_VISIBLE_ROWS)
                .then(EventEffect::visual)
                .unwrap_or_default(),
            KEY_DOWN if !command => self
                .project_search
                .navigate(true, PROJECT_SEARCH_VISIBLE_ROWS)
                .then(EventEffect::visual)
                .unwrap_or_default(),
            KEY_RETURN if !command => match self.open_project_search_selection() {
                Ok(effect) => effect,
                Err(error) => self.record_workspace_error(&error),
            },
            _ => EventEffect::default(),
        }
    }

    fn handle_project_search_ime(&mut self, event: &ImeEvent) -> EventEffect {
        let result = match event {
            ImeEvent::Started => {
                return self
                    .project_search
                    .begin_composition()
                    .then(EventEffect::visual)
                    .unwrap_or_default();
            }
            ImeEvent::Updated {
                text,
                selected_start_utf16,
                selected_length_utf16,
            } => self.project_search.update_composition(
                text,
                *selected_start_utf16,
                *selected_length_utf16,
            ),
            ImeEvent::Committed(text) => self.project_search.commit_text(text),
            ImeEvent::Cancelled => {
                return self
                    .project_search
                    .cancel_composition()
                    .then(EventEffect::visual)
                    .unwrap_or_default();
            }
        };
        match result {
            Ok(changed) => changed.then(EventEffect::visual).unwrap_or_default(),
            Err(error) => self.record_project_search_error(&error),
        }
    }

    fn command_context(&self) -> CommandContext {
        let editor_bounds = self.editor_region(self.last_viewport).ok();
        CommandContext {
            can_save: self.document.is_file() && self.document.is_dirty(),
            can_close_tab: self.tabs.len() > 1
                && !self.document.is_dirty()
                && self.last_file_error.is_none(),
            can_navigate_back: self.tabs.can_navigate_back(),
            can_navigate_forward: self.tabs.can_navigate_forward(),
            has_workspace: self.workspace.is_some(),
            can_split_right: editor_bounds
                .is_some_and(|bounds| self.panes.can_split(SplitAxis::Columns, bounds)),
            can_split_down: editor_bounds
                .is_some_and(|bounds| self.panes.can_split(SplitAxis::Rows, bounds)),
            can_close_pane: self.panes.len() > 1,
        }
    }

    fn dispatch_command(&mut self, command: StudioCommand) -> EventEffect {
        match command {
            StudioCommand::SaveFile => self.save_document(),
            StudioCommand::CloseTab => self.close_active_tab_or_record(),
            StudioCommand::NavigateBack => self.navigate_document_history(false),
            StudioCommand::NavigateForward => self.navigate_document_history(true),
            StudioCommand::OpenFind => {
                let changed = self.find.open(false);
                self.find_needs_search |= !self.find.query().is_empty();
                changed.then(EventEffect::visual).unwrap_or_default()
            }
            StudioCommand::OpenReplace => {
                let changed = self.find.open(true);
                self.find_needs_search |= !self.find.query().is_empty();
                changed.then(EventEffect::visual).unwrap_or_default()
            }
            StudioCommand::OpenQuickOpen if self.workspace.is_none() => {
                self.record_quick_open_error(&QuickOpenError::NoWorkspace)
            }
            StudioCommand::OpenQuickOpen => match self.quick_open.open(1) {
                Ok(changed) => changed.then(EventEffect::visual).unwrap_or_default(),
                Err(error) => self.record_quick_open_error(&error),
            },
            StudioCommand::OpenProjectSearch => self.open_project_search(),
            StudioCommand::ToggleFileTree if self.workspace.is_none() => {
                self.record_file_tree_error(&FileTreeError::NoWorkspace)
            }
            StudioCommand::ToggleFileTree
                if self.file_tree.is_visible() && self.file_tree.is_focused() =>
            {
                self.file_tree
                    .hide()
                    .then(EventEffect::visual)
                    .unwrap_or_default()
            }
            StudioCommand::ToggleFileTree => match self.file_tree.activate(1) {
                Ok(changed) => changed.then(EventEffect::visual).unwrap_or_default(),
                Err(error) => self.record_file_tree_error(&error),
            },
            StudioCommand::SplitRight => self.split_active_pane(SplitAxis::Columns),
            StudioCommand::SplitDown => self.split_active_pane(SplitAxis::Rows),
            StudioCommand::FocusNextPane => self.focus_next_pane(),
            StudioCommand::ClosePane => self.close_active_pane(),
        }
    }

    fn split_active_pane(&mut self, axis: SplitAxis) -> EventEffect {
        if let Err(error) = self.sync_active_pane_document() {
            return self.record_pane_error(&error);
        }
        let result = self
            .editor_region(self.last_viewport)
            .and_then(|bounds| self.panes.split(axis, self.scroll_y, bounds));
        match result {
            Ok(scroll_y) => {
                self.scroll_y = scroll_y.clamp(0.0, self.maximum_scroll());
                self.composition = None;
                self.pointer_selecting = false;
                EventEffect::visual()
            }
            Err(error) => self.record_pane_error(&error),
        }
    }

    fn focus_next_pane(&mut self) -> EventEffect {
        if let Err(error) = self.sync_active_pane_document() {
            return self.record_pane_error(&error);
        }
        match self.panes.focus_next(self.scroll_y) {
            Ok(Some(_)) => self.apply_focused_pane_document(),
            Ok(None) => EventEffect::default(),
            Err(error) => self.record_pane_error(&error),
        }
    }

    fn close_active_pane(&mut self) -> EventEffect {
        if let Err(error) = self.sync_active_pane_document() {
            return self.record_pane_error(&error);
        }
        match self.panes.close_active(self.scroll_y) {
            Ok(_) => self.apply_focused_pane_document(),
            Err(error) => self.record_pane_error(&error),
        }
    }

    fn sync_active_pane_document(&mut self) -> Result<(), PaneError> {
        let tab = self
            .tabs
            .active_id()
            .map_err(|_| PaneError::InconsistentState)?;
        self.panes
            .sync_active_document(tab, self.active_document_view())
    }

    fn apply_focused_pane_document(&mut self) -> EventEffect {
        let (target_tab, target_view) = match self.panes.active_document() {
            Ok(target) => target,
            Err(error) => return self.record_pane_error(&error),
        };
        let Ok(active_tab) = self.tabs.active_id() else {
            return self.record_pane_error(&PaneError::InconsistentState);
        };
        let mut effect = EventEffect::default();
        if target_tab != active_tab {
            let Some(index) = self.tabs.index_for_id(target_tab) else {
                return self.record_pane_error(&PaneError::InconsistentState);
            };
            effect = match self.activate_document_tab(index) {
                Ok(effect) => effect,
                Err(error) => return self.record_workspace_error(&error),
            };
        }
        self.selection = target_view.selection;
        self.scroll_y = target_view.scroll_y.clamp(0.0, self.maximum_scroll());
        self.composition = None;
        self.pointer_selecting = false;
        if let Err(error) = self.sync_active_pane_document() {
            return self.record_pane_error(&error);
        }
        effect.merge(EventEffect::visual())
    }

    fn record_pane_error(&mut self, error: &PaneError) -> EventEffect {
        self.workspace_failures = self.workspace_failures.saturating_add(1);
        self.set_local_status(LocalStatus::Workspace(Arc::from(error.to_string())))
    }

    fn record_command_palette_error(&mut self, error: &CommandPaletteError) -> EventEffect {
        self.input_failures = self.input_failures.saturating_add(1);
        self.set_local_status(LocalStatus::Command(
            format!("Command palette failed: {error}").into(),
        ))
        .merge(EventEffect::visual())
    }

    fn record_project_search_error(&mut self, error: &ProjectSearchError) -> EventEffect {
        self.workspace_failures = self.workspace_failures.saturating_add(1);
        let message: Arc<str> = format!("Project search failed: {error}").into();
        self.last_workspace_error = Some(Arc::clone(&message));
        self.set_local_status(LocalStatus::Workspace(message))
            .merge(EventEffect::visual())
    }

    fn handle_quick_open_key(&mut self, physical_key: u16, command: bool) -> EventEffect {
        match physical_key {
            KEY_ESCAPE => self
                .quick_open
                .close()
                .then(EventEffect::visual)
                .unwrap_or_default(),
            KEY_DELETE_BACKWARD if !command => match self.quick_open.delete_backward() {
                Ok(changed) => changed.then(EventEffect::visual).unwrap_or_default(),
                Err(error) => self.record_quick_open_error(&error),
            },
            KEY_UP if !command => self
                .quick_open
                .navigate(false, QUICK_OPEN_VISIBLE_ROWS)
                .then(EventEffect::visual)
                .unwrap_or_default(),
            KEY_DOWN if !command => self
                .quick_open
                .navigate(true, QUICK_OPEN_VISIBLE_ROWS)
                .then(EventEffect::visual)
                .unwrap_or_default(),
            KEY_RETURN if !command => match self.open_quick_open_selection() {
                Ok(effect) => effect,
                Err(error) => self.record_workspace_error(&error),
            },
            _ => EventEffect::default(),
        }
    }

    fn handle_file_tree_key(&mut self, physical_key: u16, command: bool) -> EventEffect {
        match physical_key {
            KEY_ESCAPE => self
                .file_tree
                .unfocus()
                .then(EventEffect::visual)
                .unwrap_or_default(),
            KEY_UP if !command => self
                .file_tree
                .navigate(false, self.visible_tree_rows())
                .then(EventEffect::visual)
                .unwrap_or_default(),
            KEY_DOWN if !command => self
                .file_tree
                .navigate(true, self.visible_tree_rows())
                .then(EventEffect::visual)
                .unwrap_or_default(),
            KEY_RETURN if !command => match self.file_tree.activate_selected() {
                Ok(action) => self.apply_file_tree_action(action),
                Err(error) => self.record_file_tree_error(&error),
            },
            _ => EventEffect::default(),
        }
    }

    fn handle_quick_open_ime(&mut self, event: &ImeEvent) -> EventEffect {
        let result = match event {
            ImeEvent::Started => {
                return self
                    .quick_open
                    .begin_composition()
                    .then(EventEffect::visual)
                    .unwrap_or_default();
            }
            ImeEvent::Updated {
                text,
                selected_start_utf16,
                selected_length_utf16,
            } => {
                let selected_end = selected_start_utf16.checked_add(*selected_length_utf16);
                let units = u32::try_from(text.encode_utf16().count()).ok();
                if selected_end.is_none_or(|end| units.is_none_or(|units| end > units)) {
                    self.input_failures = self.input_failures.saturating_add(1);
                    return EventEffect::default();
                }
                self.quick_open.update_composition(text)
            }
            ImeEvent::Committed(text) => self.quick_open.commit_text(text),
            ImeEvent::Cancelled => {
                return self
                    .quick_open
                    .cancel_composition()
                    .then(EventEffect::visual)
                    .unwrap_or_default();
            }
        };
        match result {
            Ok(changed) => changed.then(EventEffect::visual).unwrap_or_default(),
            Err(error) => self.record_quick_open_error(&error),
        }
    }

    fn handle_find_key(
        &mut self,
        physical_key: u16,
        command: bool,
        option: bool,
        shift: bool,
    ) -> EventEffect {
        match physical_key {
            KEY_ESCAPE => self
                .find
                .close()
                .then(EventEffect::visual)
                .unwrap_or_default(),
            KEY_TAB if !command => self
                .find
                .toggle_field()
                .then(EventEffect::visual)
                .unwrap_or_default(),
            KEY_DELETE_BACKWARD if !command => match self.find.delete_backward() {
                Ok(changed_query) => {
                    self.find_needs_search |= changed_query;
                    EventEffect::visual()
                }
                Err(error) => self.record_find_error(&error),
            },
            KEY_RETURN if command && option => self.replace_all_find_matches(),
            KEY_RETURN if command => self.replace_current_find_match(),
            KEY_RETURN => self.navigate_find(!shift),
            _ => EventEffect::default(),
        }
    }

    fn handle_find_ime(&mut self, event: &ImeEvent) -> EventEffect {
        let result = match event {
            ImeEvent::Started => {
                return self
                    .find
                    .begin_composition()
                    .then(EventEffect::visual)
                    .unwrap_or_default();
            }
            ImeEvent::Updated {
                text,
                selected_start_utf16,
                selected_length_utf16,
            } => {
                let selected_end = selected_start_utf16.checked_add(*selected_length_utf16);
                let units = u32::try_from(text.encode_utf16().count()).ok();
                if selected_end.is_none_or(|end| units.is_none_or(|units| end > units)) {
                    self.input_failures = self.input_failures.saturating_add(1);
                    return EventEffect::default();
                }
                self.find.update_composition(text)
            }
            ImeEvent::Committed(text) => self.find.commit_text(text),
            ImeEvent::Cancelled => {
                return self
                    .find
                    .cancel_composition()
                    .then(EventEffect::visual)
                    .unwrap_or_default();
            }
        };
        match result {
            Ok(changed_query) => {
                self.find_needs_search |= changed_query;
                EventEffect::visual()
            }
            Err(error) => self.record_find_error(&error),
        }
    }

    fn record_find_error(&mut self, error: &FindError) -> EventEffect {
        self.find.record_error(error);
        EventEffect::visual()
    }

    fn navigate_find(&mut self, forward: bool) -> EventEffect {
        let Some(navigation) = self.find.navigate(forward) else {
            return EventEffect::default();
        };
        self.apply_find_navigation(navigation)
    }

    fn apply_find_navigation(&mut self, navigation: FindNavigation) -> EventEffect {
        let Some(range) = self
            .find
            .result()
            .and_then(|result| result.range(navigation.index()))
        else {
            return EventEffect::default();
        };
        let _wrapped = navigation.wrapped();
        self.select_find_range(range)
    }

    fn select_find_range(&mut self, range: Range<usize>) -> EventEffect {
        self.selection = Selection::new(ByteOffset::new(range.start), ByteOffset::new(range.end));
        self.composition = None;
        let snapshot = self.buffer().snapshot();
        if let Ok(Some(line)) = Self::line_for_offset(&snapshot, range.start) {
            let top = usize_as_f32(line) * LINE_HEIGHT;
            let height = (self.last_viewport.height() - CONTENT_INSET * 2.0).max(LINE_HEIGHT);
            if matches!(top.total_cmp(&self.scroll_y), std::cmp::Ordering::Less) {
                self.scroll_y = top;
            } else if matches!(
                (top + LINE_HEIGHT).total_cmp(&(self.scroll_y + height)),
                std::cmp::Ordering::Greater
            ) {
                self.scroll_y = (top + LINE_HEIGHT - height).max(0.0);
            }
            self.clamp_scroll();
        }
        EventEffect::visual()
    }

    fn replace_current_find_match(&mut self) -> EventEffect {
        let buffer_revision = self.buffer().revision().get();
        let Some(range) = self
            .find
            .active_range(self.runtime_document_revision, buffer_revision)
        else {
            return self.record_find_error(&FindError::IncompleteResult);
        };
        let replacement = self.find.replacement().to_owned();
        self.replace_range(range, &replacement)
    }

    fn replace_all_find_matches(&mut self) -> EventEffect {
        let buffer_revision = self.buffer().revision().get();
        let Some(ranges) = self
            .find
            .all_ranges(self.runtime_document_revision, buffer_revision)
        else {
            return self.record_find_error(&FindError::IncompleteResult);
        };
        if ranges.is_empty() {
            return EventEffect::default();
        }
        let replacement = self.find.replacement();
        let mut transaction_bytes: usize = 0;
        for range in ranges {
            transaction_bytes =
                transaction_bytes.saturating_add(range.len().saturating_add(replacement.len()));
        }
        if transaction_bytes > MAX_REPLACEMENT_TRANSACTION_BYTES {
            return self.record_find_error(&FindError::ReplacementBudgetExceeded {
                actual: transaction_bytes,
                limit: MAX_REPLACEMENT_TRANSACTION_BYTES,
            });
        }
        let first_start = ranges[0].start;
        let Some(next_offset) = first_start.checked_add(replacement.len()) else {
            return self.record_find_error(&FindError::OffsetOverflow);
        };
        let mut transaction = Transaction::new(self.buffer().revision());
        for range in ranges {
            if transaction.replace(range.clone(), replacement).is_err() {
                return self.record_find_error(&FindError::OffsetOverflow);
            }
        }
        transaction.set_selections(SelectionSet::caret(ByteOffset::new(next_offset)));
        if self.buffer_mut().apply(transaction).is_ok() {
            self.selection = Selection::caret(ByteOffset::new(next_offset));
            self.composition = None;
            EventEffect::document()
        } else {
            self.input_failures = self.input_failures.saturating_add(1);
            EventEffect::default()
        }
    }

    fn prepare_find_request(&mut self) -> Result<Option<FindRequest>, FindError> {
        if !self.find_needs_search {
            return Ok(None);
        }
        self.find_needs_search = false;
        self.find
            .request(self.runtime_document_revision, self.buffer().snapshot())
    }

    fn prepare_quick_open_request(&mut self) -> Result<Option<QuickOpenRequest>, QuickOpenError> {
        let Some(workspace) = &self.workspace else {
            return if self.quick_open.is_open() {
                Err(QuickOpenError::NoWorkspace)
            } else {
                Ok(None)
            };
        };
        Ok(self.quick_open.take_request(workspace.root()))
    }

    fn prepare_project_search_request(
        &mut self,
    ) -> Result<Option<ProjectSearchRequest>, ProjectSearchError> {
        let Some(workspace) = &self.workspace else {
            return if self.project_search.is_open() {
                Err(ProjectSearchError::NoWorkspace)
            } else {
                Ok(None)
            };
        };
        self.project_search.take_request(workspace.root())
    }

    fn apply_project_search_output(&mut self, output: ProjectSearchWorkerOutput) -> EventEffect {
        match self.project_search.admit(output) {
            ProjectSearchAdmission::Inventory
            | ProjectSearchAdmission::Batch
            | ProjectSearchAdmission::Complete
            | ProjectSearchAdmission::Failed => EventEffect::visual(),
            ProjectSearchAdmission::Stale => EventEffect::default(),
        }
    }

    fn apply_quick_open_output(&mut self, output: QuickOpenWorkerOutput) -> EventEffect {
        match self.quick_open.admit(output) {
            QuickOpenAdmission::Inventory
            | QuickOpenAdmission::Query
            | QuickOpenAdmission::Failed => EventEffect::visual(),
            QuickOpenAdmission::Stale => EventEffect::default(),
        }
    }

    fn record_quick_open_error(&mut self, error: &QuickOpenError) -> EventEffect {
        self.workspace_failures = self.workspace_failures.saturating_add(1);
        let message: Arc<str> = format!("Quick open failed: {error}").into();
        self.last_workspace_error = Some(Arc::clone(&message));
        self.set_local_status(LocalStatus::Workspace(message))
    }

    fn update_find_after_document_change(&mut self) -> EventEffect {
        match self.find.document_changed() {
            Ok(needs_search) => {
                self.find_needs_search |= needs_search;
                EventEffect::default()
            }
            Err(error) => self.record_find_error(&error),
        }
    }

    fn apply_find_output(&mut self, output: FindWorkerOutput) -> EventEffect {
        let admission = self.find.admit(
            output,
            self.runtime_document_revision,
            self.buffer().revision().get(),
        );
        match admission {
            FindAdmission::Accepted => {
                let range = self.find.active_range(
                    self.runtime_document_revision,
                    self.buffer().revision().get(),
                );
                range.map_or_else(EventEffect::visual, |range| self.select_find_range(range))
            }
            FindAdmission::Failed => EventEffect::visual(),
            FindAdmission::Stale => EventEffect::default(),
        }
    }

    #[cfg(test)]
    fn complete_pending_find_for_test(&mut self) -> Result<EventEffect, FindError> {
        let Some(request) = self.prepare_find_request()? else {
            return Ok(EventEffect::default());
        };
        Ok(self.apply_find_output(request.execute()))
    }

    fn cancel_composition(&mut self) -> EventEffect {
        self.composition
            .take()
            .map(|_| EventEffect::visual())
            .unwrap_or_default()
    }

    fn handle_pointer(
        &mut self,
        action: PointerAction,
        position: Point,
        button: PointerButton,
        modifiers: Modifiers,
    ) -> EventEffect {
        self.last_pointer_position = Some(position);
        if self.command_palette.is_open() {
            self.pointer_selecting = false;
            return EventEffect::default();
        }
        if self.project_search.is_open() {
            self.pointer_selecting = false;
            return EventEffect::default();
        }
        if self.quick_open.is_open() {
            self.pointer_selecting = false;
            return EventEffect::default();
        }
        if action == PointerAction::Down
            && button == PointerButton::Primary
            && position.y() < TAB_BAR_HEIGHT
            && position.x() >= self.sidebar_width(self.last_viewport)
        {
            self.pointer_selecting = false;
            self.file_tree.unfocus();
            let tab_position = (position.x() - self.sidebar_width(self.last_viewport)
                + self.tab_scroll_x)
                / TAB_WIDTH;
            let Some(index) = floor_f32_to_usize(tab_position) else {
                return EventEffect::default();
            };
            return match self.activate_document_tab(index) {
                Ok(effect) => effect,
                Err(error) => self.record_workspace_error(&error),
            };
        }
        if action == PointerAction::Down
            && button == PointerButton::Primary
            && self.workspace.is_some()
            && self.file_tree.is_visible()
            && position.x() < self.sidebar_width(self.last_viewport)
        {
            self.pointer_selecting = false;
            if !self.file_tree.is_active() {
                return match self.file_tree.activate(1) {
                    Ok(_) => EventEffect::visual(),
                    Err(error) => self.record_file_tree_error(&error),
                };
            }
            let row_position =
                (position.y() - CONTENT_INSET + self.workspace_scroll_y) / TREE_ROW_HEIGHT;
            let Some(index) = floor_f32_to_usize(row_position) else {
                return EventEffect::default();
            };
            return match self.file_tree.activate_row(index) {
                Ok(action) => self.apply_file_tree_action(action),
                Err(error) => self.record_file_tree_error(&error),
            };
        }
        let pane_focus = self.focus_pane_for_pointer(action, button, position);
        if pane_focus.document_identity_advanced {
            return pane_focus;
        }
        let pointer_effect = match action {
            PointerAction::Down if button == PointerButton::Primary => {
                self.file_tree.unfocus();
                let Some(offset) = self.offset_at_point(position) else {
                    return EventEffect::default();
                };
                self.pointer_selecting = true;
                let selection = if modifiers.contains(Modifiers::SHIFT) {
                    Selection::new(self.selection.anchor(), offset)
                } else {
                    Selection::caret(offset)
                };
                self.set_selection(selection)
            }
            PointerAction::Moved if self.pointer_selecting => {
                let Some(offset) = self.offset_at_point(position) else {
                    return EventEffect::default();
                };
                self.set_selection(Selection::new(self.selection.anchor(), offset))
            }
            PointerAction::Up if button == PointerButton::Primary => {
                self.pointer_selecting = false;
                EventEffect::default()
            }
            PointerAction::Moved | PointerAction::Down | PointerAction::Up => {
                EventEffect::default()
            }
        };
        pane_focus.merge(pointer_effect)
    }

    fn focus_pane_for_pointer(
        &mut self,
        action: PointerAction,
        button: PointerButton,
        position: Point,
    ) -> EventEffect {
        if action != PointerAction::Down || button != PointerButton::Primary {
            return EventEffect::default();
        }
        if let Err(error) = self.sync_active_pane_document() {
            return self.record_pane_error(&error);
        }
        let focus = self
            .editor_region(self.last_viewport)
            .and_then(|bounds| self.panes.focus_at(position, bounds, self.scroll_y));
        match focus {
            Ok(Some(_)) => self.apply_focused_pane_document(),
            Ok(None) => EventEffect::default(),
            Err(error) => self.record_pane_error(&error),
        }
    }

    fn offset_at_point(&mut self, position: Point) -> Option<ByteOffset> {
        let bounds = self.active_pane_bounds().ok()?;
        let origin_x = bounds.origin().x();
        if position.x() < origin_x
            || position.x() >= origin_x + bounds.size().width()
            || position.y() >= bounds.origin().y() + bounds.size().height()
            || (self.panes.len() > 1 && position.y() < bounds.origin().y())
        {
            return None;
        }
        let line_position = (position.y() - bounds.origin().y() + self.scroll_y) / LINE_HEIGHT;
        if !line_position.is_finite() || line_position < 0.0 {
            return Some(ByteOffset::new(0));
        }
        let line = floor_f32_to_usize(line_position)?;
        let snapshot = self.buffer().snapshot();
        let line = line.min(snapshot.line_count().saturating_sub(1));
        let line_range = snapshot.line_byte_range(line).ok()?;
        let text = snapshot.slice(line_range.clone()).ok()?;
        let content = text.trim_end_matches(['\r', '\n']);
        let x = (position.x() - origin_x).max(0.0);
        let target_utf16 = self
            .rendered_lines
            .iter()
            .find(|rendered| rendered.line == line)
            .map_or(0, |rendered| utf16_at_x(&rendered.layout, x));
        let relative = byte_at_utf16(content, target_utf16).unwrap_or(content.len());
        Some(ByteOffset::new(line_range.start + relative))
    }

    fn set_selection(&mut self, selection: Selection) -> EventEffect {
        if selection == self.selection {
            EventEffect::default()
        } else {
            self.selection = selection;
            self.composition = None;
            EventEffect::visual()
        }
    }

    fn replace_selection(&mut self, text: &str) -> EventEffect {
        self.replace_range(self.selection.range(), text)
    }

    fn replace_range(&mut self, range: Range<usize>, text: &str) -> EventEffect {
        let Some(next_offset) = range.start.checked_add(text.len()) else {
            self.input_failures = self.input_failures.saturating_add(1);
            return EventEffect::default();
        };
        let next_selection = Selection::caret(ByteOffset::new(next_offset));
        let mut transaction = Transaction::new(self.buffer().revision());
        if transaction.replace(range, text).is_err() {
            self.input_failures = self.input_failures.saturating_add(1);
            return EventEffect::default();
        }
        transaction.set_selections(SelectionSet::caret(next_selection.head()));
        if self.buffer_mut().apply(transaction).is_ok() {
            self.selection = next_selection;
            self.composition = None;
            EventEffect::document()
        } else {
            self.input_failures = self.input_failures.saturating_add(1);
            EventEffect::default()
        }
    }

    fn delete_backward(&mut self) -> EventEffect {
        if !self.selection.range().is_empty() {
            return self.replace_selection("");
        }
        let snapshot = self.buffer().snapshot();
        let Ok(index) = snapshot.grapheme_index_of_byte(self.selection.head()) else {
            self.input_failures = self.input_failures.saturating_add(1);
            return EventEffect::default();
        };
        let start = index
            .checked_sub(1)
            .and_then(|previous| snapshot.byte_of_grapheme_index(previous).ok());
        let Some(start) = start else {
            return EventEffect::default();
        };
        self.replace_range(start.get()..self.selection.head().get(), "")
    }

    fn delete_forward(&mut self) -> EventEffect {
        if !self.selection.range().is_empty() {
            return self.replace_selection("");
        }
        let snapshot = self.buffer().snapshot();
        let Ok(index) = snapshot.grapheme_index_of_byte(self.selection.head()) else {
            self.input_failures = self.input_failures.saturating_add(1);
            return EventEffect::default();
        };
        let Ok(end) = snapshot.byte_of_grapheme_index(index.saturating_add(1)) else {
            return EventEffect::default();
        };
        self.replace_range(self.selection.head().get()..end.get(), "")
    }

    fn move_horizontal(&mut self, forward: bool, extend: bool) -> EventEffect {
        let range = self.selection.range();
        if !extend && !range.is_empty() {
            let offset = if forward { range.end } else { range.start };
            return self.set_selection(Selection::caret(ByteOffset::new(offset)));
        }
        let snapshot = self.buffer().snapshot();
        let Ok(index) = snapshot.grapheme_index_of_byte(self.selection.head()) else {
            self.input_failures = self.input_failures.saturating_add(1);
            return EventEffect::default();
        };
        let target_index = if forward {
            index.saturating_add(1)
        } else {
            index.saturating_sub(1)
        };
        let Ok(target) = snapshot.byte_of_grapheme_index(target_index) else {
            return EventEffect::default();
        };
        self.extend_or_collapse(target, extend)
    }

    fn move_vertical(&mut self, delta: isize, extend: bool) -> EventEffect {
        let snapshot = self.buffer().snapshot();
        let target = (|| {
            let line = Self::line_for_offset(&snapshot, self.selection.head().get()).ok()??;
            let line_range = snapshot.line_byte_range(line).ok()?;
            let column = self.selection.head().get().saturating_sub(line_range.start);
            let target_line = line
                .saturating_add_signed(delta)
                .min(snapshot.line_count() - 1);
            let target_range = snapshot.line_byte_range(target_line).ok()?;
            let text = snapshot.slice(target_range.clone()).ok()?;
            let content = text.trim_end_matches(['\r', '\n']);
            let mut target_column = column.min(content.len());
            while target_column > 0 && !content.is_char_boundary(target_column) {
                target_column -= 1;
            }
            Some(ByteOffset::new(target_range.start + target_column))
        })();
        let Some(target) = target else {
            return EventEffect::default();
        };
        self.extend_or_collapse(target, extend)
    }

    fn move_to_line_edge(&mut self, end: bool, extend: bool) -> EventEffect {
        let snapshot = self.buffer().snapshot();
        let offset = (|| {
            let line = Self::line_for_offset(&snapshot, self.selection.head().get()).ok()??;
            let range = snapshot.line_byte_range(line).ok()?;
            if end {
                let text = snapshot.slice(range.clone()).ok()?;
                Some(range.start + text.trim_end_matches(['\r', '\n']).len())
            } else {
                Some(range.start)
            }
        })();
        let Some(offset) = offset else {
            return EventEffect::default();
        };
        self.extend_or_collapse(ByteOffset::new(offset), extend)
    }

    fn extend_or_collapse(&mut self, target: ByteOffset, extend: bool) -> EventEffect {
        let selection = if extend {
            Selection::new(self.selection.anchor(), target)
        } else {
            Selection::caret(target)
        };
        self.set_selection(selection)
    }

    fn undo(&mut self) -> EventEffect {
        match self.buffer_mut().undo() {
            Ok(true) => {
                if let Some(selection) = self.buffer().selections().as_slice().first().copied() {
                    self.selection = selection;
                }
                self.composition = None;
                EventEffect::document()
            }
            result => {
                self.input_failures = self
                    .input_failures
                    .saturating_add(u64::from(result.is_err()));
                EventEffect::default()
            }
        }
    }

    fn redo(&mut self) -> EventEffect {
        match self.buffer_mut().redo() {
            Ok(true) => {
                if let Some(selection) = self.buffer().selections().as_slice().first().copied() {
                    self.selection = selection;
                }
                self.composition = None;
                EventEffect::document()
            }
            result => {
                self.input_failures = self
                    .input_failures
                    .saturating_add(u64::from(result.is_err()));
                EventEffect::default()
            }
        }
    }

    fn line_for_offset(
        snapshot: &BufferSnapshot,
        offset: usize,
    ) -> Result<Option<usize>, TextError> {
        if offset <= snapshot.len_bytes() {
            let mut low = 0;
            let mut high = snapshot.line_count();
            while low < high {
                let middle = low + (high - low) / 2;
                let range = snapshot.line_byte_range(middle)?;
                if offset < range.start {
                    high = middle;
                } else if offset >= range.end && middle + 1 < snapshot.line_count() {
                    low = middle + 1;
                } else {
                    return Ok(Some(middle));
                }
            }
        }
        Ok(None)
    }

    fn maximum_scroll(&self) -> f32 {
        let content_height = self
            .active_pane_bounds()
            .map_or(1.0, |bounds| bounds.size().height().max(1.0));
        (usize_as_f32(self.buffer().snapshot().line_count()) * LINE_HEIGHT - content_height)
            .max(0.0)
    }

    fn editor_region(&self, viewport: Size) -> Result<Rect, PaneError> {
        let sidebar_width = self.sidebar_width(viewport);
        let origin = Point::new(sidebar_width + CONTENT_INSET, CONTENT_INSET)
            .ok_or(PaneError::InvalidGeometry)?;
        let size = Size::new(
            (viewport.width() - sidebar_width - CONTENT_INSET * 2.0).max(1.0),
            (viewport.height() - CONTENT_INSET * 2.0).max(1.0),
        )
        .ok_or(PaneError::InvalidGeometry)?;
        Ok(Rect::new(origin, size))
    }

    fn active_pane_bounds(&self) -> Result<Rect, PaneError> {
        self.panes
            .layout(self.editor_region(self.last_viewport)?)?
            .active()
            .map(|pane| pane.bounds)
            .ok_or(PaneError::InconsistentState)
    }

    fn sidebar_width(&self, viewport: Size) -> f32 {
        if self.workspace.is_some() && self.file_tree.is_visible() {
            SIDEBAR_WIDTH.min((viewport.width() - 1.0).max(0.0))
        } else {
            0.0
        }
    }

    fn maximum_workspace_scroll(&self) -> f32 {
        let rows = self.file_tree.total_rows();
        let content_height = (self.last_viewport.height() - CONTENT_INSET).max(1.0);
        (usize_as_f32(rows) * TREE_ROW_HEIGHT - content_height).max(0.0)
    }

    fn visible_tree_rows(&self) -> usize {
        floor_f32_to_usize(self.last_viewport.height() / TREE_ROW_HEIGHT)
            .unwrap_or(1)
            .max(1)
    }

    fn apply_file_tree_action(&mut self, action: FileTreeAction) -> EventEffect {
        match action {
            FileTreeAction::Changed => EventEffect::visual(),
            FileTreeAction::Open(relative) => {
                let path = self
                    .workspace
                    .as_ref()
                    .ok_or(WorkspaceSelectionError::NoWorkspace)
                    .and_then(|workspace| {
                        workspace
                            .path_for_relative_file(Path::new(relative.as_ref()))
                            .map_err(WorkspaceSelectionError::Workspace)
                    });
                match path.and_then(|path| self.open_workspace_path(&path, None)) {
                    Ok(effect) => {
                        self.file_tree.unfocus();
                        effect.merge(EventEffect::visual())
                    }
                    Err(error) => self.record_workspace_error(&error),
                }
            }
        }
    }

    fn prepare_file_tree_request(&mut self) -> Result<Option<FileTreeRequest>, FileTreeError> {
        let Some(workspace) = &self.workspace else {
            return if self.file_tree.is_active() {
                Err(FileTreeError::NoWorkspace)
            } else {
                Ok(None)
            };
        };
        Ok(self.file_tree.take_request(workspace.root()))
    }

    fn apply_file_tree_output(&mut self, output: FileTreeWorkerOutput) -> EventEffect {
        match self.file_tree.admit(output) {
            FileTreeAdmission::Directory => EventEffect::visual(),
            FileTreeAdmission::Failed => {
                let message = self
                    .file_tree
                    .error_message()
                    .unwrap_or_else(|| Arc::from("File tree failed."));
                self.workspace_failures = self.workspace_failures.saturating_add(1);
                self.last_workspace_error = Some(Arc::clone(&message));
                self.set_local_status(LocalStatus::Workspace(message))
                    .merge(EventEffect::visual())
            }
            FileTreeAdmission::Stale => EventEffect::default(),
        }
    }

    fn record_file_tree_error(&mut self, error: &FileTreeError) -> EventEffect {
        self.workspace_failures = self.workspace_failures.saturating_add(1);
        let message: Arc<str> = format!("File tree failed: {error}").into();
        self.last_workspace_error = Some(Arc::clone(&message));
        self.set_local_status(LocalStatus::Workspace(message))
    }

    #[cfg(test)]
    fn open_workspace_entry(
        &mut self,
        index: usize,
    ) -> Result<EventEffect, WorkspaceSelectionError> {
        let path = self
            .workspace
            .as_ref()
            .ok_or(WorkspaceSelectionError::NoWorkspace)?
            .path_for_file(index)
            .map_err(WorkspaceSelectionError::Workspace)?;
        if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
            let _ = self.file_tree.select_path(name);
        }
        self.open_workspace_path(&path, Some(index))
    }

    fn open_quick_open_selection(&mut self) -> Result<EventEffect, WorkspaceSelectionError> {
        let relative = self
            .quick_open
            .selected_path()
            .map_err(WorkspaceSelectionError::QuickOpen)?;
        let path = self
            .workspace
            .as_ref()
            .ok_or(WorkspaceSelectionError::NoWorkspace)?
            .path_for_relative_file(Path::new(relative.as_ref()))
            .map_err(WorkspaceSelectionError::Workspace)?;
        let effect = self.open_workspace_path(&path, None)?;
        self.quick_open.close();
        Ok(effect.merge(EventEffect::visual()))
    }

    fn open_project_search_selection(&mut self) -> Result<EventEffect, WorkspaceSelectionError> {
        let selected = self
            .project_search
            .selected_match()
            .map_err(WorkspaceSelectionError::ProjectSearch)?;
        let path = self
            .workspace
            .as_ref()
            .ok_or(WorkspaceSelectionError::NoWorkspace)?
            .path_for_relative_file(Path::new(selected.relative.as_ref()))
            .map_err(WorkspaceSelectionError::Workspace)?;
        let effect = if let Some(tab) = self.tabs.index_for_path(&path) {
            self.ensure_document_tab_loaded(tab)?;
            let document =
                if tab == self.tabs.active_index() {
                    &self.document
                } else {
                    self.tabs.inactive_document_for_path(&path).ok_or(
                        WorkspaceSelectionError::Tabs(DocumentTabError::InvalidPayloadState),
                    )?
                };
            project_search::verify_snapshot_match(&document.buffer().snapshot(), &selected)
                .map_err(WorkspaceSelectionError::ProjectSearch)?;
            self.activate_document_tab(tab)?
        } else {
            let document = StudioDocument::open(&path).map_err(WorkspaceSelectionError::File)?;
            project_search::verify_snapshot_match(&document.buffer().snapshot(), &selected)
                .map_err(WorkspaceSelectionError::ProjectSearch)?;
            self.insert_project_search_document(&path, document)?
        };
        self.select_project_search_match(&selected);
        self.project_search.close();
        Ok(effect.merge(EventEffect::visual()))
    }

    fn insert_project_search_document(
        &mut self,
        path: &Path,
        document: StudioDocument,
    ) -> Result<EventEffect, WorkspaceSelectionError> {
        let next_revision = self
            .runtime_document_revision
            .checked_add(1)
            .ok_or(WorkspaceSelectionError::RevisionExhausted)?;
        let view = self.active_document_view();
        self.tabs
            .insert_and_activate(path, None, document, &mut self.document, view)
            .map_err(WorkspaceSelectionError::Tabs)?;
        self.runtime_document_revision = next_revision;
        self.active_workspace_entry = self.tabs.active_workspace_entry();
        self.apply_document_view(DocumentViewState::default());
        Ok(EventEffect::document_replacement())
    }

    fn select_project_search_match(&mut self, selected: &SelectedProjectMatch) {
        self.selection = Selection::new(
            ByteOffset::new(selected.start),
            ByteOffset::new(selected.end),
        );
        self.composition = None;
        self.scroll_y = (u32_as_f32(selected.line.saturating_sub(1)) * LINE_HEIGHT)
            .clamp(0.0, self.maximum_scroll());
    }

    fn open_workspace_path(
        &mut self,
        path: &Path,
        workspace_entry: Option<usize>,
    ) -> Result<EventEffect, WorkspaceSelectionError> {
        if let Some(tab) = self.tabs.index_for_path(path) {
            return self.activate_document_tab(tab);
        }
        let document = StudioDocument::open(path).map_err(WorkspaceSelectionError::File)?;
        let next_revision = self
            .runtime_document_revision
            .checked_add(1)
            .ok_or(WorkspaceSelectionError::RevisionExhausted)?;
        let view = self.active_document_view();
        self.tabs
            .insert_and_activate(path, workspace_entry, document, &mut self.document, view)
            .map_err(WorkspaceSelectionError::Tabs)?;
        self.runtime_document_revision = next_revision;
        self.active_workspace_entry = self.tabs.active_workspace_entry();
        self.apply_document_view(DocumentViewState::default());
        Ok(EventEffect::document_replacement())
    }

    fn active_document_view(&self) -> DocumentViewState {
        DocumentViewState {
            selection: self.selection,
            scroll_y: self.scroll_y,
        }
    }

    fn apply_document_view(&mut self, view: DocumentViewState) {
        self.selection = view.selection;
        self.scroll_y = view.scroll_y;
        self.composition = None;
        self.pointer_selecting = false;
        self.rendered_lines.clear();
        self.pending_cut = None;
        self.last_save = None;
        self.last_file_error = None;
        self.last_workspace_error = None;
        self.local_status = None;
        self.find.close();
        self.find_needs_search = false;
        self.quick_open.close();
        self.project_search.close();
        self.ensure_active_tab_visible();
    }

    fn activate_document_tab(
        &mut self,
        index: usize,
    ) -> Result<EventEffect, WorkspaceSelectionError> {
        if index == self.tabs.active_index() {
            return Ok(EventEffect::default());
        }
        self.ensure_document_tab_loaded(index)?;
        let next_revision = self
            .runtime_document_revision
            .checked_add(1)
            .ok_or(WorkspaceSelectionError::RevisionExhausted)?;
        let current_view = self.active_document_view();
        let view = self
            .tabs
            .activate(index, &mut self.document, current_view)
            .map_err(WorkspaceSelectionError::Tabs)?
            .ok_or(WorkspaceSelectionError::Tabs(
                DocumentTabError::InvalidPayloadState,
            ))?;
        self.runtime_document_revision = next_revision;
        self.active_workspace_entry = self.tabs.active_workspace_entry();
        let view = self.clamp_document_view(view);
        self.apply_document_view(view);
        Ok(EventEffect::document_replacement())
    }

    fn navigate_document_history(&mut self, forward: bool) -> EventEffect {
        let Some(target) = self.tabs.navigation_target(forward) else {
            return EventEffect::default();
        };
        if let Err(error) = self.ensure_document_tab_loaded(target) {
            return self.record_workspace_error(&error);
        }
        let Some(next_revision) = self.runtime_document_revision.checked_add(1) else {
            return self.record_workspace_error(&WorkspaceSelectionError::RevisionExhausted);
        };
        let current_view = self.active_document_view();
        #[cfg(test)]
        if self.force_empty_navigation_result.take().is_some() {
            self.tabs.clear_history_for_test();
        }
        let result = if forward {
            self.tabs.navigate_forward(&mut self.document, current_view)
        } else {
            self.tabs.navigate_back(&mut self.document, current_view)
        };
        match result {
            Ok(Some(view)) => {
                self.runtime_document_revision = next_revision;
                self.active_workspace_entry = self.tabs.active_workspace_entry();
                self.apply_document_view(view);
                EventEffect::document_replacement()
            }
            Ok(None) => EventEffect::default(),
            Err(error) => self.record_workspace_error(&WorkspaceSelectionError::Tabs(error)),
        }
    }

    fn close_active_tab_or_record(&mut self) -> EventEffect {
        match self.close_active_tab() {
            Ok(effect) => effect,
            Err(error) => self.record_workspace_error(&error),
        }
    }

    fn close_active_tab(&mut self) -> Result<EventEffect, WorkspaceSelectionError> {
        if self.document.is_dirty() || self.last_file_error.is_some() {
            return Err(WorkspaceSelectionError::DirtyDocument);
        }
        let target = self
            .tabs
            .close_target()
            .map_err(WorkspaceSelectionError::Tabs)?;
        self.ensure_document_tab_loaded(target)?;
        let next_revision = self
            .runtime_document_revision
            .checked_add(1)
            .ok_or(WorkspaceSelectionError::RevisionExhausted)?;
        let closed = self
            .tabs
            .active_id()
            .map_err(WorkspaceSelectionError::Tabs)?;
        let view = self
            .tabs
            .close_active(&mut self.document)
            .map_err(WorkspaceSelectionError::Tabs)?;
        self.runtime_document_revision = next_revision;
        self.active_workspace_entry = self.tabs.active_workspace_entry();
        let replacement = self
            .tabs
            .active_id()
            .map_err(WorkspaceSelectionError::Tabs)?;
        self.panes.retarget_closed_tab(closed, replacement, view);
        self.apply_document_view(view);
        Ok(EventEffect::document_replacement())
    }

    fn ensure_active_tab_visible(&mut self) {
        let available =
            (self.last_viewport.width() - self.sidebar_width(self.last_viewport)).max(TAB_WIDTH);
        let visible = floor_f32_to_usize(available / TAB_WIDTH)
            .unwrap_or(1)
            .max(1);
        let active = self.tabs.active_index();
        let first = floor_f32_to_usize(self.tab_scroll_x / TAB_WIDTH).unwrap_or(0);
        if active < first {
            self.tab_scroll_x = usize_as_f32(active) * TAB_WIDTH;
        } else if active >= first.saturating_add(visible) {
            self.tab_scroll_x =
                usize_as_f32(active.saturating_add(1).saturating_sub(visible)) * TAB_WIDTH;
        }
        let maximum = (usize_as_f32(self.tabs.len()) * TAB_WIDTH - available).max(0.0);
        self.tab_scroll_x = self.tab_scroll_x.clamp(0.0, maximum);
    }

    fn record_workspace_error(&mut self, error: &WorkspaceSelectionError) -> EventEffect {
        self.workspace_failures = self.workspace_failures.saturating_add(1);
        let message: Arc<str> = Arc::from(error.to_string());
        self.last_workspace_error = Some(Arc::clone(&message));
        self.set_local_status(LocalStatus::Workspace(message))
    }

    fn save_document(&mut self) -> EventEffect {
        match self.document.save() {
            Ok(Some(report)) => {
                self.last_save = Some(report);
                self.last_file_error = None;
                self.clear_close_status()
            }
            Ok(None) => EventEffect::default(),
            Err(error) => {
                self.save_failures = self.save_failures.saturating_add(1);
                self.last_file_error = Some(error);
                EventEffect::default()
            }
        }
    }

    fn clamp_scroll(&mut self) {
        self.scroll_y = self.scroll_y.clamp(0.0, self.maximum_scroll());
    }

    fn advance_runtime_document_identity(&mut self, identity_already_advanced: bool) {
        if identity_already_advanced {
            return;
        }
        let current = self.runtime_document_revision;
        let next = current.saturating_add(1);
        self.runtime_document_revision = next;
        self.input_failures = self
            .input_failures
            .saturating_add(u64::from(next == current));
    }
}

enum StudioWorkerOutput {
    Find(FindWorkerOutput),
    QuickOpen(QuickOpenWorkerOutput),
    ProjectSearch(ProjectSearchWorkerOutput),
    FileTree(FileTreeWorkerOutput),
}

impl AppDelegate for StudioApp {
    type WorkerOutput = StudioWorkerOutput;

    fn event(&mut self, event: &SurfaceEvent, context: &mut AppContext<'_, StudioWorkerOutput>) {
        let StudioTransition {
            mut effect,
            clipboard_write,
            cancel_close,
        } = self.handle_event_with_response(event);
        if let Some(write) = clipboard_write {
            let operation = write.operation();
            let admitted = context.write_clipboard(write);
            effect = self.resolve_clipboard_admission(effect, operation, admitted);
        }
        if cancel_close {
            let admitted = context.cancel_close();
            self.resolve_close_admission(true, admitted);
        }
        if effect.document_changed {
            if effect.document_identity_advanced {
                self.find.close();
                self.find_needs_search = false;
            } else {
                effect = effect.merge(self.update_find_after_document_change());
            }
            self.advance_runtime_document_identity(effect.document_identity_advanced);
            let revision = DocumentRevision::new(self.runtime_document_revision);
            let rejected = !context.advance_document(revision);
            self.input_failures = self.input_failures.saturating_add(u64::from(rejected));
        }
        if effect.visual_changed {
            context.invalidate();
        }
        match self.prepare_find_request() {
            Ok(Some(request)) => {
                let identity = request.identity();
                if context
                    .spawn(move || StudioWorkerOutput::Find(request.execute()))
                    .is_err()
                    && self.find.reject_submission(identity)
                {
                    context.invalidate();
                }
            }
            Ok(None) => {}
            Err(error) => {
                self.find.record_error(&error);
                context.invalidate();
            }
        }
        self.submit_quick_open_request(context);
        self.submit_project_search_request(context);
        self.submit_file_tree_request(context);
        self.publish_recovery();
    }

    fn worker_result(
        &mut self,
        _token: alpine_runtime::WorkToken,
        result: StudioWorkerOutput,
        context: &mut AppContext<'_, StudioWorkerOutput>,
    ) {
        let effect = match result {
            StudioWorkerOutput::Find(result) => self.apply_find_output(result),
            StudioWorkerOutput::QuickOpen(result) => self.apply_quick_open_output(result),
            StudioWorkerOutput::ProjectSearch(result) => self.apply_project_search_output(result),
            StudioWorkerOutput::FileTree(result) => self.apply_file_tree_output(result),
        };
        if effect.visual_changed {
            context.invalidate();
        }
        self.submit_quick_open_request(context);
        self.submit_project_search_request(context);
        self.submit_file_tree_request(context);
        self.publish_recovery();
    }

    fn frame(&mut self, context: WindowContext) -> Scene {
        self.scene(context.scene_revision(), context.viewport())
    }
}

impl StudioApp {
    fn submit_file_tree_request(&mut self, context: &mut AppContext<'_, StudioWorkerOutput>) {
        match self.prepare_file_tree_request() {
            Ok(Some(request)) => {
                let identity = request.identity();
                let failed = force_file_tree_submission_failure!(self)
                    || context
                        .spawn(move || StudioWorkerOutput::FileTree(request.execute()))
                        .is_err();
                if failed && self.file_tree.reject_submission(identity) {
                    context.invalidate();
                }
            }
            Ok(None) => {}
            Err(error) => {
                self.record_file_tree_error(&error);
                context.invalidate();
            }
        }
    }

    fn submit_quick_open_request(&mut self, context: &mut AppContext<'_, StudioWorkerOutput>) {
        match self.prepare_quick_open_request() {
            Ok(Some(request)) => {
                let identity = request.identity();
                let force_submission_failure = force_quick_open_submission_failure!(self);
                let submission_failed = force_submission_failure
                    || context
                        .spawn(move || StudioWorkerOutput::QuickOpen(request.execute()))
                        .is_err();
                if self.reject_failed_quick_open_submission(identity, submission_failed) {
                    context.invalidate();
                }
            }
            Ok(None) => {}
            Err(error) => {
                self.record_quick_open_error(&error);
                context.invalidate();
            }
        }
    }

    fn submit_project_search_request(&mut self, context: &mut AppContext<'_, StudioWorkerOutput>) {
        match self.prepare_project_search_request() {
            Ok(Some(request)) => {
                let identity = request.identity();
                let failed = force_project_search_submission_failure!(self)
                    || context
                        .spawn(move || StudioWorkerOutput::ProjectSearch(request.execute()))
                        .is_err();
                if self.reject_failed_project_search_submission(identity, failed) {
                    context.invalidate();
                }
            }
            Ok(None) => {}
            Err(error) => {
                self.record_project_search_error(&error);
                context.invalidate();
            }
        }
    }

    fn reject_failed_quick_open_submission(
        &mut self,
        identity: quick_open::RequestIdentity,
        submission_failed: bool,
    ) -> bool {
        submission_failed && self.quick_open.reject_submission(identity)
    }

    fn reject_failed_project_search_submission(
        &mut self,
        identity: project_search::RequestIdentity,
        submission_failed: bool,
    ) -> bool {
        submission_failed && self.project_search.reject_submission(identity)
    }
}

fn studio_clipboard_shortcut(event: &SurfaceEvent) -> Option<ClipboardOperation> {
    let SurfaceEvent::Keyboard {
        state: KeyState::Down,
        logical_key,
        modifiers,
        repeat: false,
        ..
    } = event
    else {
        return None;
    };
    if !modifiers.contains(Modifiers::COMMAND)
        || modifiers.contains(Modifiers::CONTROL)
        || modifiers.contains(Modifiers::OPTION)
        || modifiers.contains(Modifiers::SHIFT)
    {
        return None;
    }
    if logical_key.eq_ignore_ascii_case("c") {
        Some(ClipboardOperation::Copy)
    } else if logical_key.eq_ignore_ascii_case("x") {
        Some(ClipboardOperation::Cut)
    } else if logical_key.eq_ignore_ascii_case("v") {
        Some(ClipboardOperation::Paste)
    } else {
        None
    }
}

fn local_utf16(
    snapshot: &BufferSnapshot,
    line_start: usize,
    offset: usize,
) -> Result<u32, StudioRenderError> {
    let prefix = snapshot.slice(line_start..offset)?;
    u32::try_from(prefix.encode_utf16().count()).map_err(|_| StudioRenderError::Domain)
}

fn x_for_utf16(layout: &LineLayout, target: u32) -> f32 {
    for glyph in layout.glyphs() {
        if glyph.source_utf16() >= target {
            return glyph.x();
        }
    }
    layout.width()
}

fn utf16_at_x(layout: &LineLayout, target: f32) -> u32 {
    let mut result = 0;
    for glyph in layout.glyphs() {
        if target < glyph.x() + glyph.advance() * 0.5 {
            return glyph.source_utf16();
        }
        result = glyph.source_utf16().saturating_add(1);
    }
    result
}

fn byte_at_utf16(text: &str, target: u32) -> Option<usize> {
    let mut utf16 = 0_u32;
    for (byte, character) in text.char_indices() {
        if utf16 == target {
            return Some(byte);
        }
        utf16 = utf16.checked_add(u32::try_from(character.len_utf16()).ok()?)?;
        if utf16 > target {
            return None;
        }
    }
    (utf16 == target).then_some(text.len())
}

#[allow(
    clippy::cast_precision_loss,
    reason = "visible line geometry is already bounded to f32 coordinates"
)]
fn usize_as_f32(value: usize) -> f32 {
    value as f32
}

#[allow(
    clippy::cast_precision_loss,
    reason = "atlas dimensions are bounded far below f32's exact integer range"
)]
fn u32_as_f32(value: u32) -> f32 {
    value as f32
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the caller validates a finite non-negative viewport coordinate"
)]
fn floor_f32_to_usize(value: f32) -> Option<usize> {
    (value <= 16_777_216.0).then_some(value.floor() as usize)
}

/// Non-shipping native Alpine Studio process qualification.
#[cfg(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64"))]
#[doc(hidden)]
pub mod native_validation {
    use std::{
        cell::RefCell,
        fmt,
        fmt::Write as _,
        fs,
        path::Path,
        rc::Rc,
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    use alpine_platform_macos::{
        ClipboardError, ClipboardOperation, EventTimestamp, ImeEvent, KeyState, Modifiers,
        NativeSurface, PointerAction, PointerButton, SurfaceDescriptor, SurfaceEvent,
        SurfaceLifecycle, SurfaceResponse, native_validation as platform_validation,
    };
    use alpine_runtime::{Application, WorkerConfig};
    use alpine_text::{ByteOffset, Selection};

    use super::{
        CONTENT_INSET, DEFAULT_SCALE, FONT_FAMILY, KEY_A, KEY_DOWN, KEY_E, KEY_F, KEY_P,
        KEY_RETURN, KEY_S, KEY_UP, StudioApp, StudioError, TREE_ROW_HEIGHT, WINDOW_HEIGHT,
        WINDOW_WIDTH, Workspace, native_file_app,
    };

    const NATIVE_INPUT_FRAMES: usize = 5;
    const TREE_TOGGLE_MODIFIER_BITS: u8 = 0x09;
    const COMMAND_SHIFT_MODIFIERS: Modifiers = Modifiers::from_bits(TREE_TOGGLE_MODIFIER_BITS);

    /// Handle-free completion evidence returned across the process-test boundary.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct NativeProcessEvidence {
        input_events: usize,
        input_frames: usize,
        persisted_bytes: usize,
        released_owner_classes: usize,
    }

    impl NativeProcessEvidence {
        /// Returns the native input events consumed by Studio.
        #[must_use]
        pub const fn input_events(self) -> usize {
            self.input_events
        }

        /// Returns immutable frames admitted by the native input sequence.
        #[must_use]
        pub const fn input_frames(self) -> usize {
            self.input_frames
        }

        /// Returns the exact persisted UTF-8 document length.
        #[must_use]
        pub const fn persisted_bytes(self) -> usize {
            self.persisted_bytes
        }

        /// Returns native owner classes observed at zero after drain.
        #[must_use]
        pub const fn released_owner_classes(self) -> usize {
            self.released_owner_classes
        }
    }

    /// Handle-free evidence from the native lazy file-tree journey.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct NativeFileTreeEvidence {
        keyboard_events: usize,
        pointer_events: usize,
        worker_wakes: usize,
        admitted_frames: usize,
        persisted_bytes: usize,
        released_owner_classes: usize,
    }

    impl NativeFileTreeEvidence {
        /// Returns keyboard events dispatched through the AppKit callback.
        #[must_use]
        pub const fn keyboard_events(self) -> usize {
            self.keyboard_events
        }

        /// Returns pointer events dispatched through the AppKit callback.
        #[must_use]
        pub const fn pointer_events(self) -> usize {
            self.pointer_events
        }

        /// Returns wake events required before the directory result published.
        #[must_use]
        pub const fn worker_wakes(self) -> usize {
            self.worker_wakes
        }

        /// Returns frames admitted by tree activation, navigation, and opening.
        #[must_use]
        pub const fn admitted_frames(self) -> usize {
            self.admitted_frames
        }

        /// Returns the exact saved UTF-8 length of the pointer-opened file.
        #[must_use]
        pub const fn persisted_bytes(self) -> usize {
            self.persisted_bytes
        }

        /// Returns native owner classes observed at zero after drain.
        #[must_use]
        pub const fn released_owner_classes(self) -> usize {
            self.released_owner_classes
        }
    }

    /// Handle-free evidence from the native streaming project-search journey.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct NativeProjectSearchEvidence {
        keyboard_events: usize,
        ime_events: usize,
        worker_wakes: usize,
        admitted_frames: usize,
        matched_bytes: usize,
        released_owner_classes: usize,
    }

    impl NativeProjectSearchEvidence {
        /// Returns keyboard events dispatched through the AppKit callback.
        #[must_use]
        pub const fn keyboard_events(self) -> usize {
            self.keyboard_events
        }

        /// Returns IME events dispatched through the AppKit callback.
        #[must_use]
        pub const fn ime_events(self) -> usize {
            self.ime_events
        }

        /// Returns wake events used to publish and drain bounded worker work.
        #[must_use]
        pub const fn worker_wakes(self) -> usize {
            self.worker_wakes
        }

        /// Returns immutable frames admitted across the native journey.
        #[must_use]
        pub const fn admitted_frames(self) -> usize {
            self.admitted_frames
        }

        /// Returns the exact UTF-8 byte length replaced at the selected match.
        #[must_use]
        pub const fn matched_bytes(self) -> usize {
            self.matched_bytes
        }

        /// Returns native owner classes observed at zero after drain.
        #[must_use]
        pub const fn released_owner_classes(self) -> usize {
            self.released_owner_classes
        }
    }

    #[derive(Default)]
    struct NativeFileTreeJourney {
        keyboard: usize,
        ime: usize,
        pointer: usize,
        wakes: usize,
        frames: usize,
        maximum_glyphs: usize,
    }

    impl NativeFileTreeJourney {
        fn observe(&mut self, event: &SurfaceEvent, response: &SurfaceResponse) {
            match event {
                SurfaceEvent::Keyboard { .. } => self.keyboard = self.keyboard.saturating_add(1),
                SurfaceEvent::Ime { .. } => self.ime = self.ime.saturating_add(1),
                SurfaceEvent::Pointer { .. } => self.pointer = self.pointer.saturating_add(1),
                SurfaceEvent::Wake { .. } => self.wakes = self.wakes.saturating_add(1),
                _ => {}
            }
            if let Some(frame) = response.frame() {
                self.frames = self.frames.saturating_add(1);
                self.maximum_glyphs = self.maximum_glyphs.max(frame.scene().glyphs().len());
            }
        }
    }

    #[derive(Default)]
    struct NativeInputEvidence {
        events: usize,
        keyboard: usize,
        ime_started: usize,
        ime_updated: usize,
        ime_committed: usize,
        pointer: usize,
        scroll: usize,
        unexpected: usize,
        frame_revisions: [u64; NATIVE_INPUT_FRAMES],
        frames: usize,
    }

    impl NativeInputEvidence {
        fn observe(&mut self, event: &SurfaceEvent) {
            self.events = self.events.saturating_add(1);
            match event {
                SurfaceEvent::Keyboard { .. } => {
                    self.keyboard = self.keyboard.saturating_add(1);
                }
                SurfaceEvent::Ime {
                    event: ImeEvent::Started,
                    ..
                } => self.ime_started = self.ime_started.saturating_add(1),
                SurfaceEvent::Ime {
                    event: ImeEvent::Updated { .. },
                    ..
                } => self.ime_updated = self.ime_updated.saturating_add(1),
                SurfaceEvent::Ime {
                    event: ImeEvent::Committed(_),
                    ..
                } => self.ime_committed = self.ime_committed.saturating_add(1),
                SurfaceEvent::Pointer { .. } => {
                    self.pointer = self.pointer.saturating_add(1);
                }
                SurfaceEvent::Scroll { .. } => {
                    self.scroll = self.scroll.saturating_add(1);
                }
                SurfaceEvent::Ime {
                    event: ImeEvent::Cancelled,
                    ..
                }
                | SurfaceEvent::Focus { .. }
                | SurfaceEvent::Resize { .. }
                | SurfaceEvent::Clipboard { .. }
                | SurfaceEvent::Wake { .. }
                | SurfaceEvent::CloseRequested { .. } => {
                    self.unexpected = self.unexpected.saturating_add(1);
                }
            }
        }

        fn observe_response(&mut self, response: &SurfaceResponse) {
            let Some(frame) = response.frame() else {
                return;
            };
            assert!(self.frames < self.frame_revisions.len());
            self.frame_revisions[self.frames] = frame.scene().revision().get();
            self.frames += 1;
        }
    }

    /// Runs one real AppKit, runtime, and Studio clipboard and close journey.
    ///
    /// # Errors
    ///
    /// Returns a structured construction, rendering, pasteboard, save, or
    /// teardown failure from the production-composed validation process.
    pub fn qualify_clipboard_and_close_process()
    -> Result<NativeProcessEvidence, Box<dyn std::error::Error>> {
        let path = std::env::temp_dir().join(format!(
            "alpine-studio-native-process-{}.txt",
            std::process::id()
        ));
        let source = native_source("alpha beta")?;
        let expected_after_input = native_source("A漢字 beta")?;
        fs::write(&path, source)?;
        let result = qualify_path(&path, &expected_after_input);
        let cleanup = fs::remove_file(path);
        match (result, cleanup) {
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(Box::new(error)),
            (Ok(evidence), Ok(())) => Ok(evidence),
        }
    }

    /// Runs one real AppKit, runtime, and lazy file-tree journey.
    ///
    /// # Errors
    ///
    /// Returns a structured workspace, construction, rendering, input, save,
    /// or teardown failure from the production-composed validation process.
    pub fn qualify_file_tree_process() -> Result<NativeFileTreeEvidence, Box<dyn std::error::Error>>
    {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let root = std::env::temp_dir().join(format!(
            "alpine-studio-native-tree-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&root)?;
        let alpha = root.join("alpha.txt");
        let beta = root.join("beta.txt");
        let gamma = root.join("gamma.txt");
        fs::write(&alpha, "alpha")?;
        fs::write(&beta, "beta")?;
        fs::write(&gamma, "gamma")?;
        let result = qualify_file_tree_path(&root, &alpha, &beta, &gamma);
        let cleanup = fs::remove_dir_all(root);
        match (result, cleanup) {
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(Box::new(error)),
            (Ok(evidence), Ok(())) => Ok(evidence),
        }
    }

    /// Runs one real AppKit, runtime, and streaming project-search journey.
    ///
    /// # Errors
    ///
    /// Returns a structured workspace, construction, rendering, input, search,
    /// save, or teardown failure from the production-composed process.
    pub fn qualify_project_search_process()
    -> Result<NativeProjectSearchEvidence, Box<dyn std::error::Error>> {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let root = std::env::temp_dir().join(format!(
            "alpine-studio-native-project-search-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&root)?;
        let matched = root.join("alpha.rs");
        let unmatched = root.join("beta.rs");
        fs::write(&matched, "zero\nneedle alpha\n")?;
        fs::write(&unmatched, "zero\nother beta\n")?;
        let result = qualify_project_search_path(&root, &matched);
        let cleanup = fs::remove_dir_all(root);
        match (result, cleanup) {
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(Box::new(error)),
            (Ok(evidence), Ok(())) => Ok(evidence),
        }
    }

    fn native_source(first_line: &str) -> Result<String, fmt::Error> {
        let mut source = String::new();
        writeln!(&mut source, "{first_line}")?;
        for line in 0..128 {
            writeln!(&mut source, "line {line:03}")?;
        }
        Ok(source)
    }

    #[allow(
        clippy::too_many_lines,
        reason = "one process journey preserves surface, worker, input, save, and drain identity"
    )]
    fn qualify_file_tree_path(
        root: &Path,
        alpha: &Path,
        beta: &Path,
        gamma: &Path,
    ) -> Result<NativeFileTreeEvidence, Box<dyn std::error::Error>> {
        let workspace = Workspace::open_root(root)?;
        let mut text_system = alpine_text_layout::CoreTextSystem::new();
        text_system.register_font(FONT_FAMILY, "Menlo-Regular")?;
        let delegate = StudioApp::from_workspace(text_system, workspace)?;
        let clear = alpine_core::LinearRgba::new(0.02, 0.02, 0.02, 1.0).ok_or(
            StudioError::Runtime(alpine_runtime::RuntimeError::Surface(
                alpine_platform_macos::SurfaceError::DriverUnavailable,
            )),
        )?;
        let viewport = alpine_core::Size::new(WINDOW_WIDTH, WINDOW_HEIGHT).ok_or(
            StudioError::Runtime(alpine_runtime::RuntimeError::Surface(
                alpine_platform_macos::SurfaceError::DriverUnavailable,
            )),
        )?;
        let descriptor = SurfaceDescriptor::new(
            "Alpine Studio native file tree",
            f64::from(WINDOW_WIDTH),
            f64::from(WINDOW_HEIGHT),
            f64::from(DEFAULT_SCALE),
        )?;
        let mut application = Application::new(delegate, viewport, clear, WorkerConfig::default())?;
        let surface = platform_validation::new_surface(&descriptor)?;
        let initial_frame = application
            .frame_if_dirty()
            .ok_or("Studio did not build its initial file-tree frame")?;
        let initial_glyphs = initial_frame.scene().glyphs().len();
        let (scene, clear) = initial_frame.into_parts();
        let _revision = surface.request_frame(scene, clear)?;
        surface.show()?;
        platform_validation::run_until_frame_terminal(&surface, Duration::from_secs(5));
        assert_eq!(surface.take_error()?, None);

        let state = Rc::new(RefCell::new(application));
        let journey = Rc::new(RefCell::new(NativeFileTreeJourney::default()));
        replay_tree_events(
            &surface,
            &state,
            &journey,
            &[keyboard_event(
                100,
                KEY_E,
                "e",
                Modifiers::from_bits(TREE_TOGGLE_MODIFIER_BITS),
            )],
        )?;
        let activation_glyphs = journey.borrow().maximum_glyphs.max(initial_glyphs);
        let mut published = false;
        for timestamp in 101..613 {
            if journey.borrow().maximum_glyphs > activation_glyphs {
                published = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
            replay_tree_events(
                &surface,
                &state,
                &journey,
                &[SurfaceEvent::Wake {
                    timestamp: EventTimestamp::new(timestamp),
                }],
            )?;
        }
        assert!(published);
        assert!(journey.borrow().maximum_glyphs > activation_glyphs);
        let frames_after_publication = journey.borrow().frames;
        replay_tree_events(
            &surface,
            &state,
            &journey,
            &[SurfaceEvent::Wake {
                timestamp: EventTimestamp::new(613),
            }],
        )?;
        assert_eq!(journey.borrow().frames, frames_after_publication);

        replay_tree_events(
            &surface,
            &state,
            &journey,
            &[
                keyboard_event(614, KEY_DOWN, "ArrowDown", Modifiers::default()),
                keyboard_event(615, KEY_UP, "ArrowUp", Modifiers::default()),
            ],
        )?;
        let before_keyboard_open = state.borrow().snapshot().document_revision();
        replay_tree_events(
            &surface,
            &state,
            &journey,
            &[keyboard_event(
                616,
                KEY_RETURN,
                "Enter",
                Modifiers::default(),
            )],
        )?;
        assert_ne!(
            state.borrow().snapshot().document_revision(),
            before_keyboard_open
        );

        let pointer =
            alpine_core::Point::new(10.0, CONTENT_INSET + TREE_ROW_HEIGHT.mul_add(1.5, 0.0))
                .ok_or("invalid file-tree pointer")?;
        let before_pointer_open = state.borrow().snapshot().document_revision();
        replay_tree_events(
            &surface,
            &state,
            &journey,
            &[
                SurfaceEvent::Pointer {
                    timestamp: EventTimestamp::new(618),
                    action: PointerAction::Down,
                    position: pointer,
                    button: PointerButton::Primary,
                    modifiers: Modifiers::default(),
                },
                SurfaceEvent::Pointer {
                    timestamp: EventTimestamp::new(619),
                    action: PointerAction::Up,
                    position: pointer,
                    button: PointerButton::Primary,
                    modifiers: Modifiers::default(),
                },
            ],
        )?;
        assert_ne!(
            state.borrow().snapshot().document_revision(),
            before_pointer_open
        );
        replay_tree_events(
            &surface,
            &state,
            &journey,
            &[SurfaceEvent::Ime {
                timestamp: EventTimestamp::new(620),
                event: ImeEvent::Committed("!".into()),
            }],
        )?;
        replay_tree_events(
            &surface,
            &state,
            &journey,
            &[
                keyboard_event(621, KEY_P, "p", COMMAND_SHIFT_MODIFIERS),
                SurfaceEvent::Ime {
                    timestamp: EventTimestamp::new(622),
                    event: ImeEvent::Committed("save".into()),
                },
                keyboard_event(623, KEY_RETURN, "Enter", Modifiers::default()),
            ],
        )?;
        assert_eq!(fs::read_to_string(alpha)?, "alpha");
        assert_eq!(fs::read_to_string(beta)?, "!beta");
        assert_eq!(fs::read_to_string(gamma)?, "gamma");

        let observer = surface.observer();
        assert!(platform_validation::replay_close_with_handler(
            &surface,
            event_handler(&state),
        )?);
        assert_eq!(observer.lifecycle(), SurfaceLifecycle::Closing);
        assert!(state.borrow().snapshot().is_shutting_down());
        let (keyboard_events, pointer_events, worker_wakes, admitted_frames) = {
            let evidence = journey.borrow();
            (
                evidence.keyboard,
                evidence.pointer,
                evidence.wakes,
                evidence.frames,
            )
        };
        drop(state);
        let owner_evidence = platform_validation::close_with_owner_evidence(surface)?;
        assert_eq!(owner_evidence.active(), [0; 9]);
        assert_eq!(owner_evidence.release_order_violations(), 0);
        Ok(NativeFileTreeEvidence {
            keyboard_events,
            pointer_events,
            worker_wakes,
            admitted_frames,
            persisted_bytes: "!beta".len(),
            released_owner_classes: owner_evidence
                .active()
                .iter()
                .filter(|active| **active == 0)
                .count(),
        })
    }

    #[allow(
        clippy::too_many_lines,
        reason = "one process journey preserves surface, worker, search, save, and drain identity"
    )]
    fn qualify_project_search_path(
        root: &Path,
        matched: &Path,
    ) -> Result<NativeProjectSearchEvidence, Box<dyn std::error::Error>> {
        let workspace = Workspace::open_root(root)?;
        let mut text_system = alpine_text_layout::CoreTextSystem::new();
        text_system.register_font(FONT_FAMILY, "Menlo-Regular")?;
        let delegate = StudioApp::from_workspace(text_system, workspace)?;
        let clear = alpine_core::LinearRgba::new(0.02, 0.02, 0.02, 1.0).ok_or(
            StudioError::Runtime(alpine_runtime::RuntimeError::Surface(
                alpine_platform_macos::SurfaceError::DriverUnavailable,
            )),
        )?;
        let viewport = alpine_core::Size::new(WINDOW_WIDTH, WINDOW_HEIGHT).ok_or(
            StudioError::Runtime(alpine_runtime::RuntimeError::Surface(
                alpine_platform_macos::SurfaceError::DriverUnavailable,
            )),
        )?;
        let descriptor = SurfaceDescriptor::new(
            "Alpine Studio native project search",
            f64::from(WINDOW_WIDTH),
            f64::from(WINDOW_HEIGHT),
            f64::from(DEFAULT_SCALE),
        )?;
        let mut application = Application::new(delegate, viewport, clear, WorkerConfig::default())?;
        let surface = platform_validation::new_surface(&descriptor)?;
        let initial_frame = application
            .frame_if_dirty()
            .ok_or("Studio did not build its initial project-search frame")?;
        let (scene, clear) = initial_frame.into_parts();
        let _revision = surface.request_frame(scene, clear)?;
        surface.show()?;
        platform_validation::run_until_frame_terminal(&surface, Duration::from_secs(5));
        assert_eq!(surface.take_error()?, None);

        let state = Rc::new(RefCell::new(application));
        let journey = Rc::new(RefCell::new(NativeFileTreeJourney::default()));
        replay_tree_events(
            &surface,
            &state,
            &journey,
            &[
                keyboard_event(700, KEY_F, "f", COMMAND_SHIFT_MODIFIERS),
                SurfaceEvent::Ime {
                    timestamp: EventTimestamp::new(701),
                    event: ImeEvent::Committed("needle".into()),
                },
            ],
        )?;
        let frames_after_query = journey.borrow().frames;
        let mut latest_frames = frames_after_query;
        let mut stable_wakes = 0_usize;
        let mut published_terminal = false;
        for timestamp in 702..1_214 {
            std::thread::sleep(Duration::from_millis(1));
            replay_tree_events(
                &surface,
                &state,
                &journey,
                &[SurfaceEvent::Wake {
                    timestamp: EventTimestamp::new(timestamp),
                }],
            )?;
            let current_frames = journey.borrow().frames;
            if current_frames > latest_frames {
                latest_frames = current_frames;
                stable_wakes = 0;
            } else if latest_frames >= frames_after_query.saturating_add(2) {
                stable_wakes = stable_wakes.saturating_add(1);
            }
            if stable_wakes == 16 {
                published_terminal = true;
                break;
            }
        }
        assert!(published_terminal);
        let terminal_frames = journey.borrow().frames;
        replay_tree_events(
            &surface,
            &state,
            &journey,
            &[SurfaceEvent::Wake {
                timestamp: EventTimestamp::new(1_214),
            }],
        )?;
        assert_eq!(journey.borrow().frames, terminal_frames);

        let before_open = state.borrow().snapshot().document_revision();
        replay_tree_events(
            &surface,
            &state,
            &journey,
            &[keyboard_event(
                1_215,
                KEY_RETURN,
                "Enter",
                Modifiers::default(),
            )],
        )?;
        assert_ne!(state.borrow().snapshot().document_revision(), before_open);
        replay_tree_events(
            &surface,
            &state,
            &journey,
            &[
                SurfaceEvent::Ime {
                    timestamp: EventTimestamp::new(1_216),
                    event: ImeEvent::Committed("!".into()),
                },
                keyboard_event(1_217, KEY_S, "s", Modifiers::from_bits(Modifiers::COMMAND)),
            ],
        )?;
        assert_eq!(fs::read_to_string(matched)?, "zero\n! alpha\n");

        let observer = surface.observer();
        assert!(platform_validation::replay_close_with_handler(
            &surface,
            event_handler(&state),
        )?);
        assert_eq!(observer.lifecycle(), SurfaceLifecycle::Closing);
        assert!(state.borrow().snapshot().is_shutting_down());
        let (keyboard_events, ime_events, worker_wakes, admitted_frames) = {
            let evidence = journey.borrow();
            (
                evidence.keyboard,
                evidence.ime,
                evidence.wakes,
                evidence.frames,
            )
        };
        drop(state);
        let owner_evidence = platform_validation::close_with_owner_evidence(surface)?;
        assert_eq!(owner_evidence.active(), [0; 9]);
        assert_eq!(owner_evidence.release_order_violations(), 0);
        Ok(NativeProjectSearchEvidence {
            keyboard_events,
            ime_events,
            worker_wakes,
            admitted_frames,
            matched_bytes: "needle".len(),
            released_owner_classes: owner_evidence
                .active()
                .iter()
                .filter(|active| **active == 0)
                .count(),
        })
    }

    fn qualify_path(
        path: &Path,
        expected_after_input: &str,
    ) -> Result<NativeProcessEvidence, Box<dyn std::error::Error>> {
        let mut delegate = native_file_app(path)?;
        delegate.selection = Selection::new(ByteOffset::new(0), ByteOffset::new(5));
        let clear = alpine_core::LinearRgba::new(0.02, 0.02, 0.02, 1.0).ok_or(
            StudioError::Runtime(alpine_runtime::RuntimeError::Surface(
                alpine_platform_macos::SurfaceError::DriverUnavailable,
            )),
        )?;
        let viewport = alpine_core::Size::new(WINDOW_WIDTH, WINDOW_HEIGHT).ok_or(
            StudioError::Runtime(alpine_runtime::RuntimeError::Surface(
                alpine_platform_macos::SurfaceError::DriverUnavailable,
            )),
        )?;
        let descriptor = SurfaceDescriptor::new(
            "Alpine Studio native process",
            f64::from(WINDOW_WIDTH),
            f64::from(WINDOW_HEIGHT),
            f64::from(DEFAULT_SCALE),
        )?;
        let mut application = Application::new(delegate, viewport, clear, WorkerConfig::default())?;
        let surface = platform_validation::new_surface(&descriptor)?;
        let initial_frame = application
            .frame_if_dirty()
            .ok_or("Studio did not build its initial dirty frame")?;
        let (scene, clear) = initial_frame.into_parts();
        let _revision = surface.request_frame(scene, clear)?;
        surface.show()?;
        platform_validation::run_until_frame_terminal(&surface, Duration::from_secs(5));
        assert_eq!(surface.take_error()?, None);

        let state = Rc::new(RefCell::new(application));
        let initial_revision = state.borrow().snapshot().document_revision();
        let input_evidence = Rc::new(RefCell::new(NativeInputEvidence::default()));
        platform_validation::replay_native_input_path(
            &surface,
            observed_event_handler(&state, &input_evidence),
        )?;
        let input_revision = state.borrow().snapshot().document_revision();
        assert_ne!(input_revision, initial_revision);
        let (input_events, input_frames) = {
            let evidence = input_evidence.borrow();
            assert_eq!(evidence.events, 7);
            assert_eq!(evidence.keyboard, 1);
            assert_eq!(evidence.ime_started, 1);
            assert_eq!(evidence.ime_updated, 1);
            assert_eq!(evidence.ime_committed, 2);
            assert_eq!(evidence.pointer, 1);
            assert_eq!(evidence.scroll, 1);
            assert_eq!(evidence.unexpected, 0);
            assert_eq!(evidence.frames, NATIVE_INPUT_FRAMES);
            assert_eq!(evidence.frame_revisions, [2, 3, 4, 5, 6]);
            (evidence.events, evidence.frames)
        };
        assert!(state.borrow_mut().frame_if_dirty().is_none());
        dispatch_save(&surface, &state, 1)?;
        assert_eq!(fs::read_to_string(path)?, expected_after_input);
        dispatch_select_all(&surface, &state, 2)?;
        assert_eq!(
            state.borrow().snapshot().document_revision(),
            input_revision
        );

        platform_validation::replay_native_clipboard_operation(
            &surface,
            ClipboardOperation::Copy,
            event_handler(&state),
        )?;
        assert_eq!(
            state.borrow().snapshot().document_revision(),
            input_revision
        );
        dispatch_save(&surface, &state, 3)?;
        assert_eq!(fs::read_to_string(path)?, expected_after_input);

        platform_validation::inject_clipboard_error(&surface, ClipboardError::WriteRejected);
        platform_validation::replay_native_clipboard_operation(
            &surface,
            ClipboardOperation::Cut,
            event_handler(&state),
        )?;
        assert_eq!(
            state.borrow().snapshot().document_revision(),
            input_revision
        );
        dispatch_save(&surface, &state, 4)?;
        assert_eq!(fs::read_to_string(path)?, expected_after_input);

        platform_validation::replay_native_clipboard_operation(
            &surface,
            ClipboardOperation::Cut,
            event_handler(&state),
        )?;
        let cut_revision = state.borrow().snapshot().document_revision();
        assert_ne!(cut_revision, input_revision);
        let observer = surface.observer();
        assert!(!platform_validation::replay_close_with_handler(
            &surface,
            event_handler(&state),
        )?);
        assert_eq!(observer.lifecycle(), SurfaceLifecycle::Live);
        assert!(!state.borrow().snapshot().is_shutting_down());
        assert_eq!(fs::read_to_string(path)?, expected_after_input);

        platform_validation::replay_native_clipboard_operation(
            &surface,
            ClipboardOperation::Paste,
            event_handler(&state),
        )?;
        assert_ne!(state.borrow().snapshot().document_revision(), cut_revision);
        dispatch_save(&surface, &state, 5)?;
        assert_eq!(fs::read_to_string(path)?, expected_after_input);
        assert!(platform_validation::replay_close_with_handler(
            &surface,
            event_handler(&state),
        )?);
        assert_eq!(observer.lifecycle(), SurfaceLifecycle::Closing);
        assert!(state.borrow().snapshot().is_shutting_down());

        drop(state);
        let evidence = platform_validation::close_with_owner_evidence(surface)?;
        assert_eq!(evidence.active(), [0; 9]);
        assert_eq!(evidence.release_order_violations(), 0);
        Ok(NativeProcessEvidence {
            input_events,
            input_frames,
            persisted_bytes: expected_after_input.len(),
            released_owner_classes: evidence
                .active()
                .iter()
                .filter(|active| **active == 0)
                .count(),
        })
    }

    fn event_handler(
        state: &Rc<RefCell<Application<StudioApp>>>,
    ) -> impl FnMut(SurfaceEvent) -> SurfaceResponse + 'static {
        let state = Rc::clone(state);
        move |event| {
            state.try_borrow_mut().map_or_else(
                |_| SurfaceResponse::default(),
                |mut application| application.dispatch_with_response(&event),
            )
        }
    }

    fn observed_event_handler(
        state: &Rc<RefCell<Application<StudioApp>>>,
        evidence: &Rc<RefCell<NativeInputEvidence>>,
    ) -> impl FnMut(SurfaceEvent) -> SurfaceResponse + 'static {
        let state = Rc::clone(state);
        let evidence = Rc::clone(evidence);
        move |event| {
            evidence.borrow_mut().observe(&event);
            let response = state.try_borrow_mut().map_or_else(
                |_| SurfaceResponse::default(),
                |mut application| application.dispatch_with_response(&event),
            );
            evidence.borrow_mut().observe_response(&response);
            response
        }
    }

    fn keyboard_event(
        timestamp: u64,
        physical_key: u16,
        logical_key: &str,
        modifiers: Modifiers,
    ) -> SurfaceEvent {
        SurfaceEvent::Keyboard {
            timestamp: EventTimestamp::new(timestamp),
            state: KeyState::Down,
            physical_key,
            logical_key: logical_key.into(),
            modifiers,
            repeat: false,
        }
    }

    fn replay_tree_events(
        surface: &NativeSurface,
        state: &Rc<RefCell<Application<StudioApp>>>,
        journey: &Rc<RefCell<NativeFileTreeJourney>>,
        events: &[SurfaceEvent],
    ) -> Result<(), alpine_platform_macos::SurfaceError> {
        let state = Rc::clone(state);
        let journey = Rc::clone(journey);
        platform_validation::replay_callback_surface_events(surface, events, move |event| {
            let response = state.try_borrow_mut().map_or_else(
                |_| SurfaceResponse::default(),
                |mut application| application.dispatch_with_response(&event),
            );
            journey.borrow_mut().observe(&event, &response);
            response
        })
    }

    fn dispatch_select_all(
        surface: &NativeSurface,
        state: &Rc<RefCell<Application<StudioApp>>>,
        timestamp: u64,
    ) -> Result<(), alpine_platform_macos::SurfaceError> {
        let events = [SurfaceEvent::Keyboard {
            timestamp: EventTimestamp::new(timestamp),
            state: KeyState::Down,
            physical_key: KEY_A,
            logical_key: "a".into(),
            modifiers: Modifiers::from_bits(Modifiers::COMMAND),
            repeat: false,
        }];
        platform_validation::replay_callback_surface_events(surface, &events, event_handler(state))
    }

    fn dispatch_save(
        surface: &NativeSurface,
        state: &Rc<RefCell<Application<StudioApp>>>,
        timestamp: u64,
    ) -> Result<(), alpine_platform_macos::SurfaceError> {
        let events = [SurfaceEvent::Keyboard {
            timestamp: EventTimestamp::new(timestamp),
            state: KeyState::Down,
            physical_key: KEY_S,
            logical_key: "s".into(),
            modifiers: Modifiers::from_bits(Modifiers::COMMAND),
            repeat: false,
        }];
        platform_validation::replay_callback_surface_events(surface, &events, event_handler(state))
    }
}

#[cfg(test)]
#[path = "command_palette_tests.rs"]
mod command_palette_tests;

#[cfg(test)]
#[path = "project_search_tests.rs"]
mod project_search_tests;

#[cfg(test)]
#[path = "studio_coverage_tests.rs"]
mod tests;

#[cfg(test)]
mod session_integration_tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::thread;
    use std::time::{Duration, Instant};

    static SESSION_TEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

    fn test_state(root: &Path) -> session::SessionState {
        let alpha_view = DocumentViewState {
            selection: Selection::caret(ByteOffset::new(2)),
            scroll_y: 0.0,
        };
        let beta_view = DocumentViewState {
            selection: Selection::caret(ByteOffset::new(1)),
            scroll_y: 0.0,
        };
        session::SessionState {
            workspace: Some(root.to_path_buf()),
            tabs: vec![
                session::SessionTab {
                    path: None,
                    view: DocumentViewState::default(),
                },
                session::SessionTab {
                    path: Some(root.join("beta.rs")),
                    view: beta_view,
                },
                session::SessionTab {
                    path: Some(root.join("missing.rs")),
                    view: DocumentViewState {
                        selection: Selection::caret(ByteOffset::new(0)),
                        scroll_y: 0.0,
                    },
                },
                session::SessionTab {
                    path: Some(root.join("alpha.rs")),
                    view: alpha_view,
                },
            ],
            active_tab: 1,
            panes: session::SessionPanes {
                nodes: [
                    session::SessionNode::Split {
                        axis: session::SessionAxis::Columns,
                        first: 1,
                        second: 2,
                    },
                    session::SessionNode::Leaf { pane: 0 },
                    session::SessionNode::Leaf { pane: 1 },
                    session::SessionNode::Empty,
                    session::SessionNode::Empty,
                    session::SessionNode::Empty,
                    session::SessionNode::Empty,
                ],
                panes: [
                    Some(session::SessionPane {
                        tab: 0,
                        view: DocumentViewState::default(),
                    }),
                    Some(session::SessionPane {
                        tab: 1,
                        view: beta_view,
                    }),
                    None,
                    None,
                ],
                active_pane: 1,
            },
            file_tree: session::SessionFileTree {
                expanded: vec![PathBuf::from("src")],
                selected: Some(PathBuf::from("src/main.rs")),
            },
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "Miri isolation forbids filesystem syscalls")]
    #[allow(
        clippy::too_many_lines,
        reason = "drop, codec, restore, clamp, and missing-file controls form one durability journey"
    )]
    fn clean_session_drop_and_restore_preserve_exact_tabs_and_split()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = std::env::temp_dir().join(format!(
            "alpine-studio-restoration-{}-{}",
            std::process::id(),
            SESSION_TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root)?;
        let root = fs::canonicalize(root)?;
        fs::write(root.join("alpha.rs"), "alpha\n")?;
        fs::write(root.join("beta.rs"), "beta\n")?;
        let state = test_state(&root);
        let path = root.join("state").join("session.bin");

        let mut app = StudioApp::from_session(tests::TestTextSystem, state.clone())
            .map_err(|error| error.to_string())?;
        assert_eq!(app.tabs.is_deferred(2), Ok(true));
        assert!(matches!(
            app.activate_document_tab(2),
            Err(WorkspaceSelectionError::File(_))
        ));
        assert_eq!(app.tabs.active_index(), 1);
        assert_eq!(app.tabs.is_deferred(2), Ok(true));
        let missing_id = app.tabs.id_at(2).ok_or("missing tab identity")?;
        app.tabs.inject_forward_history_target_for_test(missing_id);
        let failures = app.workspace_failures;
        assert!(app.navigate_document_history(true).visual_changed);
        assert_eq!(app.workspace_failures, failures + 1);
        assert_eq!(app.tabs.active_index(), 1);
        assert_eq!(app.tabs.is_deferred(2), Ok(true));

        let scratch_id = app.tabs.id_at(0).ok_or("scratch tab identity")?;
        app.tabs.inject_forward_history_target_for_test(scratch_id);
        app.force_empty_navigation_result = Some(());
        assert_eq!(app.navigate_document_history(true), EventEffect::default());
        assert_eq!(app.tabs.active_index(), 1);
        assert_eq!(app.tabs.is_deferred(0), Ok(false));
        let recovery_request = app
            .capture_recovery_request()
            .map_err(|error| format!("{error:?}"))?;
        assert!(recovery_request.documents.is_empty());
        assert_eq!(
            app.capture_session()
                .map_err(|error| format!("{error:?}"))?,
            state
        );
        app.session_path = Some(path.clone());
        drop(app);

        let persisted = session::load(&path)?;
        assert_eq!(persisted, state);
        let mut restored = StudioApp::from_session(tests::TestTextSystem, persisted)
            .map_err(|error| error.to_string())?;
        restored.session_path = None;
        assert_eq!(
            restored
                .capture_session()
                .map_err(|error| format!("{error:?}"))?,
            state
        );

        let mut switched_state = state.clone();
        switched_state.active_tab = 3;
        switched_state.panes.active_pane = 0;
        switched_state.panes.panes[0] = Some(session::SessionPane {
            tab: 3,
            view: state.tabs[3].view,
        });
        let mut switched = StudioApp::from_session(tests::TestTextSystem, switched_state.clone())
            .map_err(|error| error.to_string())?;
        switched.session_path = None;
        assert_eq!(
            switched
                .capture_session()
                .map_err(|error| format!("{error:?}"))?,
            switched_state
        );

        fs::write(root.join("unicode.rs"), "🦀\n")?;
        let invalid_boundary = DocumentViewState {
            selection: Selection::caret(ByteOffset::new(3)),
            scroll_y: f32::MAX,
        };
        let clamped_state = session::SessionState {
            workspace: Some(root.clone()),
            tabs: vec![session::SessionTab {
                path: Some(root.join("unicode.rs")),
                view: invalid_boundary,
            }],
            active_tab: 0,
            panes: session::SessionPanes {
                nodes: [
                    session::SessionNode::Leaf { pane: 0 },
                    session::SessionNode::Empty,
                    session::SessionNode::Empty,
                    session::SessionNode::Empty,
                    session::SessionNode::Empty,
                    session::SessionNode::Empty,
                    session::SessionNode::Empty,
                ],
                panes: [
                    Some(session::SessionPane {
                        tab: 0,
                        view: invalid_boundary,
                    }),
                    None,
                    None,
                    None,
                ],
                active_pane: 0,
            },
            file_tree: session::SessionFileTree::default(),
        };
        let mut clamped = StudioApp::from_session(tests::TestTextSystem, clamped_state)
            .map_err(|error| error.to_string())?;
        let clamped_capture = clamped
            .capture_session()
            .map_err(|error| format!("{error:?}"))?;
        assert_eq!(
            clamped_capture.tabs[0].view.selection,
            Selection::caret(ByteOffset::new(0))
        );
        assert_eq!(
            clamped_capture.tabs[0].view.scroll_y.to_bits(),
            0.0_f32.to_bits()
        );
        assert_eq!(clamped.panes.len(), 1);
        clamped.session_path = None;

        let bounded_view = StudioApp::clamp_view_to_document(
            &StudioDocument::scratch("🦀abcd\nx\ny\nz"),
            DocumentViewState {
                selection: Selection::new(ByteOffset::new(3), ByteOffset::new(6)),
                scroll_y: f32::MAX,
            },
        );
        assert_eq!(
            bounded_view.selection,
            Selection::new(ByteOffset::new(0), ByteOffset::new(6))
        );
        assert_eq!(bounded_view.scroll_y.to_bits(), 88.0_f32.to_bits());

        let dirty_path = root.join("state").join("dirty-session.bin");
        let mut dirty = StudioApp::new(tests::TestTextSystem)?;
        if let StudioDocument::Scratch {
            buffer,
            clean_revision,
            ..
        } = &mut dirty.document
        {
            *clean_revision = buffer.revision().get().saturating_add(1);
        }
        assert_eq!(
            dirty.capture_session(),
            Err(SessionCaptureError::DirtyDocument)
        );
        dirty.session_path = Some(dirty_path.clone());
        drop(dirty);
        assert!(!dirty_path.exists());

        let expected = root.join("state").join("injected-session.bin");
        let mut defaulted =
            with_session_path(StudioApp::new(tests::TestTextSystem)?, Ok(expected.clone()));
        assert_eq!(defaulted.session_path.as_deref(), Some(expected.as_path()));
        defaulted.session_path = None;

        fs::remove_file(root.join("alpha.rs"))?;
        let mut missing_visible_state = state;
        missing_visible_state.active_tab = 1;
        missing_visible_state.panes.active_pane = 1;
        missing_visible_state.panes.panes[0] = Some(session::SessionPane {
            tab: 2,
            view: DocumentViewState::default(),
        });
        assert!(matches!(
            StudioApp::from_session(tests::TestTextSystem, missing_visible_state),
            Err(SessionRestoreError::File)
        ));
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn session_restore_error_messages_and_rejections_are_structured() {
        assert_eq!(
            RestoreAvailability::for_recovery(0),
            RestoreAvailability::Strict
        );
        assert_eq!(
            RestoreAvailability::for_recovery(1),
            RestoreAvailability::AllowPlaceholder
        );
        assert_eq!(
            RestoreAvailability::for_recovery(usize::MAX),
            RestoreAvailability::AllowPlaceholder
        );
        assert!(!RestoreAvailability::Strict.allows_placeholder());
        assert!(RestoreAvailability::AllowPlaceholder.allows_placeholder());
        assert_eq!(
            classify_session_document_error(&WorkspaceSelectionError::Tabs(
                DocumentTabError::LastTab
            )),
            SessionRestoreError::Tabs
        );
        assert_eq!(
            classify_session_document_error(&WorkspaceSelectionError::File(FileError::Io {
                operation: "restore-test",
                kind: std::io::ErrorKind::NotFound,
            })),
            SessionRestoreError::File
        );
        assert_eq!(
            classify_session_document_error(&WorkspaceSelectionError::NoWorkspace),
            SessionRestoreError::Invalid
        );
        for error in [
            SessionRestoreError::Invalid,
            SessionRestoreError::Workspace,
            SessionRestoreError::File,
            SessionRestoreError::Surface,
            SessionRestoreError::Tabs,
            SessionRestoreError::Panes,
            SessionRestoreError::FileTree,
            SessionRestoreError::Allocation,
        ] {
            assert!(!error.to_string().is_empty());
        }

        let invalid = session::SessionState {
            workspace: None,
            tabs: Vec::new(),
            active_tab: 0,
            panes: session::SessionPanes {
                nodes: [session::SessionNode::Empty; session::SESSION_NODE_CAPACITY],
                panes: [None; session::SESSION_PANE_CAPACITY],
                active_pane: 0,
            },
            file_tree: session::SessionFileTree::default(),
        };
        assert!(matches!(
            StudioApp::from_session(tests::TestTextSystem, invalid),
            Err(SessionRestoreError::Invalid)
        ));
    }

    #[test]
    fn recovery_variants_indexing_and_persistence_failures_are_explicit()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = std::env::temp_dir().join(format!(
            "alpine-studio-recovery-branches-{}-{}",
            std::process::id(),
            SESSION_TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root)?;
        let root = fs::canonicalize(root)?;
        let recovered = recovery::RecoveredDocument {
            tab: 0,
            base: Box::from("base"),
            local: Box::from("local"),
        };

        let mut scratch = StudioDocument::recover(None, &recovered)?;
        assert_eq!(scratch.buffer().snapshot().text(), "local");
        assert!(scratch.is_dirty());
        assert!(!scratch.is_file());
        assert_eq!(scratch.recovery_base().text(), "base");
        assert_eq!(scratch.buffer_mut().snapshot().text(), "local");

        let invalid = root.join("invalid.rs");
        fs::write(&invalid, [0xff])?;
        let mut conflicted = StudioDocument::recover(Some(&invalid), &recovered)?;
        assert!(conflicted.has_recovery_conflict());
        assert!(conflicted.is_dirty());
        assert_eq!(conflicted.recovery_base().text(), "base");
        assert_eq!(conflicted.buffer_mut().snapshot().text(), "local");
        assert_eq!(
            conflicted.save(),
            Err(FileError::Conflict(ExternalChange::Modified))
        );

        let mut unavailable =
            StudioDocument::open_for_restore(&invalid, RestoreAvailability::AllowPlaceholder)?;
        assert!(unavailable.is_unavailable());
        assert_eq!(
            unavailable.save(),
            Err(FileError::Conflict(ExternalChange::Modified))
        );

        assert_eq!(
            StudioApp::index_recoveries(1, vec![recovered.clone(), recovered]),
            Err(SessionRestoreError::Invalid)
        );

        let capture_path = root.join("capture").join("session.bin");
        let mut capture = StudioApp::new(tests::TestTextSystem)?;
        capture
            .configure_persistence(capture_path)
            .map_err(|error| error.to_string())?;
        capture.tabs.inject_active_index_fault();
        capture.publish_recovery();
        assert_eq!(
            capture.local_status,
            Some(LocalStatus::Workspace(Arc::from(
                "Recovery capture failed: Panes"
            )))
        );
        drop(capture);

        let blocked = root.join("not-a-directory");
        fs::write(&blocked, b"file")?;
        let mut degraded = StudioApp::new(tests::TestTextSystem)?;
        degraded
            .configure_persistence(blocked.join("recovery.bin"))
            .map_err(|error| error.to_string())?;
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let status = degraded
                .recovery
                .as_ref()
                .ok_or("recovery coordinator")?
                .status();
            if status.completed_generation >= status.published_generation
                || Instant::now() >= deadline
            {
                break;
            }
            thread::yield_now();
        }
        degraded.publish_recovery();
        assert!(degraded.last_recovery_error.is_some());
        assert!(matches!(
            degraded.local_status,
            Some(LocalStatus::Workspace(ref message))
                if message.starts_with("Dirty-buffer recovery degraded:")
        ));
        drop(degraded);

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn dirty_recovery_preserves_local_text_and_never_overwrites_external_bytes()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = std::env::temp_dir().join(format!(
            "alpine-studio-dirty-recovery-{}-{}",
            std::process::id(),
            SESSION_TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root)?;
        let root = fs::canonicalize(root)?;
        let document_path = root.join("document.rs");
        let session_path = root.join("state").join("session.bin");
        fs::write(&document_path, "base")?;

        let mut app = StudioApp::open_file(tests::TestTextSystem, &document_path)?;
        app.configure_persistence(session_path.clone())
            .map_err(|error| error.to_string())?;
        let initial_generation = app
            .recovery
            .as_ref()
            .ok_or("recovery coordinator")?
            .status()
            .published_generation;
        assert!(initial_generation > 0);
        assert!(app.replace_range(0..4, "local").document_changed);
        app.publish_recovery();
        let dirty_generation = app
            .recovery
            .as_ref()
            .ok_or("recovery coordinator")?
            .status()
            .published_generation;
        assert!(dirty_generation > initial_generation);
        app.last_recovery_error = Some(recovery::RecoveryError::Disconnected);
        app.publish_recovery();
        assert_eq!(app.last_recovery_error, None);
        drop(app);

        let recovery_path = recovery::path_for_session(&session_path);
        let recovered = recovery::load(&recovery_path)?;
        assert_eq!(recovered.documents.len(), 1);
        assert_eq!(&*recovered.documents[0].base, "base");
        assert_eq!(&*recovered.documents[0].local, "local");

        let mut equal_text = recovered.clone();
        equal_text.documents[0].local = equal_text.documents[0].base.clone();
        let equal_text = StudioApp::from_recovery(tests::TestTextSystem, equal_text)
            .map_err(|error| error.to_string())?;
        assert_eq!(equal_text.buffer().snapshot().text(), "base");
        assert!(equal_text.document.is_dirty());

        let mut unchanged = StudioApp::from_recovery(tests::TestTextSystem, recovered.clone())
            .map_err(|error| error.to_string())?;
        assert_eq!(unchanged.buffer().snapshot().text(), "local");
        assert!(unchanged.document.is_dirty());
        assert!(!unchanged.document.has_recovery_conflict());
        assert!(!unchanged.document.is_unavailable());
        #[cfg(not(target_family = "windows"))]
        {
            assert!(unchanged.document.save()?.is_some());
            assert_eq!(fs::read_to_string(&document_path)?, "local");
        }
        #[cfg(target_family = "windows")]
        {
            assert_eq!(
                unchanged.document.save(),
                Err(FileError::UnsupportedAtomicReplace)
            );
            assert_eq!(fs::read_to_string(&document_path)?, "base");
        }

        fs::write(&document_path, "external")?;
        let mut modified = StudioApp::from_recovery(tests::TestTextSystem, recovered.clone())
            .map_err(|error| error.to_string())?;
        assert_eq!(modified.buffer().snapshot().text(), "local");
        assert!(modified.document.has_recovery_conflict());
        assert!(!modified.document.is_unavailable());
        assert_eq!(
            modified.local_status,
            Some(LocalStatus::Workspace(Arc::from(
                "Recovered 1 dirty buffer(s); 1 external conflict(s) and 0 unavailable clean file(s) remain save-blocked."
            )))
        );
        assert_eq!(
            modified.document.save(),
            Err(FileError::Conflict(ExternalChange::Modified))
        );
        assert_eq!(fs::read_to_string(&document_path)?, "external");

        fs::remove_file(&document_path)?;
        let mut deleted = StudioApp::from_recovery(tests::TestTextSystem, recovered)
            .map_err(|error| error.to_string())?;
        assert_eq!(deleted.buffer().snapshot().text(), "local");
        assert!(deleted.document.has_recovery_conflict());
        assert_eq!(
            deleted.document.save(),
            Err(FileError::Conflict(ExternalChange::Deleted))
        );
        assert!(!document_path.exists());

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn unavailable_clean_active_file_cannot_hide_an_inactive_dirty_recovery()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = std::env::temp_dir().join(format!(
            "alpine-studio-partial-recovery-{}-{}",
            std::process::id(),
            SESSION_TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root)?;
        let root = fs::canonicalize(root)?;
        fs::write(root.join("alpha.rs"), "alpha\n")?;
        fs::write(root.join("beta.rs"), "beta\n")?;
        let mut state = test_state(&root);
        state.workspace = Some(root.join("missing-workspace"));
        state.active_tab = 2;
        let active_pane = usize::from(state.panes.active_pane);
        state.panes.panes[active_pane]
            .as_mut()
            .ok_or("active pane")?
            .tab = 2;
        session::validate(&state).map_err(|error| error.to_string())?;
        assert!(matches!(
            StudioApp::from_session(tests::TestTextSystem, state.clone()),
            Err(SessionRestoreError::Workspace)
        ));
        let recovery = recovery::RecoveryState {
            session: state,
            documents: vec![recovery::RecoveredDocument {
                tab: 1,
                base: Box::from("beta\n"),
                local: Box::from("local beta\n"),
            }],
        };

        let mut app = StudioApp::from_recovery(tests::TestTextSystem, recovery)
            .map_err(|error| error.to_string())?;
        assert!(app.workspace.is_none());
        assert!(app.document.is_unavailable());
        assert!(!app.document.is_dirty());
        assert_eq!(
            app.local_status,
            Some(LocalStatus::Workspace(Arc::from(
                "Recovered 1 dirty buffer(s); 0 external conflict(s) and 1 unavailable clean file(s) remain save-blocked. The prior workspace is unavailable; document recovery remains active."
            )))
        );
        assert!(app.activate_document_tab(1)?.document_changed);
        assert_eq!(app.buffer().snapshot().text(), "local beta\n");
        assert!(app.document.is_dirty());

        let mut clean = StudioApp::new(tests::TestTextSystem)?;
        clean
            .record_recovered_status(0, None)
            .map_err(|error| error.to_string())?;
        assert_eq!(clean.local_status, None);

        fs::remove_dir_all(root)?;
        Ok(())
    }
}
