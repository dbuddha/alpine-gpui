//! Overlay native text queries share the exact painted field layout.

use alpine_core::{Point, Rect, Size};
use alpine_platform_macos::{
    AccessibilityAction, AccessibilityActionResult, AccessibilityBounds, AccessibilityOperation,
    AccessibilityPayload, AccessibilitySelection, AccessibilityText, AccessibilityTextRange,
};
use alpine_text::{Buffer, Selection};
use alpine_text_layout::{FontKey, LineLayout, TextShaper};

use crate::accessibility::AccessibilityError;
use crate::overlay_field::Owner;
use crate::{
    EditorApp, EditorRenderError, EventEffect, FIND_BAR_HEIGHT, FIND_BAR_INSET, FIND_BAR_WIDTH,
    LINE_HEIGHT, TAB_BAR_HEIGHT,
};

pub(super) struct Layout {
    pub text: String,
    pub line: LineLayout,
    pub font: FontKey,
    pub bounds: Rect,
    pub origin_x: f32,
    pub top: f32,
    pub field_start_utf16: usize,
    pub field_len_utf16: usize,
    pub selection: std::ops::Range<usize>,
    pub mark: Option<std::ops::Range<usize>>,
    pub caret: usize,
}

pub(super) fn bounds(app: &EditorApp) -> Result<Rect, EditorRenderError> {
    let pane = app
        .active_pane_bounds()
        .map_err(|_| EditorRenderError::Domain)?;
    let width = FIND_BAR_WIDTH.min(pane.size().width()).max(1.0);
    let left = (pane.origin().x() + pane.size().width() - width).max(pane.origin().x());
    Ok(Rect::new(
        Point::new(left, TAB_BAR_HEIGHT + FIND_BAR_INSET).ok_or(EditorRenderError::Domain)?,
        Size::new(width, FIND_BAR_HEIGHT).ok_or(EditorRenderError::Domain)?,
    ))
}

pub(super) fn bounds_for(app: &EditorApp, owner: Owner) -> Result<Rect, EditorRenderError> {
    use crate::{
        COMMAND_PALETTE_QUERY_HEIGHT, COMMAND_PALETTE_WIDTH, CONTENT_INSET,
        PROJECT_SEARCH_QUERY_HEIGHT, PROJECT_SEARCH_WIDTH, QUICK_OPEN_QUERY_HEIGHT,
        QUICK_OPEN_WIDTH,
    };
    if owner == Owner::Find {
        return bounds(app);
    }
    if matches!(owner, Owner::Symbols | Owner::Rename) {
        let rows = if owner == Owner::Symbols {
            app.rust_diagnostics
                .symbol_visible_range(app.language_identity())
                .ok_or(EditorRenderError::Domain)?
                .len()
                + 1
        } else {
            app.workspace_edits.line_count()
        };
        let pane = app
            .active_pane_bounds()
            .map_err(|_| EditorRenderError::Domain)?;
        let bounds = EditorApp::language_overlay_bounds(pane, rows)?;
        return Ok(Rect::new(
            bounds.origin(),
            Size::new(bounds.size().width(), LINE_HEIGHT).ok_or(EditorRenderError::Domain)?,
        ));
    }
    let (width, height) = match owner {
        Owner::Palette => (COMMAND_PALETTE_WIDTH, COMMAND_PALETTE_QUERY_HEIGHT),
        Owner::QuickOpen => (QUICK_OPEN_WIDTH, QUICK_OPEN_QUERY_HEIGHT),
        Owner::ProjectSearch => (PROJECT_SEARCH_WIDTH, PROJECT_SEARCH_QUERY_HEIGHT),
        _ => unreachable!(),
    };
    let width = width.min((app.last_viewport.width() - CONTENT_INSET * 2.0).max(1.0));
    let left = ((app.last_viewport.width() - width) * 0.5).max(0.0);
    Ok(Rect::new(
        Point::new(left, TAB_BAR_HEIGHT + CONTENT_INSET).ok_or(EditorRenderError::Domain)?,
        Size::new(width, height).ok_or(EditorRenderError::Domain)?,
    ))
}

