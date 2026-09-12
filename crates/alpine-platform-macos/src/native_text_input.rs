//! Native UTF-16 projection over the focused editor and its uncommitted mark.

use super::{
    DefinedClass, ImeEvent, NSPoint, NSRange, NSRect, NativeAccessibilityAdapter, SurfaceView,
};
use crate::{AccessibilityRevision, MAX_ACCESSIBILITY_TEXT_RESPONSE_BYTES};

pub(super) fn missing_range() -> NSRange {
    // Foundation defines NSNotFound as NSIntegerMax, not NSUIntegerMax.
    NSRange::new(objc2_foundation::NSNotFound.unsigned_abs(), 0)
}

fn end(range: NSRange) -> Option<usize> {
    range.location.checked_add(range.length)
}

pub(super) fn slice_utf16(text: &str, range: NSRange) -> Option<&str> {
    let requested_end = end(range)?;
    let mut units = 0;
    let mut start = None;
    let mut finish = None;
    for (byte, character) in text.char_indices() {
        if units == range.location {
            start = Some(byte);
        }
        if units == requested_end {
            finish = Some(byte);
        }
        units += character.len_utf16();
    }
    if units == range.location {
        start = Some(text.len());
    }
    if units == requested_end {
        finish = Some(text.len());
    }
    text.get(start?..finish?)
}

impl SurfaceView {
    fn input_state(&self) -> Option<(AccessibilityRevision, usize, NSRange)> {
        if !self.ivars().input_active.get() {
            return None;
        }
        let state = NativeAccessibilityAdapter::input_metadata(self)?;
        if self.has_marked_text_value()
            && (!self.native_mark_is_current() || state.2 != self.ivars().marked_replacement.get())
        {
            return None;
        }
        Some(state)
    }

    pub(super) fn capture_native_mark_owner(&self) -> bool {
        let Some((document, buffer, node, _)) = NativeAccessibilityAdapter::input_owner(self)
        else {
            return false;
        };
        self.ivars().marked_owner.set(Some((
            document,
            buffer,
            node,
            self.ivars().input_epoch.get(),
        )));
        if let Some((_, _, selected)) = NativeAccessibilityAdapter::input_metadata(self) {
            self.ivars().marked_replacement.set(selected);
        }
        true
    }

    pub(super) fn native_marked_range(&self) -> NSRange {
        if self.input_state().is_none() || !self.has_marked_text_value() {
            return missing_range();
        }
        NSRange::new(
            self.ivars().marked_replacement.get().location,
            self.ivars().marked_text.borrow().encode_utf16().count(),
        )
    }

    pub(super) fn native_selected_range(&self) -> NSRange {
        let Some((_, _, selected)) = self.input_state() else {
            return missing_range();
        };
        if !self.has_marked_text_value() {
            return selected;
        }
        let mark = self.native_marked_range();
        let relative = self.ivars().marked_selection.get();
        if end(relative).is_none_or(|value| value > mark.length) {
            return missing_range();
        }
        mark.location
            .checked_add(relative.location)
            .map_or_else(missing_range, |start| NSRange::new(start, relative.length))
    }

    fn projected_length(&self, document_length: usize) -> Option<usize> {
        if !self.has_marked_text_value() {
            return Some(document_length);
        }
        document_length
            .checked_sub(self.ivars().marked_replacement.get().length)?
            .checked_add(self.ivars().marked_text.borrow().encode_utf16().count())
    }

