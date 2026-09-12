//! A bounded display projection. Preedit never changes the document or its history.

use super::{Buffer, BufferSnapshot, ByteOffset, Composition, StudioRenderError};
use std::ops::Range;

/// Visual intervals for a logical marked range, including separated bidi runs.
/// Only the two partially selected boundary clusters need native caret queries.
pub(super) fn visual_spans(
    text: &str,
    layout: &alpine_text_layout::LineLayout,
    font: alpine_text_layout::FontKey,
    shaper: &mut dyn alpine_text_layout::TextShaper,
    selected: Range<usize>,
) -> Result<Vec<(f32, f32)>, StudioRenderError> {
    // Retain only selected clusters and their immediate logical boundaries.
    // A short preedit on a long line must not sort/copy every glyph in the line.
    let predecessor = layout
        .glyphs()
        .iter()
        .map(|glyph| glyph.source_utf16() as usize)
        .filter(|index| *index <= selected.start)
        .max();
    let successor = layout
        .glyphs()
        .iter()
        .map(|glyph| glyph.source_utf16() as usize)
        .filter(|index| *index >= selected.end)
        .min();
    let mut clusters = Vec::new();
    clusters
        .try_reserve(layout.glyphs().len().min(selected.len().saturating_add(2)))
        .map_err(|_| StudioRenderError::Domain)?;
    for glyph in layout.glyphs() {
        let index = glyph.source_utf16() as usize;
        if (index >= selected.start && index < selected.end)
            || Some(index) == predecessor
            || Some(index) == successor
        {
            let x0 = glyph.x().min(glyph.x() + glyph.advance());
            let x1 = glyph.x().max(glyph.x() + glyph.advance());
            clusters
                .try_reserve(1)
                .map_err(|_| StudioRenderError::Domain)?;
            clusters.push((index, x0, x1));
        }
    }
    clusters.sort_unstable_by_key(|cluster| cluster.0);
    let units = text.encode_utf16().count();
    let mut spans = Vec::new();
    spans
        .try_reserve(clusters.len())
        .map_err(|_| StudioRenderError::Domain)?;
    let mut cursor = 0;
    while cursor < clusters.len() {
        let (start, mut left, mut right) = clusters[cursor];
        cursor += 1;
        while cursor < clusters.len() && clusters[cursor].0 == start {
            left = left.min(clusters[cursor].1);
            right = right.max(clusters[cursor].2);
            cursor += 1;
        }
        let end = clusters.get(cursor).map_or(units, |cluster| cluster.0);
        if start >= selected.end || end <= selected.start {
            continue;
        }
        if selected.start > start || selected.end < end {
            let x0 = shaper.caret_offset(text, font, selected.start.max(start))?;
            let x1 = shaper.caret_offset(text, font, selected.end.min(end))?;
            // A scalar inside a zero-advance combining cluster has no separate
            // visual cell. Keep the cluster's extent when the carets coincide.
            if (x0 - x1).abs() > f32::EPSILON {
                left = left.max(x0.min(x1));
                right = right.min(x0.max(x1));
            }
        }
        if right >= left {
            spans.push((left, right));
        }
    }
    spans.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
    let mut merged: Vec<(f32, f32)> = Vec::new();
    merged
        .try_reserve(spans.len())
        .map_err(|_| StudioRenderError::Domain)?;
    for (left, right) in spans {
        if let Some(last) = merged.last_mut()
            && left <= last.1 + 0.01
        {
            last.1 = last.1.max(right);
        } else {
            merged.push((left, right));
        }
    }
    if merged.is_empty() {
        let x = shaper.caret_offset(text, font, selected.start.min(units))?;
        merged.push((x, x));
    }
    Ok(merged)
}

pub(super) struct Projection {
    source: BufferSnapshot,
    segment: BufferSnapshot,
    first_line: usize,
    after_source_line: usize,
    segment_lines: usize,
    base_utf16: usize,
    source_end_utf16: usize,
    segment_utf16: usize,
    replaced: Range<usize>,
    mark: Range<usize>,
}