pub(super) fn layout_for(app: &mut EditorApp, owner: Owner) -> Result<Layout, EditorRenderError> {
    let bounds = bounds_for(app, owner)?;
    let font = app.resolved_font()?;
    let (value, edit) = owner.read(app).ok_or(EditorRenderError::Domain)?;
    let projected = edit
        .projected_value(value)
        .map_err(|_| EditorRenderError::Domain)?;
    let prefix = owner.prefix(app);
    let selected = edit.projected_selection(value);
    let offset_range = |range: std::ops::Range<usize>| {
        prefix.encode_utf16().count() + projected[..range.start].encode_utf16().count()
            ..prefix.encode_utf16().count() + projected[..range.end].encode_utf16().count()
    };
    let selection = offset_range(selected.range());
    let mark = edit.mark_range(value).map(offset_range);
    let caret =
        prefix.encode_utf16().count() + projected[..selected.head().get()].encode_utf16().count();
    let field_start_utf16 = prefix.encode_utf16().count();
    let field_len_utf16 = projected.encode_utf16().count();
    let text = match owner {
        Owner::Find => app.find.display_text()?,
        Owner::Palette => app.command_palette.display_text()?,
        Owner::QuickOpen => app.quick_open.display_text()?,
        Owner::ProjectSearch => app.project_search.display_text()?,
        Owner::Symbols => projected,
        Owner::Rename => format!("{prefix}{projected} | Enter submits"),
    };
    let caret_x = app.text_system.caret_offset(&text, font, caret)?;
    let shift = (caret_x
        - (bounds.size().width() - 2.0 * FIND_BAR_INSET - crate::CARET_WIDTH).max(0.0))
    .max(0.0);
    let origin_x = bounds.origin().x() + FIND_BAR_INSET - shift;
    let inset_y = match owner {
        Owner::Find => 6.0,
        Owner::Symbols | Owner::Rename => 3.0,
        _ => 7.0,
    };
    let top = bounds.origin().y() + inset_y;
    let line = app.text_system.shape(&text, font)?;
    Ok(Layout {
        text,
        line,
        font,
        bounds,
        origin_x,
        top,
        field_start_utf16,
        field_len_utf16,
        selection,
        mark,
        caret,
    })
}

fn unavailable() -> AccessibilityError {
    alpine_platform_macos::AccessibilityError::InvalidBounds.into()
}

pub(super) fn respond(
    app: &mut EditorApp,
    operation: &AccessibilityOperation,
) -> Result<(AccessibilityPayload, EventEffect), AccessibilityError> {
    let owner = Owner::active(app).ok_or_else(unavailable)?;
    let (value, edit) = owner.read(app).ok_or_else(unavailable)?;
    let selected = edit.selection(value);
    let composing = edit.is_composing();
    let source = Buffer::new(value).snapshot();
    let payload = match operation {
        AccessibilityOperation::Text { range, .. } => {
            let start = source.byte_of_appkit_utf16(range.start_utf16())?;
            let end = source.byte_of_appkit_utf16(range.end_utf16()?)?;
            AccessibilityPayload::Text(AccessibilityText::new(
                source.slice(start.get()..end.get())?,
            )?)
        }
        AccessibilityOperation::Selection { .. } => {
            AccessibilityPayload::Selection(AccessibilitySelection::new(
                source.appkit_utf16_of_byte(selected.anchor())?,
                source.appkit_utf16_of_byte(selected.head())?,
            ))
        }
        AccessibilityOperation::Action(AccessibilityAction::SetSelection { selection, .. }) => {
            let selection = Selection::new(
                source.byte_of_appkit_utf16(selection.anchor_utf16())?,
                source.byte_of_appkit_utf16(selection.head_utf16())?,
            );
            let changed = owner
                .set_selection(app, selection)
                .map_err(|_| unavailable())?;
            return Ok((
                AccessibilityPayload::Action(if changed {
                    AccessibilityActionResult::Applied
                } else {
                    AccessibilityActionResult::Unchanged
                }),
                if changed {
                    EventEffect::visual()
                } else {
                    EventEffect::default()
                },
            ));
        }
        AccessibilityOperation::LineForIndex { index_utf16, .. } => AccessibilityPayload::Line(
            crate::accessibility::line_for_index_from_snapshot(&source, *index_utf16)?,
        ),
        AccessibilityOperation::RangeForLine { line, .. } => AccessibilityPayload::Range(
            crate::accessibility::range_for_line_from_snapshot(&source, *line)?,
        ),
        AccessibilityOperation::RangeForIndex { index_utf16, .. } => AccessibilityPayload::Range(
            crate::accessibility::range_for_index_from_snapshot(&source, *index_utf16)?,
        ),
        AccessibilityOperation::FirstRectForRange {
            range, marked_text, ..
        } => {
            if !marked_text && composing {
                return Err(unavailable());
            }
            geometry(app, *range)?
        }
        AccessibilityOperation::IndexForPoint { point, .. } => {
            AccessibilityPayload::Index(index_at_point(app, *point)?)
        }
        _ => return Err(alpine_platform_macos::AccessibilityError::InvalidTree.into()),
    };
    Ok((payload, EventEffect::default()))
}