    fn projected_text(
        &self,
        revision: AccessibilityRevision,
        requested: NSRange,
    ) -> Option<Box<str>> {
        if !self.has_marked_text_value() {
            return NativeAccessibilityAdapter::input_text(self, revision, requested);
        }
        let replacement = self.ivars().marked_replacement.get();
        let text = self.ivars().marked_text.try_borrow().ok()?.clone();
        let mark_end = replacement
            .location
            .checked_add(text.encode_utf16().count())?;
        let requested_end = end(requested)?;
        for position in [requested.location, requested_end] {
            if position >= replacement.location && position <= mark_end {
                slice_utf16(&text, NSRange::new(position - replacement.location, 0))?;
            }
        }
        let mut output = String::new();
        // Each fragment is bounded before materialization. No complete document
        // is retained or copied for the native virtual-text projection.
        output
            .try_reserve(
                MAX_ACCESSIBILITY_TEXT_RESPONSE_BYTES.min(requested.length.checked_mul(4)?),
            )
            .ok()?;
        if requested.location < replacement.location {
            let prefix_end = requested_end.min(replacement.location);
            output.push_str(&NativeAccessibilityAdapter::input_text(
                self,
                revision,
                NSRange::new(requested.location, prefix_end - requested.location),
            )?);
        }
        let mark_start = requested.location.max(replacement.location);
        let fragment_end = requested_end.min(mark_end);
        if fragment_end > mark_start {
            output.push_str(slice_utf16(
                &text,
                NSRange::new(mark_start - replacement.location, fragment_end - mark_start),
            )?);
        }
        if requested_end > mark_end {
            let suffix_start = requested.location.max(mark_end);
            let document_start = end(replacement)?.checked_add(suffix_start - mark_end)?;
            output.push_str(&NativeAccessibilityAdapter::input_text(
                self,
                revision,
                NSRange::new(document_start, requested_end - suffix_start),
            )?);
        }
        (output.len() <= MAX_ACCESSIBILITY_TEXT_RESPONSE_BYTES).then(|| output.into_boxed_str())
    }

    pub(super) fn native_substring(&self, proposed: NSRange) -> Option<(Box<str>, NSRange)> {
        let (revision, document_length, _) = self.input_state()?;
        let length = self.projected_length(document_length)?;
        if proposed.location > length {
            return None;
        }
        let end = end(proposed)?.min(length).min(
            proposed
                .location
                .checked_add(MAX_ACCESSIBILITY_TEXT_RESPONSE_BYTES / 4 - 2)?,
        );
        if proposed.length != 0 && end == proposed.location {
            return None;
        }
        // AppKit may propose a range splitting a surrogate pair. Expand at most
        // one UTF-16 unit at either endpoint and return the actual represented range.
        for start in [proposed.location, proposed.location.saturating_sub(1)] {
            for finish in [end, end.saturating_add(1).min(length)] {
                let actual = NSRange::new(start, finish.checked_sub(start)?);
                if let Some(text) = self.projected_text(revision, actual) {
                    return Some((text, actual));
                }
            }
        }
        None
    }

    pub(super) fn native_first_rect(&self, proposed: NSRange) -> Option<(NSRect, NSRange)> {
        let (revision, document_length, _) = self.input_state()?;
        let length = self.projected_length(document_length)?;
        if proposed.location > length {
            return None;
        }
        let actual = NSRange::new(
            proposed.location,
            end(proposed)?.min(length) - proposed.location,
        );
        NativeAccessibilityAdapter::input_geometry(self, revision, actual, true)
    }

    pub(super) fn native_character_index(&self, point: NSPoint) -> Option<usize> {
        let (revision, _, _) = self.input_state()?;
        NativeAccessibilityAdapter::input_index(self, revision, point)
    }

    pub(super) fn native_mark_is_current(&self) -> bool {
        if !self.has_marked_text_value() {
            return true;
        }
        match (
            self.ivars().marked_owner.get(),
            NativeAccessibilityAdapter::input_owner(self),
        ) {
            (Some(owner), Some((document, buffer, node, composing))) => {
                owner == (document, buffer, node, self.ivars().input_epoch.get()) && composing
            }
            _ => false,
        }
    }

