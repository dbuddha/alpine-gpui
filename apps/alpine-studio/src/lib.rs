#![cfg_attr(
    not(any(test, all(target_os = "macos", target_arch = "aarch64"))),
    expect(dead_code)
)]

//! Local-only Alpine Studio editor boundary.

mod documents;
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
    Buffer, BufferSnapshot, ByteOffset, Editor, FileError, SaveReport, Selection, SelectionSet,
    TextError, Transaction,
};
use alpine_text_layout::{
    DEFAULT_ATLAS_BUDGET_BYTES, DEFAULT_LAYOUT_BUDGET_BYTES, DEFAULT_OVERSCAN_LINES, FontKey,
    GlyphAtlas, GlyphKey, GlyphRasterizer, LayoutError, LineLayout, LineLayoutCache,
    PositiveFinite, TextShaper, VisibleLines,
};
use documents::{DocumentTabError, DocumentTabLimits, DocumentTabs, DocumentViewState};
use workspace::Workspace;

#[cfg(any(test, all(target_os = "macos", target_arch = "aarch64")))]
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
const INITIAL_TEXT: &str = "fn main() {\n    println!(\"Alpine Studio\");\n}\n\n// Local, direct, and deliberately small.\n";

const KEY_A: u16 = 0;
const KEY_S: u16 = 1;
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
pub fn run() -> Result<(), RuntimeError> {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        run_native(native_app()?)
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
pub fn run_file(path: impl AsRef<Path>) -> Result<(), StudioError> {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        run_native(native_file_app(path.as_ref())?).map_err(StudioError::from)
    }

    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    {
        let _ = path;
        Err(SurfaceError::UnsupportedPlatform.into())
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
            run_native(native_file_app(path)?).map_err(StudioError::from)
        } else if metadata.is_dir() {
            let workspace = Workspace::open(path, WorkspaceLimits::default())?;
            let mut text_system = alpine_text_layout::CoreTextSystem::new();
            text_system
                .register_font(FONT_FAMILY, "Menlo-Regular")
                .map_err(|_| SurfaceError::DriverUnavailable)?;
            let app = StudioApp::from_workspace(text_system, workspace)?;
            run_native(app).map_err(StudioError::from)
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

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn native_file_app(path: &Path) -> Result<StudioApp, StudioError> {
    let document = StudioDocument::open(path)?;
    let mut text_system = alpine_text_layout::CoreTextSystem::new();
    text_system
        .register_font(FONT_FAMILY, "Menlo-Regular")
        .map_err(|_| SurfaceError::DriverUnavailable)?;
    StudioApp::from_document(text_system, document, Some(path)).map_err(StudioError::from)
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
    Workspace(Arc<str>),
}

impl LocalStatus {
    fn message(&self) -> &str {
        match self {
            Self::Clipboard(message) | Self::Workspace(message) => message,
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
    Domain,
    Text(TextError),
    Layout(LayoutError),
    Scene(SceneError),
}

impl From<TextError> for StudioRenderError {
    fn from(error: TextError) -> Self {
        Self::Text(error)
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
            Self::Domain => formatter.write_str("invalid Studio render domain value"),
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
        }
    }
}

impl Error for WorkspaceSelectionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Workspace(error) => Some(error),
            Self::File(error) => Some(error),
            Self::Tabs(error) => Some(error),
            Self::NoWorkspace | Self::DirtyDocument | Self::RevisionExhausted => None,
        }
    }
}

enum StudioDocument {
    Scratch { buffer: Buffer, clean_revision: u64 },
    File(Editor),
}

impl StudioDocument {
    fn scratch(text: &str) -> Self {
        let buffer = Buffer::new(text);
        let clean_revision = buffer.revision().get();
        Self::Scratch {
            buffer,
            clean_revision,
        }
    }

    fn open(path: impl AsRef<Path>) -> Result<Self, FileError> {
        Editor::open(path).map(Self::File)
    }

    const fn buffer(&self) -> &Buffer {
        match self {
            Self::Scratch { buffer, .. } => buffer,
            Self::File(editor) => editor.buffer(),
        }
    }

    const fn buffer_mut(&mut self) -> &mut Buffer {
        match self {
            Self::Scratch { buffer, .. } => buffer,
            Self::File(editor) => editor.buffer_mut(),
        }
    }

    fn save(&mut self) -> Result<Option<SaveReport>, FileError> {
        match self {
            Self::Scratch { .. } => Ok(None),
            Self::File(editor) => editor.save().map(Some),
        }
    }

