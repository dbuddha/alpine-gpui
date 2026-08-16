#![cfg_attr(
    not(any(test, all(target_os = "macos", target_arch = "aarch64"))),
    expect(dead_code)
)]

//! Local-only Alpine Studio editor boundary.

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
    ImeEvent, KeyState, Modifiers, PointerAction, PointerButton, SurfaceError, SurfaceEvent,
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
const INITIAL_TEXT: &str = "fn main() {\n    println!(\"Alpine Studio\");\n}\n\n// Local, direct, and deliberately small.\n";

const KEY_A: u16 = 0;
const KEY_S: u16 = 1;
const KEY_Z: u16 = 6;
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
    /// Native application construction or execution failed.
    Runtime(RuntimeError),
}

impl fmt::Display for StudioError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage => formatter.write_str("usage: alpine-studio [file]"),
            Self::File(error) => write!(formatter, "Studio file failed: {error}"),
            Self::Runtime(error) => write!(formatter, "Studio runtime failed: {error}"),
        }
    }
}

impl Error for StudioError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Usage => None,
            Self::File(error) => Some(error),
            Self::Runtime(error) => Some(error),
        }
    }
}

impl From<FileError> for StudioError {
    fn from(error: FileError) -> Self {
        Self::File(error)
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
    StudioApp::from_document(text_system, document).map_err(StudioError::from)
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
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct EventEffect {
    visual_changed: bool,
    document_changed: bool,
}

impl EventEffect {
    const fn visual() -> Self {
        Self {
            visual_changed: true,
            document_changed: false,
        }
    }

    const fn document() -> Self {
        Self {
            visual_changed: true,
            document_changed: true,
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

enum StudioDocument {
    Scratch(Buffer),
    File(Editor),
}

impl StudioDocument {
    fn scratch(text: &str) -> Self {
        Self::Scratch(Buffer::new(text))
    }

    fn open(path: impl AsRef<Path>) -> Result<Self, FileError> {
        Editor::open(path).map(Self::File)
    }

    const fn buffer(&self) -> &Buffer {
        match self {
            Self::Scratch(buffer) => buffer,
            Self::File(editor) => editor.buffer(),
        }
    }

    const fn buffer_mut(&mut self) -> &mut Buffer {
        match self {
            Self::Scratch(buffer) => buffer,
            Self::File(editor) => editor.buffer_mut(),
        }
    }

    fn save(&mut self) -> Result<Option<SaveReport>, FileError> {
        match self {
            Self::Scratch(_) => Ok(None),
            Self::File(editor) => editor.save().map(Some),
        }
    }

    #[cfg(test)]
    fn is_dirty(&self) -> bool {
        match self {
            Self::Scratch(_) => false,
            Self::File(editor) => editor.is_dirty(),
        }
    }
}

struct StudioApp {
    document: StudioDocument,
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
    last_save: Option<SaveReport>,
    last_file_error: Option<FileError>,
}

impl StudioApp {
    fn new(text_system: impl StudioTextSystem + 'static) -> Result<Self, SurfaceError> {
        Self::from_document(text_system, StudioDocument::scratch(INITIAL_TEXT))
    }

    #[cfg(test)]
    fn open_file(
        text_system: impl StudioTextSystem + 'static,
        path: impl AsRef<Path>,
    ) -> Result<Self, StudioError> {
        let document = StudioDocument::open(path)?;
        Self::from_document(text_system, document).map_err(StudioError::from)
    }

    fn from_document(
        text_system: impl StudioTextSystem + 'static,
        document: StudioDocument,
    ) -> Result<Self, SurfaceError> {
        let last_viewport =
            Size::new(WINDOW_WIDTH, WINDOW_HEIGHT).ok_or(SurfaceError::DriverUnavailable)?;
        let layout_budget = NonZeroUsize::new(DEFAULT_LAYOUT_BUDGET_BYTES)
            .ok_or(SurfaceError::DriverUnavailable)?;
        let atlas_budget =
            NonZeroUsize::new(DEFAULT_ATLAS_BUDGET_BYTES).ok_or(SurfaceError::DriverUnavailable)?;
        Ok(Self {
            document,
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
            last_save: None,
            last_file_error: None,
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
        let content_origin =
            Point::new(CONTENT_INSET, CONTENT_INSET).ok_or(StudioRenderError::Domain)?;
        let content_size = Size::new(
            (viewport.width() - CONTENT_INSET * 2.0).max(1.0),
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

        let mut builder = SceneBuilder::new(revision, viewport);
        builder.push_quad(Quad::new(Rect::new(origin, viewport), background))?;
        let clip = builder.push_clip(Clip::new(content_bounds));
        builder.push_quad(Quad::new(content_bounds, editor_background))?;

        let mut rendered_lines = Vec::new();
        let mut pending_glyphs = Vec::new();
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
                );
                selection_result?;
            }
            pending_glyphs.extend(self.collect_glyphs(&layout, font, CONTENT_INSET, baseline)?);
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
            let start_x = CONTENT_INSET + x_for_utf16(&rendered.layout, prefix_utf16);
            let composition_layout = self.text_system.shape(&composition.text, font)?;
            let composition_glyphs =
                self.collect_glyphs(&composition_layout, font, start_x, rendered.baseline);
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

        self.publish_atlas_if_needed(&pending_glyphs)?;
        if !pending_glyphs.is_empty() {
            let atlas = self
                .published_atlas
                .clone()
                .ok_or(StudioRenderError::Domain)?;
            builder.set_glyph_atlas(atlas)?;
            for pending in pending_glyphs {
                let glyph =
                    Glyph::new(pending.bounds, pending.atlas_bounds, text_color).clipped(clip);
                builder.push_glyph(glyph)?;
            }
        }
        if let Some(bounds) = composition_underline {
            builder.push_quad(Quad::new(bounds, caret_color).clipped(clip))?;
        }
        if self.focused
            && let Some(caret) = self.caret_bounds(&snapshot, &rendered_lines)?
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
        let start_x = CONTENT_INSET + x_for_utf16(layout, start_utf16);
        let end_x = CONTENT_INSET + x_for_utf16(layout, end_utf16);
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
        let x = CONTENT_INSET + x_for_utf16(&rendered.layout, utf16);
        let origin = Point::new(x, rendered.top).ok_or(StudioRenderError::Domain)?;
        let size = Size::new(CARET_WIDTH, LINE_HEIGHT).ok_or(StudioRenderError::Domain)?;
        Ok(Some(Rect::new(origin, size)))
    }

    fn handle_event(&mut self, event: &SurfaceEvent) -> EventEffect {
        match event {
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
                let before = self.scroll_y;
                self.scroll_y = (self.scroll_y - *delta_y).clamp(0.0, self.maximum_scroll());
                (self.scroll_y.to_bits() != before.to_bits())
                    .then(EventEffect::visual)
                    .unwrap_or_default()
            }
            SurfaceEvent::Focus { focused, .. } => {
                let changed = self.focused != *focused;
                self.focused = *focused;
                changed.then(EventEffect::visual).unwrap_or_default()
            }
            SurfaceEvent::Ime { event, .. } => self.handle_ime(event),
            SurfaceEvent::Keyboard { .. }
            | SurfaceEvent::Resize { .. }
            | SurfaceEvent::Clipboard { .. }
            | SurfaceEvent::Wake { .. }
            | SurfaceEvent::CloseRequested { .. } => EventEffect::default(),
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
        let x = (position.x() - CONTENT_INSET).max(0.0);
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

    fn save_document(&mut self) -> EventEffect {
        match self.document.save() {
            Ok(Some(report)) => {
                self.last_save = Some(report);
                self.last_file_error = None;
            }
            Ok(None) => {}
            Err(error) => {
                self.save_failures = self.save_failures.saturating_add(1);
                self.last_file_error = Some(error);
            }
        }
        EventEffect::default()
    }

    fn clamp_scroll(&mut self) {
        self.scroll_y = self.scroll_y.clamp(0.0, self.maximum_scroll());
    }
}

impl AppDelegate for StudioApp {
    type WorkerOutput = u64;

    fn event(&mut self, event: &SurfaceEvent, context: &mut AppContext<'_, u64>) {
        let effect = self.handle_event(event);
        if effect.document_changed {
            let revision = DocumentRevision::new(self.buffer().revision().get());
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

#[cfg(test)]
#[path = "studio_coverage_tests.rs"]
mod tests;