    pub(super) fn prepare_native_edit(
        &self,
        mut text: Box<str>,
        replacement: NSRange,
        mut selection: Option<&mut NSRange>,
    ) -> Option<Box<str>> {
        if !self.native_mark_is_current() {
            self.clear_marked_text();
            return None;
        }
        if replacement.location == missing_range().location {
            return (replacement.length == 0).then_some(text);
        }
        let (revision, document_length, _) = self.input_state()?;
        let length = self.projected_length(document_length)?;
        let replacement_end = end(replacement)?;
        if replacement_end > length {
            return None;
        }
        // Reject invalid UTF-16 boundaries before any document/selection mutation.
        self.projected_text(revision, replacement)?;
        let mut target = replacement;
        if self.has_marked_text_value() {
            let mark = self.native_marked_range();
            let mark_end = end(mark)?;
            let union_start = mark.location.min(replacement.location);
            let union_end = mark_end.max(replacement_end);
            let prefix = self.projected_text(
                revision,
                NSRange::new(union_start, replacement.location - union_start),
            )?;
            let suffix = self.projected_text(
                revision,
                NSRange::new(replacement_end, union_end - replacement_end),
            )?;
            let bytes = prefix
                .len()
                .checked_add(text.len())?
                .checked_add(suffix.len())?;
            if bytes > MAX_ACCESSIBILITY_TEXT_RESPONSE_BYTES {
                return None;
            }
            let mut combined = String::new();
            combined.try_reserve(bytes).ok()?;
            combined.push_str(&prefix);
            combined.push_str(&text);
            combined.push_str(&suffix);
            if let Some(selected) = selection.as_mut() {
                selected.location = selected
                    .location
                    .checked_add(prefix.encode_utf16().count())?;
            }
            text = combined.into_boxed_str();
            let document_end =
                end(self.ivars().marked_replacement.get())?.checked_add(union_end - mark_end)?;
            target = NSRange::new(union_start, document_end - union_start);
            self.clear_marked_text();
            self.emit_ime(ImeEvent::Cancelled);
        }
        let (revision, _, _) = self.input_state()?;
        NativeAccessibilityAdapter::input_selection(self, revision, target).then_some(text)
    }
}

#[cfg(alpine_native_validation)]
pub(super) fn validate_round_trip(
    view: &SurfaceView,
    reject_snapshot: &std::cell::Cell<bool>,
) -> Result<(), super::SurfaceError> {
    use super::{KeyState, Modifiers, NSString, NSTextInputClient, NativeInputEvent};
    let stage = std::cell::Cell::new("initial metadata");
    let failure = || {
        eprintln!(
            "native-text-round-trip: failed stage={} focus={:?}",
            stage.get(),
            view.input_focus_state()
        );
        super::SurfaceError::validation(super::SurfaceOperation::Input)
    };
    let (revision, length, selected) = view.input_state().ok_or_else(failure)?;
    stage.set("initial substring");
    let before = view
        .native_substring(NSRange::new(0, length.min(64)))
        .ok_or_else(failure)?
        .0;
    stage.set("initial selection");
    if length < 8
        || !NativeAccessibilityAdapter::input_selection(view, revision, NSRange::new(5, 2))
    {
        return Err(failure());
    }
    stage.set("mark");
    validate_mark_queries(view, length, &before)?;
    validate_edit_edges(view, length, &before)?;
    // A real provider becoming unavailable must never act like an unowned fixture.
    let before_rejection = view.input_state().ok_or_else(failure)?;
    reject_snapshot.set(true);
    validation_mark(view, &NSString::from_str("rejected"), NSRange::new(0, 0));
    validation_insert(view, "must not commit", missing_range());
    reject_snapshot.set(false);
    if view.input_state() != Some(before_rejection) {
        return Err(failure());
    }

    let marked = NSString::from_str("漢😀");
    let set_mark = |text: &NSString, selection: NSRange| validation_mark(view, text, selection);
    let undo = || {
        view.emit(NativeInputEvent::Keyboard {
            state: KeyState::Down,
            physical_key: 6,
            logical_key: "z".into(),
            modifiers: Modifiers::from_bits(Modifiers::COMMAND),
            repeat: false,
        });
    };
    for unmark in [false, true] {
        let (revision, _, _) = view.input_state().ok_or_else(failure)?;
        if !NativeAccessibilityAdapter::input_selection(view, revision, NSRange::new(5, 2)) {
            return Err(failure());
        }
        set_mark(&marked, NSRange::new(3, 0));
        if unmark {
            NSTextInputClient::unmarkText(view);
        } else {
            // SAFETY: Same retained NSString and synchronous protocol boundary as above.
            unsafe {
                NSTextInputClient::insertText_replacementRange(view, &marked, NSRange::new(5, 3));
            }
        }
        if view.has_marked_text_value()
            || view
                .native_substring(NSRange::new(5, 3))
                .is_none_or(|(text, _)| &*text != "漢😀")
        {
            eprintln!("native-text-round-trip: commit failed unmark={unmark}");
            return Err(failure());
        }
        undo();
        if view
            .native_substring(NSRange::new(0, length.min(64)))
            .is_none_or(|(text, _)| text != before)
        {
            eprintln!("native-text-round-trip: undo failed unmark={unmark}");
            return Err(failure());
        }
    }
    let (revision, _, _) = view.input_state().ok_or_else(failure)?;
    if !NativeAccessibilityAdapter::input_selection(view, revision, selected) {
        return Err(failure());
    }
    Ok(())
}