    fn is_dirty(&self) -> bool {
        match self {
            Self::Scratch {
                buffer,
                clean_revision,
            } => buffer.revision().get() != *clean_revision,
            Self::File(editor) => editor.is_dirty(),
        }
    }
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
        let omitted_entries = workspace.snapshot().omitted_entries;
        let document = StudioDocument::scratch(INITIAL_TEXT);
        let mut app = Self::from_parts(text_system, document, None, Some(workspace))?;
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
        })
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
        let editor_origin_x = sidebar_width + CONTENT_INSET;
        let content_origin =
            Point::new(editor_origin_x, CONTENT_INSET).ok_or(StudioRenderError::Domain)?;
        let content_size = Size::new(
            (viewport.width() - sidebar_width - CONTENT_INSET * 2.0).max(1.0),
            (viewport.height() - CONTENT_INSET * 2.0).max(1.0),
        )
        .ok_or(StudioRenderError::Domain)?;
        let content_bounds = Rect::new(content_origin, content_size);
        let viewport_height =
            PositiveFinite::new(content_size.height()).ok_or(StudioRenderError::Domain)?;
        let line_height = PositiveFinite::new(LINE_HEIGHT).ok_or(StudioRenderError::Domain)?;
        let wrap_width =
            PositiveFinite::new(content_size.width()).ok_or(StudioRenderError::Domain)?;
        let visible = VisibleLines::new(
            self.buffer().snapshot().line_count(),
            self.scroll_y,
            viewport_height,
            line_height,
            DEFAULT_OVERSCAN_LINES,
        )?;
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

        let mut builder = SceneBuilder::new(revision, viewport);
        builder.push_quad(Quad::new(Rect::new(origin, viewport), background))?;
        let clip = builder.push_clip(Clip::new(content_bounds));
        builder.push_quad(Quad::new(content_bounds, editor_background))?;

        let mut rendered_lines = Vec::new();
        let mut pending_glyphs = Vec::new();
        if let Some(workspace) = &self.workspace {
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
            let range = workspace.visible_range(first_visible, visible_rows, TREE_OVERSCAN_ROWS);
            let labels: Vec<(usize, Arc<str>)> = range
                .filter_map(|index| workspace.entry(index).map(|entry| (index, entry.name())))
                .collect();
            for (index, label) in labels {
                let top =
                    CONTENT_INSET + usize_as_f32(index) * TREE_ROW_HEIGHT - self.workspace_scroll_y;
                if self.active_workspace_entry == Some(index) {
                    let row_origin = Point::new(0.0, top).ok_or(StudioRenderError::Domain)?;
                    let row_size = Size::new(sidebar_width.max(1.0), TREE_ROW_HEIGHT)
                        .ok_or(StudioRenderError::Domain)?;
                    let row = Quad::new(Rect::new(row_origin, row_size), active_row_color)
                        .clipped(sidebar_clip);
                    builder.push_quad(row)?;
                }
                let layout = self.text_system.shape(&label, font)?;
                let baseline = top + layout.ascent();
                let glyphs =
                    self.collect_glyphs(&layout, font, CONTENT_INSET, baseline, sidebar_clip)?;
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
        let selected = self.selection.range();
        for line in visible.laid_out() {
            let layout = self.layout_cache.layout_line(
                &snapshot,
                line,
                font,
                wrap_width,
                &mut *self.text_system,
            )?;
            let top = CONTENT_INSET + usize_as_f32(line) * LINE_HEIGHT - self.scroll_y;
            let baseline = top + layout.ascent();
            if !selected.is_empty() {
                let selection_result = Self::paint_selection(
                    &mut builder,
                    clip,
                    &snapshot,
                    line,
                    top,
                    &layout,
                    selected.clone(),
                    selection_color,
                    editor_origin_x,
                );
                selection_result?;
            }
            let glyphs = self.collect_glyphs(&layout, font, editor_origin_x, baseline, clip)?;
            pending_glyphs.extend(glyphs);
            rendered_lines.push(RenderedLine {
                line,
                top,
                baseline,
                layout,
            });
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
            let composition_glyphs =
                self.collect_glyphs(&composition_layout, font, start_x, rendered.baseline, clip);
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
            let top = (viewport.height() - CONTENT_INSET - LINE_HEIGHT).max(CONTENT_INSET);
            let baseline = top + layout.ascent();
            pending_glyphs.extend(self.collect_glyphs(
                &layout,
                font,
                editor_origin_x + 6.0,
                baseline,
                clip,
            )?);
            let origin = Point::new(editor_origin_x, top).ok_or(StudioRenderError::Domain)?;
            let size =
                Size::new(content_size.width(), LINE_HEIGHT).ok_or(StudioRenderError::Domain)?;
            Some(Rect::new(origin, size))
        } else {
            None
        };
        if let Some(bounds) = status_background {
            builder.push_quad(Quad::new(bounds, status_background_color).clipped(clip))?;
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
            builder.push_quad(
                Quad::new(Rect::new(active_origin, active_size), active_tab_color)
                    .clipped(tab_clip),
            )?;
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
            builder.push_quad(Quad::new(bounds, caret_color).clipped(clip))?;
        }
        if self.focused
            && let Some(caret) = self.caret_bounds(&snapshot, &rendered_lines, editor_origin_x)?
        {
            builder.push_quad(Quad::new(caret, caret_color).clipped(clip))?;
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
        if action == PointerAction::Down
            && button == PointerButton::Primary
            && position.y() < TAB_BAR_HEIGHT
            && position.x() >= self.sidebar_width(self.last_viewport)
        {
            self.pointer_selecting = false;
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
            && position.x() < self.sidebar_width(self.last_viewport)
        {
            self.pointer_selecting = false;
            let row_position =
                (position.y() - CONTENT_INSET + self.workspace_scroll_y) / TREE_ROW_HEIGHT;
            let Some(index) = floor_f32_to_usize(row_position) else {
                return EventEffect::default();
            };
            return match self.open_workspace_entry(index) {
                Ok(effect) => effect,
                Err(error) => self.record_workspace_error(&error),
            };
        }
        match action {
            PointerAction::Down if button == PointerButton::Primary => {
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
        }
    }

    fn offset_at_point(&mut self, position: Point) -> Option<ByteOffset> {
        let origin_x = self.sidebar_width(self.last_viewport) + CONTENT_INSET;
        if position.x() < origin_x {
            return None;
        }
        let line_position = (position.y() - CONTENT_INSET + self.scroll_y) / LINE_HEIGHT;
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
        let content_height = (self.last_viewport.height() - CONTENT_INSET * 2.0).max(1.0);
        (usize_as_f32(self.buffer().snapshot().line_count()) * LINE_HEIGHT - content_height)
            .max(0.0)
    }

    fn sidebar_width(&self, viewport: Size) -> f32 {
        if self.workspace.is_some() {
            SIDEBAR_WIDTH.min((viewport.width() - 1.0).max(0.0))
        } else {
            0.0
        }
    }

    fn maximum_workspace_scroll(&self) -> f32 {
        let rows = self.workspace.as_ref().map_or(0, Workspace::len);
        let content_height = (self.last_viewport.height() - CONTENT_INSET).max(1.0);
        (usize_as_f32(rows) * TREE_ROW_HEIGHT - content_height).max(0.0)
    }

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
        if let Some(tab) = self.tabs.index_for_path(&path) {
            return self.activate_document_tab(tab);
        }
        let document = StudioDocument::open(&path).map_err(WorkspaceSelectionError::File)?;
        let next_revision = self
            .runtime_document_revision
            .checked_add(1)
            .ok_or(WorkspaceSelectionError::RevisionExhausted)?;
        let view = self.active_document_view();
        self.tabs
            .insert_and_activate(&path, Some(index), document, &mut self.document, view)
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
        self.ensure_active_tab_visible();
    }

    fn activate_document_tab(
        &mut self,
        index: usize,
    ) -> Result<EventEffect, WorkspaceSelectionError> {
        if index == self.tabs.active_index() {
            return Ok(EventEffect::default());
        }
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
        self.apply_document_view(view);
        Ok(EventEffect::document_replacement())
    }

    fn navigate_document_history(&mut self, forward: bool) -> EventEffect {
        let Some(next_revision) = self.runtime_document_revision.checked_add(1) else {
            return self.record_workspace_error(&WorkspaceSelectionError::RevisionExhausted);
        };
        let current_view = self.active_document_view();
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
        let next_revision = self
            .runtime_document_revision
            .checked_add(1)
            .ok_or(WorkspaceSelectionError::RevisionExhausted)?;
        let view = self
            .tabs
            .close_active(&mut self.document)
            .map_err(WorkspaceSelectionError::Tabs)?;
        self.runtime_document_revision = next_revision;
        self.active_workspace_entry = self.tabs.active_workspace_entry();
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

impl AppDelegate for StudioApp {
    type WorkerOutput = u64;

    fn event(&mut self, event: &SurfaceEvent, context: &mut AppContext<'_, u64>) {
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
            self.advance_runtime_document_identity(effect.document_identity_advanced);
            let revision = DocumentRevision::new(self.runtime_document_revision);
            let rejected = !context.advance_document(revision);
            self.input_failures = self.input_failures.saturating_add(u64::from(rejected));
        }
        if effect.visual_changed {
            context.invalidate();
        }
    }

    fn frame(&mut self, context: WindowContext) -> Scene {
        self.scene(context.scene_revision(), context.viewport())
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
    use std::{cell::RefCell, fmt, fmt::Write as _, fs, path::Path, rc::Rc, time::Duration};

    use alpine_platform_macos::{
        ClipboardError, ClipboardOperation, EventTimestamp, ImeEvent, KeyState, Modifiers,
        NativeSurface, SurfaceDescriptor, SurfaceEvent, SurfaceLifecycle, SurfaceResponse,
        native_validation as platform_validation,
    };
    use alpine_runtime::{Application, WorkerConfig};
    use alpine_text::{ByteOffset, Selection};

    use super::{
        DEFAULT_SCALE, KEY_A, KEY_S, StudioApp, StudioError, WINDOW_HEIGHT, WINDOW_WIDTH,
        native_file_app,
    };

    const NATIVE_INPUT_FRAMES: usize = 5;

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

    fn native_source(first_line: &str) -> Result<String, fmt::Error> {
        let mut source = String::new();
        writeln!(&mut source, "{first_line}")?;
        for line in 0..128 {
            writeln!(&mut source, "line {line:03}")?;
        }
        Ok(source)
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
#[path = "studio_coverage_tests.rs"]
mod tests;