pub(super) fn geometry(
    app: &mut EditorApp,
    range: AccessibilityTextRange,
) -> Result<AccessibilityPayload, AccessibilityError> {
    let owner = Owner::active(app).ok_or_else(unavailable)?;
    let (value, edit) = owner.read(app).ok_or_else(unavailable)?;
    let projected = edit.projected_value(value).map_err(|_| unavailable())?;
    let text = Buffer::new(&projected).snapshot();
    text.byte_of_appkit_utf16(range.start_utf16())?;
    text.byte_of_appkit_utf16(range.end_utf16()?)?;
    let owner = Owner::active(app).ok_or_else(unavailable)?;
    let view = layout_for(app, owner).map_err(|_| unavailable())?;
    let start = view.field_start_utf16 + range.start_utf16();
    let end = view.field_start_utf16 + range.end_utf16()?;
    let spans = if start == end {
        let x = app
            .text_system
            .caret_offset(&view.text, view.font, start)
            .map_err(|_| unavailable())?;
        vec![(x, x)]
    } else {
        crate::composition::visual_spans(
            &view.text,
            &view.line,
            view.font,
            &mut app.text_system,
            start..end,
        )
        .map_err(|_| unavailable())?
    };
    let left = spans
        .iter()
        .map(|span| span.0)
        .reduce(f32::min)
        .ok_or_else(unavailable)?
        + view.origin_x;
    let right = spans
        .iter()
        .map(|span| span.1)
        .reduce(f32::max)
        .ok_or_else(unavailable)?
        + view.origin_x;
    let left = left.max(view.bounds.origin().x());
    let right = right.min(view.bounds.origin().x() + view.bounds.size().width());
    if right < left {
        return Err(unavailable());
    }
    Ok(AccessibilityPayload::TextGeometry {
        range,
        bounds: AccessibilityBounds::new(left, view.top, right - left, LINE_HEIGHT)?,
    })
}

pub(super) fn index_at_point(
    app: &mut EditorApp,
    point: AccessibilityBounds,
) -> Result<usize, AccessibilityError> {
    let owner = Owner::active(app).ok_or_else(unavailable)?;
    let view = layout_for(app, owner).map_err(|_| unavailable())?;
    if point.x() < view.bounds.origin().x()
        || point.x() >= view.bounds.origin().x() + view.bounds.size().width()
        || point.y() < view.top
        || point.y() >= view.top + LINE_HEIGHT
    {
        return Err(unavailable());
    }
    let index = app
        .text_system
        .index_at_x(&view.text, view.font, point.x() - view.origin_x)
        .map_err(|_| unavailable())?
        .ok_or_else(unavailable)?;
    let local = index
        .checked_sub(view.field_start_utf16)
        .ok_or_else(unavailable)?;
    if local >= view.field_len_utf16 {
        return Err(unavailable());
    }
    // The native endpoint is a containing-glyph index, never a label/status hit.
    Ok(local)
}

/// Mouse selection uses nearest insertion carets; native hit testing above uses
/// containing glyphs. Preedit coordinates map back before cancelling the mark.
pub(super) fn pointer_selection(
    app: &mut EditorApp,
    point: Point,
    extend: bool,
) -> Result<bool, AccessibilityError> {
    let owner = Owner::active(app).ok_or_else(unavailable)?;
    let view = layout_for(app, owner).map_err(|_| unavailable())?;
    let owner = Owner::active(app).ok_or_else(unavailable)?;
    let (value, edit) = owner.read(app).ok_or_else(unavailable)?;
    let projected = edit.projected_value(value).map_err(|_| unavailable())?;
    let target_x = point.x() - view.origin_x;
    let field_end = view.field_start_utf16 + view.field_len_utf16;
    let cluster = view
        .line
        .glyphs()
        .iter()
        .filter(|glyph| {
            let index = glyph.source_utf16() as usize;
            index >= view.field_start_utf16 && index < field_end
        })
        .min_by(|a, b| {
            let distance = |glyph: &alpine_text_layout::ShapedGlyph| {
                let left = glyph.x().min(glyph.x() + glyph.advance());
                let right = glyph.x().max(glyph.x() + glyph.advance());
                (left - target_x).max(target_x - right).max(0.0)
            };
            distance(a).total_cmp(&distance(b))
        })
        .map(|glyph| glyph.source_utf16() as usize);
    let (start, end) = cluster.map_or((field_end, field_end), |start| {
        let end = view
            .line
            .glyphs()
            .iter()
            .map(|glyph| glyph.source_utf16() as usize)
            .filter(|index| *index > start && *index <= field_end)
            .min()
            .unwrap_or(field_end);
        (start, end)
    });
    let x0 = app
        .text_system
        .caret_offset(&view.text, view.font, start)
        .map_err(|_| unavailable())?;
    let x1 = app
        .text_system
        .caret_offset(&view.text, view.font, end)
        .map_err(|_| unavailable())?;
    let nearest = if (target_x - x0).abs() <= (target_x - x1).abs() {
        start
    } else {
        end
    };
    let nearest = Buffer::new(&projected)
        .snapshot()
        .byte_of_appkit_utf16(nearest - view.field_start_utf16)?
        .get();
    let (value, edit) = owner.read(app).ok_or_else(unavailable)?;
    let source = edit.source_index(value, nearest);
    let anchor = edit.selection(value).anchor();
    let source = alpine_text::ByteOffset::new(source);
    let selected = if extend {
        Selection::new(anchor, source)
    } else {
        Selection::caret(source)
    };
    owner
        .set_selection(app, selected)
        .map_err(|_| unavailable())
}