#[cfg(alpine_native_validation)]
fn validation_mark(view: &SurfaceView, text: &super::NSString, selection: NSRange) {
    // SAFETY: NSString is accepted by this protocol, and the retained receiver
    // and text remain live for the synchronous main-thread callback.
    unsafe {
        super::NSTextInputClient::setMarkedText_selectedRange_replacementRange(
            view,
            text,
            selection,
            missing_range(),
        );
    }
}

#[cfg(alpine_native_validation)]
fn validation_insert(view: &SurfaceView, text: &str, range: NSRange) {
    let text = super::NSString::from_str(text);
    // SAFETY: Same retained NSString and synchronous NSTextInputClient boundary.
    unsafe {
        super::NSTextInputClient::insertText_replacementRange(view, &text, range);
    }
}

#[cfg(alpine_native_validation)]
fn validation_key(view: &SurfaceView, physical_key: u16, logical_key: &str, modifiers: u8) {
    view.emit(super::NativeInputEvent::Keyboard {
        state: super::KeyState::Down,
        physical_key,
        logical_key: logical_key.into(),
        modifiers: super::Modifiers::from_bits(modifiers),
        repeat: false,
    });
}

#[cfg(alpine_native_validation)]
fn validate_mark_queries(
    view: &SurfaceView,
    length: usize,
    before: &str,
) -> Result<(), super::SurfaceError> {
    use super::NSString;
    let failure = || super::SurfaceError::validation(super::SurfaceOperation::Input);
    let marked = NSString::from_str("漢😀");
    let empty = NSString::from_str("");
    let set_mark = |text: &NSString, selection: NSRange| validation_mark(view, text, selection);
    set_mark(&marked, NSRange::new(3, 0));
    if view.native_marked_range() != NSRange::new(5, 3)
        || view.native_selected_range() != NSRange::new(8, 0)
        || view
            .native_substring(NSRange::new(5, 3))
            .is_none_or(|(text, actual)| &*text != "漢😀" || actual != NSRange::new(5, 3))
        || view
            .native_first_rect(NSRange::new(8, 0))
            .is_none_or(|(rect, actual)| {
                rect.size.width != 0.0 || rect.size.height <= 0.0 || actual != NSRange::new(8, 0)
            })
    {
        eprintln!("native-text-round-trip: marked projection failed");
        return Err(failure());
    }
    let (glyph_rect, _) = view
        .native_first_rect(NSRange::new(5, 1))
        .ok_or_else(failure)?;
    let hit = super::NSPoint::new(
        glyph_rect.origin.x + glyph_rect.size.width * 0.5,
        glyph_rect.origin.y + glyph_rect.size.height * 0.5,
    );
    if view.native_character_index(hit) != Some(5) {
        eprintln!("native-text-round-trip: marked glyph hit failed");
        return Err(failure());
    }
    // A surrogate-interior selection must not replace the valid current mark.
    set_mark(&marked, NSRange::new(2, 0));
    if view.native_selected_range() != NSRange::new(8, 0) {
        return Err(failure());
    }
    set_mark(&empty, NSRange::new(0, 0));
    if view.has_marked_text_value()
        || view
            .native_substring(NSRange::new(0, length.min(64)))
            .is_none_or(|(text, _)| &*text != before)
    {
        eprintln!("native-text-round-trip: cancellation mutated text");
        return Err(failure());
    }

    Ok(())
}