pub(super) struct Line<'a> {
    pub snapshot: &'a BufferSnapshot,
    pub local: usize,
    pub display: usize,
    pub base_utf16: usize,
    /// Original line for syntax/diagnostics, absent inside the changed paragraph.
    pub source: Option<usize>,
}

impl Projection {
    pub fn new(
        source: BufferSnapshot,
        composition: &Composition,
    ) -> Result<Self, StudioRenderError> {
        let start = composition.replacement.start;
        let end = composition.replacement.end;
        if start > end {
            return Err(StudioRenderError::Domain);
        }
        let mut first_line = source.line_of_byte(ByteOffset::new(start))?;
        let last_line = source.line_of_byte(ByteOffset::new(end))?;
        let mut first_byte = source.line_byte_range(first_line)?.start;
        // A leading LF in the projection can join the previous lone CR. Keep
        // that line in the segment so Ropey sees the real CRLF boundary.
        if first_byte > 0
            && source
                .slice(first_byte - 1..first_byte)
                .is_ok_and(|text| text == "\r")
        {
            first_line -= 1;
            first_byte = source.line_byte_range(first_line)?.start;
        }
        let last_byte = source.line_byte_range(last_line)?.end;
        let bytes = (start - first_byte)
            .checked_add(composition.text.len())
            .and_then(|v| v.checked_add(last_byte - end))
            .ok_or(StudioRenderError::Domain)?;
        // Copy only the surviving boundary fragments and preedit, never the
        // replaced span or the complete document. Bound synchronous work.
        if bytes > alpine_text_layout::DEFAULT_MAX_LINE_BYTES {
            return Err(StudioRenderError::Domain);
        }
        let mut text = String::new();
        text.try_reserve_exact(bytes)
            .map_err(|_| StudioRenderError::Domain)?;
        text.push_str(&source.slice(first_byte..start)?);
        text.push_str(&composition.text);
        text.push_str(&source.slice(end..last_byte)?);
        let segment = Buffer::new(&text).snapshot();
        let after_source_line = last_line + 1;
        let segment_lines =
            segment.line_count() - usize::from(after_source_line < source.line_count());
        let base_utf16 = source.appkit_utf16_of_byte(ByteOffset::new(first_byte))?;
        let source_end_utf16 = source.appkit_utf16_of_byte(ByteOffset::new(last_byte))?;
        let replaced = source.appkit_utf16_of_byte(ByteOffset::new(start))?
            ..source.appkit_utf16_of_byte(ByteOffset::new(end))?;
        let mark = replaced.start
            ..replaced
                .start
                .checked_add(composition.text.encode_utf16().count())
                .ok_or(StudioRenderError::Domain)?;
        let segment_utf16 = text.encode_utf16().count();
        Ok(Self {
            source,
            segment,
            first_line,
            after_source_line,
            segment_lines,
            base_utf16,
            source_end_utf16,
            segment_utf16,
            replaced,
            mark,
        })
    }

    pub fn mark(&self) -> Range<usize> {
        self.mark.clone()
    }

    pub fn line_count(&self) -> usize {
        self.first_line + self.segment_lines + self.source.line_count() - self.after_source_line
    }

