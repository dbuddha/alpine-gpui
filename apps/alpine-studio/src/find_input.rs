//! Find/Replace native text queries share the exact single-line display layout.

use alpine_core::{Point, Rect, Size};
use alpine_platform_macos::{
    AccessibilityAction, AccessibilityActionResult, AccessibilityBounds, AccessibilityOperation,
    AccessibilityPayload, AccessibilitySelection, AccessibilityText, AccessibilityTextRange,
};
use alpine_text::{Buffer, Selection};
use alpine_text_layout::{FontKey, LineLayout, TextShaper};

use crate::accessibility::AccessibilityError;
use crate::{
    EventEffect, FIND_BAR_HEIGHT, FIND_BAR_INSET, FIND_BAR_WIDTH, LINE_HEIGHT, StudioApp,
    StudioRenderError, TAB_BAR_HEIGHT,
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
}

pub(super) fn bounds(app: &StudioApp) -> Result<Rect, StudioRenderError> {
    let pane = app
        .active_pane_bounds()
        .map_err(|_| StudioRenderError::Domain)?;
    let width = FIND_BAR_WIDTH.min(pane.size().width()).max(1.0);
    let left = (pane.origin().x() + pane.size().width() - width).max(pane.origin().x());
    Ok(Rect::new(
        Point::new(left, TAB_BAR_HEIGHT + FIND_BAR_INSET).ok_or(StudioRenderError::Domain)?,
        Size::new(width, FIND_BAR_HEIGHT).ok_or(StudioRenderError::Domain)?,
    ))
}

pub(super) fn layout(app: &mut StudioApp) -> Result<Layout, StudioRenderError> {
    let bounds = bounds(app)?;
    let font = app.resolved_font()?;
    let text = app.find.display_text()?;
    let caret_byte = app.find.display_caret();
    let caret = text
        .get(..caret_byte)
        .ok_or(StudioRenderError::Domain)?
        .encode_utf16()
        .count();
    let caret_x = app.text_system.caret_offset(&text, font, caret)?;
    let shift = (caret_x
        - (bounds.size().width() - 2.0 * FIND_BAR_INSET - crate::CARET_WIDTH).max(0.0))
    .max(0.0);
    let origin_x = bounds.origin().x() + FIND_BAR_INSET - shift;
    let top = bounds.origin().y() + 6.0;
    let field_start_utf16 = app.find.display_prefix().encode_utf16().count();
    let field_len_utf16 = app.find.projected_value()?.encode_utf16().count();
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
    })
}

fn unavailable() -> AccessibilityError {
    alpine_platform_macos::AccessibilityError::InvalidBounds.into()
}

pub(super) fn respond(
    app: &mut StudioApp,
    operation: &AccessibilityOperation,
) -> Result<(AccessibilityPayload, EventEffect), AccessibilityError> {
    let source = Buffer::new(app.find.field_text()).snapshot();
    let payload = match operation {
        AccessibilityOperation::Text { range, .. } => {
            let start = source.byte_of_appkit_utf16(range.start_utf16())?;
            let end = source.byte_of_appkit_utf16(range.end_utf16()?)?;
            AccessibilityPayload::Text(AccessibilityText::new(
                source.slice(start.get()..end.get())?,
            )?)
        }
        AccessibilityOperation::Selection { .. } => {
            let selected = app.find.selection();
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
            let changed = app
                .find
                .set_selection(selection)
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
        AccessibilityOperation::FirstRectForRange {
            range, marked_text, ..
        } => {
            if !marked_text && app.find.is_composing() {
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
    app: &mut StudioApp,
    range: AccessibilityTextRange,
) -> Result<AccessibilityPayload, AccessibilityError> {
    let projected = app.find.projected_value().map_err(|_| unavailable())?;
    let text = Buffer::new(&projected).snapshot();
    text.byte_of_appkit_utf16(range.start_utf16())?;
    text.byte_of_appkit_utf16(range.end_utf16()?)?;
    let view = layout(app).map_err(|_| unavailable())?;
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
    app: &mut StudioApp,
    point: AccessibilityBounds,
) -> Result<usize, AccessibilityError> {
    let view = layout(app).map_err(|_| unavailable())?;
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
    app: &mut StudioApp,
    point: Point,
    extend: bool,
) -> Result<bool, AccessibilityError> {
    let view = layout(app).map_err(|_| unavailable())?;
    let projected = app.find.projected_value().map_err(|_| unavailable())?;
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
    let source = app.find.source_index(nearest);
    let source = alpine_text::ByteOffset::new(source);
    let selected = if extend {
        Selection::new(app.find.selection().anchor(), source)
    } else {
        Selection::caret(source)
    };
    app.find.set_selection(selected).map_err(|_| unavailable())
}