#[cfg(alpine_native_validation)]
fn validate_edit_edges(
    view: &SurfaceView,
    length: usize,
    before: &str,
) -> Result<(), super::SurfaceError> {
    let failure = || super::SurfaceError::validation(super::SurfaceOperation::Input);
    let select = |range| {
        view.input_state().is_some_and(|(revision, _, _)| {
            NativeAccessibilityAdapter::input_selection(view, revision, range)
        })
    };
    let mark = |text| validation_mark(view, &super::NSString::from_str(text), NSRange::new(0, 0));
    let unchanged = || {
        NativeAccessibilityAdapter::input_metadata(view)
            .and_then(|(revision, _, _)| {
                NativeAccessibilityAdapter::input_text(
                    view,
                    revision,
                    NSRange::new(0, length.min(64)),
                )
            })
            .is_some_and(|text| &*text == before)
    };
    if !select(NSRange::new(5, 2)) {
        return Err(failure());
    }
    mark("XYZ");
    validation_insert(view, "!", NSRange::new(6, 1));
    if view
        .native_substring(NSRange::new(5, 3))
        .is_none_or(|(text, _)| &*text != "X!Z")
        || view.native_selected_range() != NSRange::new(7, 0)
    {
        return Err(failure());
    }
    validation_key(view, 6, "z", super::Modifiers::COMMAND);
    if !unchanged() {
        return Err(failure());
    }
    validation_key(
        view,
        6,
        "z",
        super::Modifiers::COMMAND | super::Modifiers::SHIFT,
    );
    if view.native_selected_range() != NSRange::new(7, 0)
        || view
            .native_substring(NSRange::new(5, 3))
            .is_none_or(|(text, _)| &*text != "X!Z")
    {
        return Err(failure());
    }
    validation_key(view, 6, "z", super::Modifiers::COMMAND);
    if !unchanged() || !select(NSRange::new(5, 2)) {
        return Err(failure());
    }
    mark("漢😀");
    let old = view.input_state().ok_or_else(failure)?;
    for invalid in [
        NSRange::new(7, 0),
        NSRange::new(7, 1),
        NSRange::new(usize::MAX, 0),
    ] {
        validation_insert(view, "bad", invalid);
        if view.input_state() != Some(old) || !unchanged() {
            return Err(failure());
        }
    }
    validation_insert(view, "", NSRange::new(5, 3));
    if view
        .input_state()
        .is_none_or(|(_, size, selected)| size != length - 2 || selected != NSRange::new(5, 0))
    {
        return Err(failure());
    }
    validation_key(view, 6, "z", super::Modifiers::COMMAND);
    if !unchanged() || !select(NSRange::new(5, 2)) {
        return Err(failure());
    }
    mark("old mark");
    let (revision, _, _) = view.input_state().ok_or_else(failure)?;
    if !NativeAccessibilityAdapter::input_selection(view, revision, NSRange::new(0, 0)) {
        return Err(failure());
    }
    validation_insert(view, "stale", missing_range());
    if !unchanged() {
        return Err(failure());
    }
    validation_key(view, 3, "f", super::Modifiers::COMMAND);
    mark("overlay");
    validation_key(view, 53, "", 0);
    validation_insert(view, "stale overlay", missing_range());
    if !unchanged() {
        return Err(failure());
    }
    Ok(())
}