    pub fn line(&self, display: usize) -> Result<Line<'_>, StudioRenderError> {
        let after = self.first_line + self.segment_lines;
        let (snapshot, local, source) = if display < self.first_line {
            (&self.source, display, Some(display))
        } else if display < after {
            (&self.segment, display - self.first_line, None)
        } else {
            let local = self.after_source_line + display - after;
            (&self.source, local, Some(local))
        };
        let bytes = snapshot.line_byte_range(local)?;
        let mut base_utf16 = snapshot.appkit_utf16_of_byte(ByteOffset::new(bytes.start))?;
        if source.is_none() {
            base_utf16 += self.base_utf16;
        } else if display >= after {
            base_utf16 = base_utf16 - self.source_end_utf16 + self.base_utf16 + self.segment_utf16;
        }
        Ok(Line {
            snapshot,
            local,
            display,
            base_utf16,
            source,
        })
    }

    pub fn line_at_utf16(&self, index: usize) -> Result<Line<'_>, StudioRenderError> {
        let segment_end = self.base_utf16 + self.segment_utf16;
        let display = if index < self.base_utf16 {
            self.source
                .line_of_byte(self.source.byte_of_appkit_utf16(index)?)?
        } else if index < segment_end || self.after_source_line == self.source.line_count() {
            let byte = self.segment.byte_of_appkit_utf16(index - self.base_utf16)?;
            self.first_line + self.segment.line_of_byte(byte)?
        } else {
            let source_index = index - segment_end + self.source_end_utf16;
            let line = self
                .source
                .line_of_byte(self.source.byte_of_appkit_utf16(source_index)?)?;
            self.first_line + self.segment_lines + line - self.after_source_line
        };
        self.line(display)
    }

    pub fn source_to_display(&self, index: usize, downstream: bool) -> Option<usize> {
        if downstream && self.replaced.is_empty() && index == self.replaced.start {
            return Some(self.mark.end);
        }
        if index <= self.replaced.start {
            Some(index)
        } else if index < self.replaced.end {
            None
        } else {
            index
                .checked_sub(self.replaced.end)?
                .checked_add(self.mark.end)
        }
    }

    pub fn display_to_source(&self, index: usize) -> Option<usize> {
        if index <= self.mark.start {
            Some(index)
        } else if index < self.mark.end {
            Some(self.replaced.start)
        } else {
            index
                .checked_sub(self.mark.end)?
                .checked_add(self.replaced.end)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacing_a_large_span_retains_only_boundary_text() -> Result<(), StudioRenderError> {
        let source_text = format!("prefix {} suffix", "discard\n".repeat(100_000));
        let source = Buffer::new(&source_text).snapshot();
        let projection = Projection::new(
            source,
            &Composition {
                replacement: 7..source_text.len() - 7,
                text: "漢".into(),
                selected_start_utf16: 1,
                selected_length_utf16: 0,
            },
        )?;
        assert_eq!(projection.segment.text(), "prefix 漢 suffix");
        assert_eq!(projection.segment.len_bytes(), 17);
        assert_eq!(projection.line_count(), 1);
        Ok(())
    }

    #[test]
    fn multiline_unicode_projection_preserves_source_and_coordinates()
    -> Result<(), StudioRenderError> {
        for (text, range, preedit) in [
            ("prefix old suffix\nnext", 7..10, "漢😀"),
            ("before\none\ntwo\nafter\n", 9..13, "x\ny"),
            ("a\r\nb\r\nc", 1..4, "\n😀\n"),
            ("a\nb\n", 0..4, ""),
            ("abc", 3..3, "x\ny\n"),
            ("a\rb", 2..2, "\n"),
            ("a\rx\nb", 2..3, ""),
        ] {
            let source = Buffer::new(text).snapshot();
            let mut expected = text.to_owned();
            expected.replace_range(range.clone(), preedit);
            let expected = Buffer::new(&expected).snapshot();
            let projection = Projection::new(
                source.clone(),
                &Composition {
                    replacement: range,
                    text: preedit.into(),
                    selected_start_utf16: 0,
                    selected_length_utf16: 0,
                },
            )?;
            assert_eq!(source.text(), text);
            assert_eq!(projection.line_count(), expected.line_count());
            for display in 0..expected.line_count() {
                let line = projection.line(display)?;
                let expected_bytes = expected.line_byte_range(display)?;
                assert_eq!(
                    line.snapshot
                        .slice(line.snapshot.line_byte_range(line.local)?)?,
                    expected.slice(expected_bytes.clone())?
                );
                assert_eq!(
                    line.base_utf16,
                    expected.appkit_utf16_of_byte(ByteOffset::new(expected_bytes.start))?
                );
            }
            for (byte, _) in expected
                .text()
                .char_indices()
                .chain(std::iter::once((expected.len_bytes(), '\0')))
            {
                let units = expected.appkit_utf16_of_byte(ByteOffset::new(byte))?;
                assert_eq!(
                    projection.line_at_utf16(units)?.display,
                    expected.line_of_byte(ByteOffset::new(byte))?
                );
            }
        }
        Ok(())
    }
}
